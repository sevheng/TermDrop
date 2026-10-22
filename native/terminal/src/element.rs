//! Painting the grid.
//!
//! Each frame takes one short lock on the terminal, walks the visible cells
//! once, and turns them into two lists: background rectangles (adjacent cells
//! of one colour merged into a single quad) and text batches (adjacent cells
//! of one style shaped as one line). Shaping goes through GPUI's line cache,
//! so a row that did not change is not reshaped.
//!
//! Every batch is shaped with `force_width` set to the cell width, which puts
//! each glyph on its cell even when it came from a fallback font with another
//! advance. That only works when one glyph is one cell, so blanks end a batch
//! (they need no glyph) and a double-width character is a batch of its own.

use std::time::Instant;

use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::Point as GridPoint;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::vte::ansi::{Color, CursorShape, NamedColor};
use gpui_kit::*;

use crate::palette::{dim, hsla, rgb};
use crate::view::{CellMetrics, TerminalView};

pub(crate) struct TerminalElement {
    view: Entity<TerminalView>,
}

impl TerminalElement {
    pub(crate) fn new(view: Entity<TerminalView>) -> Self {
        Self { view }
    }
}

impl IntoElement for TerminalElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

/// The style a batch of glyphs shares.
#[derive(Clone, Copy, PartialEq)]
struct GlyphStyle {
    fg: Hsla,
    bold: bool,
    italic: bool,
    underline: bool,
    strikeout: bool,
}

struct Batch {
    row: usize,
    col: usize,
    /// The column after the last cell in the batch.
    end: usize,
    text: String,
    style: GlyphStyle,
}

struct CursorLayout {
    shape: CursorShape,
    bounds: Bounds<Pixels>,
    color: Hsla,
    /// The character under a block cursor, redrawn in the background colour.
    text: Option<ShapedLine>,
}

pub(crate) struct LayoutState {
    background: Hsla,
    rects: Vec<(Bounds<Pixels>, Hsla)>,
    lines: Vec<(Point<Pixels>, ShapedLine)>,
    cursor: Option<CursorLayout>,
    link_underline: Option<(Bounds<Pixels>, Hsla)>,
    preedit: Option<(Bounds<Pixels>, ShapedLine)>,
    started: Instant,
    bytes_shown: u64,
}

/// Background merging state for one row.
struct RectRun {
    row: usize,
    start: usize,
    end: usize,
    color: Hsla,
}

fn cell_bounds(
    origin: Point<Pixels>,
    m: CellMetrics,
    col: usize,
    row: usize,
    cols: usize,
) -> Bounds<Pixels> {
    Bounds::new(
        point(
            origin.x + m.width * col as f32,
            origin.y + m.height * row as f32,
        ),
        size(m.width * cols as f32, m.height),
    )
}

impl Element for TerminalElement {
    type RequestLayoutState = ();
    type PrepaintState = LayoutState;

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
        let started = Instant::now();
        let focused = self.view.read(cx).focus_handle.is_focused(window);
        self.view.update(cx, |view, _| {
            view.prepaint_grid(bounds, focused, started, window)
        })
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        layout: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus_handle = self.view.read(cx).focus_handle.clone();
        window.handle_input(
            &focus_handle,
            ElementInputHandler::new(bounds, self.view.clone()),
            cx,
        );

        let height = self
            .view
            .read(cx)
            .metrics
            .map(|m| m.height)
            .unwrap_or(px(16.0));

        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            window.paint_quad(fill(bounds, layout.background));
            for (rect, color) in layout.rects.drain(..) {
                window.paint_quad(fill(rect, color));
            }
            for (origin, line) in &layout.lines {
                let _ = line.paint(*origin, height, TextAlign::Left, None, window, cx);
            }
            if let Some((rect, color)) = layout.link_underline {
                window.paint_quad(fill(rect, color));
            }
            if let Some(cursor) = &layout.cursor {
                paint_cursor(cursor, height, window, cx);
            }
            if let Some((rect, line)) = &layout.preedit {
                window.paint_quad(fill(*rect, layout.background));
                let _ = line.paint(rect.origin, height, TextAlign::Left, None, window, cx);
            }
        });

        self.view
            .read(cx)
            .shared
            .stats
            .record_paint(layout.started, layout.bytes_shown);
    }
}

fn paint_cursor(cursor: &CursorLayout, line_height: Pixels, window: &mut Window, cx: &mut App) {
    let b = cursor.bounds;
    let thickness = px(2.0);
    match cursor.shape {
        CursorShape::Block => {
            window.paint_quad(fill(b, cursor.color));
            if let Some(text) = &cursor.text {
                let _ = text.paint(b.origin, line_height, TextAlign::Left, None, window, cx);
            }
        }
        CursorShape::Beam => {
            window.paint_quad(fill(
                Bounds::new(b.origin, size(thickness, b.size.height)),
                cursor.color,
            ));
        }
        CursorShape::Underline => {
            window.paint_quad(fill(
                Bounds::new(
                    point(b.origin.x, b.origin.y + b.size.height - thickness),
                    size(b.size.width, thickness),
                ),
                cursor.color,
            ));
        }
        CursorShape::HollowBlock => {
            window.paint_quad(outline(b, cursor.color, BorderStyle::Solid));
        }
        CursorShape::Hidden => {}
    }
}

impl TerminalView {
    /// Builds the frame. Runs inside prepaint so the grid size follows the
    /// element's bounds before anything is laid out against it.
    fn prepaint_grid(
        &mut self,
        bounds: Bounds<Pixels>,
        focused: bool,
        started: Instant,
        window: &mut Window,
    ) -> LayoutState {
        let m = self.layout_grid(bounds, window);
        let palette = self.options.palette.clone();
        let origin = bounds.origin;

        // Lower the flag before the snapshot: output that lands after this
        // point wakes the view again, so nothing is left unpainted.
        self.shared.clear_dirty();
        let state = self.shared.state.lock();
        let term = &state.term;
        let content = term.renderable_content();
        // A copy, so the lock can be released before the cursor is built.
        let colors = *content.colors;
        let display_offset = content.display_offset;
        let selection = content.selection;
        let cursor = content.cursor;
        let screen_lines = term.screen_lines();
        let bytes_shown = self.feed().stats().bytes_fed;

        let default_bg = rgb(palette.background);
        let background = hsla(colors[NamedColor::Background].unwrap_or(default_bg));
        let selection_bg = hsla(rgb(palette.selection_background));

        let mut rects: Vec<(Bounds<Pixels>, Hsla)> = Vec::new();
        let mut batches: Vec<Batch> = Vec::new();
        let mut rect_run: Option<RectRun> = None;
        let mut batch: Option<Batch> = None;
        let mut cursor_glyph: Option<(char, GlyphStyle, Hsla, bool)> = None;

        let flush_rect = |run: &mut Option<RectRun>, rects: &mut Vec<(Bounds<Pixels>, Hsla)>| {
            if let Some(r) = run.take() {
                rects.push((
                    cell_bounds(origin, m, r.start, r.row, r.end - r.start),
                    r.color,
                ));
            }
        };

        let cursor_row = cursor.point.line.0 + display_offset as i32;
        for indexed in content.display_iter {
            let cell = indexed.cell;
            let GridPoint { line, column } = indexed.point;
            let row = (line.0 + display_offset as i32) as usize;
            let col = column.0;
            if row >= screen_lines {
                continue;
            }

            let flags = cell.flags;
            let mut fg = cell.fg;
            // Bold text in the eight normal colours is drawn bright, like
            // xterm.js's drawBoldTextInBrightColors default.
            if let Color::Named(n) = fg {
                if flags.contains(Flags::BOLD) && (n as usize) < 8 {
                    fg = Color::Named(n.to_bright());
                }
            }
            let mut fg_rgb = palette.resolve(fg, &colors);
            let mut bg_rgb = palette.resolve(cell.bg, &colors);
            if flags.contains(Flags::DIM) {
                fg_rgb = dim(fg_rgb);
            }
            if flags.contains(Flags::INVERSE) {
                std::mem::swap(&mut fg_rgb, &mut bg_rgb);
            }
            let fg_hsla = hsla(fg_rgb);
            let selected = selection.is_some_and(|s| s.contains(indexed.point));
            let bg = if selected {
                Some(selection_bg)
            } else if cell.bg != Color::Named(NamedColor::Background)
                || flags.contains(Flags::INVERSE)
            {
                Some(hsla(bg_rgb))
            } else {
                None
            };

            // Background: extend the current run or start a new one.
            match (&mut rect_run, bg) {
                (Some(r), Some(color)) if r.row == row && r.end == col && r.color == color => {
                    r.end = col + 1
                }
                (_, Some(color)) => {
                    flush_rect(&mut rect_run, &mut rects);
                    rect_run = Some(RectRun {
                        row,
                        start: col,
                        end: col + 1,
                        color,
                    });
                }
                (_, None) => flush_rect(&mut rect_run, &mut rects),
            }

            if flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER) {
                continue;
            }

            let style = GlyphStyle {
                fg: fg_hsla,
                bold: flags.contains(Flags::BOLD),
                italic: flags.contains(Flags::ITALIC),
                underline: flags.intersects(Flags::ALL_UNDERLINES),
                strikeout: flags.contains(Flags::STRIKEOUT),
            };
            let wide = flags.contains(Flags::WIDE_CHAR);
            if line.0 == cursor.point.line.0 && column == cursor.point.column {
                cursor_glyph = Some((cell.c, style, hsla(bg_rgb), wide));
            }

            let blank = (cell.c == ' ' || cell.c == '\t' || flags.contains(Flags::HIDDEN))
                && !style.underline
                && !style.strikeout;
            let continues = batch
                .as_ref()
                .is_some_and(|b| b.row == row && b.style == style && b.end == col);
            if blank || wide || !continues {
                if let Some(b) = batch.take() {
                    batches.push(b);
                }
            }
            if blank {
                continue;
            }
            let mut text = String::new();
            text.push(if flags.contains(Flags::HIDDEN) {
                ' '
            } else {
                cell.c
            });
            if let Some(extra) = cell.zerowidth() {
                text.extend(extra.iter());
            }
            match &mut batch {
                Some(b) if continues && !wide => {
                    b.text.push_str(&text);
                    b.end = col + 1;
                }
                _ => {
                    batch = Some(Batch {
                        row,
                        col,
                        end: col + 1,
                        text,
                        style,
                    })
                }
            }
            if wide {
                batches.extend(batch.take());
            }
        }
        flush_rect(&mut rect_run, &mut rects);
        if let Some(b) = batch.take() {
            batches.push(b);
        }
        drop(state);

        // Shape outside the lock: the producer can keep parsing meanwhile.
        let ts = window.text_system().clone();
        let font_size = self.options.font_size;
        let shape = |text: String, style: &GlyphStyle, color: Hsla| -> ShapedLine {
            let font = Font {
                weight: if style.bold {
                    FontWeight::BOLD
                } else {
                    FontWeight::NORMAL
                },
                style: if style.italic {
                    FontStyle::Italic
                } else {
                    FontStyle::Normal
                },
                ..self.font.clone()
            };
            let run = TextRun {
                len: text.len(),
                font,
                color,
                background_color: None,
                underline: style.underline.then_some(UnderlineStyle {
                    thickness: px(1.0),
                    color: Some(color),
                    wavy: false,
                }),
                strikethrough: style.strikeout.then_some(StrikethroughStyle {
                    thickness: px(1.0),
                    color: Some(color),
                }),
            };
            ts.shape_line(text.into(), font_size, &[run], Some(m.width))
        };

        let lines = batches
            .into_iter()
            .map(|b| {
                let origin = point(
                    origin.x + m.width * b.col as f32,
                    origin.y + m.height * b.row as f32,
                );
                (origin, shape(b.text, &b.style, b.style.fg))
            })
            .collect();

        // The cursor. Unfocused terminals show a hollow block, as xterm does.
        let cursor_visible = cursor.shape != CursorShape::Hidden
            && cursor_row >= 0
            && (cursor_row as usize) < screen_lines;
        self.cursor_cell = cursor_visible.then_some((cursor.point.column.0, cursor_row as usize));
        let cursor_color = hsla(colors[NamedColor::Cursor].unwrap_or(rgb(palette.cursor)));
        let cursor_layout = if cursor_visible && self.marked_text.is_none() {
            let wide = cursor_glyph.is_some_and(|g| g.3);
            let shape_kind = if focused {
                cursor.shape
            } else {
                CursorShape::HollowBlock
            };
            let bounds = cell_bounds(
                origin,
                m,
                cursor.point.column.0,
                cursor_row as usize,
                if wide { 2 } else { 1 },
            );
            let text = match (shape_kind, cursor_glyph) {
                (CursorShape::Block, Some((c, style, bg, _))) if c != ' ' => {
                    Some(shape(c.to_string(), &style, bg))
                }
                _ => None,
            };
            Some(CursorLayout {
                shape: shape_kind,
                bounds,
                color: cursor_color,
                text,
            })
        } else {
            None
        };

        let link_underline = self.hovered_url.as_ref().map(|hit| {
            let b = cell_bounds(origin, m, hit.cols.start, hit.row, hit.cols.len());
            (
                Bounds::new(
                    point(b.origin.x, b.origin.y + b.size.height - px(1.0)),
                    size(b.size.width, px(1.0)),
                ),
                hsla(rgb(palette.foreground)),
            )
        });

        let preedit = match (&self.marked_text, self.cursor_cell) {
            (Some(text), Some((col, row))) => {
                let style = GlyphStyle {
                    fg: hsla(rgb(palette.foreground)),
                    bold: false,
                    italic: false,
                    underline: true,
                    strikeout: false,
                };
                let width = unicode_width::UnicodeWidthStr::width(text.as_str()).max(1);
                let line = ts.shape_line(
                    text.clone().into(),
                    font_size,
                    &[TextRun {
                        len: text.len(),
                        font: self.font.clone(),
                        color: style.fg,
                        background_color: None,
                        underline: Some(UnderlineStyle {
                            thickness: px(1.0),
                            color: Some(style.fg),
                            wavy: false,
                        }),
                        strikethrough: None,
                    }],
                    None,
                );
                Some((cell_bounds(origin, m, col, row, width), line))
            }
            _ => None,
        };

        LayoutState {
            background,
            rects,
            lines,
            cursor: cursor_layout,
            link_underline,
            preedit,
            started,
            bytes_shown,
        }
    }
}
