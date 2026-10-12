//! Small builtins that carry no state: `max_bars_back`, the `ticker.*` builders, drawing-type
//! casts (`box(na)`), the `log.*` no-ops and the `request.*` data feeds this app does not have.
//! The feeds read as `na`, and `ignored_features` warns about every call, so none is silent.

use super::ast::Argument;
use super::runtime_types::RuntimeContext;
use super::runtime_value::{RefKind, RuntimeValue};
use super::session::Session;
use super::token::{Diagnostic, SourceRange};

/// `request.*` functions that return a series from a feed the app has no access to.
pub const UNAVAILABLE_FEEDS: [&str; 8] = [
    "request.seed",
    "request.dividends",
    "request.earnings",
    "request.economic",
    "request.financial",
    "request.quandl",
    "request.splits",
    "request.currency_rate",
];

/// `ticker.*` builders that would change how the symbol is charted. The plain symbol comes back.
pub const CHART_TYPE_TICKERS: [&str; 5] = [
    "ticker.heikinashi",
    "ticker.renko",
    "ticker.kagi",
    "ticker.linebreak",
    "ticker.pointfigure",
];

fn drawing_kind(name: &str) -> Option<RefKind> {
    Some(match name {
        "line" => RefKind::Line,
        "label" => RefKind::Label,
        "box" => RefKind::Box,
        "table" => RefKind::Table,
        "linefill" => RefKind::Linefill,
        "polyline" => RefKind::Polyline,
        _ => return None,
    })
}

impl Session {
    pub(crate) fn misc_call(
        &mut self,
        name: &str,
        arguments: &[Argument],
        _range: SourceRange,
        context: &mut RuntimeContext,
    ) -> Result<Option<RuntimeValue>, Diagnostic> {
        if let Some(kind) = drawing_kind(name) {
            // `box(na)` types an `na`; a handle of the same kind passes through.
            let value = self.arg_value(arguments, 0, None, context)?;
            return Ok(Some(match value {
                RuntimeValue::Ref(found, _) if found == kind => value,
                _ => RuntimeValue::Na,
            }));
        }
        Ok(Some(match name {
            "max_bars_back" => {
                // The whole history is always kept, so a larger buffer changes nothing.
                RuntimeValue::Void
            }
            "log.info" | "log.warning" | "log.error" => {
                self.eval_all(arguments, context)?;
                RuntimeValue::Void
            }
            "ticker.new" => {
                let text = |value: &RuntimeValue| value.text_value().unwrap_or_default().to_owned();
                let prefix = text(&self.arg_value(arguments, 0, Some("prefix"), context)?);
                let ticker = text(&self.arg_value(arguments, 1, Some("ticker"), context)?);
                RuntimeValue::String(if prefix.is_empty() {
                    ticker
                } else {
                    format!("{prefix}:{ticker}")
                })
            }
            "ticker.standard" | "ticker.modify" | "ticker.inherit" => {
                // The modifiers (session, adjustment) are not modelled: the symbol stays as given,
                // for `inherit` the one that takes the modifiers.
                let index = usize::from(name == "ticker.inherit");
                self.eval_all(arguments, context)?
                    .get(index)
                    .cloned()
                    .unwrap_or(RuntimeValue::Na)
            }
            _ if CHART_TYPE_TICKERS.contains(&name) => self
                .eval_all(arguments, context)?
                .first()
                .cloned()
                .unwrap_or(RuntimeValue::Na),
            "request.currency_rate" => {
                let values = self.eval_all(arguments, context)?;
                match (values.first(), values.get(1)) {
                    (Some(from), Some(to)) if from.text_value().is_some() && from == to => {
                        RuntimeValue::Float(1.0)
                    }
                    _ => RuntimeValue::Na,
                }
            }
            _ if UNAVAILABLE_FEEDS.contains(&name) => RuntimeValue::Na,
            _ => return Ok(None),
        }))
    }
}
