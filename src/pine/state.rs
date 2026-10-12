//! Everything a script run accumulates, plus the alert event and `request.security` site the
//! state holds. Ported from `PineRuntimeState.swift`, `PineAlertEvent.swift`, `PineSecuritySite.swift`.

use std::collections::HashMap;

use super::alert_frequency::AlertFrequency;
use super::broker::Broker;
use super::containers::{Map, Matrix, Object};
use super::outputs::{
    BoxOutput, CandleOutput, ColorOutput, FillOutput, HorizontalLine, LabelOutput, LineOutput,
    LinefillOutput, MarkerOutput, PlotOutput, PolylineOutput, TableOutput,
};
use super::runtime_value::RuntimeValue;
use super::ta;
use crate::model::Kline;

/// An `alert()` / `alertcondition()` that fired.
#[derive(Clone, Debug, PartialEq)]
pub struct AlertEvent {
    pub id: usize,
    /// Call-site key, so `alert.freq_once_per_bar` can fire once per bar per call.
    pub site: usize,
    pub bar: usize,
    pub time: i64,
    pub message: String,
    pub frequency: AlertFrequency,
    /// Whether the execution ran on a realtime bar. Historical executions never notify.
    pub is_realtime: bool,
    /// Whether the execution was the bar's closing one.
    pub is_confirmed: bool,
}

/// What one `request.security` call site remembers between chart bars.
#[derive(Clone, Debug)]
pub struct SecuritySite {
    /// Open of the higher-timeframe bucket the chart bars are currently in.
    pub bucket: Option<i64>,
    /// The confirmed chart bars of that bucket so far, folded into one candle.
    pub accumulated: Option<Kline>,
    /// Whether `accumulated` holds bars whose bucket has not been completed yet.
    pub has_pending_bars: bool,
    /// Whether the history from before the chart's first bar has been fed in.
    pub is_warmed_up: bool,
    /// Another symbol's series: how many of its candles have been completed and evaluated.
    pub completed_candles: usize,
    /// The expression's own state, not shared with the chart script.
    pub state: Box<RuntimeState>,
    /// The expression's value on the last completed higher-timeframe bar.
    pub last_result: RuntimeValue,
    /// The chart bar this site last answered, and what it answered, for a second call.
    pub processed_bar: Option<i64>,
    pub bar_result: RuntimeValue,
}

impl Default for SecuritySite {
    fn default() -> Self {
        Self {
            bucket: None,
            accumulated: None,
            has_pending_bars: false,
            is_warmed_up: false,
            completed_candles: 0,
            state: Box::default(),
            last_result: RuntimeValue::Na,
            processed_bar: None,
            bar_result: RuntimeValue::Na,
        }
    }
}

/// Everything a script run accumulates. The session keeps a `committed` and a `working` copy:
/// a realtime tick runs on `working` and is thrown away unless the bar confirms.
#[derive(Clone, Debug, Default)]
pub struct RuntimeState {
    pub variables: HashMap<String, RuntimeValue>,
    pub histories: HashMap<String, Vec<RuntimeValue>>,
    /// This bar's value of every `expression[n]` whose base is not a plain name, by history key.
    pub expression_values: HashMap<String, RuntimeValue>,
    pub calls: HashMap<usize, Vec<RuntimeValue>>,
    pub call_inputs: HashMap<usize, Vec<RuntimeValue>>,
    /// Complete state for each `ta.*` call site. It belongs to the working state so an
    /// unconfirmed realtime tick is rolled back together with variables and containers.
    pub ta_states: HashMap<usize, ta::State>,
    pub plots: HashMap<usize, PlotOutput>,
    pub hlines: HashMap<usize, HorizontalLine>,
    pub markers: HashMap<usize, MarkerOutput>,
    pub backgrounds: HashMap<usize, ColorOutput>,
    pub bar_colors: HashMap<usize, ColorOutput>,
    pub fills: HashMap<usize, FillOutput>,
    // Reference-typed objects live in the state so realtime rollback restores them
    // together with the variables that point at them.
    pub arrays: HashMap<usize, Vec<RuntimeValue>>,
    pub maps: HashMap<usize, Map>,
    pub matrices: HashMap<usize, Matrix>,
    /// Instances of script-defined types, by handle.
    pub instances: HashMap<usize, Object>,
    /// `request.security` call sites, by call-site key. Each holds a state of its own.
    pub securities: HashMap<usize, SecuritySite>,
    pub lines: HashMap<usize, LineOutput>,
    pub labels: HashMap<usize, LabelOutput>,
    pub boxes: HashMap<usize, BoxOutput>,
    pub linefills: HashMap<usize, LinefillOutput>,
    pub polylines: HashMap<usize, PolylineOutput>,
    pub tables: HashMap<usize, TableOutput>,
    pub candles: HashMap<usize, CandleOutput>,
    pub alerts: Vec<AlertEvent>,
    pub broker: Broker,
    pub next_reference: usize,
    pub bar_index: i64,
    pub instructions: usize,
}
