//! The modal sheets: New Script, the delete confirmation and the error alert. Ported from
//! `ScriptNameSheet` and the `ScriptManagerView` dialogs.

use super::{AlertSheet, ScriptManager, ScriptManagerAction, Sheet};
use crate::model::pine_alert::ArmingFacts;
use crate::ui::pine_alert_editor::{self, PineAlertEditorInput, PineAlertEditorOutcome};
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
        egui::Modal::new(egui::Id::new("script-manager-alert")).show(ctx, |ui| {
            ui.set_width(300.0);
            ui.vertical(|ui| {
                ui.label(
                    RichText::new("Script Manager")
                        .size(13.0)
                        .strong()
                        .color(theme::TEXT_PRIMARY),
                );
                ui.add_space(6.0);
                ui.label(
                    RichText::new(&message)
                        .size(12.0)
                        .color(theme::TEXT_SECONDARY),
                );
                ui.add_space(14.0);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui
                        .add(egui::Button::new("OK").min_size(Vec2::new(64.0, 24.0)))
                        .clicked()
                        || ui.input(|i| i.key_pressed(Key::Enter) || i.key_pressed(Key::Escape))
                    {
                        dismiss = true;
                    }
                });
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
        egui::Modal::new(egui::Id::new("script-manager-new")).show(ctx, |ui| {
            ui.set_width(300.0);
            ui.spacing_mut().item_spacing.y = 12.0;
            ui.label(
                RichText::new("New Script")
                    .size(13.0)
                    .strong()
                    .color(theme::TEXT_PRIMARY),
            );
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
            // The error line reserves its space so the buttons don't jump.
            ui.label(
                RichText::new(problem.as_deref().unwrap_or(" "))
                    .size(10.0)
                    .color(theme::syntax::STRING),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let create_button =
                    egui::Button::new(RichText::new("Create").color(egui::Color32::WHITE))
                        .fill(theme::SYSTEM_ACCENT)
                        .min_size(Vec2::new(64.0, 24.0));
                if ui.add_enabled(can_create, create_button).clicked()
                    || (can_create && field.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)))
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
        let mut cancel = false;
        let mut delete = false;
        egui::Modal::new(egui::Id::new("script-manager-delete")).show(ctx, |ui| {
            ui.set_width(300.0);
            ui.label(
                RichText::new("Delete Script")
                    .size(13.0)
                    .strong()
                    .color(theme::TEXT_PRIMARY),
            );
            ui.add_space(6.0);
            ui.label(
                RichText::new("The script file will be moved to the Trash.")
                    .size(12.0)
                    .color(theme::TEXT_SECONDARY),
            );
            ui.add_space(14.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let destructive = egui::Button::new(
                    RichText::new(format!("Delete \"{name}\"")).color(egui::Color32::WHITE),
                )
                .fill(egui::Color32::from_rgb(0xFF, 0x45, 0x3A))
                .min_size(Vec2::new(64.0, 24.0));
                if ui.add(destructive).clicked() {
                    delete = true;
                }
                if ui
                    .add(egui::Button::new("Cancel").min_size(Vec2::new(64.0, 24.0)))
                    .clicked()
                    || ui.input(|i| i.key_pressed(Key::Escape))
                {
                    cancel = true;
                }
            });
        });
        if delete {
            self.delete_script(&id);
            return None;
        }
        if cancel {
            return None;
        }
        Some(Sheet::ConfirmDelete { id, name })
    }
}
