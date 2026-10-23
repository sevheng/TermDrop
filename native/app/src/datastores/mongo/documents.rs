//! One collection's documents: filter, sort, page size, Prev/Next, a table
//! or JSON view, the count and the stats line. Port of
//! `MongoDocumentsView.vue`; the indexes view is the native tab's own
//! (`mongodb_list_indexes` had no Vue caller).
//!
//! Built once per opened collection (the Vue `:key`), so switching
//! collections starts clean. Read-only: the core has no write path here.

use std::collections::HashMap;

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::{
    Disableable as _, Icon, IndexPath, Selectable as _, Sizable as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, AppContext as _, ClipboardItem, Context, Entity, FontWeight, InteractiveElement,
    IntoElement, ParentElement, Render, SharedString, StatefulInteractiveElement as _, Styled,
    Subscription, Task, Window, div, px,
};
use termdrop_core::mongodb::{CountResult, FindResult};

use super::bson::{
    Json, MAX_COLUMNS, SUMMARY_LIMIT, cell_text, derive_columns, index_name, parse_documents,
    pretty_print, summarize,
};
use super::query::{
    DEFAULT_PAGE_SIZE, PAGE_SIZES, clamp_page_size, describe_range, last_page, query_arg, skip_for,
    stats_summary, validate_json_input,
};
use crate::datastores::ServiceTabContext;
use crate::datastores::common::{JsonLine, json_lines, json_view, list_head, list_row, text_2xs};
use crate::ui::choice::Choice;
use crate::ui::empty_state::EmptyState;
use crate::ui::icon_button::IconButton;
use crate::ui::theme::{Palette, mono_font, palette};
use crate::ui::toast::ToastKind;

/// Lines of one expanded document drawn before it is cut short.
const MAX_DOC_LINES: usize = 2000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Table,
    Json,
    Indexes,
}

enum Indexes {
    NotLoaded,
    Loading,
    Loaded(Vec<(String, Pretty)>),
    Failed(String),
}

/// A document ready to draw expanded: the text (also what Copy copies) and
/// its coloured lines, or `None` to draw the text raw.
struct Pretty {
    text: String,
    lines: Option<(Vec<JsonLine>, bool)>,
}

impl Pretty {
    fn of(json: &str) -> Self {
        let text = pretty_print(json);
        let lines = json_lines(&text, MAX_DOC_LINES);
        Self { text, lines }
    }
}

/// The page, parsed once for the table rather than per cell.
#[derive(Default)]
struct Page {
    documents: Vec<String>,
    parsed: Vec<Option<Vec<(String, Json)>>>,
    columns: Vec<String>,
    widths: Vec<f32>,
    hidden: usize,
    summaries: Vec<String>,
}

impl Page {
    fn new(documents: Vec<String>) -> Self {
        let parsed = parse_documents(&documents);
        let (columns, hidden) = derive_columns(&parsed, MAX_COLUMNS);
        let widths = columns
            .iter()
            .map(|c| {
                let longest = parsed
                    .iter()
                    .map(|d| cell_text(d.as_ref(), c).chars().count())
                    .chain(std::iter::once(c.chars().count()))
                    .max()
                    .unwrap_or(0);
                column_width(longest)
            })
            .collect();
        let summaries = documents
            .iter()
            .map(|d| summarize(d, SUMMARY_LIMIT))
            .collect();
        Self {
            documents,
            parsed,
            columns,
            widths,
            hidden,
            summaries,
        }
    }
}

/// A column's width from its longest text, capped at 16rem like the Vue
/// cells (`max-w-[16rem] truncate`).
fn column_width(chars: usize) -> f32 {
    (chars.min(48) as f32 * 7.0 + 16.0).clamp(48.0, 256.0)
}

pub struct DocumentsView {
    ctx: ServiceTabContext,
    db: String,
    collection: String,
    mode: Mode,
    filter: Entity<InputState>,
    sort: Entity<InputState>,
    filter_text: String,
    sort_text: String,
    page_size: u64,
    page_select: Entity<SelectState<Vec<Choice<u64>>>>,
    page: u64,
    rows: Page,
    expanded: HashMap<usize, Pretty>,
    loading: bool,
    truncated: bool,
    elapsed_ms: Option<u64>,
    query_error: Option<String>,
    total: u64,
    estimated: bool,
    stats: String,
    indexes: Indexes,
    query_task: Option<Task<()>>,
    stats_task: Option<Task<()>>,
    index_task: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl DocumentsView {
    pub fn new(
        ctx: ServiceTabContext,
        db: String,
        collection: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder(r#"{"field": "value"}"#));
        let sort = cx.new(|cx| InputState::new(window, cx).placeholder(r#"{"_id": -1}"#));
        let choices: Vec<Choice<u64>> = PAGE_SIZES
            .iter()
            .map(|n| Choice::new(*n, n.to_string()))
            .collect();
        let start = PAGE_SIZES.iter().position(|n| *n == DEFAULT_PAGE_SIZE);
        let page_select =
            cx.new(|cx| SelectState::new(choices, start.map(IndexPath::new), window, cx));

        let mut subscriptions = Vec::new();
        for (input, is_filter) in [(&filter, true), (&sort, false)] {
            subscriptions.push(cx.subscribe_in(
                input,
                window,
                move |this: &mut Self, input, event: &InputEvent, window, cx| match event {
                    InputEvent::Change => {
                        let text = input.read(cx).value().to_string();
                        if is_filter {
                            this.filter_text = text;
                        } else {
                            this.sort_text = text;
                        }
                        cx.notify();
                    }
                    InputEvent::PressEnter { .. } => this.run_query(0, window, cx),
                    _ => {}
                },
            ));
        }
        subscriptions.push(cx.subscribe_in(
            &page_select,
            window,
            |this: &mut Self, _, event: &SelectEvent<Vec<Choice<u64>>>, window, cx| {
                let SelectEvent::Confirm(Some(size)) = event else {
                    return;
                };
                if *size != this.page_size {
                    this.page_size = *size;
                    this.run_query(0, window, cx);
                }
            },
        ));

        let mut view = Self {
            ctx,
            db,
            collection,
            mode: Mode::Table,
            filter,
            sort,
            filter_text: String::new(),
            sort_text: String::new(),
            page_size: DEFAULT_PAGE_SIZE,
            page_select,
            page: 0,
            rows: Page::default(),
            expanded: HashMap::new(),
            loading: false,
            truncated: false,
            elapsed_ms: None,
            query_error: None,
            total: 0,
            estimated: false,
            stats: String::new(),
            indexes: Indexes::NotLoaded,
            query_task: None,
            stats_task: None,
            index_task: None,
            _subscriptions: subscriptions,
        };
        view.run_query(0, window, cx);
        view.load_stats(window, cx);
        view
    }

    fn filter_error(&self) -> Option<String> {
        validate_json_input(&self.filter_text, "filter")
    }

    fn input_error(&self) -> Option<String> {
        self.filter_error()
            .or_else(|| validate_json_input(&self.sort_text, "sort"))
    }

    fn max_page(&self) -> u64 {
        last_page(self.total, self.page_size)
    }

    fn range_label(&self) -> String {
        describe_range(
            self.page,
            self.page_size,
            self.rows.documents.len(),
            self.total,
            self.estimated,
        )
    }

    /// Count first, so the page can be clamped to one that exists, then
    /// fetch it.
    fn run_query(&mut self, target: u64, window: &mut Window, cx: &mut Context<Self>) {
        if self.input_error().is_some() || self.loading {
            return;
        }
        self.loading = true;
        self.query_error = None;
        cx.notify();
        let core = self.ctx.core().clone();
        let host = self.ctx.host_id;
        let (db, coll) = (self.db.clone(), self.collection.clone());
        let filter = query_arg(&self.filter_text);
        let sort = query_arg(&self.sort_text);
        let page_size = self.page_size;
        self.query_task = Some(self.ctx.run(
            window,
            cx,
            async move {
                let counted: CountResult = core
                    .mongodb_count(host, db.clone(), coll.clone(), filter.clone())
                    .await?;
                let page = target.min(last_page(counted.count, page_size));
                let found: FindResult = core
                    .mongodb_find(
                        host,
                        db,
                        coll,
                        filter,
                        sort,
                        None,
                        skip_for(page, page_size),
                        clamp_page_size(page_size) as i64,
                    )
                    .await?;
                Ok((counted, found, page))
            },
            |this, result, _, cx| {
                this.loading = false;
                match result {
                    Ok((counted, found, page)) => {
                        this.total = counted.count;
                        this.estimated = counted.estimated;
                        this.rows = Page::new(found.documents);
                        this.truncated = found.truncated;
                        this.elapsed_ms = Some(found.elapsed_ms);
                        this.page = page;
                        this.expanded.clear();
                    }
                    // Inline, not a toast: a bad query is corrected in place.
                    Err(e) => {
                        this.query_error = Some(e);
                        this.rows = Page::default();
                        this.truncated = false;
                    }
                }
                cx.notify();
            },
        ));
    }

    fn load_stats(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let core = self.ctx.core().clone();
        let host = self.ctx.host_id;
        let (db, coll) = (self.db.clone(), self.collection.clone());
        self.stats_task = Some(self.ctx.run(
            window,
            cx,
            async move { core.mongodb_collection_stats(host, db, coll).await },
            |this, result, _, cx| {
                // Stats are a nicety; the collection is browsable without.
                this.stats = result
                    .ok()
                    .and_then(|raw| stats_summary(&raw))
                    .unwrap_or_default();
                cx.notify();
            },
        ));
    }

    fn load_indexes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.indexes = Indexes::Loading;
        cx.notify();
        let core = self.ctx.core().clone();
        let host = self.ctx.host_id;
        let (db, coll) = (self.db.clone(), self.collection.clone());
        self.index_task = Some(self.ctx.run(
            window,
            cx,
            async move { core.mongodb_list_indexes(host, db, coll).await },
            |this, result, _, cx| {
                this.indexes = match result {
                    Ok(specs) => Indexes::Loaded(
                        specs
                            .iter()
                            .map(|s| (index_name(s), Pretty::of(s)))
                            .collect(),
                    ),
                    Err(e) => Indexes::Failed(e),
                };
                cx.notify();
            },
        ));
    }

    fn set_mode(&mut self, mode: Mode, window: &mut Window, cx: &mut Context<Self>) {
        self.mode = mode;
        if mode == Mode::Indexes && matches!(self.indexes, Indexes::NotLoaded) {
            self.load_indexes(window, cx);
        }
        cx.notify();
    }

    fn toggle(&mut self, i: usize, cx: &mut Context<Self>) {
        if self.expanded.remove(&i).is_none()
            && let Some(doc) = self.rows.documents.get(i)
        {
            self.expanded.insert(i, Pretty::of(doc));
        }
        cx.notify();
    }

    fn copy_document(&self, i: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(doc) = self.rows.documents.get(i) else {
            return;
        };
        cx.write_to_clipboard(ClipboardItem::new_string(pretty_print(doc)));
        self.ctx
            .toast(window, cx, "Document copied", ToastKind::Success);
    }

    fn clear_filter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.filter_text.clear();
        self.filter.update(cx, |s, cx| s.set_value("", window, cx));
        self.run_query(0, window, cx);
    }

    // ---- drawing -------------------------------------------------------

    fn render_head(&self, p: &Palette) -> impl IntoElement {
        let sub = if self.stats.is_empty() {
            "read-only".to_string()
        } else {
            format!("read-only \u{b7} {}", self.stats)
        };
        v_flex()
            .flex_none()
            .px_4()
            .py_2()
            .border_b_1()
            .border_color(p.line)
            .child(
                h_flex()
                    .min_w_0()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(p.ink)
                    .child(
                        div()
                            .flex_none()
                            .text_color(p.accent)
                            .child(SharedString::from(self.db.clone())),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .child(SharedString::from(format!(".{}", self.collection))),
                    ),
            )
            .child(text_2xs(div()).mt_0p5().text_color(p.ink_3).child(sub))
    }

    fn render_controls(&self, p: &Palette, cx: &mut Context<Self>) -> impl IntoElement {
        let filter_bad = self.filter_error().is_some();
        let sort_bad = validate_json_input(&self.sort_text, "sort").is_some();
        let mono = mono_font(cx);
        let field = |label: &'static str, input: &Entity<InputState>, bad: bool| {
            v_flex()
                .gap_1()
                .child(text_2xs(div()).text_color(p.ink_2).child(label))
                .child(
                    div()
                        .rounded_sm()
                        .border_1()
                        .border_color(if bad {
                            p.bad
                        } else {
                            gpui_kit::transparent_black()
                        })
                        .font_family(mono.clone())
                        .child(Input::new(input).small()),
                )
        };
        let mode_button = |id: &'static str, label: &'static str, mode: Mode| {
            Button::new(id)
                .small()
                .ghost()
                .label(label)
                .selected(self.mode == mode)
                .on_click(cx.listener(move |this, _, window, cx| this.set_mode(mode, window, cx)))
        };
        let note: AnyElement = if let Some(e) = self.input_error() {
            text_2xs(div())
                .font_family(mono.clone())
                .text_color(p.bad)
                .child(e)
                .into_any_element()
        } else if let Some(e) = &self.query_error {
            text_2xs(div())
                .font_family(mono.clone())
                .text_color(p.bad)
                .child(SharedString::from(e.clone()))
                .into_any_element()
        } else {
            text_2xs(div())
                .text_color(p.ink_3)
                .child("Ctrl/Cmd+Enter to run. A 24-character hex _id is treated as an ObjectId.")
                .into_any_element()
        };
        v_flex()
            .flex_none()
            .px_4()
            .py_2()
            .gap_2()
            .bg(p.surface)
            .border_b_1()
            .border_color(p.line)
            .child(
                h_flex()
                    .gap_2()
                    .items_end()
                    .child(div().flex_1().min_w_0().child(field(
                        "Filter",
                        &self.filter,
                        filter_bad,
                    )))
                    .child(div().w(px(192.)).child(field("Sort", &self.sort, sort_bad)))
                    .child(
                        h_flex()
                            .gap_0p5()
                            .child(mode_button("mongo-mode-table", "Table", Mode::Table))
                            .child(mode_button("mongo-mode-json", "JSON", Mode::Json))
                            .child(mode_button("mongo-mode-indexes", "Indexes", Mode::Indexes)),
                    )
                    .child(
                        Button::new("mongo-run")
                            .small()
                            .primary()
                            .label(if self.loading {
                                "Running\u{2026}"
                            } else {
                                "Run"
                            })
                            .disabled(self.loading)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.run_query(0, window, cx)),
                            ),
                    ),
            )
            .child(note)
    }

    fn render_pretty(
        &self,
        id: String,
        pretty: &Pretty,
        p: &Palette,
        mono: SharedString,
    ) -> AnyElement {
        let body: AnyElement = match &pretty.lines {
            Some((lines, cut)) => v_flex()
                .child(json_view(lines, p, mono.clone()))
                .when(*cut, |d| {
                    d.child(
                        text_2xs(div())
                            .pt_1()
                            .text_color(p.ink_3)
                            .child("\u{2026} cut short; Copy takes the whole document."),
                    )
                })
                .into_any_element(),
            None => div()
                .font_family(mono)
                .text_xs()
                .text_color(p.ink)
                .child(SharedString::from(pretty.text.clone()))
                .into_any_element(),
        };
        div()
            .id(SharedString::from(id))
            .px_3()
            .py_2()
            .bg(p.canvas)
            .overflow_x_scroll()
            .child(body)
            .into_any_element()
    }

    fn copy_button(&self, i: usize, cx: &mut Context<Self>) -> impl IntoElement {
        IconButton::new(
            SharedString::from(format!("mongo-copy-{i}")),
            IconName::Copy,
            "Copy document",
        )
        .on_click(cx.listener(move |this, _, window, cx| {
            cx.stop_propagation();
            this.copy_document(i, window, cx);
        }))
    }

    fn chevron(open: bool, p: &Palette) -> Icon {
        Icon::new(if open {
            IconName::ChevronDown
        } else {
            IconName::ChevronRight
        })
        .xsmall()
        .text_color(p.ink_2)
    }

    fn render_table(&self, p: &Palette, cx: &mut Context<Self>) -> AnyElement {
        let mono = mono_font(cx);
        let rows = &self.rows;
        let total_w: f32 = 16.0 + rows.widths.iter().sum::<f32>() + 28.0;
        let head = list_head(h_flex(), p)
            .px_0()
            .min_w(px(total_w))
            .child(div().w(px(16.)).flex_none())
            .children(rows.columns.iter().zip(&rows.widths).map(|(c, w)| {
                div()
                    .w(px(*w))
                    .flex_none()
                    .px_2()
                    .truncate()
                    .child(SharedString::from(c.clone()))
            }));
        let body = rows.documents.iter().enumerate().map(|(i, _)| {
            let open = self.expanded.get(&i);
            let doc = rows.parsed.get(i).and_then(Option::as_ref);
            let row = h_flex()
                .id(SharedString::from(format!("mongo-row-{i}")))
                .min_w(px(total_w))
                .font_family(mono.clone())
                .text_xs()
                .text_color(p.ink);
            let row = list_row(row, p, false, true)
                .child(
                    div()
                        .w(px(16.))
                        .flex_none()
                        .pl_1()
                        .child(Self::chevron(open.is_some(), p)),
                )
                .children(rows.columns.iter().zip(&rows.widths).map(|(c, w)| {
                    div()
                        .w(px(*w))
                        .flex_none()
                        .px_2()
                        .py_1()
                        .truncate()
                        .child(SharedString::from(cell_text(doc, c)))
                }))
                .child(div().w(px(28.)).flex_none().child(self.copy_button(i, cx)))
                .on_click(cx.listener(move |this, _, _, cx| this.toggle(i, cx)));
            v_flex()
                .child(row)
                .when_some(open, |d, pretty| {
                    d.child(self.render_pretty(
                        format!("mongo-pretty-{i}"),
                        pretty,
                        p,
                        mono.clone(),
                    ))
                })
                .into_any_element()
        });
        let notes = v_flex()
            .when(rows.hidden > 0, |d| {
                d.child(text_2xs(div()).pt_2().text_color(p.ink_2).child(format!(
                    "{} more field{} not shown \u{2014} open a row, or switch to JSON, to see everything.",
                    rows.hidden,
                    if rows.hidden == 1 { "" } else { "s" }
                )))
            })
            .when(self.truncated, |d| d.child(truncated_note(p)));
        v_flex()
            .child(
                div()
                    .id("mongo-table")
                    .overflow_x_scroll()
                    .child(v_flex().min_w(px(total_w)).child(head).children(body)),
            )
            .child(notes)
            .into_any_element()
    }

    fn render_json(&self, p: &Palette, cx: &mut Context<Self>) -> AnyElement {
        let mono = mono_font(cx);
        v_flex()
            .gap_1()
            .children(self.rows.summaries.iter().enumerate().map(|(i, summary)| {
                let open = self.expanded.get(&i);
                v_flex()
                    .rounded_sm()
                    .border_1()
                    .border_color(p.line.opacity(0.6))
                    .overflow_hidden()
                    .child(
                        h_flex()
                            .id(SharedString::from(format!("mongo-doc-{i}")))
                            .gap_2()
                            .px_2()
                            .py_0p5()
                            .font_family(mono.clone())
                            .text_xs()
                            .cursor_pointer()
                            .hover(|s| s.bg(p.raised))
                            .child(Self::chevron(open.is_some(), p))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_color(p.ink)
                                    .child(SharedString::from(summary.clone())),
                            )
                            .child(self.copy_button(i, cx))
                            .on_click(cx.listener(move |this, _, _, cx| this.toggle(i, cx))),
                    )
                    .when_some(open, |d, pretty| {
                        d.child(self.render_pretty(
                            format!("mongo-pretty-{i}"),
                            pretty,
                            p,
                            mono.clone(),
                        ))
                    })
            }))
            .when(self.truncated, |d| d.child(truncated_note(p)))
            .into_any_element()
    }

    fn render_indexes(&self, p: &Palette, cx: &mut Context<Self>) -> AnyElement {
        let view = cx.entity().downgrade();
        match &self.indexes {
            Indexes::NotLoaded | Indexes::Loading => {
                EmptyState::loading("mongo-indexes-loading", "Loading indexes\u{2026}")
                    .into_any_element()
            }
            Indexes::Failed(e) => EmptyState::error(
                "mongo-indexes-error",
                "Could not list indexes",
                "Retry",
                move |_, window, cx| {
                    let _ = view.update(cx, |this, cx| this.load_indexes(window, cx));
                },
            )
            .hint(e.clone())
            .into_any_element(),
            Indexes::Loaded(specs) if specs.is_empty() => EmptyState::empty(
                "mongo-indexes-empty",
                "No indexes",
                "Refresh",
                move |_, window, cx| {
                    let _ = view.update(cx, |this, cx| this.load_indexes(window, cx));
                },
            )
            .icon(Icon::new(IconName::ListTree))
            .into_any_element(),
            Indexes::Loaded(specs) => {
                let mono = mono_font(cx);
                v_flex()
                    .gap_2()
                    .child(text_2xs(div()).text_color(p.ink_3).child(format!(
                        "{} index{}",
                        specs.len(),
                        if specs.len() == 1 { "" } else { "es" }
                    )))
                    .children(specs.iter().enumerate().map(|(i, (name, pretty))| {
                        v_flex()
                            .rounded_sm()
                            .border_1()
                            .border_color(p.line.opacity(0.6))
                            .overflow_hidden()
                            .child(
                                div()
                                    .px_2()
                                    .py_1()
                                    .text_xs()
                                    .font_weight(FontWeight::MEDIUM)
                                    .font_family(mono.clone())
                                    .text_color(p.ink)
                                    .child(SharedString::from(name.clone())),
                            )
                            .child(self.render_pretty(
                                format!("mongo-index-{i}"),
                                pretty,
                                p,
                                mono.clone(),
                            ))
                    }))
                    .into_any_element()
            }
        }
    }

    fn render_results(&self, p: &Palette, cx: &mut Context<Self>) -> AnyElement {
        if self.mode == Mode::Indexes {
            return self.render_indexes(p, cx);
        }
        if self.loading {
            return EmptyState::loading("mongo-querying", "Querying\u{2026}").into_any_element();
        }
        let view = cx.entity().downgrade();
        if let Some(e) = &self.query_error {
            return EmptyState::error("mongo-query-error", "Query failed", "Retry", {
                let view = view.clone();
                move |_, window, cx| {
                    let _ = view.update(cx, |this, cx| this.run_query(this.page, window, cx));
                }
            })
            .hint(e.clone())
            .into_any_element();
        }
        if self.rows.documents.is_empty() {
            // A filter that matched nothing is not an empty collection.
            return if self.filter_text.trim().is_empty() {
                EmptyState::empty(
                    "mongo-docs-empty",
                    self.range_label(),
                    "Refresh",
                    move |_, window, cx| {
                        let _ = view.update(cx, |this, cx| this.run_query(0, window, cx));
                    },
                )
                .icon(Icon::new(IconName::FileSearch))
                .into_any_element()
            } else {
                EmptyState::filtered(
                    "mongo-docs-filtered",
                    self.range_label(),
                    "Clear filter",
                    move |_, window, cx| {
                        let _ = view.update(cx, |this, cx| this.clear_filter(window, cx));
                    },
                )
                .icon(Icon::new(IconName::FileSearch))
                .hint("No document matches this filter")
                .into_any_element()
            };
        }
        match self.mode {
            Mode::Table => self.render_table(p, cx),
            _ => self.render_json(p, cx),
        }
    }

    fn render_footer(&self, p: &Palette, cx: &mut Context<Self>) -> impl IntoElement {
        let mut label = self.range_label();
        if let Some(ms) = self.elapsed_ms {
            label.push_str(&format!(" \u{b7} {ms} ms"));
        }
        let page = self.page;
        let max_page = self.max_page();
        h_flex()
            .flex_none()
            .px_4()
            .py_1p5()
            .gap_2()
            .bg(p.surface)
            .border_t_1()
            .border_color(p.line)
            .child(
                text_2xs(div())
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_color(p.ink_3)
                    .child(label),
            )
            .child(text_2xs(div()).text_color(p.ink_2).child("Page size"))
            .child(
                div()
                    .w(px(72.))
                    .child(Select::new(&self.page_select).xsmall()),
            )
            .child(
                Button::new("mongo-prev")
                    .small()
                    .ghost()
                    .label("Prev")
                    .disabled(self.loading || page == 0)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.run_query(page.saturating_sub(1), window, cx)
                    })),
            )
            .child(
                Button::new("mongo-next")
                    .small()
                    .ghost()
                    .label("Next")
                    .disabled(self.loading || page >= max_page)
                    .on_click(
                        cx.listener(move |this, _, window, cx| {
                            this.run_query(page + 1, window, cx)
                        }),
                    ),
            )
    }
}

fn truncated_note(p: &Palette) -> impl IntoElement {
    text_2xs(div())
        .pt_1()
        .text_color(p.warn)
        .child("Results were cut short because the page exceeded the size limit.")
}

impl Render for DocumentsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = *palette(cx);
        v_flex()
            .size_full()
            .min_w_0()
            .child(self.render_head(&p))
            .child(self.render_controls(&p, cx))
            .child(
                div()
                    .id("mongo-results")
                    .flex_1()
                    .min_h_0()
                    .px_4()
                    .py_2()
                    .overflow_y_scroll()
                    .child(self.render_results(&p, cx)),
            )
            .child(self.render_footer(&p, cx))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn columns_are_sized_by_their_text_within_the_vue_cap() {
        assert_eq!(column_width(0), 48.0);
        assert_eq!(column_width(10), 86.0);
        assert_eq!(column_width(1000), 256.0);
    }

    #[test]
    fn a_page_is_parsed_once_with_its_columns_and_summaries() {
        let page = Page::new(vec![
            r#"{"_id":{"$oid":"1"},"name":"a"}"#.into(),
            r#"{"_id":{"$oid":"2"},"age":{"$numberInt":"3"}}"#.into(),
        ]);
        assert_eq!(page.columns, ["_id", "name", "age"]);
        assert_eq!(page.widths.len(), 3);
        assert_eq!(page.summaries[1], "_id: ObjectId('2'), age: 3");
        assert_eq!(page.hidden, 0);
    }
}
