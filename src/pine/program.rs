//! The compiled program and the script's declaration, inputs, bar events, symbol facts, theme
//! and visual output, ported from `Pine/Model/*` + `PineCompiledProgram`.

use std::collections::HashSet;

use super::ast::Statement;
use super::outputs::{
    BoxOutput, CandleOutput, ColorOutput, FillOutput, HorizontalLine, LabelOutput, LineOutput,
    LinefillOutput, MarkerOutput, PlotOutput, PolylineOutput, TableOutput,
};
use super::state::AlertEvent;
use super::strategy::{StrategyReport, StrategySettings};
use crate::model::Kline;
use crate::pine::{Diagnostic, SourceRange, ValueType};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ScriptType {
    #[default]
    Indicator,
    Strategy,
    Library,
}

impl ScriptType {
    pub fn display_name(self) -> &'static str {
        match self {
            Self::Indicator => "Indicator",
            Self::Strategy => "Strategy",
            Self::Library => "Library",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum InputValue {
    Int(i64),
    Float(f64),
    Bool(bool),
    String(String),
    Color(u32),
    Source(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct InputDefinition {
    pub id: String,
    pub kind: ValueType,
    pub default_value: InputValue,
    pub title: Option<String>,
    pub tooltip: Option<String>,
    pub group: Option<String>,
    pub inline: Option<String>,
    pub confirm: bool,
    pub min_value: Option<f64>,
    pub max_value: Option<f64>,
    pub step: Option<f64>,
    pub options: Option<Vec<InputValue>>,
    /// What to show for each of `options`, when it differs from the value.
    pub option_titles: Option<Vec<String>>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct InputSchema {
    pub inputs: Vec<InputDefinition>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DeclarationMetadata {
    pub script_type: ScriptType,
    pub pine_version: Option<i64>,
    pub title: String,
    pub short_title: Option<String>,
    pub overlay: bool,
    pub format: Option<String>,
    pub precision: Option<i64>,
    pub max_bars_back: Option<i64>,
    pub max_lines_count: Option<i64>,
    pub max_labels_count: Option<i64>,
    pub max_boxes_count: Option<i64>,
    pub max_polylines_count: Option<i64>,
    /// `behind_chart`: an overlay script's plots and drawings sit behind the candles.
    pub behind_chart: Option<bool>,
    /// Present for `strategy()` scripts.
    pub strategy: Option<StrategySettings>,
}

impl Default for DeclarationMetadata {
    fn default() -> Self {
        Self {
            script_type: ScriptType::Indicator,
            pine_version: None,
            title: String::new(),
            short_title: None,
            overlay: false,
            format: None,
            precision: None,
            max_bars_back: None,
            max_lines_count: None,
            max_labels_count: None,
            max_boxes_count: None,
            max_polylines_count: None,
            behind_chart: None,
            strategy: None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct BarEvent {
    pub candle: Kline,
    pub phase: BarPhase,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BarPhase {
    Historical,
    RealtimeTick { is_new: bool },
    RealtimeClose { is_new: bool },
}

impl BarPhase {
    /// Whether the bar's values are final.
    pub fn is_confirmed(self) -> bool {
        !matches!(self, Self::RealtimeTick { .. })
    }

    pub fn is_realtime(self) -> bool {
        !matches!(self, Self::Historical)
    }
}

/// Symbol facts a script can read through `syminfo.*`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SymbolInfo {
    pub ticker: String,
    pub ticker_id: String,
    pub currency: String,
    pub kind: String,
    /// The asset a pair is priced in terms of (`BTC` of `BTCUSDT`); None when not a pair.
    pub base_currency: Option<String>,
}

impl Default for SymbolInfo {
    fn default() -> Self {
        Self {
            ticker: String::new(),
            ticker_id: String::new(),
            currency: "USD".to_owned(),
            kind: "crypto".to_owned(),
            base_currency: None,
        }
    }
}

const QUOTES: [&str; 14] = [
    "FDUSD", "USDT", "USDC", "BUSD", "TUSD", "USD", "EUR", "GBP", "TRY", "JPY", "BRL", "BTC",
    "ETH", "BNB",
];

/// The base of an exchange pair ticker: `BTC` from `BTC-USD`, `BTC/USD` and `BTCUSDT`.
pub fn base_currency_of_pair(ticker: &str) -> Option<String> {
    let upper = ticker.to_uppercase();
    if let Some(index) = upper.find(['-', '/', '_']) {
        let base = &upper[..index];
        return if base.is_empty() {
            None
        } else {
            Some(base.to_owned())
        };
    }
    for quote in QUOTES {
        if upper.len() > quote.len() && upper.ends_with(quote) {
            return Some(upper[..upper.len() - quote.len()].to_owned());
        }
    }
    None
}

/// `chart.fg_color` / `chart.bg_color`.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ChartTheme {
    pub foreground: u32,
    pub background: u32,
}

impl ChartTheme {
    pub const DARK: Self = Self {
        foreground: 0xd1d4_dcff,
        background: 0x1317_22ff,
    };
    pub const LIGHT: Self = Self {
        foreground: 0x1317_22ff,
        background: 0xffff_ffff,
    };
}

impl Default for ChartTheme {
    fn default() -> Self {
        Self::DARK
    }
}

/// The rendered output of one full script run.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct VisualOutput {
    pub overlay: bool,
    /// Number of bars the script ran over.
    pub bar_count: usize,
    pub plots: Vec<PlotOutput>,
    pub hlines: Vec<HorizontalLine>,
    pub markers: Vec<MarkerOutput>,
    pub backgrounds: Vec<ColorOutput>,
    pub bar_colors: Vec<ColorOutput>,
    pub fills: Vec<FillOutput>,
    pub lines: Vec<LineOutput>,
    pub labels: Vec<LabelOutput>,
    pub boxes: Vec<BoxOutput>,
    pub linefills: Vec<LinefillOutput>,
    pub polylines: Vec<PolylineOutput>,
    pub tables: Vec<TableOutput>,
    pub candles: Vec<CandleOutput>,
    pub alerts: Vec<AlertEvent>,
    pub strategy: Option<StrategyReport>,
    /// `behind_chart` of an overlay script.
    pub behind_chart: bool,
}

#[derive(Clone, Debug)]
pub struct Import {
    pub path: String,
    pub alias: String,
    pub range: SourceRange,
    pub library: Option<Box<CompiledProgram>>,
}

#[derive(Clone, Debug)]
pub struct CompiledProgram {
    pub source: String,
    pub statements: Vec<Statement>,
    pub declaration: DeclarationMetadata,
    pub input_schema: InputSchema,
    pub diagnostics: Vec<Diagnostic>,
    /// Names declared with `method`, which `value.name(…)` resolves ahead of builtins.
    pub method_names: HashSet<String>,
    /// Names a library declares with `export`.
    pub exported_names: HashSet<String>,
    /// The libraries this script imports, with the compiled program of each that was found.
    pub imports: Vec<Import>,
}

impl CompiledProgram {
    pub fn is_valid(&self) -> bool {
        !self
            .diagnostics
            .iter()
            .any(|d| d.severity == crate::pine::DiagnosticSeverity::Error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_currency_splits_pairs() {
        assert_eq!(base_currency_of_pair("BTCUSDT"), Some("BTC".to_owned()));
        assert_eq!(base_currency_of_pair("BTC-USD"), Some("BTC".to_owned()));
        assert_eq!(base_currency_of_pair("BTC/USD"), Some("BTC".to_owned()));
        assert_eq!(base_currency_of_pair("AAPL"), None);
    }

    #[test]
    fn bar_phase_flags() {
        assert!(!BarPhase::Historical.is_realtime());
        assert!(BarPhase::Historical.is_confirmed());
        assert!(BarPhase::RealtimeTick { is_new: false }.is_realtime());
        assert!(!BarPhase::RealtimeTick { is_new: false }.is_confirmed());
        assert!(BarPhase::RealtimeClose { is_new: true }.is_confirmed());
    }
}
