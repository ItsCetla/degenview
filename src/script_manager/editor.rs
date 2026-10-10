//! The code editor pane: a line-numbered monospaced text field with token colouring, diagnostic
//! underlines, the current-line band, indent guides and the Swift editing assists, above the
//! status/Save bar. Ported from `ScriptEditorView` and `LineNumberedTextEditorView`.
//!
//! A pure view: it never compiles. The host feeds `diagnostics` and `status` back after a
//! change (`EditorOutput::changed`).

use super::edit_ops::{self, Context, Edit};
use super::model::CompileStatus;
use super::syntax::{self, Span};
use crate::pine::token::{Diagnostic, DiagnosticSeverity};
use crate::ui::icons::{self, Icon};
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
            theme::TEXT_SECONDARY,
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
            theme::syntax::STRING,
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
    let gutter = Rect::from_min_max(area.min, egui::pos2(area.min.x + GUTTER_WIDTH, area.max.y));
    let text_area = Rect::from_min_max(egui::pos2(gutter.max.x, area.min.y), area.max);

    ui.painter()
        .rect_filled(area, 0.0, theme::EDITOR_BACKGROUND);
    ui.painter()
        .rect_filled(gutter, 0.0, theme::GUTTER_BACKGROUND);

    let mut edited = apply_assists(ui.ctx(), state);
    let rows = text_view(ui, state, text_area, &mut edited);
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
    let flash = state.flash;

    let mut layouter = |ui: &Ui, text: &dyn TextBuffer, _wrap: f32| {
        // While the buffer is what was classified, reuse it; mid-edit text is classified now.
        let fresh;
        let spans: &[Span] = if text_hash(text.as_str()) == current {
            &spans
        } else {
            fresh = syntax::classify(text.as_str());
            &fresh
        };
        let mut job = syntax::layout_job(text.as_str(), spans, font.clone(), theme::TEXT_PRIMARY);
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
                    theme::white_alpha(15),
                ));
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
                        DiagnosticSeverity::Error => theme::syntax::STRING,
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
            theme::TEXT_SECONDARY,
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
                theme::TEXT_SECONDARY.gamma_multiply(0.6)
            } else {
                theme::white_alpha(22)
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
    match key {
        Key::Enter if plain && !modifiers.shift => Some(edit_ops::newline(&context)),
        Key::Tab if plain && !modifiers.shift => Some(edit_ops::indent(&context)),
        Key::Tab if plain && modifiers.shift => edit_ops::outdent(&context),
        Key::Backspace if plain && !modifiers.shift => edit_ops::backspace(&context),
        Key::Slash if modifiers.command && !modifiers.alt => edit_ops::toggle_comment(&context),
        Key::ArrowUp | Key::ArrowDown if modifiers.alt && !modifiers.command => {
            let down = key == Key::ArrowDown;
            if modifiers.shift {
                edit_ops::duplicate(&context, down)
            } else {
                edit_ops::move_lines(&context, down)
            }
        }
        _ => None,
    }
}

// MARK: Status bar

/// The chip and the Save button. Returns whether Save was clicked.
fn status_bar(ui: &mut Ui, bar: Rect, state: &EditorState) -> bool {
    ui.painter().rect_filled(bar, 0.0, theme::BACKGROUND);
    ui.painter().line_segment(
        [bar.left_top(), bar.right_top()],
        Stroke::new(1.0, theme::SEPARATOR),
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
            .on_hover_text("Save the script (⌘S)")
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
}
