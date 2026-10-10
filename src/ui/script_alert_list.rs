//! The Alerts window's Scripts tab: subscriptions to Pine `alert()` calls, each with its status and
//! Pause / Re-arm / Delete. A port of `PineAlertListView` (the "Fired" history half needs a stored
//! script-notification log the Rust app does not keep yet).

use super::alert_center::{
    AlertCenterAction, GUTTER, Tone, card_rect, centered, fitted, paint_card, tone_color,
};
use crate::model::DataSource;
use crate::model::pine_alert::{PineAlertState, PineAlertSubscription};
use eframe::egui::{self, Align2, Color32, FontId, Id, Pos2, Rect, RichText, Sense, Vec2};

/// One script alert as the list shows it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScriptAlertRow {
    pub id: u64,
    pub script_name: String,
    /// The ticker part of the symbol key.
    pub symbol: String,
    pub source: Option<DataSource>,
    pub timeframe: String,
    pub note: String,
    pub webhook_count: usize,
    pub status: &'static str,
    pub tone: Tone,
    pub active: bool,
    pub can_rearm: bool,
    pub created_at_ms: i64,
}

/// What a script alert's badge says: whether it can fire now, and if not, why. `dataset` is the
/// `(symbol_key, timeframe)` of the chart the alert was created on, when that chart is open.
pub fn status_of(
    subscription: &PineAlertSubscription,
    dataset: Option<(&str, &str)>,
) -> (&'static str, Tone) {
    match subscription.state {
        PineAlertState::Paused => ("Paused", Tone::Neutral),
        PineAlertState::ScriptChanged => ("Script changed — re-arm", Tone::Warning),
        PineAlertState::ScriptDeleted => ("Script deleted", Tone::Bad),
        PineAlertState::Active => match dataset {
            None => ("Chart not open", Tone::Neutral),
            Some((key, timeframe)) if subscription.watches(key, timeframe) => {
                ("Active", Tone::Good)
            }
            Some(_) => ("Other symbol on chart", Tone::Warning),
        },
    }
}

/// The data source a `"<SLUG>:<ticker>"` key's prefix names.
fn source_of(prefix: &str) -> Option<DataSource> {
    [
        DataSource::Binance,
        DataSource::Coinbase,
        DataSource::CoinGecko,
        DataSource::DexScreener,
        DataSource::Alpaca,
        DataSource::Polymarket,
        DataSource::Kalshi,
        DataSource::CoinMarketCap,
    ]
    .into_iter()
    .find(|source| source.watchlist_slug().eq_ignore_ascii_case(prefix))
}

impl ScriptAlertRow {
    pub fn from_subscription(
        subscription: &PineAlertSubscription,
        dataset: Option<(&str, &str)>,
    ) -> Self {
        let (status, tone) = status_of(subscription, dataset);
        let (prefix, ticker) = subscription
            .symbol_key
            .split_once(':')
            .unwrap_or(("", &subscription.symbol_key));
        Self {
            id: subscription.id,
            script_name: subscription.script_name.clone(),
            symbol: ticker.to_owned(),
            source: source_of(prefix),
            timeframe: subscription.timeframe.clone(),
            note: subscription.note.clone(),
            webhook_count: subscription.webhook_endpoint_ids.len(),
            status,
            tone,
            active: subscription.is_active(),
            // A chart showing this market must be open to re-arm; a deleted script can't be.
            can_rearm: subscription.state != PineAlertState::ScriptDeleted
                && dataset.is_some_and(|(key, timeframe)| subscription.watches(key, timeframe)),
            created_at_ms: subscription.created_at_ms,
        }
    }

    /// "BTCUSDT · 1h · 2 webhooks", or the note in place of the market.
    pub fn detail(&self) -> String {
        let base = if self.note.is_empty() {
            format!("{} · {}", self.symbol, self.timeframe)
        } else {
            self.note.clone()
        };
        match self.webhook_count {
            0 => base,
            1 => format!("{base} · 1 webhook"),
            n => format!("{base} · {n} webhooks"),
        }
    }
}

/// Rows matching the search (script name or symbol), newest first.
pub fn rows_for<'a>(rows: &'a [ScriptAlertRow], search: &str) -> Vec<&'a ScriptAlertRow> {
    let needle = search.trim().to_lowercase();
    let mut found: Vec<&ScriptAlertRow> = rows
        .iter()
        .filter(|row| {
            needle.is_empty()
                || row.script_name.to_lowercase().contains(&needle)
                || row.symbol.to_lowercase().contains(&needle)
        })
        .collect();
    found.sort_by_key(|row| std::cmp::Reverse(row.created_at_ms));
    found
}

pub fn view(
    ui: &mut egui::Ui,
    search: &str,
    all: &[ScriptAlertRow],
    actions: &mut Vec<AlertCenterAction>,
) {
    let rows = rows_for(all, search);
    let footer_height = 44.0;
    let body_height = (ui.available_height() - footer_height).max(60.0);
    ui.allocate_ui(Vec2::new(ui.available_width(), body_height), |ui| {
        if rows.is_empty() {
            if all.is_empty() {
                centered(
                    ui,
                    "No Script Alerts",
                    "Apply a script with alert() calls, then create an alert from it.",
                );
            } else {
                centered(ui, "No Results", "Nothing matches this search.");
            }
            return;
        }
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 6.0;
                ui.horizontal(|ui| {
                    ui.add_space(GUTTER);
                    ui.label(RichText::new("Alerts").strong());
                    ui.label(RichText::new(rows.len().to_string()).small().weak());
                });
                for row in rows {
                    card(ui, row, actions);
                }
                ui.add_space(8.0);
            });
    });

    ui.separator();
    ui.horizontal(|ui| {
        ui.add_space(GUTTER);
        ui.label(
            RichText::new("Script alerts are created from alert() calls in your scripts.")
                .small()
                .weak(),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(GUTTER);
            if ui
                .button("{} Open Script Manager")
                .on_hover_text("Write and manage the scripts whose alert() calls appear here")
                .clicked()
            {
                actions.push(AlertCenterAction::OpenScriptManager);
            }
        });
    });
}

fn card(ui: &mut egui::Ui, row: &ScriptAlertRow, actions: &mut Vec<AlertCenterAction>) {
    let (rect, response) = card_rect(ui);
    let hovered = response.hovered();
    let painter = ui.painter_at(rect);
    let dim = if row.active { 1.0 } else { 0.7 };
    paint_card(&painter, rect, hovered);
    let text = ui.visuals().text_color().gamma_multiply(dim);
    let weak = ui.visuals().weak_text_color().gamma_multiply(dim);
    let center_y = rect.center().y;

    // A script glyph in a tinted disc, the market's source marked on the corner.
    let icon = Rect::from_center_size(Pos2::new(rect.left() + 28.0, center_y), Vec2::splat(34.0));
    let accent = crate::ui::theme::SYSTEM_ACCENT;
    painter.circle_filled(icon.center(), 17.0, accent.gamma_multiply(0.14));
    crate::ui::icons::paint(
        &painter,
        Rect::from_center_size(icon.center(), Vec2::splat(15.0)),
        crate::ui::icons::Icon::Curlybraces,
        accent,
    );
    if let Some(source) = row.source {
        let mark = icon.right_bottom() + Vec2::new(-1.0, -1.0);
        painter.circle(
            mark,
            7.0,
            crate::ui::theme::SURFACE,
            egui::Stroke::new(1.0, crate::ui::theme::BORDER),
        );
        painter.text(
            mark,
            Align2::CENTER_CENTER,
            source.glyph(),
            FontId::proportional(8.5),
            Color32::from_gray(200),
        );
    }

    // Right to left: the hover buttons, then the status badge.
    let actions_left = rect.right() - 12.0 - 60.0;
    let status_color = tone_color(row.tone);
    let status = painter.layout_no_wrap(
        row.status.to_owned(),
        FontId::proportional(11.0),
        status_color.gamma_multiply(dim),
    );
    let badge = Rect::from_min_size(
        Pos2::new(actions_left - 8.0 - status.size().x - 18.0, center_y - 10.0),
        Vec2::new(status.size().x + 18.0, 20.0),
    );
    painter.rect_filled(badge, 999.0, status_color.gamma_multiply(0.16 * dim));
    painter.galley(badge.min + Vec2::new(9.0, 3.0), status, status_color);

    let name_left = rect.left() + 56.0;
    let width = (badge.left() - name_left - 12.0).max(40.0);
    let name = fitted(
        &painter,
        &row.script_name,
        FontId::proportional(14.0),
        text,
        width,
    );
    let detail = fitted(
        &painter,
        &row.detail(),
        FontId::proportional(11.0),
        weak,
        width,
    );
    let top = center_y - (name.size().y + 2.0 + detail.size().y) / 2.0;
    painter.galley(Pos2::new(name_left, top), name.clone(), text);
    painter.galley(
        Pos2::new(name_left, top + name.size().y + 2.0),
        detail,
        weak,
    );

    // Pause / re-arm as a hover button, like the price-alert cards.
    let (glyph, tip, enabled, action) = if row.active {
        ("⏸", "Pause", true, AlertCenterAction::PauseScript(row.id))
    } else {
        (
            "▶",
            "Re-arm",
            row.can_rearm,
            AlertCenterAction::RearmScript(row.id),
        )
    };
    let button =
        Rect::from_center_size(Pos2::new(actions_left + 13.0, center_y), Vec2::splat(26.0));
    let button_response = ui.interact(
        button,
        Id::new(("script-alert-toggle", row.id)),
        Sense::click(),
    );
    if (hovered || button_response.hovered()) && enabled {
        if button_response.hovered() {
            painter.rect_filled(button, 6.0, Color32::from_white_alpha(20));
        }
        painter.text(
            button.center(),
            Align2::CENTER_CENTER,
            glyph,
            FontId::proportional(13.0),
            ui.visuals().weak_text_color(),
        );
    }
    if enabled && button_response.on_hover_text(tip).clicked() {
        actions.push(action.clone());
    }

    response.context_menu(|ui| {
        if row.active {
            if ui.button("Pause").clicked() {
                actions.push(AlertCenterAction::PauseScript(row.id));
                ui.close();
            }
        } else if ui
            .add_enabled(row.can_rearm, egui::Button::new("Re-arm"))
            .on_disabled_hover_text("Open its chart on the same market to re-arm it")
            .clicked()
        {
            actions.push(AlertCenterAction::RearmScript(row.id));
            ui.close();
        }
        ui.separator();
        if ui
            .button(RichText::new("Delete").color(crate::ui::theme::DOWN))
            .clicked()
        {
            actions.push(AlertCenterAction::DeleteScript(row.id));
            ui.close();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn subscription(state: PineAlertState) -> PineAlertSubscription {
        PineAlertSubscription {
            id: 4,
            chart_id: 1,
            instance_id: None,
            script_id: None,
            script_name: "Momentum".into(),
            source: String::new(),
            symbol_key: "binance:BTCUSDT".into(),
            timeframe: "1h".into(),
            source_hash: "h".into(),
            note: String::new(),
            state,
            created_at_ms: 5,
            webhook_endpoint_ids: vec![1, 2],
        }
    }

    #[test]
    fn the_badge_says_why_an_alert_cannot_fire() {
        let active = subscription(PineAlertState::Active);
        assert_eq!(status_of(&active, None), ("Chart not open", Tone::Neutral));
        assert_eq!(
            status_of(&active, Some(("binance:BTCUSDT", "1h"))),
            ("Active", Tone::Good)
        );
        assert_eq!(
            status_of(&active, Some(("binance:ETHUSDT", "1h"))),
            ("Other symbol on chart", Tone::Warning)
        );
        assert_eq!(
            status_of(&subscription(PineAlertState::Paused), None),
            ("Paused", Tone::Neutral)
        );
        assert_eq!(
            status_of(&subscription(PineAlertState::ScriptChanged), None).0,
            "Script changed — re-arm"
        );
        assert_eq!(
            status_of(&subscription(PineAlertState::ScriptDeleted), None),
            ("Script deleted", Tone::Bad)
        );
    }

    #[test]
    fn a_row_reads_market_webhooks_and_when_it_can_be_re_armed() {
        let paused = subscription(PineAlertState::Paused);
        let open = ScriptAlertRow::from_subscription(&paused, Some(("binance:BTCUSDT", "1h")));
        assert_eq!(open.detail(), "BTCUSDT · 1h · 2 webhooks");
        assert!(open.can_rearm && !open.active);
        let closed = ScriptAlertRow::from_subscription(&paused, None);
        assert!(!closed.can_rearm);
        let deleted = ScriptAlertRow::from_subscription(
            &subscription(PineAlertState::ScriptDeleted),
            Some(("binance:BTCUSDT", "1h")),
        );
        assert!(!deleted.can_rearm, "a deleted script can't be re-armed");
        let mut noted = subscription(PineAlertState::Active);
        noted.note = "Breakout".into();
        noted.webhook_endpoint_ids = vec![1];
        assert_eq!(
            ScriptAlertRow::from_subscription(&noted, None).detail(),
            "Breakout · 1 webhook"
        );
    }

    #[test]
    fn search_matches_name_or_symbol_newest_first() {
        let mut rows = vec![
            ScriptAlertRow::from_subscription(&subscription(PineAlertState::Active), None),
            ScriptAlertRow::from_subscription(&subscription(PineAlertState::Active), None),
        ];
        rows[1].script_name = "Trend".into();
        rows[1].created_at_ms = 9;
        let names = |search: &str| -> Vec<String> {
            rows_for(&rows, search)
                .iter()
                .map(|r| r.script_name.clone())
                .collect()
        };
        assert_eq!(names(""), ["Trend", "Momentum"]);
        assert_eq!(names("mom"), ["Momentum"]);
        assert_eq!(names("btc"), ["Trend", "Momentum"]);
        assert!(names("zzz").is_empty());
    }

    #[test]
    fn the_list_draws_headlessly_empty_populated_and_searched() {
        let rows = [
            ScriptAlertRow::from_subscription(
                &subscription(PineAlertState::Active),
                Some(("binance:BTCUSDT", "1h")),
            ),
            ScriptAlertRow::from_subscription(&subscription(PineAlertState::Paused), None),
        ];
        let ctx = egui::Context::default();
        for (search, list) in [("", &rows[..0]), ("", &rows[..]), ("zzz", &rows[..])] {
            let mut actions = Vec::new();
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                view(ui, search, list, &mut actions);
            });
            output.textures_delta.clear();
            assert!(actions.is_empty(), "nothing was clicked");
        }
    }
}
