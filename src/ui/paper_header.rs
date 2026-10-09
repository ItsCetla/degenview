//! The Paper Trading panel's top and bottom bars. The header holds the PAPER badge, the account
//! switcher, the section tabs with their counts and the panel's own buttons; the footer holds the
//! one-line summary of the section and its bulk action (Close All, Cancel All, Export) behind a
//! confirmation. Pure views returning actions; the host owns the store and the engine.

// Transient: the paper-trading panel hosts these once the host is wired.
#![allow(dead_code)]

use crate::model::paper::PaperCurrency;
use crate::ui::paper_tables::{FooterFacts, PaperTab, footer_note};
use crate::ui::theme;
use eframe::egui::{self, Align2, Color32, Id, RichText, Sense, Vec2};

/// Width below which the tabs drop to a second row instead of being cut off.
pub const COMPACT_WIDTH: f32 = 640.0;

#[derive(Clone, Debug, PartialEq)]
pub struct AccountChoice {
    pub id: u64,
    pub name: String,
}

pub struct PaperHeaderInput<'a> {
    pub accounts: &'a [AccountChoice],
    pub selected: Option<u64>,
    pub currency: Option<PaperCurrency>,
    pub position_count: usize,
    pub working_order_count: usize,
    pub show_trading_on_charts: bool,
}

#[derive(Default)]
pub struct PaperHeaderState {
    pub tab: PaperTab,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PaperHeaderAction {
    SelectAccount(u64),
    NewAccount,
    ExportData,
    ResetAccount,
    ToggleShowOnCharts,
    Close,
}

/// The count shown beside a tab; only the live sections carry one.
pub fn tab_count(tab: PaperTab, input: &PaperHeaderInput) -> Option<usize> {
    match tab {
        PaperTab::Positions => Some(input.position_count),
        PaperTab::Orders => Some(input.working_order_count),
        PaperTab::History | PaperTab::Trades | PaperTab::Journal => None,
    }
}

/// "Positions 2", or just "Positions" for a section without a count.
pub fn tab_text(tab: PaperTab, count: Option<usize>) -> String {
    match count {
        Some(count) => format!("{} {count}", tab.title()),
        None => tab.title().to_owned(),
    }
}

pub fn account_name<'a>(input: &'a PaperHeaderInput) -> &'a str {
    input
        .selected
        .and_then(|id| input.accounts.iter().find(|account| account.id == id))
        .map_or("No account", |account| account.name.as_str())
}

pub fn eye_label(showing: bool) -> &'static str {
    if showing {
        "Hide trading on charts"
    } else {
        "Show trading on charts"
    }
}

/// The one "PAPER" marker, so simulated money is never mistaken for the real thing.
pub fn badge(ui: &mut egui::Ui) {
    egui::Frame::new()
        .fill(theme::ACCENT.gamma_multiply(0.14))
        .corner_radius(999.0)
        .inner_margin(egui::Margin::symmetric(8, 3))
        .show(ui, |ui| {
            ui.label(
                RichText::new("▤ PAPER")
                    .size(10.5)
                    .strong()
                    .color(theme::ACCENT),
            )
            .on_hover_text("Paper trading");
        });
}

/// An icon-only button; `label` is its tooltip.
pub fn icon_button(ui: &mut egui::Ui, glyph: &str, label: &str, active: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(24.0), Sense::click());
    let color = if active {
        theme::ACCENT
    } else if response.hovered() {
        ui.visuals().strong_text_color()
    } else {
        Color32::from_gray(150)
    };
    if response.hovered() {
        ui.painter()
            .rect_filled(rect, 6.0, Color32::from_white_alpha(14));
    }
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        glyph,
        egui::FontId::proportional(13.0),
        color,
    );
    response.on_hover_text(label)
}

pub fn header(
    ui: &mut egui::Ui,
    state: &mut PaperHeaderState,
    input: &PaperHeaderInput,
) -> Vec<PaperHeaderAction> {
    let mut actions = Vec::new();
    let compact = ui.available_width() < COMPACT_WIDTH;
    egui::Frame::new()
        .inner_margin(egui::Margin::symmetric(12, 8))
        .show(ui, |ui| {
            if compact {
                ui.horizontal(|ui| {
                    identity(ui, input, &mut actions);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        buttons(ui, input, &mut actions);
                    });
                });
                ui.add_space(6.0);
                egui::ScrollArea::horizontal()
                    .id_salt("paper-tabs")
                    .show(ui, |ui| tabs(ui, state, input));
            } else {
                ui.horizontal(|ui| {
                    identity(ui, input, &mut actions);
                    ui.add_space(8.0);
                    tabs(ui, state, input);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        buttons(ui, input, &mut actions);
                    });
                });
            }
        });
    actions
}

fn identity(ui: &mut egui::Ui, input: &PaperHeaderInput, actions: &mut Vec<PaperHeaderAction>) {
    badge(ui);
    let title = match input.currency {
        Some(currency) => format!("{}  {}", account_name(input), currency.code()),
        None => account_name(input).to_owned(),
    };
    ui.menu_button(RichText::new(format!("{title} ⌄")).strong(), |ui| {
        for account in input.accounts {
            let selected = input.selected == Some(account.id);
            let label = if selected {
                format!("✓ {}", account.name)
            } else {
                account.name.clone()
            };
            if ui.button(label).clicked() {
                actions.push(PaperHeaderAction::SelectAccount(account.id));
                ui.close();
            }
        }
        ui.separator();
        if ui.button("New Account…").clicked() {
            actions.push(PaperHeaderAction::NewAccount);
            ui.close();
        }
    })
    .response
    .on_hover_text("Switch paper account");
}

fn tabs(ui: &mut egui::Ui, state: &mut PaperHeaderState, input: &PaperHeaderInput) {
    for tab in PaperTab::ALL {
        let text = tab_text(tab, tab_count(tab, input));
        let selected = state.tab == tab;
        let rich = if selected {
            RichText::new(text).strong().color(theme::ACCENT)
        } else {
            RichText::new(text).color(Color32::from_gray(160))
        };
        if ui.selectable_label(selected, rich).clicked() {
            state.tab = tab;
        }
    }
}

fn buttons(ui: &mut egui::Ui, input: &PaperHeaderInput, actions: &mut Vec<PaperHeaderAction>) {
    // Right-to-left, so the last one added sits leftmost.
    if icon_button(ui, "✕", "Close Paper Trading panel", false).clicked() {
        actions.push(PaperHeaderAction::Close);
    }
    ui.menu_button("⋯", |ui| {
        if ui.button("New Account…").clicked() {
            actions.push(PaperHeaderAction::NewAccount);
            ui.close();
        }
        ui.separator();
        if ui.button("Export Trading Data…").clicked() {
            actions.push(PaperHeaderAction::ExportData);
            ui.close();
        }
        if ui
            .button(RichText::new("Reset Account…").color(theme::DOWN))
            .clicked()
        {
            actions.push(PaperHeaderAction::ResetAccount);
            ui.close();
        }
    })
    .response
    .on_hover_text("Account actions");
    let showing = input.show_trading_on_charts;
    let glyph = if showing { "◉" } else { "◎" };
    if icon_button(ui, glyph, eye_label(showing), showing).clicked() {
        actions.push(PaperHeaderAction::ToggleShowOnCharts);
    }
}

// MARK: Footer

/// What the footer's bulk button asks the host to do once confirmed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PaperFooterAction {
    CloseAllPositions,
    CancelAllOrders,
    Export,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PaperFooterConfirm {
    CloseAll,
    CancelAll,
}

#[derive(Default)]
pub struct PaperFooterState {
    pub pending: Option<PaperFooterConfirm>,
}

/// "Close 1 Position" / "Cancel 3 Orders".
pub fn confirm_button(confirm: PaperFooterConfirm, count: usize) -> String {
    let (verb, noun) = match confirm {
        PaperFooterConfirm::CloseAll => ("Close", "Position"),
        PaperFooterConfirm::CancelAll => ("Cancel", "Order"),
    };
    format!("{verb} {count} {noun}{}", if count == 1 { "" } else { "s" })
}

pub fn confirm_title(confirm: PaperFooterConfirm) -> &'static str {
    match confirm {
        PaperFooterConfirm::CloseAll => "Close all positions?",
        PaperFooterConfirm::CancelAll => "Cancel all working orders?",
    }
}

pub fn confirm_message(confirm: PaperFooterConfirm) -> &'static str {
    match confirm {
        PaperFooterConfirm::CloseAll => "Each position is closed at market.",
        PaperFooterConfirm::CancelAll => {
            "Protective take-profit and stop-loss orders are canceled too."
        }
    }
}

/// The label on the section's bulk button.
pub fn bulk_label(tab: PaperTab) -> &'static str {
    match tab {
        PaperTab::Positions => "Close All",
        PaperTab::Orders => "Cancel All",
        PaperTab::History | PaperTab::Trades | PaperTab::Journal => "Export…",
    }
}

/// Whether the bulk button has anything to act on.
pub fn bulk_enabled(tab: PaperTab, facts: &FooterFacts) -> bool {
    match tab {
        PaperTab::Positions => !facts.positions.is_empty(),
        PaperTab::Orders => facts.working_orders > 0,
        PaperTab::History | PaperTab::Trades | PaperTab::Journal => true,
    }
}

pub fn footer(
    ui: &mut egui::Ui,
    state: &mut PaperFooterState,
    tab: PaperTab,
    facts: &FooterFacts,
) -> Vec<PaperFooterAction> {
    let mut actions = Vec::new();
    ui.separator();
    ui.horizontal(|ui| {
        ui.add_space(12.0);
        ui.label(RichText::new(footer_note(tab, facts)).small().weak());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(12.0);
            let enabled = bulk_enabled(tab, facts);
            if ui
                .add_enabled(enabled, egui::Button::new(bulk_label(tab)))
                .clicked()
            {
                match tab {
                    PaperTab::Positions => state.pending = Some(PaperFooterConfirm::CloseAll),
                    PaperTab::Orders => state.pending = Some(PaperFooterConfirm::CancelAll),
                    _ => actions.push(PaperFooterAction::Export),
                }
            }
        });
    });
    confirm_dialog(ui.ctx(), state, facts, &mut actions);
    actions
}

fn confirm_dialog(
    ctx: &egui::Context,
    state: &mut PaperFooterState,
    facts: &FooterFacts,
    actions: &mut Vec<PaperFooterAction>,
) {
    let Some(confirm) = state.pending else {
        return;
    };
    let count = match confirm {
        PaperFooterConfirm::CloseAll => facts.positions.len(),
        PaperFooterConfirm::CancelAll => facts.working_orders,
    };
    let mut done = false;
    egui::Window::new(confirm_title(confirm))
        .id(Id::new("paper-footer-confirm"))
        .collapsible(false)
        .resizable(false)
        .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
        .show(ctx, |ui| {
            ui.set_max_width(340.0);
            ui.label(confirm_message(confirm));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui
                    .button(RichText::new(confirm_button(confirm, count)).color(theme::DOWN))
                    .clicked()
                {
                    actions.push(match confirm {
                        PaperFooterConfirm::CloseAll => PaperFooterAction::CloseAllPositions,
                        PaperFooterConfirm::CancelAll => PaperFooterAction::CancelAllOrders,
                    });
                    done = true;
                }
                if ui.button("Cancel").clicked() {
                    done = true;
                }
            });
        });
    if done || ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
        state.pending = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input<'a>(accounts: &'a [AccountChoice], selected: Option<u64>) -> PaperHeaderInput<'a> {
        PaperHeaderInput {
            accounts,
            selected,
            currency: Some(PaperCurrency::Usd),
            position_count: 2,
            working_order_count: 0,
            show_trading_on_charts: true,
        }
    }

    fn accounts() -> Vec<AccountChoice> {
        vec![
            AccountChoice {
                id: 1,
                name: "Main".into(),
            },
            AccountChoice {
                id: 2,
                name: "Swing".into(),
            },
        ]
    }

    #[test]
    fn only_live_sections_carry_a_count() {
        let accounts = accounts();
        let input = input(&accounts, Some(1));
        assert_eq!(tab_count(PaperTab::Positions, &input), Some(2));
        assert_eq!(tab_count(PaperTab::Orders, &input), Some(0));
        for tab in [PaperTab::History, PaperTab::Trades, PaperTab::Journal] {
            assert_eq!(tab_count(tab, &input), None);
        }
        assert_eq!(tab_text(PaperTab::Positions, Some(2)), "Positions 2");
        assert_eq!(tab_text(PaperTab::Journal, None), "Journal");
    }

    #[test]
    fn the_switcher_names_the_selected_account() {
        let accounts = accounts();
        assert_eq!(account_name(&input(&accounts, Some(2))), "Swing");
        assert_eq!(account_name(&input(&accounts, None)), "No account");
        assert_eq!(account_name(&input(&accounts, Some(9))), "No account");
    }

    #[test]
    fn the_eye_says_what_a_click_will_do() {
        assert_eq!(eye_label(true), "Hide trading on charts");
        assert_eq!(eye_label(false), "Show trading on charts");
    }

    #[test]
    fn confirmations_count_and_pluralise() {
        assert_eq!(
            confirm_button(PaperFooterConfirm::CloseAll, 1),
            "Close 1 Position"
        );
        assert_eq!(
            confirm_button(PaperFooterConfirm::CancelAll, 3),
            "Cancel 3 Orders"
        );
        assert_eq!(
            confirm_title(PaperFooterConfirm::CloseAll),
            "Close all positions?"
        );
        assert!(confirm_message(PaperFooterConfirm::CancelAll).contains("take-profit"));
    }

    #[test]
    fn the_bulk_button_follows_the_section() {
        assert_eq!(bulk_label(PaperTab::Positions), "Close All");
        assert_eq!(bulk_label(PaperTab::Orders), "Cancel All");
        assert_eq!(bulk_label(PaperTab::Journal), "Export…");
        let facts = FooterFacts {
            positions: &[],
            unrealized_total: 0.0,
            working_orders: 0,
            events: 0,
            trades: &[],
            journal_entries: 0,
            currency: PaperCurrency::Usd,
        };
        assert!(!bulk_enabled(PaperTab::Positions, &facts));
        assert!(!bulk_enabled(PaperTab::Orders, &facts));
        assert!(
            bulk_enabled(PaperTab::History, &facts),
            "export is always on"
        );
    }
}
