//! Call-site wiring for the `ta.*` functions in [`ta_extra`](super::ta_extra): binds the Pine
//! parameters, finds the call site's state and hands both to the pure implementation.

use super::ast::Argument;
use super::runtime_types::RuntimeContext;
use super::runtime_value::RuntimeValue;
use super::session::Session;
use super::ta_extra;
use super::token::{Diagnostic, SourceRange};

/// Where the cumulative variables keep their state in `ta_states`, clear of any call-site key.
const VARIABLE_KEYS: [&str; 8] = [
    "ta.obv",
    "ta.accdist",
    "ta.iii",
    "ta.nvi",
    "ta.pvi",
    "ta.pvt",
    "ta.wad",
    "ta.wvad",
];

fn number(
    bound: &std::collections::HashMap<String, RuntimeValue>,
    name: &str,
    default: f64,
) -> f64 {
    bound
        .get(name)
        .and_then(RuntimeValue::number)
        .unwrap_or(default)
}

fn length(bound: &std::collections::HashMap<String, RuntimeValue>, name: &str) -> i64 {
    bound
        .get(name)
        .and_then(RuntimeValue::int_value)
        .unwrap_or(0)
}

impl Session {
    /// `ta.alma`, `ta.mfi`, `ta.valuewhen`, … None when `name` is not one of them.
    pub(crate) fn extra_ta_call(
        &mut self,
        name: &str,
        arguments: &[Argument],
        site: usize,
        _range: SourceRange,
        context: &mut RuntimeContext,
    ) -> Result<Option<RuntimeValue>, Diagnostic> {
        let parameters: &[&str] = match name {
            "ta.alma" => &["series", "length", "offset", "sigma", "floor"],
            "ta.cog" | "ta.mode" => &["source", "length"],
            "ta.percentile_nearest_rank" | "ta.percentile_linear_interpolation" => {
                &["source", "length", "percentage"]
            }
            "ta.valuewhen" => &["condition", "source", "occurrence"],
            "ta.mfi" => &["series", "length"],
            "ta.vwma" => &["source", "length"],
            "ta.stoch" => &["source", "high", "low", "length"],
            "ta.wpr" => &["length"],
            "ta.kc" | "ta.kcw" => &["series", "length", "mult", "useTrueRange"],
            "ta.bbw" => &["series", "length", "mult"],
            "ta.tsi" => &["source", "short_length", "long_length"],
            _ => return Ok(None),
        };
        let bound = self.bind_named(arguments, parameters, context)?;
        let na = RuntimeValue::Na;
        let source = |key: &str| bound.get(key).unwrap_or(&na).clone();
        let key = self.site_key(site, context);
        let bar = context.bar.clone();
        let state = self.working.ta_states.entry(key).or_default();
        let value = match name {
            "ta.alma" => ta_extra::alma(
                &source("series"),
                length(&bound, "length"),
                number(&bound, "offset", 0.85),
                number(&bound, "sigma", 6.0),
                bound
                    .get("floor")
                    .and_then(RuntimeValue::bool)
                    .unwrap_or(false),
                state,
            ),
            "ta.cog" => ta_extra::cog(&source("source"), length(&bound, "length"), state),
            "ta.mode" => ta_extra::mode(&source("source"), length(&bound, "length"), state),
            "ta.percentile_nearest_rank" | "ta.percentile_linear_interpolation" => {
                ta_extra::percentile(
                    &source("source"),
                    length(&bound, "length"),
                    number(&bound, "percentage", f64::NAN),
                    name == "ta.percentile_linear_interpolation",
                    state,
                )
            }
            "ta.valuewhen" => ta_extra::valuewhen(
                &source("condition"),
                &source("source"),
                length(&bound, "occurrence"),
                state,
            ),
            "ta.mfi" => ta_extra::mfi(&source("series"), length(&bound, "length"), &bar, state),
            "ta.vwma" => ta_extra::vwma(&source("source"), length(&bound, "length"), &bar, state),
            "ta.stoch" => ta_extra::stoch(
                &source("source"),
                &source("high"),
                &source("low"),
                length(&bound, "length"),
                state,
            ),
            "ta.wpr" => ta_extra::wpr(length(&bound, "length"), &bar, state),
            "ta.kc" => {
                let (basis, upper, lower) = ta_extra::keltner(
                    &source("series"),
                    length(&bound, "length"),
                    number(&bound, "mult", f64::NAN),
                    bound
                        .get("useTrueRange")
                        .and_then(RuntimeValue::bool)
                        .unwrap_or(true),
                    &bar,
                    state,
                );
                let float =
                    |value: Option<f64>| value.map_or(RuntimeValue::Na, RuntimeValue::Float);
                RuntimeValue::Tuple(vec![float(basis), float(upper), float(lower)])
            }
            "ta.kcw" => ta_extra::keltner_width(
                &source("series"),
                length(&bound, "length"),
                number(&bound, "mult", f64::NAN),
                bound
                    .get("useTrueRange")
                    .and_then(RuntimeValue::bool)
                    .unwrap_or(true),
                &bar,
                state,
            ),
            "ta.bbw" => ta_extra::bollinger_width(
                &source("series"),
                length(&bound, "length"),
                number(&bound, "mult", 2.0),
                state,
            ),
            _ => ta_extra::tsi(
                &source("source"),
                length(&bound, "short_length"),
                length(&bound, "long_length"),
                state,
            ),
        };
        Ok(Some(value))
    }

    /// `ta.obv`, `ta.accdist`, … read as variables. None when `name` is not one of them.
    pub(crate) fn cumulative_variable(
        &mut self,
        name: &str,
        context: &RuntimeContext,
    ) -> Option<RuntimeValue> {
        let slot = VARIABLE_KEYS
            .iter()
            .position(|candidate| *candidate == name)?;
        let key = usize::MAX - slot;
        // The state leaves the map while it reads the committed history out of `working`.
        let mut state = self.working.ta_states.remove(&key).unwrap_or_default();
        let value = ta_extra::cumulative(name, &context.bar, &self.working, &mut state);
        self.working.ta_states.insert(key, state);
        value
    }
}
