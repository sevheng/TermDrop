//! The element that draws the remote screen and takes its pointer.
//!
//! It paints the tab's tile images ([`super::renderer`]) scaled into place,
//! one `paint_image` per tile, with every tile edge rounded to a device
//! pixel so neighbouring tiles meet without a seam. It also registers the
//! input handler (for the input method) and window-level mouse listeners:
//! window-level so that a drag which leaves the image keeps going to the
//! remote until the button is released, as the Vue canvas's pointer capture
//! did.

use gpui_kit::*;

use super::VncTab;
use super::framebuffer::tile_rect;
use super::layout::{FitMode, actual_offset, layout};

pub(super) struct VncScreen {
    view: Entity<VncTab>,
}

impl VncScreen {
    pub(super) fn new(view: Entity<VncTab>) -> Self {
        Self { view }
    }
}

impl IntoElement for VncScreen {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

/// Where the framebuffer is drawn this frame.
#[derive(Debug, Clone, Copy)]
pub(super) struct Geometry {
    pub viewport: Bounds<Pixels>,
    /// The image's rectangle; may be larger than the viewport at 1:1.
    pub image: Bounds<Pixels>,
    pub fb: (u16, u16),
    fit: FitMode,
    scale_factor: f32,
}

impl Geometry {
    /// The framebuffer pixel under a window position, clamped to the screen
    /// (a drag past the edge pins to it, as the browser's did).
    pub fn framebuffer_pixel(self, position: Point<Pixels>) -> (u16, u16) {
        super::layout::pointer_to_framebuffer(
            (
                f32::from(self.image.origin.x),
                f32::from(self.image.origin.y),
            ),
            (
                f32::from(self.image.size.width),
                f32::from(self.image.size.height),
            ),
            self.fb,
            (f32::from(position.x), f32::from(position.y)),
        )
    }

    /// Whether the image overflows the viewport, so it pans with the pointer.
    pub fn pans(&self) -> bool {
        self.image.size.width > self.viewport.size.width
            || self.image.size.height > self.viewport.size.height
    }

    /// This geometry with the pointer at `pointer`: where the image will be
    /// drawn once the pointer is there. At 1:1 on a desktop larger than the
    /// viewport the image pans with the pointer, so an event must be mapped
    /// through the pan for its own position, not the one the last paint
    /// read; otherwise every move lands off by the distance moved since.
    pub fn at(self, pointer: Point<Pixels>) -> Self {
        match self.fit {
            FitMode::Fit => self,
            FitMode::Actual => {
                geometry_at(self.viewport, self.fb, self.fit, self.scale_factor, pointer)
            }
        }
    }

    /// Whether a press or wheel at `pointer` belongs to the remote: on the
    /// image, not in the letterbox around it. The Vue canvas was only the
    /// image's size, so the bars took no input; clamping a click in a bar
    /// to the edge pixel would press whatever sits on the remote's edge.
    pub fn takes(self, pointer: Point<Pixels>) -> bool {
        self.viewport.contains(&pointer) && self.image.contains(&pointer)
    }
}

/// The geometry for a pointer at `pointer` (only 1:1 panning reads it).
pub(super) fn geometry_at(
    viewport: Bounds<Pixels>,
    fb: (u16, u16),
    fit: FitMode,
    scale_factor: f32,
    pointer: Point<Pixels>,
) -> Geometry {
    let (vw, vh) = (
        f32::from(viewport.size.width),
        f32::from(viewport.size.height),
    );
    let l = layout(fb.0 as f32, fb.1 as f32, vw, vh, fit, scale_factor);
    let (ox, oy) = match fit {
        FitMode::Fit => ((vw - l.width) / 2.0, (vh - l.height) / 2.0),
        FitMode::Actual => {
            let fx = f32::from(pointer.x - viewport.origin.x) / vw.max(1.0);
            let fy = f32::from(pointer.y - viewport.origin.y) / vh.max(1.0);
            (
                actual_offset(l.width, vw, fx),
                actual_offset(l.height, vh, fy),
            )
        }
    };
    Geometry {
        viewport,
        image: Bounds::new(
            point(viewport.origin.x + px(ox), viewport.origin.y + px(oy)),
            size(px(l.width), px(l.height)),
        ),
        fb,
        fit,
        scale_factor,
    }
}

pub(super) fn geometry(
    viewport: Bounds<Pixels>,
    fb: (u16, u16),
    fit: FitMode,
    window: &Window,
) -> Geometry {
    geometry_at(
        viewport,
        fb,
        fit,
        window.scale_factor(),
        window.mouse_position(),
    )
}

pub(super) struct Prepaint {
    geometry: Geometry,
    hitbox: Hitbox,
}

impl Element for VncScreen {
    type RequestLayoutState = ();
    type PrepaintState = Prepaint;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let style = Style {
            size: Size {
                width: relative(1.).into(),
                height: relative(1.).into(),
            },
            ..Style::default()
        };
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let view = self.view.read(cx);
        // The size of the framebuffer the tiles were built for, not the
        // latest resize event's: until the frame that rebuilds the grid has
        // been applied, the old tiles must keep their old layout.
        let geometry = geometry(bounds, view.drawn_size, view.fit, window);
        let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
        Prepaint { geometry, hitbox }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let geo = prepaint.geometry;
        let focus = self.view.read(cx).focus.clone();
        window.handle_input(
            &focus,
            ElementInputHandler::new(geo.image, self.view.clone()),
            cx,
        );
        window.set_cursor_style(CursorStyle::Arrow, &prepaint.hitbox);

        let sf = window.scale_factor();
        let snap = |v: f32| (v * sf).round() / sf;
        let view = self.view.read(cx);
        let (fw, fh) = (geo.fb.0 as usize, geo.fb.1 as usize);
        let (cols, _) = view.grid;
        let scale_x = f32::from(geo.image.size.width) / fw.max(1) as f32;
        let scale_y = f32::from(geo.image.size.height) / fh.max(1) as f32;
        let (ix, iy) = (f32::from(geo.image.origin.x), f32::from(geo.image.origin.y));
        let accent = view.accent;
        let focused = focus.is_focused(window);
        let tiles = view.tiles.clone();

        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            // The canvas started black before the first frame drew.
            window.paint_quad(fill(geo.image, black()));
            for (index, tile) in tiles.iter().enumerate() {
                let Some(image) = tile else { continue };
                let (x, y, w, h) = tile_rect(fw, fh, cols, index);
                let left = snap(ix + x as f32 * scale_x);
                let top = snap(iy + y as f32 * scale_y);
                let right = snap(ix + (x + w) as f32 * scale_x);
                let bottom = snap(iy + (y + h) as f32 * scale_y);
                let tile_bounds = Bounds::new(
                    point(px(left), px(top)),
                    size(px(right - left), px(bottom - top)),
                );
                if let Err(e) = window.paint_image(
                    bounds,
                    tile_bounds,
                    Corners::default(),
                    image.clone(),
                    0,
                    false,
                ) {
                    tracing::warn!(target: "termdrop::vnc", "could not paint a screen tile: {e}");
                }
            }
            if focused {
                window.paint_quad(outline(geo.image, accent, BorderStyle::Solid));
            }
        });

        // Window-level, so a drag that leaves the image keeps reaching the
        // remote until its button comes up.
        let hitbox = prepaint.hitbox.clone();
        let view = self.view.clone();
        window.on_mouse_event({
            let (view, hitbox) = (view.clone(), hitbox.clone());
            move |e: &MouseDownEvent, phase, window, cx| {
                let geo = geo.at(e.position);
                if phase != DispatchPhase::Bubble
                    || !hitbox.is_hovered(window)
                    || !geo.takes(e.position)
                {
                    return;
                }
                view.update(cx, |v, cx| {
                    v.mouse_button(e.button, true, e.position, geo, window, cx)
                });
                cx.stop_propagation();
            }
        });
        window.on_mouse_event({
            let view = view.clone();
            move |e: &MouseUpEvent, phase, window, cx| {
                if phase != DispatchPhase::Bubble {
                    return;
                }
                let geo = geo.at(e.position);
                view.update(cx, |v, cx| {
                    v.mouse_button(e.button, false, e.position, geo, window, cx)
                });
            }
        });
        window.on_mouse_event({
            let (view, hitbox) = (view.clone(), hitbox.clone());
            move |e: &MouseMoveEvent, phase, window, cx| {
                if phase != DispatchPhase::Bubble {
                    return;
                }
                let geo = geo.at(e.position);
                let hovered = hitbox.is_hovered(window) && geo.takes(e.position);
                view.update(cx, |v, cx| v.mouse_move(e.position, hovered, geo, cx));
            }
        });
        window.on_mouse_event(move |e: &ScrollWheelEvent, phase, window, cx| {
            let geo = geo.at(e.position);
            if phase != DispatchPhase::Bubble
                || !hitbox.is_hovered(window)
                || !geo.takes(e.position)
            {
                return;
            }
            view.update(cx, |v, _| v.wheel(e, geo));
            cx.stop_propagation();
        });
    }
}

#[cfg(test)]
mod tests {
    // Not `super::*`: that brings in gpui's own `test` attribute.
    use super::{FitMode, geometry_at};
    use gpui_kit::{Bounds, Pixels, point, px, size};

    fn viewport(w: f32, h: f32) -> Bounds<Pixels> {
        Bounds::new(point(px(100.), px(50.)), size(px(w), px(h)))
    }

    #[test]
    fn a_pan_is_mapped_through_the_events_own_position() {
        // A 2560-wide desktop at 1:1 in a 1000px viewport pans by 1.56px per
        // pointer pixel.
        let vp = viewport(1000., 800.);
        let painted = geometry_at(
            vp,
            (2560, 800),
            FitMode::Actual,
            1.,
            point(px(300.), px(60.)),
        );
        let to = point(px(310.), px(60.));
        let fresh = geometry_at(vp, (2560, 800), FitMode::Actual, 1., to);
        assert_eq!(
            painted.at(to).framebuffer_pixel(to),
            fresh.framebuffer_pixel(to)
        );
        // What the stale geometry got wrong: 10px of motion, ~16px of pan.
        let stale = painted.framebuffer_pixel(to).0;
        assert!(fresh.framebuffer_pixel(to).0 - stale >= 15, "{stale}");
        // The far edge reaches the last remote pixel.
        let edge = point(px(1100.), px(60.));
        assert_eq!(painted.at(edge).framebuffer_pixel(edge).0, 2559);
    }

    #[test]
    fn fit_does_not_depend_on_the_pointer() {
        let vp = viewport(1000., 800.);
        let g = geometry_at(vp, (800, 600), FitMode::Fit, 1., point(px(0.), px(0.)));
        let to = point(px(900.), px(700.));
        assert_eq!(g.at(to).image, g.image);
    }

    #[test]
    fn the_letterbox_takes_no_input() {
        // A 4:3 desktop in a wide viewport: bars left and right.
        let vp = viewport(1600., 600.);
        let g = geometry_at(vp, (800, 600), FitMode::Fit, 1., point(px(0.), px(0.)));
        assert_eq!(f32::from(g.image.origin.x), 500.);
        assert!(!g.takes(point(px(150.), px(100.))), "left bar");
        assert!(!g.takes(point(px(1650.), px(60.))), "right bar");
        assert!(g.takes(point(px(600.), px(300.))), "the image");
        // 1:1 and smaller than the viewport: centred, bars all round.
        let g = geometry_at(vp, (400, 300), FitMode::Actual, 1., point(px(0.), px(0.)));
        assert!(!g.takes(point(px(110.), px(60.))));
        assert!(g.takes(point(px(900.), px(350.))));
        // 1:1 and larger: the image covers the viewport, and nothing
        // outside the viewport counts even though the image extends there.
        let g = geometry_at(vp, (4000, 3000), FitMode::Actual, 1., point(px(0.), px(0.)));
        let inside = point(px(800.), px(300.));
        assert!(g.at(inside).takes(inside));
        let outside = point(px(1800.), px(300.));
        assert!(!g.at(outside).takes(outside));
    }
}
