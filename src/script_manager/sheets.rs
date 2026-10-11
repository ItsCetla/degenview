//! The modal sheets: New Script, the delete confirmation and the error alert. Ported from
//! `ScriptNameSheet` and the `ScriptManagerView` dialogs.

use super::{AlertSheet, ScriptManager, ScriptManagerAction, Sheet};
use crate::model::pine_alert::ArmingFacts;
use crate::ui::icons::Icon;
use crate::ui::kit::{self, Confirm, ConfirmOutcome};
use crate::ui::pine_alert_editor::{self, PineAlertEditorInput, PineAlertEditorOutcome};
use crate::ui::sheet;
use crate::ui::theme;
use eframe::egui::{self, Align, Key, Layout, RichText, Vec2};

impl ScriptManager {
    pub(super) fn sheets_ui(&mut self, ctx: &egui::Context) {
        if self.error.is_some() {
            self.error_alert(ctx);
        } else if let Some(sheet) = self.sheet.take() {
            self.sheet = match sheet {
                Sheet::New {
                    name,
                    error,
                    needs_focus,
                } => self.new_script_sheet(ctx, name, error, needs_focus),
                Sheet::ConfirmDelete { id, name } => self.delete_sheet(ctx, id, name),
                Sheet::Alert(sheet) => self.alert_sheet(ctx, sheet),
            };
        }
    }

    fn error_alert(&mut self, ctx: &egui::Context) {
        let message = self.error.clone().unwrap_or_default();
        let mut dismiss = false;
        sheet::show(ctx, egui::Id::new("script-manager-alert"), 340.0, |ui| {
            ui.add_space(20.0);
            sheet::inset(ui, |ui| {
                sheet::header(
                    ui,
                    Icon::Triangle,
                    "Script Manager",
                    &message,
                    theme::ORANGE,
                );
                ui.add_space(20.0);
            });
            ui.separator();
            sheet::inset(ui, |ui| {
                ui.add_space(12.0);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui
                        .add(egui::Button::new("OK").min_size(Vec2::new(64.0, 24.0)))
                        .clicked()
                        || ui.input(|i| i.key_pressed(Key::Enter) || i.key_pressed(Key::Escape))
                    {
                        dismiss = true;
                    }
                });
                ui.add_space(12.0);
            });
        });
        if dismiss {
            self.error = None;
        }
    }

    fn new_script_sheet(
        &mut self,
        ctx: &egui::Context,
        mut name: String,
        _error: Option<String>,
        mut needs_focus: bool,
    ) -> Option<Sheet> {
        let problem = self.name_problem(&name);
        let can_create =
            !name.trim().is_empty() && problem.is_none() && super::name::validate(&name).is_ok();
        let mut cancel = false;
        let mut create = false;
        let mut submit_on_enter = false;
        sheet::show(ctx, egui::Id::new("script-manager-new"), 340.0, |ui| {
            ui.add_space(20.0);
            sheet::inset(ui, |ui| {
                sheet::header(
                    ui,
                    Icon::Curlybraces,
                    "New Script",
                    "Create a script file in your library.",
                    theme::ACCENT,
                );
                ui.add_space(18.0);
                let field = ui.add(
                    egui::TextEdit::singleline(&mut name)
                        .hint_text("Script name")
                        .desired_width(f32::INFINITY)
                        .margin(egui::Margin::symmetric(8, 6)),
                );
                if needs_focus {
                    field.request_focus();
                    needs_focus = false;
                }
                submit_on_enter = field.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                // The error line reserves its space so the buttons don't jump.
                ui.add_space(8.0);
                ui.label(
                    RichText::new(problem.as_deref().unwrap_or(" "))
                        .size(10.0)
                        .color(theme::syntax::string()),
                );
                ui.add_space(18.0);
            });
            ui.separator();
            sheet::inset(ui, |ui| {
                ui.add_space(12.0);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let create_button =
                        egui::Button::new(RichText::new("Create").color(egui::Color32::WHITE))
                            .fill(theme::SYSTEM_ACCENT)
                            .min_size(Vec2::new(64.0, 24.0));
                    if ui.add_enabled(can_create, create_button).clicked()
                        || (can_create && submit_on_enter)
                    {
                        create = true;
                    }
                    if ui
                        .add(egui::Button::new("Cancel").min_size(Vec2::new(64.0, 24.0)))
                        .clicked()
                        || ui.input(|i| i.key_pressed(Key::Escape))
                    {
                        cancel = true;
                    }
                });
                ui.add_space(12.0);
            });
        });
        if cancel {
            return None;
        }
        if create {
            return match self.create_script(&name) {
                Ok(()) => None,
                Err(message) => Some(Sheet::New {
                    name,
                    error: Some(message),
                    needs_focus: true,
                }),
            };
        }
        Some(Sheet::New {
            name,
            error: None,
            needs_focus,
        })
    }

    /// The Create Script Alert sheet: arms the script's `alert()` calls on the focused chart.
    fn alert_sheet(&mut self, ctx: &egui::Context, mut sheet: Box<AlertSheet>) -> Option<Sheet> {
        let chart = self.chart.clone()?;
        let facts = ArmingFacts {
            loaded: true,
            instance_exists: true,
            source_loaded: true,
            call_sites: sheet.call_sites.len(),
            duplicate_exists: sheet.duplicate_exists,
        };
        let input = PineAlertEditorInput {
            chart_title: &chart.title,
            source_name: &chart.source_name,
            timeframe: &chart.timeframe,
            script_name: &sheet.name,
            facts,
            call_sites: &sheet.call_sites,
            endpoints: &sheet.endpoints,
        };
        match pine_alert_editor::show(ctx, &mut sheet.state, &input) {
            Some(PineAlertEditorOutcome::Create {
                note,
                webhook_endpoint_ids,
            }) => {
                self.actions.push(ScriptManagerAction::CreateAlert {
                    script_name: sheet.name.clone(),
                    source: sheet.source.clone(),
                    note,
                    webhook_endpoint_ids,
                });
                None
            }
            Some(PineAlertEditorOutcome::Cancel) => None,
            None => Some(Sheet::Alert(sheet)),
        }
    }

    fn delete_sheet(&mut self, ctx: &egui::Context, id: String, name: String) -> Option<Sheet> {
        let title = format!("Delete \"{name}\"?");
        let result = kit::confirm(
            ctx,
            egui::Id::new("script-manager-delete"),
            &Confirm {
                icon: Icon::Trash,
                title: &title,
                message: "The script file will be moved to the Trash.",
                destructive: "Delete Script",
            },
        );
        if result == Some(ConfirmOutcome::Confirmed) {
            self.delete_script(&id);
            return None;
        }
        if result == Some(ConfirmOutcome::Cancelled) {
            return None;
        }
        Some(Sheet::ConfirmDelete { id, name })
    }
}
