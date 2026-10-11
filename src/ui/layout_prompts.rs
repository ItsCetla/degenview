//! The sheets around saved layouts: the name prompt (save, copy, rename), the "save changes?"
//! question before switching layouts, and the error alert. Pure views: the workspace owns the
//! state and acts on what they report.

use crate::ui::icons::{self, Icon};
use crate::ui::kit::{self, REGULAR, size};
use crate::ui::sheet;
use crate::ui::theme;
use eframe::egui::{self, Id};

const WIDTH: f32 = 420.0;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NamePrompt {
    /// First save of an unnamed layout, optionally followed by opening another one.
    Save {
        then_open: Option<u64>,
    },
    Copy,
    Rename,
}

impl NamePrompt {
    fn title(self) -> &'static str {
        match self {
            Self::Save { .. } => "Save Layout",
            Self::Copy => "Make a Copy",
            Self::Rename => "Rename Layout",
        }
    }

    fn message(self) -> &'static str {
        match self {
            Self::Save { .. } => "Save the current charts, timeframe and layout under a name.",
            Self::Copy => "The copy keeps what you see now and saves separately from the original.",
            Self::Rename => "The name is also the tab title.",
        }
    }

    fn button(self) -> &'static str {
        match self {
            Self::Rename => "Rename",
            Self::Save { .. } | Self::Copy => "Save",
        }
    }

    fn icon(self) -> Icon {
        match self {
            Self::Save { .. } => Icon::Layers,
            Self::Copy => Icon::CopyDoc,
            Self::Rename => Icon::Pencil,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum NameOutcome {
    Confirm(String),
    Cancel,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UnsavedChoice {
    Save,
    Discard,
    Cancel,
}

/// A name for the layout. `name` is edited in place; `focus` is true on the first frame so the
/// field takes the cursor.
pub fn show_name(
    ctx: &egui::Context,
    prompt: NamePrompt,
    name: &mut String,
    focus: &mut bool,
) -> Option<NameOutcome> {
    let mut outcome = None;
    let field_id = Id::new("layout-prompt-name");
    let can_confirm = !name.trim().is_empty();
    sheet::show(ctx, Id::new("layout-name-prompt"), WIDTH, |ui| {
        ui.add_space(24.0);
        sheet::inset(ui, |ui| {
            sheet::header(
                ui,
                prompt.icon(),
                prompt.title(),
                prompt.message(),
                theme::ACCENT,
            );
            ui.add_space(20.0);
            let focused = ui.memory(|memory| memory.has_focus(field_id));
            kit::field_frame(focused).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    icons::inline(ui, Icon::Layers, 13.0, theme::text_secondary());
                    let width = ui.available_width();
                    let response = kit::plain_field(
                        ui,
                        field_id,
                        name,
                        "Layout name",
                        size::BODY,
                        REGULAR,
                        width,
                    );
                    if std::mem::take(focus) {
                        response.request_focus();
                        // Select the suggested name so typing replaces it.
                        if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), field_id) {
                            state
                                .cursor
                                .set_char_range(Some(egui::text::CCursorRange::two(
                                    egui::text::CCursor::new(0),
                                    egui::text::CCursor::new(name.chars().count()),
                                )));
                            state.store(ui.ctx(), field_id);
                        }
                    }
                });
            });
            ui.add_space(22.0);
        });
        footer(ui, |ui| {
            if kit::sheet_button(ui, prompt.button(), true, can_confirm).clicked() {
                outcome = Some(NameOutcome::Confirm(name.trim().to_owned()));
            }
            if kit::sheet_button(ui, "Cancel", false, true).clicked() {
                outcome = Some(NameOutcome::Cancel);
            }
        });
    });
    if ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
        outcome = Some(NameOutcome::Cancel);
    } else if outcome.is_none()
        && can_confirm
        && ctx.input(|input| input.key_pressed(egui::Key::Enter))
    {
        outcome = Some(NameOutcome::Confirm(name.trim().to_owned()));
    }
    outcome
}

/// "Save changes to “X”?" before another layout opens over unsaved edits.
pub fn show_unsaved(ctx: &egui::Context, layout_name: &str) -> Option<UnsavedChoice> {
    let mut choice = None;
    sheet::show(ctx, Id::new("layout-unsaved"), WIDTH, |ui| {
        ui.add_space(24.0);
        sheet::inset(ui, |ui| {
            sheet::header(
                ui,
                Icon::TriangleFill,
                &format!("Save changes to “{layout_name}”?"),
                "Your changes will be lost if you switch layouts without saving.",
                kit::ORANGE,
            );
            ui.add_space(22.0);
        });
        footer(ui, |ui| {
            if kit::sheet_button(ui, "Save", true, true).clicked() {
                choice = Some(UnsavedChoice::Save);
            }
            if kit::sheet_button(ui, "Cancel", false, true).clicked() {
                choice = Some(UnsavedChoice::Cancel);
            }
            if kit::sheet_button(ui, "Don’t Save", false, true).clicked() {
                choice = Some(UnsavedChoice::Discard);
            }
        });
    });
    if ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
        choice = Some(UnsavedChoice::Cancel);
    } else if choice.is_none() && ctx.input(|input| input.key_pressed(egui::Key::Enter)) {
        choice = Some(UnsavedChoice::Save);
    }
    choice
}

/// A layout operation failed; true once the alert is dismissed.
pub fn show_error(ctx: &egui::Context, message: &str) -> bool {
    let mut dismissed = false;
    sheet::show(ctx, Id::new("layout-error"), WIDTH, |ui| {
        ui.add_space(24.0);
        sheet::inset(ui, |ui| {
            sheet::header(ui, Icon::TriangleFill, "Layout", message, kit::ORANGE);
            ui.add_space(22.0);
        });
        footer(ui, |ui| {
            dismissed = kit::sheet_button(ui, "OK", true, true).clicked();
        });
    });
    dismissed
        || ctx.input(|input| {
            input.key_pressed(egui::Key::Escape) || input.key_pressed(egui::Key::Enter)
        })
}

/// The hairline and the button row, right to left. The row sits in a `horizontal` so it takes
/// its own height only; a bare right-to-left layout claims all remaining height and the sheet
/// grows every frame.
fn footer(ui: &mut egui::Ui, buttons: impl FnOnce(&mut egui::Ui)) {
    ui.separator();
    sheet::inset(ui, |ui| {
        ui.add_space(14.0);
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 12.0;
                buttons(ui);
            });
        });
        ui.add_space(14.0);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{Rect, pos2, vec2};

    /// The tallest the sheet got over `frames` frames.
    fn heights(mut draw: impl FnMut(&egui::Context), frames: usize) -> Vec<f32> {
        let ctx = egui::Context::default();
        (0..frames)
            .map(|_| {
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(900.0, 700.0))),
                        ..Default::default()
                    },
                    |ui| draw(ui.ctx()),
                );
                output.textures_delta.clear();
                ctx.memory(|memory| {
                    memory
                        .area_rect(Id::new("layout-name-prompt"))
                        .or_else(|| memory.area_rect(Id::new("layout-unsaved")))
                        .or_else(|| memory.area_rect(Id::new("layout-error")))
                        .map_or(0.0, |rect| rect.height())
                })
            })
            .collect()
    }

    #[test]
    fn the_sheets_keep_their_height_from_frame_to_frame() {
        let mut name = String::from("Morning scan");
        let mut focus = true;
        let name_heights = heights(
            |ctx| {
                show_name(
                    ctx,
                    NamePrompt::Save { then_open: None },
                    &mut name,
                    &mut focus,
                );
            },
            8,
        );
        let unsaved = heights(
            |ctx| {
                show_unsaved(ctx, "Morning scan");
            },
            8,
        );
        let error = heights(
            |ctx| {
                show_error(ctx, "Couldn’t save the layout.");
            },
            8,
        );
        for series in [name_heights, unsaved, error] {
            let settled = series[3];
            assert!(settled > 100.0, "the sheet laid out: {series:?}");
            assert!(
                series[3..]
                    .iter()
                    .all(|height| (height - settled).abs() < 0.5),
                "a sheet that grows each frame is broken: {series:?}"
            );
            assert!(settled < 400.0, "a short prompt stays short: {series:?}");
        }
    }

    #[test]
    fn a_blank_name_cannot_be_confirmed() {
        let mut name = String::from("   ");
        let mut focus = false;
        let ctx = egui::Context::default();
        let mut outcome = None;
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            outcome = show_name(ui.ctx(), NamePrompt::Rename, &mut name, &mut focus);
        });
        output.textures_delta.clear();
        assert_eq!(outcome, None);
        assert_eq!(NamePrompt::Rename.button(), "Rename");
        assert_eq!(NamePrompt::Copy.button(), "Save");
    }
}
