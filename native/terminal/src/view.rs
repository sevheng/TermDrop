//! `TerminalView`: the entity that owns a terminal, turns input into bytes,
//! and hands the grid to [`crate::element::TerminalElement`] to paint.

use std::ops::Range;
use std::sync::Arc;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use alacritty_terminal::event::{Event, WindowSize};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line, Point as GridPoint, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::TermMode;
use alacritty_terminal::vte::ansi::Rgb;
use gpui_kit::*;

use crate::element::TerminalElement;
use crate::feed::{GridSize, Msg, Shared, TerminalFeed};
use crate::keys::{self, KeyModes};
use crate::mouse::{mouse_report, ReportButton, ReportEncoding, ReportKind, ReportModifiers};
use crate::palette::Palette;
use crate::paste::{paste_chunks, paste_payload};
use crate::search::{self, ActiveSearch, SearchDirection};
use crate::url::{is_openable, url_at};

/// The terminal font: JetBrains Mono, as in the Tauri app.
pub const DEFAULT_FONT_FAMILY: &str = "JetBrains Mono";
/// Icon glyphs for prompts and file listings, tried after the main font.
pub const NERD_SYMBOLS_FAMILY: &str = "Symbols Nerd Font Mono";

/// Monospace families tried, in order, when the requested one is not
/// installed. GPUI panics on the first line laid out in a family it cannot
/// find, so the family is always checked before use. The app embeds and
/// registers JetBrains Mono and the Nerd Font symbols at startup
/// (`app/src/ui/fonts.rs`), so there this list is never reached; it is for
/// the crate on its own, such as the `local_shell` example.
const MONO_ALTERNATES: &[&str] = &[
    "JetBrains Mono",
    "SF Mono",
    "Menlo",
    "Cascadia Mono",
    "Consolas",
    "DejaVu Sans Mono",
    "Noto Sans Mono",
    "Liberation Mono",
    "Ubuntu Mono",
    "Courier New",
];

/// The key context the terminal's element declares.
pub const KEY_CONTEXT: &str = "Terminal";

/// Registers the terminal's key bindings. Call once at startup, after
/// `gpui_kit::init`.
///
/// Bindings resolve before an element sees the key, and GPUI Kit's root binds
/// Tab and Shift+Tab to focus traversal and Ctrl+C (Cmd+C on macOS) to copy.
/// In a terminal those keys belong to the program, so they are bound to
/// `NoAction` in the terminal's context, which out-ranks the root's binding
/// and lets the key fall through to the view. Application shortcuts the root
/// binds to other keys keep working while a terminal has focus.
pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("tab", NoAction {}, Some(KEY_CONTEXT)),
        KeyBinding::new("shift-tab", NoAction {}, Some(KEY_CONTEXT)),
        KeyBinding::new("ctrl-c", NoAction {}, Some(KEY_CONTEXT)),
        KeyBinding::new("cmd-c", NoAction {}, Some(KEY_CONTEXT)),
    ]);
}

/// How long to wait between paste chunks, as `TerminalTab.vue` does.
const PASTE_CHUNK_DELAY: Duration = Duration::from_millis(1);
/// Lines scrolled per wheel notch.
const WHEEL_LINES: f32 = 3.0;
/// Producer messages handled per UI update before yielding to paint.
const MAX_MSGS_PER_UPDATE: usize = 256;

/// Construction options for a terminal.
#[derive(Clone, Debug)]
pub struct TerminalOptions {
    /// Preferred family; replaced by an installed monospace font if missing.
    pub font_family: SharedString,
    pub font_size: Pixels,
    pub palette: Palette,
    /// Lines of scrollback history.
    pub scrollback: usize,
}

impl Default for TerminalOptions {
    fn default() -> Self {
        Self {
            font_family: DEFAULT_FONT_FAMILY.into(),
            font_size: px(14.0),
            palette: Palette::dark(),
            // xterm.js's default, which is what the Tauri app runs with.
            scrollback: 1000,
        }
    }
}

/// What a terminal reports to its owner.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TerminalEvent {
    /// OSC 0/2 set the title; an empty string means it was reset.
    TitleChanged(String),
    Bell,
    /// OSC 52: the program asked to put this text on the clipboard. The view
    /// does not write it itself, so the owner can decide (and tell the user).
    ClipboardStore(String),
    /// The output source ended ([`TerminalFeed::exit`]).
    Exited,
}

pub(crate) type InputFn = Arc<dyn Fn(Vec<u8>) + Send + Sync>;
pub(crate) type ResizeFn = Box<dyn Fn(u16, u16) + Send + Sync>;

/// Cell geometry for the current font.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct CellMetrics {
    pub(crate) width: Pixels,
    pub(crate) height: Pixels,
}

/// A link under the pointer while the open-link modifier is held, in viewport
/// cells.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct HoveredUrl {
    pub(crate) row: usize,
    pub(crate) cols: Range<usize>,
    pub(crate) url: String,
}

pub struct TerminalView {
    pub(crate) shared: Arc<Shared>,
    pub(crate) focus_handle: FocusHandle,
    pub(crate) options: TerminalOptions,
    pub(crate) font: Font,
    input: InputFn,
    on_resize: ResizeFn,
    /// The size last given to the terminal and to `on_resize`.
    grid: Option<(u16, u16)>,
    pub(crate) metrics: Option<CellMetrics>,
    /// Where the grid was painted last frame, for mouse hit-testing.
    pub(crate) bounds: Option<Bounds<Pixels>>,
    /// The cursor's viewport cell last frame, for the IME candidate window.
    pub(crate) cursor_cell: Option<(usize, usize)>,
    /// IME composition in progress, shown at the cursor until committed.
    pub(crate) marked_text: Option<String>,
    pub(crate) hovered_url: Option<HoveredUrl>,
    /// The find bar's query, while it has one: its on-screen matches are
    /// highlighted.
    pub(crate) search: Option<ActiveSearch>,
    selecting: bool,
    /// Fractional wheel lines carried to the next event.
    scroll_lines: f32,
    /// The last cell a motion report was sent for, so a move within one cell
    /// sends nothing.
    last_report_cell: Option<(usize, usize)>,
    copy_on_select: bool,
    alt_is_meta: bool,
    exited: bool,
    paste_task: Option<Task<()>>,
    _wake_task: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<TerminalEvent> for TerminalView {}

impl Focusable for TerminalView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

/// The installed families, listed once per process (it is slow on macOS).
fn installed_fonts(cx: &App) -> &'static [String] {
    static NAMES: OnceLock<Vec<String>> = OnceLock::new();
    NAMES.get_or_init(|| cx.text_system().all_font_names())
}

/// `requested` if installed, else the first installed monospace alternate,
/// else GPUI's always-present UI font (the forced cell width keeps it on the
/// grid even though it is proportional).
pub(crate) fn resolve_family(requested: &str, installed: &[String]) -> SharedString {
    std::iter::once(requested)
        .chain(MONO_ALTERNATES.iter().copied())
        .find(|f| installed.iter().any(|name| name == f))
        .unwrap_or(".SystemUIFont")
        .to_string()
        .into()
}

fn terminal_font(family: SharedString) -> Font {
    Font {
        family,
        // Ligatures would join cells; they are deferred (plan, Phase 3).
        features: FontFeatures::disable_ligatures(),
        fallbacks: Some(FontFallbacks::from_fonts(vec![
            NERD_SYMBOLS_FAMILY.to_string()
        ])),
        weight: FontWeight::NORMAL,
        style: FontStyle::Normal,
    }
}

impl TerminalView {
    /// Creates a terminal. `input` receives everything the user (or the
    /// terminal itself, e.g. a cursor-position reply) sends to the program;
    /// `on_resize` receives the grid size whenever layout changes it,
    /// including once at the first layout.
    pub fn new(
        window: &mut Window,
        cx: &mut App,
        options: TerminalOptions,
        input: Box<dyn Fn(Vec<u8>) + Send + Sync>,
        on_resize: Box<dyn Fn(u16, u16) + Send + Sync>,
    ) -> Entity<Self> {
        let family = resolve_family(&options.font_family, installed_fonts(cx));
        cx.new(|cx| {
            let (tx, rx) = async_channel::unbounded();
            let shared = Shared::new(options.scrollback, tx);
            let focus_handle = cx.focus_handle();

            let wake_task = cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
                while let Ok(first) = rx.recv().await {
                    let mut batch = vec![first];
                    while batch.len() < MAX_MSGS_PER_UPDATE {
                        let Ok(more) = rx.try_recv() else { break };
                        batch.push(more);
                    }
                    let full = batch.len() == MAX_MSGS_PER_UPDATE;
                    if this
                        .update(cx, |view, cx| view.handle_msgs(batch, cx))
                        .is_err()
                    {
                        break;
                    }
                    if full {
                        // A program flooding queries (`yes $'\e[c'`) would
                        // otherwise hold the UI thread for the whole queue.
                        cx.background_executor().timer(Duration::ZERO).await;
                    }
                }
            });

            let subscriptions = vec![
                cx.on_focus(&focus_handle, window, |view, _, cx| {
                    view.focus_changed(true, cx)
                }),
                cx.on_blur(&focus_handle, window, |view, _, cx| {
                    view.focus_changed(false, cx)
                }),
            ];

            Self {
                shared,
                focus_handle,
                font: terminal_font(family),
                options,
                input: Arc::from(input),
                on_resize,
                grid: None,
                metrics: None,
                bounds: None,
                cursor_cell: None,
                marked_text: None,
                hovered_url: None,
                search: None,
                selecting: false,
                scroll_lines: 0.0,
                last_report_cell: None,
                copy_on_select: false,
                alt_is_meta: !cfg!(target_os = "macos"),
                exited: false,
                paste_task: None,
                _wake_task: wake_task,
                _subscriptions: subscriptions,
            }
        })
    }

    /// The handle to feed output through, from any thread.
    pub fn feed(&self) -> TerminalFeed {
        TerminalFeed(self.shared.clone())
    }

    /// Readies the view for a new session in place of one that ended
    /// (reconnect): modes the old program left on are cleared and the parser
    /// starts clean, while the screen and scrollback stay.
    pub fn reset_for_new_session(&mut self, cx: &mut Context<Self>) {
        self.shared.reset_for_new_session();
        self.paste_task = None;
        self.selecting = false;
        self.last_report_cell = None;
        self.exited = false;
        cx.notify();
    }

    /// The current grid size (cols, rows), once laid out.
    pub fn size(&self) -> Option<(u16, u16)> {
        self.grid
    }

    /// Copy the selection to the clipboard as soon as a drag ends.
    pub fn set_copy_on_select(&mut self, on: bool) {
        self.copy_on_select = on;
    }

    /// Whether Alt/Option sends ESC-prefixed keys. Defaults to on, except on
    /// macOS where Option types characters.
    pub fn set_alt_is_meta(&mut self, on: bool) {
        self.alt_is_meta = on;
    }

    /// Changes the font size (zoom). The grid is re-measured on the next
    /// layout and `on_resize` fires if the size changed.
    pub fn set_font_size(&mut self, size: Pixels, cx: &mut Context<Self>) {
        self.options.font_size = size;
        self.metrics = None;
        cx.notify();
    }

    pub fn set_palette(&mut self, palette: Palette, cx: &mut Context<Self>) {
        self.options.palette = palette;
        cx.notify();
    }

    /// The selected text, if any.
    pub fn selection_text(&self) -> Option<String> {
        self.shared
            .state
            .lock()
            .term
            .selection_to_string()
            .filter(|s| !s.is_empty())
    }

    /// Finds `query` (a literal, not a pattern) from the selection onwards,
    /// selects the match and scrolls to it, wrapping at either end: xterm.js's
    /// `findNext` / `findPrevious`. Until [`Self::clear_search`] every match
    /// on screen is highlighted. Returns whether anything matched; an empty
    /// query does nothing.
    pub fn find(
        &mut self,
        query: &str,
        case_sensitive: bool,
        direction: SearchDirection,
        cx: &mut Context<Self>,
    ) -> bool {
        let wanted = search::pattern(query, case_sensitive);
        if wanted.is_none() {
            return false;
        }
        if self.search.as_ref().map(|s| &s.pattern) != wanted.as_ref() {
            self.search = ActiveSearch::new(query, case_sensitive);
        }
        let Some(active) = self.search.as_mut() else {
            return false;
        };
        let found = search::find(
            &mut self.shared.state.lock().term,
            &mut active.regex,
            direction,
        );
        cx.notify();
        found.is_some()
    }

    /// Drops the find bar's highlights (`clearDecorations`). The selection a
    /// search made stays, as it does in xterm.js.
    pub fn clear_search(&mut self, cx: &mut Context<Self>) {
        if self.search.take().is_some() {
            cx.notify();
        }
    }

    /// Selects the whole buffer, scrollback included (`term.selectAll()`).
    pub fn select_all(&mut self, cx: &mut Context<Self>) {
        search::select_all(&mut self.shared.state.lock().term);
        cx.notify();
    }

    /// Copies the selection to the clipboard, if there is one.
    pub fn copy(&mut self, cx: &mut Context<Self>) {
        self.copy_selection(cx);
    }

    /// Pastes the clipboard's text, as Ctrl+Shift+V does.
    pub fn paste_from_clipboard(&mut self, cx: &mut Context<Self>) {
        self.paste_clipboard(cx);
    }

    /// Sends the user's input to the program, the way typing does: the view
    /// jumps back to the bottom of the scrollback first.
    pub fn write_input(&mut self, bytes: Vec<u8>, cx: &mut Context<Self>) {
        if bytes.is_empty() {
            return;
        }
        let mut state = self.shared.state.lock();
        if state.term.grid().display_offset() != 0 {
            state.term.scroll_display(Scroll::Bottom);
            cx.notify();
        }
        drop(state);
        (self.input)(bytes);
    }

    /// Pastes `text`, bracketed if the program enabled it, in 512-byte chunks
    /// 1 ms apart when it is large. Pastes queue behind each other.
    pub fn paste(&mut self, text: &str, cx: &mut Context<Self>) {
        let bracketed = self.mode().contains(TermMode::BRACKETED_PASTE);
        let payload = paste_payload(text, bracketed);
        let mut chunks = paste_chunks(&payload).into_iter();
        let Some(first) = chunks.next() else { return };
        let rest: Vec<Vec<u8>> = chunks.collect();
        if rest.is_empty() && self.paste_task.is_none() {
            self.write_input(first, cx);
            return;
        }
        let previous = self.paste_task.take();
        let input = self.input.clone();
        self.paste_task = Some(
            cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
                if let Some(previous) = previous {
                    previous.await;
                }
                let _ = this.update(cx, |view, cx| view.write_input(first, cx));
                for chunk in rest {
                    cx.background_executor().timer(PASTE_CHUNK_DELAY).await;
                    input(chunk);
                }
            }),
        );
    }

    fn mode(&self) -> TermMode {
        *self.shared.state.lock().term.mode()
    }

    fn handle_msgs(&mut self, batch: Vec<Msg>, cx: &mut Context<Self>) {
        let mut notify = false;
        for msg in batch {
            match msg {
                Msg::Wake => notify = true,
                Msg::Exited => self.exit(cx),
                Msg::SyncDeadline(deadline) => self.watch_sync(deadline, cx),
                Msg::Notices => {
                    let (bell, title) = self.shared.notices.take();
                    if let Some(title) = title {
                        cx.emit(TerminalEvent::TitleChanged(title.unwrap_or_default()));
                    }
                    if bell {
                        cx.emit(TerminalEvent::Bell);
                    }
                }
                Msg::Term(event) => match event {
                    Event::ClipboardStore(_, text) => cx.emit(TerminalEvent::ClipboardStore(text)),
                    // Replies the terminal owes the program (DA, DSR, ...).
                    Event::PtyWrite(text) => (self.input)(text.into_bytes()),
                    Event::ColorRequest(index, format) => {
                        let rgb = self.color_for_index(index);
                        (self.input)(format(rgb).into_bytes());
                    }
                    Event::TextAreaSizeRequest(format) => {
                        if let (Some((cols, rows)), Some(m)) = (self.grid, self.metrics) {
                            let size = WindowSize {
                                num_cols: cols,
                                num_lines: rows,
                                cell_width: f32::from(m.width) as u16,
                                cell_height: f32::from(m.height) as u16,
                            };
                            (self.input)(format(size).into_bytes());
                        }
                    }
                    Event::Exit | Event::ChildExit(_) => self.exit(cx),
                    // OSC 52 reads stay denied (alacritty's default), and
                    // the rest needs nothing from the view.
                    _ => {}
                },
            }
        }
        if notify {
            cx.notify();
        }
    }

    /// Waits out a synchronized update's deadline and releases it if the
    /// program has not ended it by then.
    fn watch_sync(&mut self, deadline: Instant, cx: &mut Context<Self>) {
        let shared = self.shared.clone();
        cx.spawn(async move |_, cx: &mut AsyncApp| {
            let mut deadline = deadline;
            loop {
                let wait = deadline.saturating_duration_since(Instant::now());
                cx.background_executor().timer(wait).await;
                match shared.flush_expired_sync() {
                    Some(next) => deadline = next,
                    None => break,
                }
            }
        })
        .detach();
    }

    fn exit(&mut self, cx: &mut Context<Self>) {
        if !self.exited {
            self.exited = true;
            cx.emit(TerminalEvent::Exited);
        }
    }

    fn color_for_index(&self, index: usize) -> Rgb {
        let state = self.shared.state.lock();
        let palette = &self.options.palette;
        state.term.colors()[index].unwrap_or_else(|| match index {
            0..=255 => palette.indexed(index as u8),
            257 => crate::palette::rgb(palette.background),
            258 => crate::palette::rgb(palette.cursor),
            _ => crate::palette::rgb(palette.foreground),
        })
    }

    fn focus_changed(&mut self, focused: bool, cx: &mut Context<Self>) {
        if self.mode().contains(TermMode::FOCUS_IN_OUT) {
            (self.input)(if focused {
                b"\x1b[I".to_vec()
            } else {
                b"\x1b[O".to_vec()
            });
        }
        cx.notify();
    }

    /// Called from prepaint: measures cells, fits the grid to `bounds`, and
    /// tells the terminal and the owner if the size changed.
    pub(crate) fn layout_grid(
        &mut self,
        bounds: Bounds<Pixels>,
        window: &mut Window,
    ) -> CellMetrics {
        let metrics = match self.metrics {
            Some(m) => m,
            None => {
                let ts = window.text_system();
                let font_id = ts.resolve_font(&self.font);
                let size = self.options.font_size;
                let width = ts
                    .advance(font_id, size, 'm')
                    .map(|s| s.width)
                    .unwrap_or(size * 0.6);
                // xterm.js's lineHeight 1.0: the font's own ascent + descent.
                let height = (ts.ascent(font_id, size) + ts.descent(font_id, size).abs()).ceil();
                let m = CellMetrics {
                    width,
                    height: height.max(px(1.0)),
                };
                self.metrics = Some(m);
                m
            }
        };
        self.bounds = Some(bounds);

        let cols =
            ((bounds.size.width / metrics.width).floor() as usize).clamp(2, u16::MAX as usize);
        let rows =
            ((bounds.size.height / metrics.height).floor() as usize).clamp(1, u16::MAX as usize);
        let size = (cols as u16, rows as u16);
        if self.grid != Some(size) {
            self.grid = Some(size);
            self.shared
                .state
                .lock()
                .term
                .resize(GridSize { cols, lines: rows });
            (self.on_resize)(size.0, size.1);
        }
        metrics
    }

    /// The viewport cell under a window position, clamped to the grid, and
    /// which half of the cell it is in.
    fn cell_at(&self, position: Point<Pixels>) -> Option<(usize, usize, Side)> {
        let (bounds, m, (cols, rows)) = (self.bounds?, self.metrics?, self.grid?);
        let x = f32::from(position.x - bounds.origin.x).max(0.0);
        let y = f32::from(position.y - bounds.origin.y).max(0.0);
        let cw = f32::from(m.width);
        let col = ((x / cw) as usize).min(cols as usize - 1);
        let row = ((y / f32::from(m.height)) as usize).min(rows as usize - 1);
        let side = if x - col as f32 * cw > cw / 2.0 {
            Side::Right
        } else {
            Side::Left
        };
        Some((col, row, side))
    }

    fn grid_point(&self, col: usize, row: usize) -> GridPoint {
        let offset = self.shared.state.lock().term.grid().display_offset();
        GridPoint::new(Line(row as i32 - offset as i32), Column(col))
    }

    /// Whether mouse events go to the program. Shift bypasses reporting so
    /// the user can still select, as in xterm.
    fn reports_mouse(&self, shift: bool) -> bool {
        !shift && self.mode().intersects(TermMode::MOUSE_MODE)
    }

    fn report(
        &mut self,
        button: ReportButton,
        kind: ReportKind,
        modifiers: &Modifiers,
        col: usize,
        row: usize,
    ) {
        let mode = self.mode();
        let enc = ReportEncoding {
            sgr: mode.contains(TermMode::SGR_MOUSE),
            utf8: mode.contains(TermMode::UTF8_MOUSE),
        };
        let mods = ReportModifiers {
            shift: modifiers.shift,
            alt: modifiers.alt,
            control: modifiers.control,
        };
        if let Some(bytes) = mouse_report(button, kind, mods, col, row, enc) {
            (self.input)(bytes);
        }
    }

    fn report_button(button: MouseButton) -> Option<ReportButton> {
        match button {
            MouseButton::Left => Some(ReportButton::Left),
            MouseButton::Middle => Some(ReportButton::Middle),
            MouseButton::Right => Some(ReportButton::Right),
            _ => None,
        }
    }

    /// The modifier that turns a click into "open this link".
    fn link_modifier(m: &Modifiers) -> bool {
        if cfg!(target_os = "macos") {
            m.platform
        } else {
            m.control
        }
    }

    /// The text of one viewport row, and for each character the column it
    /// starts at.
    fn row_text(&self, row: usize) -> (String, Vec<usize>) {
        let state = self.shared.state.lock();
        let term = &state.term;
        let line = Line(row as i32 - term.grid().display_offset() as i32);
        let mut text = String::new();
        let mut cols = Vec::new();
        for col in 0..term.columns() {
            let cell = &term.grid()[line][Column(col)];
            if cell.flags.intersects(
                alacritty_terminal::term::cell::Flags::WIDE_CHAR_SPACER
                    | alacritty_terminal::term::cell::Flags::LEADING_WIDE_CHAR_SPACER,
            ) {
                continue;
            }
            text.push(cell.c);
            cols.push(col);
        }
        (text, cols)
    }

    fn url_at_cell(&self, col: usize, row: usize) -> Option<HoveredUrl> {
        let (text, cols) = self.row_text(row);
        let char_index = cols.iter().position(|&c| c >= col)?;
        let (url, range) = url_at(&text, char_index)?;
        let start = cols[range.start];
        let end = cols.get(range.end).copied().unwrap_or(start + range.len());
        Some(HoveredUrl {
            row,
            cols: start..end,
            url,
        })
    }

    fn copy_selection(&mut self, cx: &mut Context<Self>) {
        if let Some(text) = self.selection_text() {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    fn paste_clipboard(&mut self, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.paste(&text, cx);
        }
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        // While the IME is composing, keys belong to it.
        if self.marked_text.is_some() {
            return;
        }
        let ks = &event.keystroke;
        // An AltGr character is text: leave it, unhandled, to the input
        // handler, which inserts `key_char`. Encoding it would send the
        // Ctrl+Alt control code Windows reports AltGr as.
        if keys::is_altgr_text(ks, event.prefer_character_input) {
            return;
        }
        if keys::is_copy(ks) {
            self.copy_selection(cx);
            cx.stop_propagation();
            return;
        }
        if keys::is_paste(ks) {
            self.paste_clipboard(cx);
            cx.stop_propagation();
            return;
        }
        let mode = self.mode();
        let m = &ks.modifiers;
        if m.shift && !m.control && !m.alt && !mode.contains(TermMode::ALT_SCREEN) {
            let scroll = match ks.key.as_str() {
                "pageup" => Some(Scroll::PageUp),
                "pagedown" => Some(Scroll::PageDown),
                "home" => Some(Scroll::Top),
                "end" => Some(Scroll::Bottom),
                _ => None,
            };
            if let Some(scroll) = scroll {
                self.shared.state.lock().term.scroll_display(scroll);
                cx.notify();
                cx.stop_propagation();
                return;
            }
        }
        let modes = KeyModes {
            app_cursor: mode.contains(TermMode::APP_CURSOR),
            alt_is_meta: self.alt_is_meta,
        };
        if let Some(bytes) = keys::encode_keystroke(ks, modes) {
            self.write_input(bytes, cx);
            cx.stop_propagation();
        }
    }

    fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus_handle, cx);
        let Some((col, row, side)) = self.cell_at(event.position) else {
            return;
        };

        if event.button == MouseButton::Left && Self::link_modifier(&event.modifiers) {
            if let Some(hit) = self.url_at_cell(col, row) {
                if is_openable(&hit.url) {
                    let _ = open::that_detached(&hit.url);
                }
                cx.stop_propagation();
                return;
            }
        }

        if self.reports_mouse(event.modifiers.shift) {
            if let Some(button) = Self::report_button(event.button) {
                self.report(button, ReportKind::Press, &event.modifiers, col, row);
                self.last_report_cell = Some((col, row));
            }
            cx.stop_propagation();
            return;
        }

        match event.button {
            MouseButton::Left => {
                let point = self.grid_point(col, row);
                let ty = match event.click_count {
                    2 => SelectionType::Semantic,
                    n if n >= 3 => SelectionType::Lines,
                    _ => SelectionType::Simple,
                };
                let mut state = self.shared.state.lock();
                // Shift+click extends the existing selection.
                match state.term.selection.as_mut() {
                    Some(sel) if event.modifiers.shift && event.click_count == 1 => {
                        sel.update(point, side)
                    }
                    _ => state.term.selection = Some(Selection::new(ty, point, side)),
                }
                drop(state);
                self.selecting = true;
                cx.notify();
            }
            #[cfg(target_os = "linux")]
            MouseButton::Middle => {
                if let Some(text) = cx.read_from_primary().and_then(|item| item.text()) {
                    self.paste(&text, cx);
                }
            }
            _ => {}
        }
    }

    fn on_mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        let Some((col, row, side)) = self.cell_at(event.position) else {
            return;
        };

        let hovered = if Self::link_modifier(&event.modifiers) {
            self.url_at_cell(col, row)
        } else {
            None
        };
        if hovered != self.hovered_url {
            self.hovered_url = hovered;
            cx.notify();
        }

        if self.selecting && event.pressed_button == Some(MouseButton::Left) {
            let point = self.grid_point(col, row);
            if let Some(sel) = self.shared.state.lock().term.selection.as_mut() {
                sel.update(point, side);
            }
            cx.notify();
            return;
        }

        let mode = self.mode();
        if self.reports_mouse(event.modifiers.shift) && self.last_report_cell != Some((col, row)) {
            let button = match event.pressed_button {
                Some(b) if mode.intersects(TermMode::MOUSE_DRAG | TermMode::MOUSE_MOTION) => {
                    Self::report_button(b)
                }
                None if mode.contains(TermMode::MOUSE_MOTION) => Some(ReportButton::None),
                _ => None,
            };
            if let Some(button) = button {
                self.report(button, ReportKind::Motion, &event.modifiers, col, row);
                self.last_report_cell = Some((col, row));
            }
        }
    }

    fn on_mouse_up(&mut self, event: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.selecting && event.button == MouseButton::Left {
            self.selecting = false;
            if let Some(text) = self.selection_text() {
                #[cfg(target_os = "linux")]
                cx.write_to_primary(ClipboardItem::new_string(text.clone()));
                if self.copy_on_select {
                    cx.write_to_clipboard(ClipboardItem::new_string(text));
                }
            }
            return;
        }
        if !self.reports_mouse(event.modifiers.shift) {
            return;
        }
        if let (Some((col, row, _)), Some(button)) = (
            self.cell_at(event.position),
            Self::report_button(event.button),
        ) {
            self.report(button, ReportKind::Release, &event.modifiers, col, row);
        }
    }

    fn on_scroll_wheel(
        &mut self,
        event: &ScrollWheelEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(m) = self.metrics else { return };
        let delta = match event.delta {
            ScrollDelta::Lines(p) => p.y * WHEEL_LINES,
            ScrollDelta::Pixels(p) => p.y / m.height,
        };
        self.scroll_lines += delta;
        let lines = self.scroll_lines.trunc() as i32;
        if lines == 0 {
            return;
        }
        self.scroll_lines -= lines as f32;
        cx.stop_propagation();

        let mode = self.mode();
        if self.reports_mouse(event.modifiers.shift) {
            if let Some((col, row, _)) = self.cell_at(event.position) {
                let button = if lines > 0 {
                    ReportButton::WheelUp
                } else {
                    ReportButton::WheelDown
                };
                for _ in 0..lines.unsigned_abs().min(10) {
                    self.report(button, ReportKind::Press, &event.modifiers, col, row);
                }
            }
        } else if mode.contains(TermMode::ALT_SCREEN | TermMode::ALTERNATE_SCROLL)
            && !event.modifiers.shift
        {
            // Full-screen programs without mouse support (less, man) get
            // arrow keys, so the wheel still scrolls them.
            let key = match (lines > 0, mode.contains(TermMode::APP_CURSOR)) {
                (true, true) => b"\x1bOA",
                (true, false) => b"\x1b[A",
                (false, true) => b"\x1bOB",
                (false, false) => b"\x1b[B",
            };
            let bytes = key.repeat(lines.unsigned_abs() as usize);
            (self.input)(bytes);
        } else {
            self.shared
                .state
                .lock()
                .term
                .scroll_display(Scroll::Delta(lines));
            cx.notify();
        }
    }
}

impl Render for TerminalView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let cursor = if self.hovered_url.is_some() {
            CursorStyle::PointingHand
        } else {
            CursorStyle::IBeam
        };
        div()
            .id("terminal")
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus_handle)
            .size_full()
            .cursor(cursor)
            .on_key_down(cx.listener(Self::on_key_down))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_down(MouseButton::Middle, cx.listener(Self::on_mouse_down))
            .on_mouse_down(MouseButton::Right, cx.listener(Self::on_mouse_down))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up(MouseButton::Middle, cx.listener(Self::on_mouse_up))
            .on_mouse_up(MouseButton::Right, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_scroll_wheel(cx.listener(Self::on_scroll_wheel))
            .child(TerminalElement::new(cx.entity()))
    }
}

impl EntityInputHandler for TerminalView {
    fn text_for_range(
        &mut self,
        _range: Range<usize>,
        _adjusted_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        None
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: 0..0,
            reversed: false,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        self.marked_text
            .as_ref()
            .map(|text| 0..text.encode_utf16().count())
    }

    fn unmark_text(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.marked_text = None;
        cx.notify();
    }

    fn replace_text_in_range(
        &mut self,
        _range: Option<Range<usize>>,
        text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.marked_text.take().is_some() {
            cx.notify();
        }
        self.write_input(text.as_bytes().to_vec(), cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        _range: Option<Range<usize>>,
        new_text: &str,
        _new_selected_range: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.marked_text = (!new_text.is_empty()).then(|| new_text.to_string());
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        _range_utf16: Range<usize>,
        element_bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        // The candidate window goes at the cursor.
        let ((col, row), m) = (self.cursor_cell?, self.metrics?);
        Some(Bounds::new(
            point(
                element_bounds.origin.x + m.width * col as f32,
                element_bounds.origin.y + m.height * row as f32,
            ),
            size(m.width, m.height),
        ))
    }

    fn character_index_for_point(
        &mut self,
        _point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        None
    }
}

#[cfg(test)]
mod tests {
    // Not `super::*`: the GPUI glob brings its own `test` macro, which would
    // shadow Rust's.
    use super::resolve_family;

    #[test]
    fn missing_family_falls_back_to_an_installed_mono() {
        let installed = vec!["DejaVu Sans Mono".to_string(), "Noto Sans".to_string()];
        assert_eq!(
            resolve_family("JetBrains Mono", &installed),
            "DejaVu Sans Mono"
        );
        assert_eq!(resolve_family("Noto Sans", &installed), "Noto Sans");
        assert_eq!(resolve_family("Nope", &[]), ".SystemUIFont");
    }
}
