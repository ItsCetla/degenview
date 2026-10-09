//! Minimal Pine runtime host: compiles a script and runs it over a series of bars, returning the
//! chart `VisualOutput` the renderer consumes. Input resolution and library imports are wired
//! incrementally; the session itself owns the per-bar state machine.
#![allow(dead_code)] // transient until the chart wires this

use crate::model::Kline;
use crate::pine::compiler::{self, NoLibraries};
use crate::pine::program::{BarEvent, BarPhase, SymbolInfo, VisualOutput};
use crate::pine::session::Session;
use crate::pine::state::AlertEvent;
use crate::pine::token::Limits;
use std::collections::HashMap;

/// Compiles `source` and runs it once over every bar (historical phase), returning the chart
/// output. A script with syntax/type errors yields an empty output plus the program's diagnostics.
pub fn run_script(source: &str, bars: &[Kline], symbol: &SymbolInfo) -> VisualOutput {
    let program = compiler::compile(source, Limits::default(), &NoLibraries);
    if !program.is_valid() || program.statements.is_empty() {
        return VisualOutput {
            overlay: false,
            bar_count: 0,
            ..Default::default()
        };
    }
    let mut session = Session::new(
        program,
        HashMap::new(),
        Limits::default(),
        None,
        symbol.clone(),
        None,
        None,
    );
    session.infer_mintick(bars);
    for (index, bar) in bars.iter().enumerate() {
        let is_last = index + 1 == bars.len();
        if session
            .execute(
                BarEvent {
                    candle: bar.clone(),
                    phase: BarPhase::Historical,
                },
                is_last,
                None,
            )
            .is_err()
        {
            break;
        }
    }
    session.output()
}

/// Compiles + runs `source` over the bars, replaying the last bar as a realtime close so Pine
/// `alert()`/`alertcondition()` calls fire, and returns the alerts that fired on that bar.
pub fn run_script_alerts(source: &str, bars: &[Kline], symbol: &SymbolInfo) -> Vec<AlertEvent> {
    let program = compiler::compile(source, Limits::default(), &NoLibraries);
    if !program.is_valid() || program.statements.is_empty() {
        return Vec::new();
    }
    let mut session = Session::new(
        program,
        HashMap::new(),
        Limits::default(),
        None,
        symbol.clone(),
        None,
        None,
    );
    session.infer_mintick(bars);
    for (index, bar) in bars.iter().enumerate() {
        let is_last = index + 1 == bars.len();
        let phase = if is_last {
            BarPhase::RealtimeClose { is_new: true }
        } else {
            BarPhase::Historical
        };
        if session
            .execute(
                BarEvent {
                    candle: bar.clone(),
                    phase,
                },
                is_last,
                None,
            )
            .is_err()
        {
            return Vec::new();
        }
    }
    session.emitted_alerts.clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bars() -> Vec<Kline> {
        (0..5)
            .map(|i| Kline {
                open_time: i as i64 * 3_600_000,
                close_time: (i as i64 + 1) * 3_600_000,
                open: 100.0 + i as f64,
                high: 105.0 + i as f64,
                low: 95.0 + i as f64,
                close: 102.0 + i as f64,
                volume: 1_000.0,
            })
            .collect()
    }

    #[test]
    fn a_plot_script_produces_output() {
        let source = "//@version=6\nindicator(\"T\")\nplot(close)";
        let output = run_script(source, &bars(), &SymbolInfo::default());
        assert!(output.bar_count >= 5);
        assert_eq!(output.plots.len(), 1);
    }

    #[test]
    fn an_invalid_script_produces_empty_output() {
        let source = "this is not pine";
        let output = run_script(source, &bars(), &SymbolInfo::default());
        assert!(output.plots.is_empty());
    }

    #[test]
    fn an_alert_script_fires_on_the_realtime_bar() {
        let source = "//@version=6\nindicator(\"T\")\nalert(\"hi\", alert.freq_all)";
        let alerts = run_script_alerts(source, &bars(), &SymbolInfo::default());
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].message, "hi");
        assert!(alerts[0].is_realtime);
    }

    #[test]
    fn mintick_is_inferred_from_low_priced_bars() {
        // Four-decimal prices infer a 0.0001 tick, not the 0.01 default.
        let low_bars: Vec<Kline> = (0..3)
            .map(|i| Kline {
                open_time: i as i64 * 3_600_000,
                close_time: (i as i64 + 1) * 3_600_000,
                open: 0.0001 + i as f64 * 0.0001,
                high: 0.0002 + i as f64 * 0.0001,
                low: 0.0001,
                close: 0.0001 + i as f64 * 0.0001,
                volume: 1_000.0,
            })
            .collect();
        let source = "//@version=6\nindicator(\"T\")\nplot(syminfo.mintick)";
        let output = run_script(source, &low_bars, &SymbolInfo::default());
        let last = output.plots[0].values.last().copied().flatten();
        assert_eq!(last, Some(0.0001));
    }
}
