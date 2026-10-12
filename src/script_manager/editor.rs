//! The code editor pane: a line-numbered monospaced text field with token colouring, diagnostic
//! underlines, the current-line band, indent guides and the Swift editing assists, above the
//! status/Save bar. Ported from `ScriptEditorView` and `LineNumberedTextEditorView`.
//!
//! A pure view: it never compiles. The host feeds `diagnostics` and `status` back after a
//! change (`EditorOutput::changed`).

use super::completion::{self, Completion};
use super::edit_ops::{self, Context, Edit};
use super::find::{self, FindEvent, FindState};
use super::model::CompileStatus;
use super::signature;
use super::syntax::{self, Span};
use crate::pine::token::{Diagnostic, DiagnosticSeverity};
use crate::ui::icons::{self, Icon};
use crate::ui::shortcut;
use crate::ui::theme;
use eframe::egui::{
    self, Align, Align2, Color32, Event, FontId, Id, Key, Margin, Modifiers, Rect, Sense, Shape,
    Stroke, TextBuffer, Ui, UiBuilder, Vec2,
    text::{CCursor, CCursorRange},
    text_edit::TextEditState,
};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::{Duration, Instant};

const GUTTER_WIDTH: f32 = 42.0;
const TEXT_INSET: f32 = 8.0;
const FONT_SIZE: f32 = 13.0;
const LINE_NUMBER_SIZE: f32 = 11.0;
const BAR_HEIGHT: f32 = 38.0;
/// A draft is written this long after the last keystroke.
pub const DRAFT_DELAY: Duration = Duration::from_secs(1);
const FLASH: Duration = Duration::from_millis(700);

pub struct EditorState {
    id: Id,
    pub source: String,
    saved_source: String,
    pub diagnostics: Vec<Diagnostic>,
    pub status: CompileStatus,
    highlighted: Option<(u64, Arc<Vec<Span>>)>,
    /// A UTF-16 offset to move the caret to on the next frame (a Problems row was clicked).
    jump_to: Option<usize>,
    flash: Option<(usize, Instant)>,
    last_edit: Option<Instant>,
    pub(super) find: FindState,
    completion: Option<CompletionView>,
    /// Typing (or an explicit request) asked for suggestions; moving the caret withdraws it.
    completion_wanted: bool,
    completion_explicit: bool,
    last_caret: Option<usize>,
    caret_rect: Option<Rect>,
    /// A match to select (char range) on the next frame.
    select_to: Option<std::ops::Range<usize>>,
}

/// The open suggestion list and its highlighted row.
struct CompletionView {
    completion: Completion,
    selected: usize,
}

#[derive(Default)]
pub struct EditorOutput {
    /// The text changed this frame: compile it.
    pub changed: bool,
    pub save: bool,
}

fn text_hash(text: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}

impl EditorState {
    pub fn new(script_id: &str, source: &str, saved_source: &str) -> Self {
        Self {
            id: Id::new(("script-editor", script_id)),
            source: source.to_owned(),
            saved_source: saved_source.to_owned(),
            diagnostics: Vec::new(),
            status: CompileStatus::NotCompiled,
            highlighted: None,
            jump_to: None,
            flash: None,
            last_edit: None,
            find: FindState::default(),
            completion: None,
            completion_wanted: false,
            completion_explicit: false,
            last_caret: None,
            caret_rect: None,
            select_to: None,
        }
    }

    pub fn is_dirty(&self) -> bool {
        self.source != self.saved_source
    }

    pub fn mark_saved(&mut self) {
        self.saved_source = self.source.clone();
        self.last_edit = None;
    }

    pub fn saved_source(&self) -> &str {
        &self.saved_source
    }

    /// The pending draft, once typing has paused for `DRAFT_DELAY`.
    pub fn take_draft_due(&mut self, now: Instant) -> bool {
        match self.last_edit {
            Some(at) if now.duration_since(at) >= DRAFT_DELAY => {
                self.last_edit = None;
                self.is_dirty()
            }
            _ => false,
        }
    }

    pub fn has_pending_draft(&self) -> bool {
        self.last_edit.is_some()
    }

    /// Moves the caret to a diagnostic's start and flashes it, next frame.
    pub fn reveal(&mut self, diagnostic: &Diagnostic) {
        self.jump_to = Some(diagnostic.range.start.offset);
    }

    fn spans(&mut self) -> Arc<Vec<Span>> {
        let key = text_hash(&self.source);
        if let Some((cached, spans)) = &self.highlighted
            && *cached == key
        {
            return Arc::clone(spans);
        }
        let spans = Arc::new(syntax::classify(&self.source));
        self.highlighted = Some((key, Arc::clone(&spans)));
        spans
    }
}

/// "No problems", "1 warning", "2 errors" — the chip's text and look.
pub fn status_chip(status: CompileStatus, diagnostics: &[Diagnostic]) -> (Icon, Color32, String) {
    let errors = diagnostics
        .iter()
        .filter(|d| d.severity == DiagnosticSeverity::Error)
        .count();
    let warnings = diagnostics.len() - errors;
    let plural = |n: usize, noun: &str| format!("{n} {noun}{}", if n == 1 { "" } else { "s" });
    match status {
        CompileStatus::NotCompiled => (
            Icon::CircleDashed,
            theme::text_secondary(),
            "Not compiled".to_owned(),
        ),
        CompileStatus::Valid => (
            Icon::CheckCircleFill,
            Color32::from_rgb(0x30, 0xD1, 0x58),
            "No problems".to_owned(),
        ),
        CompileStatus::Warning => (
            Icon::TriangleFill,
            theme::YELLOW,
            plural(warnings.max(1), "warning"),
        ),
        CompileStatus::Error => (
            Icon::XmarkOctagonFill,
            theme::syntax::string(),
            plural(errors.max(1), "error"),
        ),
    }
}

pub fn show(ui: &mut Ui, state: &mut EditorState) -> EditorOutput {
    let mut output = EditorOutput::default();
    let full = ui.available_rect_before_wrap();
    ui.allocate_rect(full, Sense::hover());
    let bar = Rect::from_min_max(egui::pos2(full.min.x, full.max.y - BAR_HEIGHT), full.max);
    let area = Rect::from_min_max(full.min, egui::pos2(full.max.x, bar.min.y));
    handle_find_shortcuts(ui.ctx(), state);
    let find_height = if state.find.open {
        find::BAR_HEIGHT
    } else {
        0.0
    };
    let code = Rect::from_min_max(egui::pos2(area.min.x, area.min.y + find_height), area.max);
    let gutter = Rect::from_min_max(code.min, egui::pos2(code.min.x + GUTTER_WIDTH, code.max.y));
    let text_area = Rect::from_min_max(egui::pos2(gutter.max.x, code.min.y), code.max);

    ui.painter()
        .rect_filled(area, 0.0, theme::editor_background());
    ui.painter()
        .rect_filled(gutter, 0.0, theme::gutter_background());
    if state.find.open {
        let bar_rect =
            Rect::from_min_max(area.min, egui::pos2(area.max.x, area.min.y + find_height));
        run_find_bar(ui, bar_rect, state);
    }

    let mut edited = completion_keys(ui.ctx(), state);
    edited |= apply_assists(ui.ctx(), state);
    let rows = text_view(ui, state, text_area, &mut edited);
    edited |= update_completion(ui, state, edited);
    draw_signature(ui, state);
    paint_gutter(ui, gutter, &rows);
    output.changed = edited;
    if edited {
        state.last_edit = Some(Instant::now());
    }
    if state.last_edit.is_some() {
        ui.ctx().request_repaint_after(DRAFT_DELAY);
    }

    output.save = status_bar(ui, bar, state);
    output
}

// MARK: Completion

/// Handles the keys the suggestion list owns while it is open (↑ ↓ Return Tab Esc), and the
/// explicit invocations (`shortcut::COMPLETE`) always. Returns whether the text changed.
fn completion_keys(ctx: &egui::Context, state: &mut EditorState) -> bool {
    let focused = ctx.memory(|memory| memory.has_focus(state.id));
    if !focused {
        return false;
    }
    let mut accept = false;
    ctx.input_mut(|input| {
        let events = std::mem::take(&mut input.events);
        for event in events {
            let Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } = &event
            else {
                input.events.push(event);
                continue;
            };
            let plain = !modifiers.command && !modifiers.alt && !modifiers.ctrl && !modifiers.shift;
            if shortcut::COMPLETE.matches_for(shortcut::is_mac(), *key, *modifiers) {
                state.completion_wanted = true;
                state.completion_explicit = true;
                continue;
            }
            let Some(view) = state.completion.as_mut().filter(|_| plain) else {
                input.events.push(event);
                continue;
            };
            let count = view.completion.items.len();
            match key {
                Key::ArrowDown => view.selected = (view.selected + 1) % count,
                Key::ArrowUp => view.selected = (view.selected + count - 1) % count,
                Key::Enter | Key::Tab => accept = true,
                Key::Escape => {
                    state.completion = None;
                    state.completion_wanted = false;
                    state.completion_explicit = false;
                }
                _ => input.events.push(event),
            }
        }
    });
    accept && accept_completion(ctx, state)
}

/// Replaces the typed prefix with the highlighted suggestion. Returns whether the text changed.
fn accept_completion(ctx: &egui::Context, state: &mut EditorState) -> bool {
    let Some(view) = state.completion.take() else {
        return false;
    };
    state.completion_wanted = false;
    state.completion_explicit = false;
    let Some(item) = view.completion.items.get(view.selected) else {
        return false;
    };
    let range = view.completion.replace;
    let edit = Edit {
        range: range.clone(),
        replacement: item.label.clone(),
        selection: {
            let end = range.start + item.label.chars().count();
            end..end
        },
    };
    state.source = edit.apply(&state.source);
    if let Some(mut edit_state) = TextEditState::load(ctx, state.id) {
        edit_state
            .cursor
            .set_char_range(Some(CCursorRange::one(CCursor::new(edit.selection.start))));
        edit_state.store(ctx, state.id);
    }
    true
}

/// Recomputes the suggestions after the frame's edits and draws the list under the caret.
/// Returns whether a row click changed the text.
fn update_completion(ui: &mut Ui, state: &mut EditorState, edited: bool) -> bool {
    let ctx = ui.ctx().clone();
    let focused = ctx.memory(|memory| memory.has_focus(state.id));
    let caret = TextEditState::load(&ctx, state.id)
        .and_then(|edit| edit.cursor.char_range())
        .filter(|range| range.primary == range.secondary)
        .map(|range| range.primary.index.0);
    if edited {
        state.completion_wanted = true;
    } else if caret != state.last_caret && !state.completion_explicit {
        // The caret moved on its own (click, arrows): the suggestions no longer apply.
        state.completion_wanted = false;
    }
    state.last_caret = caret;
    let Some(caret) = caret.filter(|_| focused && state.completion_wanted) else {
        state.completion = None;
        return false;
    };
    let fresh = completion::complete(&state.source, caret, state.completion_explicit);
    state.completion = match (fresh, state.completion.take()) {
        (Some(next), Some(old)) => {
            let same = old.completion.items == next.items;
            Some(CompletionView {
                selected: if same {
                    old.selected.min(next.items.len() - 1)
                } else {
                    0
                },
                completion: next,
            })
        }
        (Some(next), None) => Some(CompletionView {
            completion: next,
            selected: 0,
        }),
        (None, _) => None,
    };
    let (Some(view), Some(caret_rect)) = (state.completion.as_ref(), state.caret_rect) else {
        return false;
    };
    let mut clicked = None;
    let selected = view.selected;
    egui::Area::new(Id::new("script-completion"))
        .order(egui::Order::Foreground)
        .fixed_pos(egui::pos2(caret_rect.left(), caret_rect.bottom() + 4.0))
        .show(&ctx, |ui| {
            egui::Frame::new()
                .fill(theme::chrome())
                .stroke(Stroke::new(1.0, theme::separator()))
                .corner_radius(6.0)
                .inner_margin(Margin::same(4))
                .show(ui, |ui| {
                    ui.set_width(412.0);
                    egui::ScrollArea::vertical()
                        .max_height(8.0 * 22.0)
                        .show(ui, |ui| {
                            ui.spacing_mut().item_spacing.y = 0.0;
                            for (index, item) in view.completion.items.iter().enumerate() {
                                let (rect, response) =
                                    ui.allocate_exact_size(Vec2::new(412.0, 22.0), Sense::click());
                                let on = index == selected;
                                if on {
                                    ui.painter().rect_filled(rect, 4.0, theme::SYSTEM_ACCENT);
                                    if ui.input(|i| {
                                        i.key_pressed(Key::ArrowDown) || i.key_pressed(Key::ArrowUp)
                                    }) {
                                        ui.scroll_to_rect(rect, None);
                                    }
                                } else if response.hovered() {
                                    ui.painter().rect_filled(rect, 4.0, theme::ink(14));
                                }
                                let ink = if on {
                                    Color32::WHITE
                                } else {
                                    theme::text_primary()
                                };
                                let dim = if on {
                                    Color32::WHITE.gamma_multiply(0.8)
                                } else {
                                    theme::text_secondary()
                                };
                                let badge = Rect::from_center_size(
                                    egui::pos2(rect.min.x + 14.0, rect.center().y),
                                    Vec2::splat(16.0),
                                );
                                ui.painter().rect_stroke(
                                    badge,
                                    3.0,
                                    Stroke::new(1.0, dim),
                                    egui::StrokeKind::Inside,
                                );
                                ui.painter().text(
                                    badge.center(),
                                    Align2::CENTER_CENTER,
                                    item.kind.badge().to_ascii_uppercase(),
                                    FontId::monospace(10.0),
                                    dim,
                                );
                                ui.painter().text(
                                    egui::pos2(rect.min.x + 30.0, rect.center().y),
                                    Align2::LEFT_CENTER,
                                    &item.label,
                                    FontId::monospace(12.0),
                                    ink,
                                );
                                ui.painter().text(
                                    egui::pos2(rect.max.x - 8.0, rect.center().y),
                                    Align2::RIGHT_CENTER,
                                    item.kind.detail(),
                                    FontId::proportional(11.0),
                                    dim,
                                );
                                if response.clicked() {
                                    clicked = Some(index);
                                }
                            }
                        });
                });
        });
    match clicked {
        Some(index) => {
            if let Some(view) = state.completion.as_mut() {
                view.selected = index;
            }
            accept_completion(&ctx, state)
        }
        None => false,
    }
}

/// The signature panel over the caret line: `ta.sma(source, length) → series float`, the
/// argument under the caret underlined. Quiet while the suggestion list is open.
fn draw_signature(ui: &mut Ui, state: &EditorState) {
    let ctx = ui.ctx().clone();
    if state.completion.is_some() || !ctx.memory(|memory| memory.has_focus(state.id)) {
        return;
    }
    let Some(caret_rect) = state.caret_rect else {
        return;
    };
    let Some(range) = TextEditState::load(&ctx, state.id).and_then(|edit| edit.cursor.char_range())
    else {
        return;
    };
    if range.primary != range.secondary {
        return;
    }
    let Some(help) = signature::help(&state.source, range.primary.index.0) else {
        return;
    };
    let mono = FontId::monospace(12.0);
    let plain = |color: Color32| egui::TextFormat::simple(mono.clone(), color);
    let mut job = egui::text::LayoutJob::default();
    job.append(
        &format!("{}(", help.name),
        0.0,
        plain(theme::text_primary()),
    );
    for (index, parameter) in help.parameters.iter().enumerate() {
        if index > 0 {
            job.append(", ", 0.0, plain(theme::text_primary()));
        }
        let mut format = plain(if Some(index) == help.active {
            Color32::WHITE
        } else {
            theme::text_primary()
        });
        if Some(index) == help.active {
            format.underline = Stroke::new(1.5, theme::SYSTEM_ACCENT);
        }
        job.append(parameter, 0.0, format);
    }
    job.append(")", 0.0, plain(theme::text_primary()));
    if !help.returns.is_empty() {
        job.append(
            &format!(" → {}", help.returns),
            0.0,
            plain(theme::text_secondary()),
        );
    }
    job.wrap.max_width = 540.0;
    egui::Area::new(Id::new("script-signature"))
        .order(egui::Order::Foreground)
        .interactable(false)
        .pivot(Align2::LEFT_BOTTOM)
        .fixed_pos(egui::pos2(caret_rect.left(), caret_rect.top() - 4.0))
        .show(&ctx, |ui| {
            egui::Frame::new()
                .fill(theme::chrome())
                .stroke(Stroke::new(1.0, theme::separator()))
                .corner_radius(6.0)
                .inner_margin(Margin::symmetric(10, 6))
                .show(ui, |ui| {
                    ui.add(egui::Label::new(job).wrap());
                });
        });
}

/// `shortcut::FIND` opens the find bar (seeded with a single-line selection); `FIND_NEXT` /
/// `FIND_PREVIOUS` step through matches.
fn handle_find_shortcuts(ctx: &egui::Context, state: &mut EditorState) {
    let (open, next, previous) = ctx.input(|i| {
        (
            shortcut::FIND.peek(i),
            shortcut::FIND_NEXT.peek(i),
            shortcut::FIND_PREVIOUS.peek(i),
        )
    });
    if open {
        // Like Xcode: Find with a single-line selection searches for that text.
        let selected = TextEditState::load(ctx, state.id)
            .and_then(|edit| edit.cursor.char_range())
            .map(|range| {
                let (a, b) = (range.primary.index.0, range.secondary.index.0);
                state
                    .source
                    .chars()
                    .skip(a.min(b))
                    .take(a.abs_diff(b))
                    .collect::<String>()
            })
            .filter(|text| !text.is_empty() && !text.contains('\n'));
        if let Some(text) = selected {
            state.find.query = text;
            state.find.current = 0;
        }
        state.find.open = true;
        state.find.needs_focus = true;
    } else if (next || previous) && state.find.open {
        step_match(state, next);
    }
}

fn step_match(state: &mut EditorState, forward: bool) {
    let found = find::matches(&state.source, &state.find.query);
    if let Some(index) = find::step(
        state.find.current.min(found.len().saturating_sub(1)),
        found.len(),
        forward,
    ) {
        state.find.current = index;
        state.select_to = Some(found[index].clone());
    }
}

fn run_find_bar(ui: &mut Ui, rect: Rect, state: &mut EditorState) {
    let count = find::matches(&state.source, &state.find.query).len();
    match find::show(ui, rect, &mut state.find, count) {
        FindEvent::None => {}
        FindEvent::QueryChanged => {
            let found = find::matches(&state.source, &state.find.query);
            let caret = TextEditState::load(ui.ctx(), state.id)
                .and_then(|edit| edit.cursor.char_range())
                .map_or(0, |range| {
                    range.primary.index.0.min(range.secondary.index.0)
                });
            state.find.current = find::nearest(&found, caret);
            state.select_to = found.get(state.find.current).cloned();
        }
        FindEvent::Next => step_match(state, true),
        FindEvent::Previous => step_match(state, false),
        FindEvent::Close => {
            state.find.open = false;
            ui.memory_mut(|memory| memory.request_focus(state.id));
        }
    }
}

/// What the gutter needs from the text view: each line's top and height on screen.
struct GutterRows {
    rows: Vec<(f32, f32)>,
}

fn text_view(ui: &mut Ui, state: &mut EditorState, area: Rect, edited: &mut bool) -> GutterRows {
    let id = state.id;
    let font = FontId::monospace(FONT_SIZE);
    let spans = state.spans();
    let current = text_hash(&state.source);

    if let Some(offset) = state.jump_to.take() {
        let index = syntax::utf16_to_char_index(&state.source, offset);
        let mut edit_state = TextEditState::load(ui.ctx(), id).unwrap_or_default();
        edit_state
            .cursor
            .set_char_range(Some(CCursorRange::one(CCursor::new(index))));
        edit_state.store(ui.ctx(), id);
        ui.memory_mut(|memory| memory.request_focus(id));
        state.flash = Some((index, Instant::now()));
        ui.ctx().request_repaint();
    }
    if let Some(range) = state.select_to.take() {
        let mut edit_state = TextEditState::load(ui.ctx(), id).unwrap_or_default();
        edit_state.cursor.set_char_range(Some(CCursorRange::two(
            CCursor::new(range.start),
            CCursor::new(range.end),
        )));
        edit_state.store(ui.ctx(), id);
        // The find field keeps focus so typing refines the search; the match is selected and
        // scrolled into view.
        state.flash = Some((range.start, Instant::now()));
        ui.ctx().request_repaint();
    }
    let flash = state.flash;
    let find_matches = if state.find.open {
        find::matches(&state.source, &state.find.query)
    } else {
        Vec::new()
    };
    let find_current = state.find.current;

    let mut layouter = |ui: &Ui, text: &dyn TextBuffer, _wrap: f32| {
        // While the buffer is what was classified, reuse it; mid-edit text is classified now.
        let fresh;
        let spans: &[Span] = if text_hash(text.as_str()) == current {
            &spans
        } else {
            fresh = syntax::classify(text.as_str());
            &fresh
        };
        let mut job = syntax::layout_job(text.as_str(), spans, font.clone(), theme::text_primary());
        job.wrap.max_width = f32::INFINITY;
        ui.fonts_mut(|fonts| fonts.layout_job(job))
    };

    let mut pane = ui.new_child(UiBuilder::new().max_rect(area));
    pane.set_clip_rect(area.intersect(ui.clip_rect()));
    let char_width = pane.fonts_mut(|fonts| fonts.glyph_width(&FontId::monospace(FONT_SIZE), ' '));
    let mut rows = GutterRows { rows: Vec::new() };
    let scroll = egui::ScrollArea::both()
        .id_salt(("script-editor-scroll", id))
        .auto_shrink([false, false])
        .show_viewport(&mut pane, |ui, viewport| {
            // Decorations that sit behind the text are painted into a slot reserved first.
            let behind = ui.painter().add(Shape::Noop);
            let visible_width = viewport.width().max(ui.available_width());
            let text_edit = egui::TextEdit::multiline(&mut state.source)
                .id(id)
                .font(font.clone())
                .frame(egui::Frame::NONE)
                .margin(Margin::same(TEXT_INSET as i8))
                .desired_width(f32::INFINITY)
                .min_size(Vec2::new(visible_width, viewport.height()))
                .lock_focus(true)
                .layouter(&mut layouter)
                .show(ui);
            if text_edit.response.response.changed() {
                *edited = true;
            }
            let galley = &text_edit.galley;
            let origin = text_edit.galley_pos;
            for placed in &galley.rows {
                rows.rows.push((origin.y + placed.pos.y, placed.row.size.y));
            }

            let caret = text_edit.cursor_range;
            state.caret_rect = caret.map(|range| {
                galley
                    .pos_from_cursor(range.primary)
                    .translate(origin.to_vec2())
            });
            let mut under = Vec::new();
            let clip = ui.clip_rect();
            let band_x = clip.x_range();
            // Current-line band (caret only) and indent guides, behind the text.
            let mut behind_shapes = Vec::new();
            if let Some(range) = caret
                && range.primary == range.secondary
                && let Some(row) = galley
                    .rows
                    .get(galley.layout_from_cursor(range.primary).row)
            {
                let top = origin.y + row.pos.y;
                behind_shapes.push(Shape::rect_filled(
                    Rect::from_x_y_ranges(band_x, top..=top + row.row.size.y),
                    0.0,
                    theme::ink(15),
                ));
            }
            for (index, span) in find_matches.iter().enumerate() {
                let a = galley.pos_from_cursor(CCursor::new(span.start));
                let b = galley.pos_from_cursor(CCursor::new(span.end));
                if (a.min.y - b.min.y).abs() < 1.0 {
                    let rect = Rect::from_min_max(a.min, egui::pos2(b.min.x, a.max.y))
                        .translate(origin.to_vec2());
                    let alpha = if index == find_current { 0.6 } else { 0.28 };
                    behind_shapes.push(Shape::rect_filled(
                        rect,
                        2.0,
                        theme::with_alpha(theme::YELLOW, alpha),
                    ));
                }
            }
            // Same-identifier highlights and the bracket pair, only while the caret is a point.
            if let Some(range) = caret
                && range.primary == range.secondary
            {
                let at = range.primary.index.0;
                let span_rect = |span: std::ops::Range<usize>| {
                    let a = galley.pos_from_cursor(CCursor::new(span.start));
                    let b = galley.pos_from_cursor(CCursor::new(span.end));
                    ((a.min.y - b.min.y).abs() < 1.0).then(|| {
                        Rect::from_min_max(a.min, egui::pos2(b.min.x, a.max.y))
                            .translate(origin.to_vec2())
                    })
                };
                for (index, span) in edit_ops::occurrences(&state.source, at)
                    .into_iter()
                    .enumerate()
                {
                    if let Some(rect) = span_rect(span) {
                        // The token under the caret is stronger (0.7) than the others (0.45).
                        let alpha = if index == 0 { 178 } else { 115 };
                        behind_shapes.push(Shape::rect_filled(
                            rect,
                            2.0,
                            Color32::from_rgba_unmultiplied(63, 99, 148, alpha),
                        ));
                    }
                }
                if let Some((position, partner)) = edit_ops::bracket_match(&state.source, at) {
                    let (color, spans) = match partner {
                        Some(other) => (theme::ink(56), vec![position, other]),
                        None => (
                            theme::with_alpha(theme::syntax::string(), 0.3),
                            vec![position],
                        ),
                    };
                    for position in spans {
                        if let Some(rect) = span_rect(position..position + 1) {
                            behind_shapes.push(Shape::rect_filled(rect, 2.0, color));
                        }
                    }
                }
            }
            behind_shapes.extend(indent_guides(
                &state.source,
                galley,
                origin,
                char_width,
                caret.map(|c| c.primary.index.0),
            ));
            ui.painter().set(behind, Shape::Vec(behind_shapes));

            // Diagnostic underlines and their merged hover messages (over the text).
            for diagnostic in &state.diagnostics {
                if let Some(rect) = underline_rect(&state.source, galley, origin, diagnostic) {
                    let color = match diagnostic.severity {
                        DiagnosticSeverity::Error => theme::syntax::string(),
                        DiagnosticSeverity::Warning => theme::YELLOW,
                    };
                    ui.painter().line_segment(
                        [rect.left_bottom(), rect.right_bottom()],
                        Stroke::new(1.0, color),
                    );
                    under.push((rect, diagnostic.message.clone()));
                }
            }
            if let Some(pointer) = ui.input(|i| i.pointer.hover_pos())
                && text_edit.response.response.hovered()
            {
                let messages: Vec<&str> = under
                    .iter()
                    .filter(|(rect, _)| rect.expand2(Vec2::new(0.0, 2.0)).contains(pointer))
                    .map(|(_, message)| message.as_str())
                    .collect();
                if !messages.is_empty() {
                    text_edit
                        .response
                        .response
                        .clone()
                        .on_hover_text(messages.join("\n"));
                }
            }

            // Flash the revealed position, and bring it into view.
            if let Some((index, started)) = flash {
                let age = started.elapsed();
                let cursor_rect = galley.pos_from_cursor(CCursor::new(index));
                let placed = cursor_rect.translate(origin.to_vec2());
                if age < FLASH {
                    if age.is_zero() || age < Duration::from_millis(40) {
                        ui.scroll_to_rect(
                            placed.expand2(Vec2::new(0.0, 40.0)),
                            Some(Align::Center),
                        );
                    }
                    let fade = 1.0 - age.as_secs_f32() / FLASH.as_secs_f32();
                    let size = Vec2::new(char_width * 1.2, placed.height());
                    ui.painter().rect_filled(
                        Rect::from_min_size(placed.min - Vec2::new(2.0, 0.0), size).expand(3.0),
                        4.0,
                        theme::with_alpha(theme::YELLOW, 0.35 * fade),
                    );
                    ui.ctx().request_repaint();
                }
            }
        });
    if state.flash.is_some_and(|(_, at)| at.elapsed() >= FLASH) {
        state.flash = None;
    }
    let _ = scroll;
    rows
}

fn paint_gutter(ui: &Ui, gutter: Rect, rows: &GutterRows) {
    let painter = ui.painter().with_clip_rect(gutter);
    let font = FontId::monospace(LINE_NUMBER_SIZE);
    for (index, (top, height)) in rows.rows.iter().enumerate() {
        if top + height < gutter.min.y || *top > gutter.max.y {
            continue;
        }
        painter.text(
            egui::pos2(gutter.max.x - 8.0, top + height / 2.0),
            Align2::RIGHT_CENTER,
            (index + 1).to_string(),
            font.clone(),
            theme::text_secondary(),
        );
    }
}

/// Leading-space level of each line, with blank lines keeping only the guides both neighbours
/// share.
fn indent_levels(source: &str) -> Vec<usize> {
    let lines: Vec<&str> = source.split('\n').collect();
    let raw: Vec<Option<usize>> = lines
        .iter()
        .map(|line| {
            if line.trim().is_empty() {
                None
            } else {
                let mut width = 0;
                for c in line.chars() {
                    match c {
                        ' ' => width += 1,
                        '\t' => width += 4,
                        _ => break,
                    }
                }
                Some(width / 4)
            }
        })
        .collect();
    (0..raw.len())
        .map(|i| match raw[i] {
            Some(level) => level,
            None => {
                let before = raw[..i].iter().rev().find_map(|l| *l);
                let after = raw[i + 1..].iter().find_map(|l| *l);
                match (before, after) {
                    (Some(a), Some(b)) => a.min(b),
                    _ => 0,
                }
            }
        })
        .collect()
}

fn indent_guides(
    source: &str,
    galley: &egui::Galley,
    origin: egui::Pos2,
    char_width: f32,
    caret_index: Option<usize>,
) -> Vec<Shape> {
    let levels = indent_levels(source);
    let caret_line = caret_index.map(|c| source.chars().take(c).filter(|&ch| ch == '\n').count());
    let active_level = caret_line.and_then(|line| levels.get(line).copied());
    // The active guide runs over the contiguous lines around the caret that still have it.
    let active_span = caret_line
        .zip(active_level)
        .filter(|(_, l)| *l > 0)
        .map(|(line, level)| {
            let mut start = line;
            while start > 0 && levels[start - 1] >= level {
                start -= 1;
            }
            let mut end = line;
            while end + 1 < levels.len() && levels[end + 1] >= level {
                end += 1;
            }
            (start, end, level)
        });
    let mut shapes = Vec::new();
    for (line, level) in levels.iter().enumerate() {
        let Some(row) = galley.rows.get(line) else {
            break;
        };
        let top = origin.y + row.pos.y;
        let bottom = top + row.row.size.y;
        for guide in 1..=*level {
            let x = origin.x + char_width * (4 * (guide - 1)) as f32 + 0.5;
            let active = active_span.is_some_and(|(s, e, l)| guide == l && line >= s && line <= e);
            let color = if active {
                theme::text_secondary().gamma_multiply(0.6)
            } else {
                theme::ink(22)
            };
            shapes.push(Shape::line_segment(
                [egui::pos2(x, top), egui::pos2(x, bottom)],
                Stroke::new(1.0, color),
            ));
        }
    }
    shapes
}

/// The screen rect a diagnostic underlines: its start to its end on the first line it touches
/// (one character wide when empty).
fn underline_rect(
    source: &str,
    galley: &egui::Galley,
    origin: egui::Pos2,
    diagnostic: &Diagnostic,
) -> Option<Rect> {
    let start = syntax::utf16_to_char_index(source, diagnostic.range.start.offset);
    let mut end = syntax::utf16_to_char_index(source, diagnostic.range.end.offset).max(start);
    // Underline only the first line of a multi-line range.
    let line_end = source
        .chars()
        .skip(start)
        .position(|c| c == '\n')
        .map_or(source.chars().count(), |n| start + n);
    end = end.min(line_end);
    if end == start {
        end = (start + 1).min(line_end.max(start + 1));
    }
    let a = galley.pos_from_cursor(CCursor::new(start));
    let b = galley.pos_from_cursor(CCursor::new(end));
    if (a.min.y - b.min.y).abs() > 1.0 {
        return None;
    }
    Some(
        Rect::from_min_max(
            egui::pos2(a.min.x, a.min.y),
            egui::pos2(b.min.x.max(a.min.x + 4.0), a.max.y),
        )
        .translate(origin.to_vec2()),
    )
}

// MARK: Editing assists

/// Handles Return, Tab, Backspace, paired typing and the line shortcuts before the text field
/// sees them. Returns whether any edit changed the text.
fn apply_assists(ctx: &egui::Context, state: &mut EditorState) -> bool {
    let id = state.id;
    if !ctx.memory(|memory| memory.has_focus(id)) {
        return false;
    }
    let Some(mut edit_state) = TextEditState::load(ctx, id) else {
        return false;
    };
    let mut selection = edit_state
        .cursor
        .char_range()
        .map(|range| {
            let (a, b) = (range.primary.index.0, range.secondary.index.0);
            a.min(b)..a.max(b)
        })
        .unwrap_or(0..0);
    let mut changed = false;
    let mut handled_any = false;

    let events = ctx.input(|input| input.events.clone());
    let mut remaining = Vec::with_capacity(events.len());
    for event in events {
        let edit = match &event {
            Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } => key_edit(&state.source, selection.clone(), *key, *modifiers),
            Event::Text(text) => {
                edit_ops::typed(&Context::new(&state.source, selection.clone()), text)
            }
            _ => None,
        };
        match edit {
            Some(edit) => {
                state.source = edit.apply(&state.source);
                selection = edit.selection.clone();
                changed = true;
                handled_any = true;
            }
            None => remaining.push(event),
        }
    }
    if handled_any {
        ctx.input_mut(|input| input.events = remaining);
        edit_state.cursor.set_char_range(Some(CCursorRange::two(
            CCursor::new(selection.start),
            CCursor::new(selection.end),
        )));
        edit_state.store(ctx, id);
    }
    changed
}

fn key_edit(
    source: &str,
    selection: std::ops::Range<usize>,
    key: Key,
    modifiers: Modifiers,
) -> Option<Edit> {
    let context = Context::new(source, selection);
    let plain = !modifiers.command && !modifiers.alt && !modifiers.ctrl;
    let is_mac = shortcut::is_mac();
    let chord = |shortcut: shortcut::Shortcut| shortcut.matches_for(is_mac, key, modifiers);
    match key {
        Key::Enter if plain && !modifiers.shift => Some(edit_ops::newline(&context)),
        Key::Tab if plain && !modifiers.shift => Some(edit_ops::indent(&context)),
        Key::Tab if plain && modifiers.shift => edit_ops::outdent(&context),
        Key::Backspace if plain && !modifiers.shift => edit_ops::backspace(&context),
        Key::Slash if chord(shortcut::TOGGLE_COMMENT) => edit_ops::toggle_comment(&context),
        Key::ArrowUp | Key::ArrowDown => {
            let down = key == Key::ArrowDown;
            if chord(shortcut::DUPLICATE_LINE_UP) || chord(shortcut::DUPLICATE_LINE_DOWN) {
                edit_ops::duplicate(&context, down)
            } else if chord(shortcut::MOVE_LINE_UP) || chord(shortcut::MOVE_LINE_DOWN) {
                edit_ops::move_lines(&context, down)
            } else {
                None
            }
        }
        _ => None,
    }
}

// MARK: Status bar

/// The chip and the Save button. Returns whether Save was clicked.
fn status_bar(ui: &mut Ui, bar: Rect, state: &EditorState) -> bool {
    ui.painter().rect_filled(bar, 0.0, theme::background());
    ui.painter().line_segment(
        [bar.left_top(), bar.right_top()],
        Stroke::new(1.0, theme::separator()),
    );
    let mut save = false;
    let mut inner = ui.new_child(
        UiBuilder::new()
            .max_rect(bar.shrink2(Vec2::new(12.0, 8.0)))
            .layout(egui::Layout::left_to_right(Align::Center)),
    );
    let (icon, color, text) = status_chip(state.status, &state.diagnostics);
    inner.spacing_mut().item_spacing.x = 5.0;
    icons::inline(&mut inner, icon, 12.0, color);
    inner
        .label(egui::RichText::new(text).size(10.0).color(color))
        .on_hover_text("Compile status of the code in the editor");
    inner.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
        let button = egui::Button::new(egui::RichText::new("Save").size(13.0));
        if ui
            .add(button)
            .on_hover_text(shortcut::SAVE.hint("Save the script"))
            .clicked()
        {
            save = true;
        }
    });
    save
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pine::token::{DiagnosticCategory, SourcePosition, SourceRange};

    fn diagnostic(
        severity: DiagnosticSeverity,
        line: usize,
        column: usize,
        offset: usize,
    ) -> Diagnostic {
        Diagnostic {
            code: "PINE3001".into(),
            severity,
            category: DiagnosticCategory::Semantic,
            message: "message".into(),
            range: SourceRange::new(
                SourcePosition::new(line, column, offset),
                SourcePosition::new(line, column + 1, offset + 1),
            ),
        }
    }

    #[test]
    fn the_chip_reads_like_swift() {
        let error = diagnostic(DiagnosticSeverity::Error, 1, 1, 0);
        let warning = diagnostic(DiagnosticSeverity::Warning, 1, 1, 0);
        assert_eq!(
            status_chip(CompileStatus::NotCompiled, &[]).2,
            "Not compiled"
        );
        assert_eq!(status_chip(CompileStatus::Valid, &[]).2, "No problems");
        assert_eq!(
            status_chip(CompileStatus::Warning, std::slice::from_ref(&warning)).2,
            "1 warning"
        );
        assert_eq!(
            status_chip(CompileStatus::Warning, &[warning.clone(), warning]).2,
            "2 warnings"
        );
        assert_eq!(
            status_chip(CompileStatus::Error, &[error.clone(), error]).2,
            "2 errors"
        );
    }

    #[test]
    fn blank_lines_keep_only_shared_guides() {
        let levels = indent_levels("a\n    b\n        c\n\n    d\n\n");
        assert_eq!(levels, [0, 1, 2, 1, 1, 0, 0]);
    }

    #[test]
    fn dirty_tracking_and_draft_debounce() {
        let mut state = EditorState::new("s", "plot(close)", "plot(close)");
        assert!(!state.is_dirty());
        state.source.push('\n');
        assert!(state.is_dirty());
        let start = Instant::now();
        state.last_edit = Some(start);
        assert!(!state.take_draft_due(start + Duration::from_millis(400)));
        assert!(state.take_draft_due(start + DRAFT_DELAY + Duration::from_millis(10)));
        assert!(
            !state.take_draft_due(start + Duration::from_secs(30)),
            "once per edit"
        );
        state.mark_saved();
        assert!(!state.is_dirty());
    }

    #[test]
    fn the_editor_draws_headlessly_with_problems_and_a_jump() {
        let mut state = EditorState::new("s", "//@version=6\nindicator(\"x\")\nplot(close)\n", "");
        state.diagnostics = vec![diagnostic(DiagnosticSeverity::Error, 3, 1, 26)];
        state.status = CompileStatus::Error;
        let ctx = egui::Context::default();
        for frame in 0..3 {
            if frame == 1 {
                let d = state.diagnostics[0].clone();
                state.reveal(&d);
            }
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                let result = show(ui, &mut state);
                assert!(!result.save);
            });
            output.textures_delta.clear();
        }
    }

    #[test]
    fn the_find_bar_steps_through_matches_and_draws() {
        let mut state = EditorState::new("s", "plot(close)\nplot(open)\nplot(high)\n", "");
        state.find.open = true;
        state.find.query = "PLOT".into();
        step_match(&mut state, true);
        assert_eq!(state.find.current, 1);
        assert_eq!(state.select_to, Some(12..16));
        step_match(&mut state, true);
        step_match(&mut state, true);
        assert_eq!(state.find.current, 0, "wraps past the last match");
        step_match(&mut state, false);
        assert_eq!(state.find.current, 2, "and back");
        state.find.query = "nothing".into();
        step_match(&mut state, true);
        assert_eq!(state.find.current, 2, "no matches: nowhere to go");

        state.find.query = "plot".into();
        let ctx = egui::Context::default();
        for _ in 0..3 {
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                show(ui, &mut state);
            });
            output.textures_delta.clear();
        }
    }

    #[test]
    fn accepting_a_suggestion_replaces_the_typed_prefix() {
        let mut state = EditorState::new("s", "x = ta.pi", "");
        let caret = state.source.chars().count();
        let completion = completion::complete(&state.source, caret, false).expect("suggestions");
        assert_eq!(completion.items[0].label, "ta.pivothigh");
        state.completion = Some(CompletionView {
            completion,
            selected: 1,
        });
        let ctx = egui::Context::default();
        assert!(accept_completion(&ctx, &mut state));
        assert_eq!(state.source, "x = ta.pivotlow");
        assert!(state.completion.is_none());
        assert!(
            !accept_completion(&ctx, &mut state),
            "nothing left to accept"
        );
    }

    #[test]
    fn the_editor_draws_with_suggestions_open() {
        let mut state = EditorState::new("s", "plot(cl", "");
        state.completion_wanted = true;
        let ctx = egui::Context::default();
        for _ in 0..3 {
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                show(ui, &mut state);
            });
            output.textures_delta.clear();
        }
    }

    #[test]
    fn the_editor_draws_a_signature_panel_inside_a_call() {
        let mut state = EditorState::new("s", "plot(close, ", "");
        let ctx = egui::Context::default();
        // Put the caret at the end of the text and focus the editor, then draw a few frames.
        let mut edit = TextEditState::default();
        edit.cursor
            .set_char_range(Some(CCursorRange::one(CCursor::new(12usize))));
        edit.store(&ctx, state.id);
        ctx.memory_mut(|memory| memory.request_focus(state.id));
        for _ in 0..3 {
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                show(ui, &mut state);
            });
            output.textures_delta.clear();
        }
        assert!(signature::help(&state.source, 12).is_some());
        assert!(
            ctx.memory(|memory| memory.area_rect(Id::new("script-signature")))
                .is_some(),
            "the panel was laid out"
        );
    }
}
