//! The Pine script editor: a name field, a monospaced text area with line numbers and syntax
//! colouring, the diagnostics list, and the actions that act on the script — Save, Add to Chart,
//! Create Alert. A pure view: it never compiles or runs anything. It asks for a compile
//! (`PineEditorAction::CompileRequested`) a moment after typing pauses and shows the diagnostics
//! the host feeds back; the Pine session does the rest.
//!
//! Colouring comes from the real lexer, so strings, numbers, keywords and calls are classified
//! exactly as the compiler will read them; `//` comments (which the lexer drops) are found by a
//! small scan that respects string tokens.

// Transient: the Script Manager hosts this once the Pine session compiles.
#![allow(dead_code)]

use crate::pine::lexer::Lexer;
use crate::pine::token::{Diagnostic, DiagnosticSeverity, TokenKind};
use eframe::egui::{
    self, Color32, FontId, RichText, TextFormat, Vec2,
    text::LayoutJob,
    text::{CCursor, CCursorRange},
    text_edit::TextEditState,
};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// How long typing must pause before a compile is requested.
pub const COMPILE_DELAY: Duration = Duration::from_millis(500);

// MARK: Highlighting

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Highlight {
    Keyword,
    Number,
    Text,
    Color,
    Comment,
    Call,
    Plain,
}

impl Highlight {
    pub fn color(self) -> Color32 {
        match self {
            Self::Keyword => Color32::from_rgb(207, 142, 255),
            Self::Number => Color32::from_rgb(120, 190, 255),
            Self::Text => Color32::from_rgb(130, 220, 160),
            Self::Color => Color32::from_rgb(255, 170, 90),
            Self::Comment => Color32::from_gray(120),
            Self::Call => Color32::from_rgb(255, 214, 120),
            Self::Plain => Color32::from_gray(215),
        }
    }
}

/// A coloured run of the source, in UTF-8 byte offsets.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Span {
    pub range: Range<usize>,
    pub highlight: Highlight,
}

/// Byte offset of every UTF-16 offset in `source`, plus one past the end.
pub fn utf16_to_byte_offsets(source: &str) -> Vec<usize> {
    let mut offsets = Vec::with_capacity(source.len() + 1);
    for (byte, character) in source.char_indices() {
        for _ in 0..character.len_utf16() {
            offsets.push(byte);
        }
    }
    offsets.push(source.len());
    offsets
}

/// The character index (what egui's cursor counts) at a UTF-16 offset.
pub fn utf16_to_char_index(source: &str, utf16: usize) -> usize {
    let mut units = 0;
    for (index, character) in source.chars().enumerate() {
        if units >= utf16 {
            return index;
        }
        units += character.len_utf16();
    }
    source.chars().count()
}

/// Classifies the source with the Pine lexer. Spans are sorted, non-overlapping and cover only
/// what is coloured; everything else is plain.
pub fn highlight_spans(source: &str) -> Vec<Span> {
    let result = Lexer::new(source).lex();
    let offsets = utf16_to_byte_offsets(source);
    let byte = |utf16: usize| offsets.get(utf16).copied().unwrap_or(source.len());
    let mut spans: Vec<Span> = Vec::new();
    let mut strings: Vec<Range<usize>> = Vec::new();
    for (index, token) in result.tokens.iter().enumerate() {
        let range = byte(token.range.start.offset)..byte(token.range.end.offset);
        if range.start >= range.end {
            continue;
        }
        let highlight = match &token.kind {
            TokenKind::Number { .. } => Highlight::Number,
            TokenKind::String(_) => {
                strings.push(range.clone());
                Highlight::Text
            }
            TokenKind::Color(_) => Highlight::Color,
            TokenKind::If
            | TokenKind::Else
            | TokenKind::Var
            | TokenKind::Varip
            | TokenKind::For
            | TokenKind::Break
            | TokenKind::Continue
            | TokenKind::While
            | TokenKind::Switch
            | TokenKind::And
            | TokenKind::Or
            | TokenKind::Not
            | TokenKind::Bool(_)
            | TokenKind::Na
            | TokenKind::TypeKeyword(_) => Highlight::Keyword,
            TokenKind::Identifier(_)
                if matches!(
                    result.tokens.get(index + 1).map(|next| &next.kind),
                    Some(TokenKind::LeftParen)
                ) =>
            {
                Highlight::Call
            }
            _ => continue,
        };
        spans.push(Span { range, highlight });
    }
    // `//` comments: the lexer drops them, so find each outside any string.
    let mut line_start = 0;
    for line in source.split('\n') {
        let mut search = 0;
        while let Some(found) = line[search..].find("//") {
            let at = line_start + search + found;
            if strings.iter().any(|string| string.contains(&at)) {
                search += found + 2;
                continue;
            }
            spans.push(Span {
                range: at..line_start + line.len(),
                highlight: Highlight::Comment,
            });
            break;
        }
        line_start += line.len() + 1;
    }
    spans.sort_by_key(|span| span.range.start);
    // A comment that begins inside an already-coloured run (a stray `//` token boundary) wins.
    spans.dedup_by(|later, earlier| later.range.start < earlier.range.end);
    spans
}

/// The source as a coloured layout.
pub fn layout_job(source: &str, spans: &[Span], font: FontId) -> LayoutJob {
    let mut job = LayoutJob::default();
    let mut cursor = 0;
    let format = |highlight: Highlight| TextFormat::simple(font.clone(), highlight.color());
    for span in spans {
        if span.range.start < cursor || span.range.end > source.len() {
            continue;
        }
        if span.range.start > cursor {
            job.append(
                &source[cursor..span.range.start],
                0.0,
                format(Highlight::Plain),
            );
        }
        job.append(&source[span.range.clone()], 0.0, format(span.highlight));
        cursor = span.range.end;
    }
    if cursor < source.len() {
        job.append(&source[cursor..], 0.0, format(Highlight::Plain));
    }
    job
}

// MARK: Diagnostics

/// "2 errors · 1 warning" or "No problems".
pub fn diagnostics_summary(diagnostics: &[Diagnostic]) -> String {
    let errors = diagnostics
        .iter()
        .filter(|d| d.severity == DiagnosticSeverity::Error)
        .count();
    let warnings = diagnostics.len() - errors;
    let plural =
        |count: usize, noun: &str| format!("{count} {noun}{}", if count == 1 { "" } else { "s" });
    match (errors, warnings) {
        (0, 0) => "No problems".to_owned(),
        (errors, 0) => plural(errors, "error"),
        (0, warnings) => plural(warnings, "warning"),
        (errors, warnings) => format!(
            "{} · {}",
            plural(errors, "error"),
            plural(warnings, "warning")
        ),
    }
}

pub fn has_errors(diagnostics: &[Diagnostic]) -> bool {
    diagnostics
        .iter()
        .any(|d| d.severity == DiagnosticSeverity::Error)
}

/// "Line 3, column 7".
pub fn location_text(diagnostic: &Diagnostic) -> String {
    format!(
        "Line {}, column {}",
        diagnostic.range.start.line, diagnostic.range.start.column
    )
}

/// How many lines the source has (a trailing newline does not add one).
pub fn line_count(source: &str) -> usize {
    source
        .strip_suffix('\n')
        .unwrap_or(source)
        .split('\n')
        .count()
        .max(1)
}

fn text_hash(text: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}

// MARK: State

pub struct PineEditorState {
    pub name: String,
    pub source: String,
    /// What was last saved; the editor is dirty when `source` or `name` differ from it.
    saved_name: String,
    saved_source: String,
    /// The last change, waiting for typing to pause before a compile is requested.
    pending_compile: Option<Instant>,
    /// A UTF-16 offset to move the caret to on the next frame.
    jump_to: Option<usize>,
    id: egui::Id,
    /// The colouring of the text as of `highlighted`'s hash, so an idle frame does not re-lex.
    highlighted: Option<(u64, Arc<Vec<Span>>)>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PineEditorAction {
    /// Typing paused: compile `source` and feed the diagnostics back.
    CompileRequested,
    Save,
    AddToChart,
    CreateAlert,
}

pub struct PineEditorInput<'a> {
    pub diagnostics: &'a [Diagnostic],
    /// A chart is focused to add the indicator to.
    pub has_chart: bool,
    /// The compiled script has alert calls to create an alert from.
    pub has_alerts: bool,
    pub is_compiling: bool,
}

impl PineEditorState {
    pub fn new(id: u64, name: &str, source: &str) -> Self {
        Self {
            name: name.to_owned(),
            source: source.to_owned(),
            saved_name: name.to_owned(),
            saved_source: source.to_owned(),
            pending_compile: Some(Instant::now()),
            jump_to: None,
            id: egui::Id::new(("pine-editor", id)),
            highlighted: None,
        }
    }

    pub fn is_dirty(&self) -> bool {
        self.name != self.saved_name || self.source != self.saved_source
    }

    /// The host saved the script.
    pub fn mark_saved(&mut self) {
        self.saved_name = self.name.clone();
        self.saved_source = self.source.clone();
    }

    /// Replaces the text (for example after the host reloaded the script); asks for a compile.
    pub fn set_source(&mut self, source: &str) {
        self.source = source.to_owned();
        self.pending_compile = Some(Instant::now());
    }

    fn note_edit(&mut self, now: Instant) {
        self.pending_compile = Some(now);
    }

    /// Whether a compile should be requested now: typing has paused for `COMPILE_DELAY`.
    pub fn take_compile_request(&mut self, now: Instant) -> bool {
        match self.pending_compile {
            Some(since) if now.duration_since(since) >= COMPILE_DELAY => {
                self.pending_compile = None;
                true
            }
            _ => false,
        }
    }

    pub fn is_compile_pending(&self) -> bool {
        self.pending_compile.is_some()
    }

    /// The colouring of the current text, lexed only when the text changed since last asked.
    pub fn spans(&mut self) -> Arc<Vec<Span>> {
        let key = text_hash(&self.source);
        if let Some((cached, spans)) = &self.highlighted
            && *cached == key
        {
            return Arc::clone(spans);
        }
        let spans = Arc::new(highlight_spans(&self.source));
        self.highlighted = Some((key, Arc::clone(&spans)));
        spans
    }

    /// Moves the caret to a diagnostic's start on the next frame.
    pub fn go_to(&mut self, diagnostic: &Diagnostic) {
        self.jump_to = Some(diagnostic.range.start.offset);
    }
}

// MARK: View

pub fn show(
    ui: &mut egui::Ui,
    state: &mut PineEditorState,
    input: &PineEditorInput,
) -> Vec<PineEditorAction> {
    let mut actions = Vec::new();
    toolbar(ui, state, input, &mut actions);
    ui.separator();
    let diagnostics_height = if input.diagnostics.is_empty() {
        28.0
    } else {
        120.0
    };
    let editor_height = (ui.available_height() - diagnostics_height - 8.0).max(120.0);
    editor(ui, state, editor_height);
    ui.separator();
    diagnostics_panel(ui, state, input, diagnostics_height);
    if state.take_compile_request(Instant::now()) {
        actions.push(PineEditorAction::CompileRequested);
    }
    if state.is_compile_pending() {
        ui.ctx().request_repaint_after(Duration::from_millis(100));
    }
    actions
}

fn toolbar(
    ui: &mut egui::Ui,
    state: &mut PineEditorState,
    input: &PineEditorInput,
    actions: &mut Vec<PineEditorAction>,
) {
    ui.horizontal(|ui| {
        ui.add(
            egui::TextEdit::singleline(&mut state.name)
                .hint_text("Script name")
                .desired_width(220.0),
        );
        if state.is_dirty() {
            ui.label(
                RichText::new("● unsaved")
                    .small()
                    .color(Color32::from_rgb(255, 159, 10)),
            );
        }
        let (text, color) = status(input);
        ui.label(RichText::new(text).small().color(color));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let runnable = !has_errors(input.diagnostics) && !input.is_compiling;
            if ui
                .add_enabled(
                    runnable && input.has_alerts,
                    egui::Button::new("Create Alert"),
                )
                .on_hover_text(if input.has_alerts {
                    "Notify when this script's alert() fires"
                } else {
                    "The script has no alert() or alertcondition() calls"
                })
                .clicked()
            {
                actions.push(PineEditorAction::CreateAlert);
            }
            if ui
                .add_enabled(
                    runnable && input.has_chart,
                    egui::Button::new(RichText::new("Add to Chart").strong()),
                )
                .on_hover_text("Apply this script to the focused chart")
                .clicked()
            {
                actions.push(PineEditorAction::AddToChart);
            }
            if ui
                .add_enabled(state.is_dirty(), egui::Button::new("Save"))
                .clicked()
            {
                actions.push(PineEditorAction::Save);
            }
        });
    });
}

fn status(input: &PineEditorInput) -> (String, Color32) {
    if input.is_compiling {
        return ("Compiling…".to_owned(), Color32::from_gray(150));
    }
    let color = if has_errors(input.diagnostics) {
        Color32::from_rgb(245, 72, 76)
    } else if input.diagnostics.is_empty() {
        Color32::from_rgb(23, 201, 114)
    } else {
        Color32::from_rgb(255, 149, 0)
    };
    (diagnostics_summary(input.diagnostics), color)
}

fn editor(ui: &mut egui::Ui, state: &mut PineEditorState, height: f32) {
    let font = FontId::monospace(13.0);
    let lines = line_count(&state.source);
    let gutter = lines.to_string().len().max(2) as f32 * 8.5 + 14.0;
    let id = state.id;
    if let Some(offset) = state.jump_to.take() {
        let mut edit_state = TextEditState::load(ui.ctx(), id).unwrap_or_default();
        let index = utf16_to_char_index(&state.source, offset);
        edit_state
            .cursor
            .set_char_range(Some(CCursorRange::one(CCursor::new(index))));
        edit_state.store(ui.ctx(), id);
        ui.memory_mut(|memory| memory.request_focus(id));
    }
    let spans = state.spans();
    let current = text_hash(&state.source);
    let mut layouter = |ui: &egui::Ui, text: &dyn egui::TextBuffer, wrap_width: f32| {
        // While the buffer is what was lexed, reuse it; mid-edit text is lexed on the spot.
        let fresh;
        let spans: &[Span] = if text_hash(text.as_str()) == current {
            &spans
        } else {
            fresh = highlight_spans(text.as_str());
            &fresh
        };
        let mut job = layout_job(text.as_str(), spans, font.clone());
        job.wrap.max_width = wrap_width;
        ui.fonts_mut(|fonts| fonts.layout_job(job))
    };
    egui::ScrollArea::both()
        .id_salt(("pine-editor-scroll", id))
        .max_height(height)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.horizontal_top(|ui| {
                let numbers: String = (1..=lines).map(|n| format!("{n}\n")).collect();
                ui.allocate_ui(Vec2::new(gutter, 0.0), |ui| {
                    ui.set_width(gutter);
                    ui.label(
                        RichText::new(numbers.trim_end())
                            .font(FontId::monospace(13.0))
                            .color(Color32::from_gray(110)),
                    );
                });
                let response = ui.add(
                    egui::TextEdit::multiline(&mut state.source)
                        .id(id)
                        .font(FontId::monospace(13.0))
                        .code_editor()
                        .desired_width(f32::INFINITY)
                        .desired_rows(24)
                        .lock_focus(true)
                        .layouter(&mut layouter),
                );
                if response.changed() {
                    state.note_edit(Instant::now());
                }
            });
        });
}

fn diagnostics_panel(
    ui: &mut egui::Ui,
    state: &mut PineEditorState,
    input: &PineEditorInput,
    height: f32,
) {
    if input.diagnostics.is_empty() {
        ui.label(RichText::new("No problems").small().weak());
        return;
    }
    let mut jump: Option<usize> = None;
    egui::ScrollArea::vertical()
        .id_salt(("pine-editor-diagnostics", state.id))
        .max_height(height)
        .show(ui, |ui| {
            for (index, diagnostic) in input.diagnostics.iter().enumerate() {
                let (glyph, color) = match diagnostic.severity {
                    DiagnosticSeverity::Error => ("✕", Color32::from_rgb(245, 72, 76)),
                    DiagnosticSeverity::Warning => ("⚠", Color32::from_rgb(255, 149, 0)),
                };
                let text = format!(
                    "{glyph}  {}  {}  —  {}",
                    diagnostic.code,
                    diagnostic.message,
                    location_text(diagnostic)
                );
                if ui
                    .add(
                        egui::Label::new(RichText::new(text).small().color(color))
                            .sense(egui::Sense::click()),
                    )
                    .on_hover_text("Go to this line")
                    .clicked()
                {
                    jump = Some(index);
                }
            }
        });
    if let Some(index) = jump {
        state.go_to(&input.diagnostics[index]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pine::token::{DiagnosticCategory, SourcePosition, SourceRange};

    fn text_of<'a>(source: &'a str, span: &Span) -> &'a str {
        &source[span.range.clone()]
    }

    fn highlights(source: &str) -> Vec<(String, Highlight)> {
        highlight_spans(source)
            .iter()
            .map(|span| (text_of(source, span).to_owned(), span.highlight))
            .collect()
    }

    #[test]
    fn the_lexer_classifies_keywords_numbers_strings_and_calls() {
        let source = "x = ta.sma(close, 14)\nif x > 1.5\n    plot(x, \"avg\", color=#FF8800)\n";
        let found = highlights(source);
        assert!(
            found.contains(&("if".into(), Highlight::Keyword)),
            "{found:?}"
        );
        assert!(found.contains(&("14".into(), Highlight::Number)));
        assert!(found.contains(&("1.5".into(), Highlight::Number)));
        assert!(found.contains(&("\"avg\"".into(), Highlight::Text)));
        assert!(found.contains(&("plot".into(), Highlight::Call)));
        assert!(found.contains(&("sma".into(), Highlight::Call)));
        assert!(
            !found.iter().any(|(text, _)| text == "x"),
            "plain identifiers stay plain"
        );
    }

    #[test]
    fn comments_are_found_outside_strings_only() {
        let source = "a = 1 // trailing\nb = \"http://x\" // real\n// whole line\n";
        let found = highlights(source);
        let comments: Vec<&str> = found
            .iter()
            .filter(|(_, h)| *h == Highlight::Comment)
            .map(|(t, _)| t.as_str())
            .collect();
        assert_eq!(comments, ["// trailing", "// real", "// whole line"]);
        assert!(found.contains(&("\"http://x\"".into(), Highlight::Text)));
    }

    #[test]
    fn highlighting_survives_multibyte_text() {
        let source = "// 日本語 ✓\nlabel = \"héllo\"\nplot(1)\n";
        let found = highlights(source);
        assert!(
            found.contains(&("// 日本語 ✓".into(), Highlight::Comment)),
            "{found:?}"
        );
        assert!(found.contains(&("\"héllo\"".into(), Highlight::Text)));
        assert!(found.contains(&("plot".into(), Highlight::Call)));
    }

    #[test]
    fn utf16_offsets_map_to_bytes_and_character_indexes() {
        let source = "a😀b";
        assert_eq!(utf16_to_byte_offsets(source), [0, 1, 1, 5, 6]);
        assert_eq!(utf16_to_char_index(source, 0), 0);
        assert_eq!(utf16_to_char_index(source, 1), 1);
        assert_eq!(
            utf16_to_char_index(source, 3),
            2,
            "after the surrogate pair"
        );
        assert_eq!(utf16_to_char_index(source, 99), 3, "past the end clamps");
    }

    #[test]
    fn the_layout_covers_the_whole_source_in_order() {
        let source = "plot(1) // note";
        let spans = highlight_spans(source);
        let job = layout_job(source, &spans, FontId::monospace(13.0));
        assert_eq!(job.text, source);
        assert!(job.sections.len() >= 3);
        let end = job.sections.last().unwrap().byte_range.end;
        assert_eq!(end.0, source.len());
        assert_eq!(layout_job("", &[], FontId::monospace(13.0)).text, "");
    }

    fn diagnostic(severity: DiagnosticSeverity, line: usize, column: usize) -> Diagnostic {
        Diagnostic {
            code: "PINE3001".into(),
            severity,
            category: DiagnosticCategory::Semantic,
            message: "message".into(),
            range: SourceRange::new(
                SourcePosition::new(line, column, 10),
                SourcePosition::new(line, column + 1, 11),
            ),
        }
    }

    #[test]
    fn the_summary_counts_errors_and_warnings() {
        use DiagnosticSeverity::{Error, Warning};
        assert_eq!(diagnostics_summary(&[]), "No problems");
        assert_eq!(diagnostics_summary(&[diagnostic(Error, 1, 1)]), "1 error");
        assert_eq!(
            diagnostics_summary(&[diagnostic(Warning, 1, 1), diagnostic(Warning, 2, 1)]),
            "2 warnings"
        );
        assert_eq!(
            diagnostics_summary(&[
                diagnostic(Error, 1, 1),
                diagnostic(Error, 2, 1),
                diagnostic(Warning, 3, 1)
            ]),
            "2 errors · 1 warning"
        );
        assert!(has_errors(&[diagnostic(Error, 1, 1)]));
        assert!(!has_errors(&[diagnostic(Warning, 1, 1)]));
        assert_eq!(location_text(&diagnostic(Error, 3, 7)), "Line 3, column 7");
    }

    #[test]
    fn lines_are_counted_like_an_editor_does() {
        assert_eq!(line_count(""), 1);
        assert_eq!(line_count("a"), 1);
        assert_eq!(line_count("a\nb"), 2);
        assert_eq!(
            line_count("a\nb\n"),
            2,
            "a trailing newline is not another line"
        );
        assert_eq!(line_count("\n\n"), 2);
    }

    #[test]
    fn a_compile_is_requested_once_typing_pauses() {
        let mut state = PineEditorState::new(1, "Demo", "plot(close)");
        let start = Instant::now();
        // A freshly opened script compiles once its delay has passed.
        assert!(!state.take_compile_request(start));
        assert!(state.take_compile_request(start + COMPILE_DELAY + Duration::from_millis(50)));
        assert!(!state.is_compile_pending());
        assert!(
            !state.take_compile_request(start + Duration::from_secs(60)),
            "only once per edit"
        );
        state.note_edit(start + Duration::from_secs(61));
        assert!(
            !state
                .take_compile_request(start + Duration::from_secs(61) + Duration::from_millis(100))
        );
        state.note_edit(start + Duration::from_secs(61) + Duration::from_millis(400));
        assert!(
            !state
                .take_compile_request(start + Duration::from_secs(61) + Duration::from_millis(800)),
            "another keystroke restarts the wait"
        );
        assert!(state.take_compile_request(start + Duration::from_secs(63)));
    }

    #[test]
    fn dirty_means_name_or_text_differs_from_the_last_save() {
        let mut state = PineEditorState::new(1, "Demo", "plot(close)");
        assert!(!state.is_dirty());
        state.source.push('\n');
        assert!(state.is_dirty());
        state.mark_saved();
        assert!(!state.is_dirty());
        state.name = "Renamed".into();
        assert!(state.is_dirty());
        state.set_source("plot(open)");
        assert!(state.is_compile_pending());
        state.go_to(&diagnostic(DiagnosticSeverity::Error, 1, 3));
        assert_eq!(state.jump_to, Some(10));
    }

    #[test]
    fn the_editor_draws_headlessly_with_and_without_problems() {
        for diagnostics in [
            Vec::new(),
            vec![diagnostic(DiagnosticSeverity::Error, 1, 1)],
        ] {
            let mut state =
                PineEditorState::new(7, "Demo", "//@version=6\nindicator(\"x\")\nplot(close)\n");
            let ctx = egui::Context::default();
            let mut actions = Vec::new();
            for _ in 0..2 {
                let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                    actions = show(
                        ui,
                        &mut state,
                        &PineEditorInput {
                            diagnostics: &diagnostics,
                            has_chart: true,
                            has_alerts: true,
                            is_compiling: false,
                        },
                    );
                });
                output.textures_delta.clear();
            }
            assert!(
                actions.is_empty(),
                "nothing clicked, and the compile delay has not passed"
            );
        }
    }
}
