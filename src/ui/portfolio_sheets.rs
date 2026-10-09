//! The sheets that make and manage portfolios: "New Portfolio" (a name and a base
//! currency) and "Manage Portfolios" (rename, reorder, duplicate, delete). Pure views over
//! plain values; the host owns the ledger and applies the actions.

// Transient: the Portfolio window hosts these once the portfolio store is wired.
#![allow(dead_code)]

use crate::model::portfolio::{
    MAX_NAME_LENGTH, Portfolio, PortfolioCurrency, is_duplicate_name, is_valid_name, normalize_name,
};
use crate::model::portfolio_format::PORTFOLIO_CURRENCIES;
use crate::ui::theme;
use eframe::egui::{self, Align2, Color32, Id, RichText, Sense, Vec2};

// MARK: New portfolio

#[derive(Clone, Debug, PartialEq)]
pub enum CreateOutcome {
    Create {
        name: String,
        currency: PortfolioCurrency,
    },
    Cancel,
}

pub struct CreateState {
    pub name: String,
    pub currency: PortfolioCurrency,
    focus: bool,
}

impl Default for CreateState {
    fn default() -> Self {
        Self {
            name: String::new(),
            currency: PortfolioCurrency::Usd,
            focus: true,
        }
    }
}

impl CreateState {
    pub fn is_valid(&self) -> bool {
        is_valid_name(&self.name)
    }

    pub fn is_duplicate(&self, existing: &[String]) -> bool {
        is_duplicate_name(&self.name, existing)
    }

    /// The name is cut to the limit as it is typed.
    pub fn clamp_name(&mut self) {
        if self.name.chars().count() > MAX_NAME_LENGTH {
            self.name = self.name.chars().take(MAX_NAME_LENGTH).collect();
        }
    }

    pub fn outcome(&self) -> Option<CreateOutcome> {
        self.is_valid().then(|| CreateOutcome::Create {
            name: normalize_name(&self.name),
            currency: self.currency,
        })
    }
}

pub fn show_create(
    ctx: &egui::Context,
    state: &mut CreateState,
    existing_names: &[String],
) -> Option<CreateOutcome> {
    let mut outcome = None;
    egui::Window::new("New Portfolio")
        .id(Id::new("portfolio-create"))
        .collapsible(false)
        .resizable(false)
        .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
        .fixed_size(Vec2::new(460.0, 0.0))
        .show(ctx, |ui| {
            ui.label(RichText::new("Group transactions and track them together.").small().weak());
            ui.add_space(10.0);
            ui.label(RichText::new("Name").strong());
            let response = ui.add(
                egui::TextEdit::singleline(&mut state.name)
                    .hint_text("e.g. Long-term holds")
                    .desired_width(f32::INFINITY),
            );
            if std::mem::take(&mut state.focus) {
                response.request_focus();
            }
            state.clamp_name();
            if response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)) {
                outcome = state.outcome();
            }
            if state.is_duplicate(existing_names) {
                ui.label(
                    RichText::new("⚠ A portfolio with this name already exists.")
                        .small()
                        .color(theme::REPLAY),
                );
            }
            ui.add_space(10.0);
            ui.label(RichText::new("Base currency").strong());
            currency_grid(ui, &mut state.currency);
            ui.label(
                RichText::new(format!(
                    "Transactions in this portfolio are valued in {}. You can still view it in any currency later.",
                    state.currency.code()
                ))
                .small()
                .weak(),
            );
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add_enabled(state.is_valid(), egui::Button::new("Create Portfolio"))
                        .clicked()
                    {
                        outcome = state.outcome();
                    }
                    if ui.button("Cancel").clicked() {
                        outcome = Some(CreateOutcome::Cancel);
                    }
                });
            });
        });
    if ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
        outcome = Some(CreateOutcome::Cancel);
    }
    outcome
}

/// Three columns of selectable currency tiles: glyph, code and name.
fn currency_grid(ui: &mut egui::Ui, selected: &mut PortfolioCurrency) {
    for row in PORTFOLIO_CURRENCIES.chunks(3) {
        ui.columns(3, |columns| {
            for (column, currency) in columns.iter_mut().zip(row) {
                let is_selected = *selected == *currency;
                let tint = if is_selected {
                    theme::ACCENT
                } else {
                    Color32::from_gray(150)
                };
                let (rect, response) = column
                    .allocate_exact_size(Vec2::new(column.available_width(), 46.0), Sense::click());
                let painter = column.painter_at(rect);
                painter.rect_filled(
                    rect,
                    10.0,
                    if is_selected {
                        theme::ACCENT.gamma_multiply(0.10)
                    } else if response.hovered() {
                        Color32::from_white_alpha(18)
                    } else {
                        Color32::from_white_alpha(8)
                    },
                );
                painter.rect_stroke(
                    rect,
                    10.0,
                    egui::Stroke::new(
                        if is_selected { 1.5 } else { 1.0 },
                        if is_selected {
                            theme::ACCENT
                        } else {
                            theme::BORDER
                        },
                    ),
                    egui::StrokeKind::Inside,
                );
                let badge = rect.left_center() + Vec2::new(24.0, 0.0);
                painter.circle_filled(badge, 15.0, tint.gamma_multiply(0.18));
                painter.text(
                    badge,
                    Align2::CENTER_CENTER,
                    currency.glyph(),
                    egui::FontId::proportional(14.0),
                    tint,
                );
                painter.text(
                    rect.left_center() + Vec2::new(46.0, -8.0),
                    Align2::LEFT_CENTER,
                    currency.code(),
                    egui::FontId::proportional(13.0),
                    column.visuals().strong_text_color(),
                );
                painter.text(
                    rect.left_center() + Vec2::new(46.0, 8.0),
                    Align2::LEFT_CENTER,
                    currency.display_name(),
                    egui::FontId::proportional(10.5),
                    column.visuals().weak_text_color(),
                );
                if response.clicked() {
                    *selected = *currency;
                }
            }
        });
    }
}

// MARK: Manage portfolios

#[derive(Clone, Debug, PartialEq)]
pub enum ManageAction {
    Rename {
        id: u64,
        name: String,
    },
    Duplicate(u64),
    Delete(u64),
    /// Same meaning as `PortfolioLedger::reorder`: move `from` to sit before `destination`.
    Reorder {
        from: Vec<usize>,
        destination: usize,
    },
    /// "New Portfolio" was pressed; the host opens the create sheet.
    New,
    Done,
}

/// One portfolio's summary line input.
pub struct ManageRow<'a> {
    pub portfolio: &'a Portfolio,
    pub transactions: usize,
    pub assets: usize,
}

pub struct ManageInput<'a> {
    pub rows: &'a [ManageRow<'a>],
    pub current: Option<u64>,
}

#[derive(Default)]
pub struct ManageState {
    /// Name edits in progress, by portfolio id.
    drafts: std::collections::HashMap<u64, String>,
    pending_delete: Option<u64>,
}

/// "USD · 3 transactions · 2 assets".
pub fn summary_line(currency: PortfolioCurrency, transactions: usize, assets: usize) -> String {
    let plural =
        |n: usize, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
    format!(
        "{} · {} · {}",
        currency.code(),
        plural(transactions, "transaction", "transactions"),
        plural(assets, "asset", "assets")
    )
}

/// The reorder that moves the item at `index` one place up, or `None` at the top.
pub fn move_up(index: usize) -> Option<(Vec<usize>, usize)> {
    (index > 0).then(|| (vec![index], index - 1))
}

/// The reorder that moves the item at `index` one place down, or `None` at the bottom. The
/// destination is an insertion point in the list as it is now, so it is two past the item.
pub fn move_down(index: usize, len: usize) -> Option<(Vec<usize>, usize)> {
    (index + 1 < len).then(|| (vec![index], index + 2))
}

/// A renamed portfolio's new name, or `None` when the draft is blank (put the old one back)
/// or unchanged.
pub fn rename_commit(draft: &str, current: &str) -> Option<String> {
    let cleaned = normalize_name(draft);
    (!cleaned.is_empty() && cleaned != current).then_some(cleaned)
}

pub fn delete_message(transactions: usize) -> String {
    format!(
        "This permanently deletes the portfolio and its {transactions} {}. It can't be undone.",
        if transactions == 1 {
            "transaction"
        } else {
            "transactions"
        }
    )
}

pub fn show_manage(
    ctx: &egui::Context,
    state: &mut ManageState,
    input: &ManageInput,
) -> Vec<ManageAction> {
    let mut actions = Vec::new();
    egui::Window::new("Manage Portfolios")
        .id(Id::new("portfolio-manage"))
        .collapsible(false)
        .resizable(false)
        .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
        .fixed_size(Vec2::new(560.0, 0.0))
        .show(ctx, |ui| {
            ui.label(
                RichText::new(format!(
                    "{} · Reorder with the arrows, click a name to rename.",
                    if input.rows.len() == 1 {
                        "1 portfolio".to_owned()
                    } else {
                        format!("{} portfolios", input.rows.len())
                    }
                ))
                .small()
                .weak(),
            );
            ui.add_space(8.0);
            if input.rows.is_empty() {
                ui.add_space(30.0);
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new("No Portfolios").strong());
                    ui.label(
                        RichText::new("Create a portfolio to start tracking investments.").weak(),
                    );
                });
                ui.add_space(30.0);
            }
            egui::ScrollArea::vertical()
                .max_height(380.0)
                .show(ui, |ui| {
                    for (index, row) in input.rows.iter().enumerate() {
                        manage_row(ui, state, input, index, row, &mut actions);
                    }
                });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("＋ New Portfolio").clicked() {
                    actions.push(ManageAction::New);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Done").clicked() {
                        actions.push(ManageAction::Done);
                    }
                });
            });
        });
    delete_dialog(ctx, state, input, &mut actions);
    actions
}

fn manage_row(
    ui: &mut egui::Ui,
    state: &mut ManageState,
    input: &ManageInput,
    index: usize,
    row: &ManageRow,
    actions: &mut Vec<ManageAction>,
) {
    let portfolio = row.portfolio;
    egui::Frame::new()
        .fill(Color32::from_white_alpha(8))
        .stroke(egui::Stroke::new(1.0, theme::BORDER))
        .corner_radius(10.0)
        .inner_margin(egui::Margin::symmetric(12, 9))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    let draft = state
                        .drafts
                        .entry(portfolio.id)
                        .or_insert_with(|| portfolio.name.clone());
                    let response = ui.add(
                        egui::TextEdit::singleline(draft)
                            .font(egui::TextStyle::Heading)
                            .frame(egui::Frame::NONE)
                            .desired_width(280.0),
                    );
                    if draft.chars().count() > MAX_NAME_LENGTH {
                        *draft = draft.chars().take(MAX_NAME_LENGTH).collect();
                    }
                    let committed = response.lost_focus();
                    if committed {
                        match rename_commit(draft, &portfolio.name) {
                            Some(name) => actions.push(ManageAction::Rename {
                                id: portfolio.id,
                                name,
                            }),
                            // A blank or unchanged name is put back.
                            None => *draft = portfolio.name.clone(),
                        }
                    } else if !response.has_focus() && *draft != portfolio.name {
                        // Renamed elsewhere while not editing: follow it.
                        *draft = portfolio.name.clone();
                    }
                    ui.label(
                        RichText::new(summary_line(
                            portfolio.base_currency,
                            row.transactions,
                            row.assets,
                        ))
                        .small()
                        .weak(),
                    );
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("🗑").on_hover_text("Delete…").clicked() {
                        state.pending_delete = Some(portfolio.id);
                    }
                    if ui
                        .small_button("⧉")
                        .on_hover_text("Duplicate, including its transactions")
                        .clicked()
                    {
                        actions.push(ManageAction::Duplicate(portfolio.id));
                    }
                    if ui
                        .add_enabled(index + 1 < input.rows.len(), egui::Button::new("▼").small())
                        .on_hover_text("Move down")
                        .clicked()
                        && let Some((from, destination)) = move_down(index, input.rows.len())
                    {
                        actions.push(ManageAction::Reorder { from, destination });
                    }
                    if ui
                        .add_enabled(index > 0, egui::Button::new("▲").small())
                        .on_hover_text("Move up")
                        .clicked()
                        && let Some((from, destination)) = move_up(index)
                    {
                        actions.push(ManageAction::Reorder { from, destination });
                    }
                    if portfolio.is_archived {
                        ui.label(RichText::new("Archived").small().weak());
                    }
                    if input.current == Some(portfolio.id) {
                        ui.label(
                            RichText::new("Viewing")
                                .small()
                                .strong()
                                .color(theme::ACCENT),
                        );
                    }
                });
            });
        });
    ui.add_space(4.0);
}

fn delete_dialog(
    ctx: &egui::Context,
    state: &mut ManageState,
    input: &ManageInput,
    actions: &mut Vec<ManageAction>,
) {
    let Some(id) = state.pending_delete else {
        return;
    };
    let Some(row) = input.rows.iter().find(|row| row.portfolio.id == id) else {
        state.pending_delete = None;
        return;
    };
    let mut done = false;
    egui::Window::new(format!("Delete {}?", row.portfolio.name))
        .id(Id::new("portfolio-delete"))
        .collapsible(false)
        .resizable(false)
        .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
        .show(ctx, |ui| {
            ui.set_max_width(380.0);
            ui.label(delete_message(row.transactions));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui
                    .button(RichText::new("Delete Portfolio and Transactions").color(theme::DOWN))
                    .clicked()
                {
                    actions.push(ManageAction::Delete(id));
                    done = true;
                }
                if ui.button("Cancel").clicked() {
                    done = true;
                }
            });
        });
    if done || ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
        state.pending_delete = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_create_sheet_needs_a_name_and_trims_it() {
        let mut state = CreateState::default();
        assert!(!state.is_valid() && state.outcome().is_none());
        state.name = "   ".into();
        assert!(state.outcome().is_none());
        state.name = "  Long-term holds ".into();
        state.currency = PortfolioCurrency::Eur;
        assert_eq!(
            state.outcome(),
            Some(CreateOutcome::Create {
                name: "Long-term holds".into(),
                currency: PortfolioCurrency::Eur
            })
        );
    }

    #[test]
    fn names_are_cut_to_the_limit_and_duplicates_are_flagged_case_insensitively() {
        let mut state = CreateState {
            name: "x".repeat(60),
            ..CreateState::default()
        };
        state.clamp_name();
        assert_eq!(state.name.chars().count(), MAX_NAME_LENGTH);
        state.name = " main ".into();
        let existing = vec!["Main".to_owned(), "Other".to_owned()];
        assert!(state.is_duplicate(&existing));
        state.name = "Mainframe".into();
        assert!(!state.is_duplicate(&existing));
        state.name = "".into();
        assert!(
            !state.is_duplicate(&existing),
            "blank is invalid, not a duplicate"
        );
        assert_eq!(CreateState::default().currency, PortfolioCurrency::Usd);
    }

    #[test]
    fn summaries_pluralise_each_count() {
        assert_eq!(
            summary_line(PortfolioCurrency::Usd, 1, 1),
            "USD · 1 transaction · 1 asset"
        );
        assert_eq!(
            summary_line(PortfolioCurrency::Eur, 0, 3),
            "EUR · 0 transactions · 3 assets"
        );
    }

    #[test]
    fn arrows_reorder_by_insertion_point() {
        assert_eq!(move_up(0), None);
        assert_eq!(move_up(2), Some((vec![2], 1)));
        assert_eq!(move_down(2, 3), None);
        assert_eq!(
            move_down(0, 3),
            Some((vec![0], 2)),
            "insertion point is past the neighbour"
        );
        assert_eq!(move_down(1, 3), Some((vec![1], 3)));
    }

    /// `PortfolioLedger::reorder`'s rule: `destination` is an insertion point in the list as
    /// it stands, so removing items above it shifts it down.
    fn reorder(order: &mut Vec<u64>, from: &[usize], destination: usize) {
        let moving: Vec<u64> = from.iter().map(|&index| order[index]).collect();
        let mut descending = from.to_vec();
        descending.sort_unstable_by(|a, b| b.cmp(a));
        for index in descending {
            order.remove(index);
        }
        let removed_before = from.iter().filter(|&&index| index < destination).count();
        let insert_at = destination.saturating_sub(removed_before);
        order.splice(insert_at..insert_at, moving);
    }

    #[test]
    fn the_arrows_move_one_place_under_the_ledgers_reorder_rule() {
        let mut order = vec![1, 2, 3];
        let (from, destination) = move_down(0, 3).unwrap();
        reorder(&mut order, &from, destination);
        assert_eq!(order, [2, 1, 3]);
        let mut order = vec![1, 2, 3];
        let (from, destination) = move_up(2).unwrap();
        reorder(&mut order, &from, destination);
        assert_eq!(order, [1, 3, 2]);
        let mut order = vec![1, 2, 3];
        let (from, destination) = move_down(1, 3).unwrap();
        reorder(&mut order, &from, destination);
        assert_eq!(order, [1, 3, 2]);
    }

    #[test]
    fn a_rename_commits_only_a_changed_non_blank_name() {
        assert_eq!(rename_commit("  Growth ", "Main"), Some("Growth".into()));
        assert_eq!(rename_commit("Main", "Main"), None);
        assert_eq!(
            rename_commit("   ", "Main"),
            None,
            "a blank name is put back"
        );
        assert_eq!(
            rename_commit(&"y".repeat(60), "Main")
                .unwrap()
                .chars()
                .count(),
            MAX_NAME_LENGTH
        );
    }

    #[test]
    fn deleting_warns_with_the_transaction_count() {
        assert!(delete_message(1).contains("its 1 transaction."));
        assert!(delete_message(5).contains("its 5 transactions."));
        assert!(delete_message(0).contains("0 transactions"));
    }
}
