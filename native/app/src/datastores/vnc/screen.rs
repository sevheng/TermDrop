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
}

pub(super) fn geometry(
    viewport: Bounds<Pixels>,
    fb: (u16, u16),
    fit: FitMode,
    window: &Window,
) -> Geometry {
    let (vw, vh) = (
        f32::from(viewport.size.width),
        f32::from(viewport.size.height),
    );
    let l = layout(fb.0 as f32, fb.1 as f32, vw, vh, fit, window.scale_factor());
    let (ox, oy) = match fit {
        FitMode::Fit => ((vw - l.width) / 2.0, (vh - l.height) / 2.0),
        FitMode::Actual => {
            let m = window.mouse_position();
            let fx = f32::from(m.x - viewport.origin.x) / vw.max(1.0);
            let fy = f32::from(m.y - viewport.origin.y) / vh.max(1.0);
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
    }
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
        let geometry = geometry(bounds, view.size, view.fit, window);
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
                if phase != DispatchPhase::Bubble || !hitbox.is_hovered(window) {
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
                let hovered = hitbox.is_hovered(window);
                view.update(cx, |v, cx| v.mouse_move(e.position, hovered, geo, cx));
            }
        });
        window.on_mouse_event(move |e: &ScrollWheelEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble || !hitbox.is_hovered(window) {
                return;
            }
            view.update(cx, |v, _| v.wheel(e, geo));
            cx.stop_propagation();
        });
    }
}
