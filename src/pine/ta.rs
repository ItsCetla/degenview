//! `ta.*` indicators over the per-call-site history the session keeps, ported from
//! `Builtins/PineTA.swift`. Pure: everything an indicator needs is in `State`.
//!
//! The session calls `evaluate` once per bar per call site. The state remembers every source value
//! (`inputs`, `na` included) and every result (`results`), so windows are read off the history and
//! the recursive averages (`ema`, `rma`) continue from the last result. Faithful quirks kept on
//! purpose: `ta.rsi` is the simple (not Wilder-smoothed) average of gains and losses, `ta.variance`
//! and `ta.stdev` are population statistics, and a window with any `na` yields `na`.

use super::runtime_value::RuntimeValue;
use crate::model::Kline;

/// What one call site remembers between bars.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct State {
    /// One entry per evaluation. Usually the indicator's value; for `ta.cross*` the
    /// `[source, second]` pair, for `ta.macd` the `[macd, signal, histogram]` tuple.
    pub results: Vec<RuntimeValue>,
    /// The source series, one entry per evaluation, `na` included.
    pub inputs: Vec<RuntimeValue>,
}

const MACD_FAST: usize = 12;
const MACD_SLOW: usize = 26;
const MACD_SIGNAL: usize = 9;

/// The `ta.*` names `evaluate` computes from one source series and a length. Others that go
/// through the generic call (`ta.sar`, `ta.supertrend`…) are not implemented.
pub const SUPPORTED: [&str; 30] = [
    "ta.sma",
    "ta.ema",
    "ta.rma",
    "ta.highest",
    "ta.lowest",
    "ta.change",
    "ta.wma",
    "ta.stdev",
    "ta.rising",
    "ta.falling",
    "ta.mom",
    "ta.roc",
    "ta.rsi",
    "ta.macd",
    "ta.cross",
    "ta.crossover",
    "ta.crossunder",
    "ta.median",
    "ta.range",
    "ta.variance",
    "ta.dev",
    "ta.swma",
    "ta.cmo",
    "ta.cci",
    "ta.hma",
    "ta.highestbars",
    "ta.lowestbars",
    "ta.percentrank",
    "ta.max",
    "ta.min",
];

pub fn is_supported(name: &str) -> bool {
    SUPPORTED.contains(&name) || matches!(name, "ta.linreg" | "math.sum")
}

fn float(value: f64) -> RuntimeValue {
    RuntimeValue::Float(value)
}

fn optional(value: Option<f64>) -> RuntimeValue {
    value.map_or(RuntimeValue::Na, float)
}

/// Evaluates `name` for the current bar and records the bar in `state`.
pub fn evaluate(
    name: &str,
    source: &RuntimeValue,
    second: &RuntimeValue,
    length: i64,
    state: &mut State,
) -> RuntimeValue {
    state.inputs.push(source.clone());
    // A length that is not a positive count reads as no window (most indicators then yield `na`).
    let count = usize::try_from(length).unwrap_or(0);
    let inputs = &state.inputs;
    let result = match name {
        "ta.sma" => sma(inputs, count),
        "ta.ema" | "ta.rma" => exponential(name, source, state, count),
        "ta.highest" => extremum(inputs, count, f64::max),
        "ta.lowest" => extremum(inputs, count, f64::min),
        "ta.change" => change(inputs, if count > 0 { count } else { 1 }),
        "ta.wma" => wma(inputs, count),
        "ta.stdev" => stdev(inputs, count),
        "ta.rising" => trend(inputs, count, true),
        "ta.falling" => trend(inputs, count, false),
        "ta.mom" | "ta.roc" => momentum(inputs, count, name == "ta.roc"),
        "ta.rsi" => rsi(inputs, count),
        "ta.macd" => macd(state),
        "ta.median" => statistic(inputs, count, median),
        "ta.range" => statistic(inputs, count, |values| {
            let high = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
            let low = values.iter().copied().fold(f64::INFINITY, f64::min);
            if values.is_empty() { 0.0 } else { high - low }
        }),
        "ta.variance" => statistic(inputs, count, variance),
        "ta.dev" => statistic(inputs, count, mean_deviation),
        "ta.swma" => statistic(inputs, 4, |v| (v[0] + 2.0 * v[1] + 2.0 * v[2] + v[3]) / 6.0),
        "ta.cmo" => cmo(inputs, count),
        "ta.cci" => cci(inputs, count),
        "ta.hma" => hma(inputs, count),
        "ta.highestbars" => extremum_bars(inputs, count, true),
        "ta.lowestbars" => extremum_bars(inputs, count, false),
        "ta.percentrank" => percent_rank(inputs, count),
        "ta.linreg" => linreg(inputs, count, second.int_value().unwrap_or(0)),
        "math.sum" => statistic(inputs, count, |values| values.iter().sum()),
        "ta.max" => running_extreme(source, state.results.last(), f64::max),
        "ta.min" => running_extreme(source, state.results.last(), f64::min),
        "ta.cross" | "ta.crossover" | "ta.crossunder" => {
            let crossed = cross(name, source, second, state.results.last());
            state
                .results
                .push(RuntimeValue::Tuple(vec![source.clone(), second.clone()]));
            return crossed;
        }
        _ => RuntimeValue::Na,
    };
    state.results.push(result.clone());
    result
}

/// True range: the widest of the bar's range and its gaps from the previous close.
pub fn true_range(bar: &Kline, previous_close: Option<f64>) -> f64 {
    (bar.high - bar.low)
        .max(previous_close.map_or(0.0, |close| (bar.high - close).abs()))
        .max(previous_close.map_or(0.0, |close| (bar.low - close).abs()))
}

// MARK: Windows

/// The last `count` non-`na` inputs, oldest first, or None when fewer exist.
fn last_valid(inputs: &[RuntimeValue], count: usize) -> Option<Vec<f64>> {
    if count == 0 || count > inputs.len() {
        return None;
    }
    let mut found: Vec<f64> = inputs
        .iter()
        .rev()
        .filter_map(RuntimeValue::number)
        .take(count)
        .collect();
    if found.len() != count {
        return None;
    }
    found.reverse();
    Some(found)
}

/// The last `count` inputs as numbers, or None when there are fewer or any is `na`.
fn last_window(inputs: &[RuntimeValue], count: usize) -> Option<Vec<f64>> {
    if count == 0 || count > inputs.len() {
        return None;
    }
    let numbers: Vec<f64> = inputs[inputs.len() - count..]
        .iter()
        .filter_map(RuntimeValue::number)
        .collect();
    (numbers.len() == count).then_some(numbers)
}

fn latest(inputs: &[RuntimeValue]) -> Option<f64> {
    inputs.last().and_then(RuntimeValue::number)
}

// MARK: Indicators

fn sma(inputs: &[RuntimeValue], length: usize) -> RuntimeValue {
    optional(last_valid(inputs, length).map(|values| values.iter().sum::<f64>() / length as f64))
}

/// EMA and RMA differ only in alpha. Both seed from the simple average of the first `length`
/// values.
fn exponential(name: &str, source: &RuntimeValue, state: &State, length: usize) -> RuntimeValue {
    if length == 0 {
        return RuntimeValue::Na;
    }
    let Some(prior) = state.results.last().and_then(RuntimeValue::number) else {
        return sma(&state.inputs, length);
    };
    let Some(x) = source.number() else {
        return RuntimeValue::Na;
    };
    let alpha = if name == "ta.ema" {
        2.0 / (length as f64 + 1.0)
    } else {
        1.0 / length as f64
    };
    float(alpha * x + (1.0 - alpha) * prior)
}

fn extremum(inputs: &[RuntimeValue], length: usize, pick: fn(f64, f64) -> f64) -> RuntimeValue {
    optional(last_valid(inputs, length).and_then(|values| values.into_iter().reduce(pick)))
}

fn change(inputs: &[RuntimeValue], offset: usize) -> RuntimeValue {
    if inputs.len() <= offset {
        return RuntimeValue::Na;
    }
    let (Some(current), Some(previous)) =
        (latest(inputs), inputs[inputs.len() - 1 - offset].number())
    else {
        return RuntimeValue::Na;
    };
    float(current - previous)
}

fn wma(inputs: &[RuntimeValue], length: usize) -> RuntimeValue {
    let Some(window) = last_window(inputs, length) else {
        return RuntimeValue::Na;
    };
    let weighted: f64 = window
        .iter()
        .enumerate()
        .map(|(index, value)| (index + 1) as f64 * value)
        .sum();
    float(weighted / (length * (length + 1) / 2) as f64)
}

fn stdev(inputs: &[RuntimeValue], length: usize) -> RuntimeValue {
    let Some(window) = last_window(inputs, length) else {
        return RuntimeValue::Na;
    };
    float(variance(&window).sqrt())
}

fn trend(inputs: &[RuntimeValue], length: usize, rising: bool) -> RuntimeValue {
    if length == 0 {
        return RuntimeValue::Bool(false);
    }
    let Some(window) = last_window(inputs, length + 1) else {
        return RuntimeValue::Bool(false);
    };
    RuntimeValue::Bool(window.windows(2).all(|pair| {
        if rising {
            pair[1] > pair[0]
        } else {
            pair[1] < pair[0]
        }
    }))
}

fn momentum(inputs: &[RuntimeValue], length: usize, is_rate: bool) -> RuntimeValue {
    if length == 0 || inputs.len() <= length {
        return RuntimeValue::Na;
    }
    let (Some(current), Some(previous)) =
        (latest(inputs), inputs[inputs.len() - 1 - length].number())
    else {
        return RuntimeValue::Na;
    };
    if !is_rate {
        return float(current - previous);
    }
    if previous == 0.0 {
        RuntimeValue::Na
    } else {
        float(100.0 * (current - previous) / previous)
    }
}

fn rsi(inputs: &[RuntimeValue], length: usize) -> RuntimeValue {
    if length == 0 {
        return RuntimeValue::Na;
    }
    let Some(values) = last_valid(inputs, length + 1) else {
        return RuntimeValue::Na;
    };
    let changes: Vec<f64> = values.windows(2).map(|pair| pair[1] - pair[0]).collect();
    let gain = changes.iter().map(|c| c.max(0.0)).sum::<f64>() / length as f64;
    let loss = changes.iter().map(|c| (-c).max(0.0)).sum::<f64>() / length as f64;
    float(if loss == 0.0 {
        100.0
    } else {
        100.0 - 100.0 / (1.0 + gain / loss)
    })
}

fn cross(
    name: &str,
    source: &RuntimeValue,
    second: &RuntimeValue,
    prior: Option<&RuntimeValue>,
) -> RuntimeValue {
    let no = RuntimeValue::Bool(false);
    let Some(RuntimeValue::Tuple(previous)) = prior else {
        return no;
    };
    let (Some(a), Some(b)) = (source.number(), second.number()) else {
        return no;
    };
    let [first, other] = previous.as_slice() else {
        return no;
    };
    let (Some(pa), Some(pb)) = (first.number(), other.number()) else {
        return no;
    };
    let crossed_over = a > b && pa <= pb;
    let crossed_under = a < b && pa >= pb;
    RuntimeValue::Bool(match name {
        "ta.crossover" => crossed_over,
        "ta.crossunder" => crossed_under,
        _ => crossed_over || crossed_under,
    })
}

// MARK: MACD

/// An exponential average seeded from the first `length` values.
fn exponential_average(values: &[f64], length: usize) -> Option<f64> {
    if length == 0 || values.len() < length {
        return None;
    }
    let mut average = values[..length].iter().sum::<f64>() / length as f64;
    for x in &values[length..] {
        average = (2.0 * x + (length as f64 - 1.0) * average) / (length as f64 + 1.0);
    }
    Some(average)
}

fn macd(state: &State) -> RuntimeValue {
    let values: Vec<f64> = state
        .inputs
        .iter()
        .filter_map(RuntimeValue::number)
        .collect();
    let fast = exponential_average(&values, MACD_FAST);
    let slow = exponential_average(&values, MACD_SLOW);
    let line = fast.zip(slow).map(|(fast, slow)| fast - slow);
    let mut line_history: Vec<f64> = state
        .results
        .iter()
        .filter_map(|result| match result {
            RuntimeValue::Tuple(parts) => parts.first().and_then(RuntimeValue::number),
            _ => None,
        })
        .collect();
    line_history.extend(line);
    let signal = exponential_average(&line_history, MACD_SIGNAL);
    RuntimeValue::Tuple(vec![
        optional(line),
        optional(signal),
        optional(line.zip(signal).map(|(line, signal)| line - signal)),
    ])
}

/// `ta.linreg(source, length, offset)`: the least-squares line through the last `length` values,
/// read `offset` bars before the newest (0 is the current bar, a negative offset projects forward).
fn linreg(inputs: &[RuntimeValue], length: usize, offset: i64) -> RuntimeValue {
    let Some(window) = last_window(inputs, length) else {
        return RuntimeValue::Na;
    };
    let count = length as f64;
    let mean_x = (count - 1.0) / 2.0;
    let mean_y = window.iter().sum::<f64>() / count;
    let (mut covariance, mut spread) = (0.0, 0.0);
    for (index, value) in window.iter().enumerate() {
        let dx = index as f64 - mean_x;
        covariance += dx * (value - mean_y);
        spread += dx * dx;
    }
    let slope = if spread == 0.0 {
        0.0
    } else {
        covariance / spread
    };
    let intercept = mean_y - slope * mean_x;
    float(intercept + slope * (count - 1.0 - offset as f64))
}

/// `ta.max` / `ta.min`: the highest or lowest value the series has had so far; `na` does not
/// change it.
fn running_extreme(
    source: &RuntimeValue,
    prior: Option<&RuntimeValue>,
    pick: fn(f64, f64) -> f64,
) -> RuntimeValue {
    let previous = prior.and_then(RuntimeValue::number);
    let Some(value) = source.number() else {
        return optional(previous);
    };
    float(previous.map_or(value, |previous| pick(previous, value)))
}

// MARK: Window statistics

/// `compute` over the last `length` inputs, all of which must be numbers.
fn statistic(
    inputs: &[RuntimeValue],
    length: usize,
    compute: impl FnOnce(&[f64]) -> f64,
) -> RuntimeValue {
    optional(last_window(inputs, length).map(|window| compute(&window)))
}

fn median(values: &[f64]) -> f64 {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let middle = sorted.len() / 2;
    if sorted.len() % 2 == 1 {
        sorted[middle]
    } else {
        (sorted[middle - 1] + sorted[middle]) / 2.0
    }
}

/// The population variance, as `ta.variance` computes by default.
fn variance(values: &[f64]) -> f64 {
    let mean = values.iter().sum::<f64>() / values.len() as f64;
    values.iter().map(|v| (v - mean) * (v - mean)).sum::<f64>() / values.len() as f64
}

fn mean_deviation(values: &[f64]) -> f64 {
    let mean = values.iter().sum::<f64>() / values.len() as f64;
    values.iter().map(|v| (v - mean).abs()).sum::<f64>() / values.len() as f64
}

/// Chande momentum oscillator: the share of the net movement in all movement over `length`
/// changes.
fn cmo(inputs: &[RuntimeValue], length: usize) -> RuntimeValue {
    if length == 0 {
        return RuntimeValue::Na;
    }
    let Some(window) = last_window(inputs, length + 1) else {
        return RuntimeValue::Na;
    };
    let changes: Vec<f64> = window.windows(2).map(|pair| pair[1] - pair[0]).collect();
    let up: f64 = changes.iter().filter(|c| **c > 0.0).sum();
    let down: f64 = -changes.iter().filter(|c| **c < 0.0).sum::<f64>();
    float(if up + down == 0.0 {
        0.0
    } else {
        100.0 * (up - down) / (up + down)
    })
}

/// Commodity channel index: the distance from the mean in units of 0.015 mean deviations.
fn cci(inputs: &[RuntimeValue], length: usize) -> RuntimeValue {
    let Some(window) = last_window(inputs, length) else {
        return RuntimeValue::Na;
    };
    let current = window[window.len() - 1];
    let mean = window.iter().sum::<f64>() / length as f64;
    let deviation = mean_deviation(&window);
    float(if deviation == 0.0 {
        0.0
    } else {
        (current - mean) / (0.015 * deviation)
    })
}

/// The weighted average of the `length` values ending at `end` (inclusive), or None.
fn weighted_average(values: &[Option<f64>], end: usize, length: usize) -> Option<f64> {
    if length == 0 || end + 1 < length || end >= values.len() {
        return None;
    }
    let mut weighted = 0.0;
    for (offset, index) in (end + 1 - length..=end).enumerate() {
        weighted += (offset + 1) as f64 * values[index]?;
    }
    Some(weighted / (length * (length + 1) / 2) as f64)
}

/// Hull moving average: a weighted average (length √n) of 2 · wma(n/2) − wma(n).
fn hma(inputs: &[RuntimeValue], length: usize) -> RuntimeValue {
    if length <= 1 {
        return RuntimeValue::Na;
    }
    let values: Vec<Option<f64>> = inputs.iter().map(RuntimeValue::number).collect();
    let half = (length / 2).max(1);
    let smoothing = ((length as f64).sqrt() as usize).max(1);
    let last = values.len() - 1;
    if last + 1 < smoothing {
        return RuntimeValue::Na;
    }
    let mut difference: Vec<Option<f64>> = vec![None; values.len()];
    for (index, slot) in difference.iter_mut().enumerate().skip(last + 1 - smoothing) {
        if let (Some(fast), Some(slow)) = (
            weighted_average(&values, index, half),
            weighted_average(&values, index, length),
        ) {
            *slot = Some(2.0 * fast - slow);
        }
    }
    optional(weighted_average(&difference, last, smoothing))
}

/// Bars back to the highest (or lowest) of the last `length` inputs: 0 for the current bar,
/// negative further back, the most recent one on a tie.
fn extremum_bars(inputs: &[RuntimeValue], length: usize, highest: bool) -> RuntimeValue {
    let Some(window) = last_window(inputs, length) else {
        return RuntimeValue::Na;
    };
    let mut best = 0;
    for back in 0..length {
        let value = window[length - 1 - back];
        let current = window[length - 1 - best];
        if if highest {
            value > current
        } else {
            value < current
        } {
            best = back;
        }
    }
    RuntimeValue::Int(-(best as i64))
}

/// The share, in percent, of the previous `length` values that are at or below the current one.
fn percent_rank(inputs: &[RuntimeValue], length: usize) -> RuntimeValue {
    if length == 0 || inputs.len() <= length {
        return RuntimeValue::Na;
    }
    let Some(current) = latest(inputs) else {
        return RuntimeValue::Na;
    };
    let previous: Vec<f64> = inputs[inputs.len() - 1 - length..inputs.len() - 1]
        .iter()
        .filter_map(RuntimeValue::number)
        .collect();
    if previous.len() != length {
        return RuntimeValue::Na;
    }
    float(previous.iter().filter(|value| **value <= current).count() as f64 / length as f64 * 100.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs `name` over `series` one bar at a time, as the session does.
    fn run(name: &str, series: &[f64], length: i64) -> Vec<RuntimeValue> {
        let mut state = State::default();
        series
            .iter()
            .map(|value| evaluate(name, &float(*value), &RuntimeValue::Na, length, &mut state))
            .collect()
    }

    fn numbers(values: &[RuntimeValue]) -> Vec<Option<f64>> {
        values.iter().map(RuntimeValue::number).collect()
    }

    fn close(a: Option<f64>, b: f64) -> bool {
        a.is_some_and(|a| (a - b).abs() < 1e-9)
    }

    #[test]
    fn sma_needs_a_full_window_and_slides() {
        let result = numbers(&run("ta.sma", &[1.0, 2.0, 3.0, 4.0, 5.0], 3));
        assert_eq!(result, [None, None, Some(2.0), Some(3.0), Some(4.0)]);
        assert_eq!(
            numbers(&run("ta.sma", &[1.0, 2.0], 0)),
            [None, None],
            "no window: na"
        );
        assert_eq!(numbers(&run("ta.sma", &[1.0, 2.0], -3)), [None, None]);
    }

    #[test]
    fn a_gap_in_the_source_delays_a_window_that_skips_na() {
        let mut state = State::default();
        let mut feed =
            |value: RuntimeValue| evaluate("ta.sma", &value, &RuntimeValue::Na, 2, &mut state);
        assert_eq!(feed(float(2.0)), RuntimeValue::Na);
        assert_eq!(feed(RuntimeValue::Na), RuntimeValue::Na);
        // sma looks at the last two non-na values (2 and 4), skipping the gap.
        assert_eq!(feed(float(4.0)), float(3.0));
        // wma needs the last two inputs themselves to be numbers.
        let mut wma = State::default();
        for value in [float(1.0), RuntimeValue::Na, float(3.0)] {
            evaluate("ta.wma", &value, &RuntimeValue::Na, 2, &mut wma);
        }
        assert_eq!(wma.results.last(), Some(&RuntimeValue::Na));
    }

    #[test]
    fn ema_seeds_from_the_sma_then_recurses() {
        let result = numbers(&run("ta.ema", &[2.0, 4.0, 6.0, 8.0], 2));
        assert_eq!(result[0], None);
        assert_eq!(result[1], Some(3.0), "the simple average seeds it");
        let alpha = 2.0 / 3.0;
        assert!(close(result[2], alpha * 6.0 + (1.0 - alpha) * 3.0));
        let rma = numbers(&run("ta.rma", &[2.0, 4.0, 6.0], 2));
        assert!(
            close(rma[2], 0.5 * 6.0 + 0.5 * 3.0),
            "rma uses alpha = 1/length"
        );
        assert_eq!(numbers(&run("ta.ema", &[1.0], 0)), [None]);
    }

    #[test]
    fn extremes_over_a_window_and_running_extremes() {
        assert_eq!(
            numbers(&run("ta.highest", &[1.0, 5.0, 3.0, 2.0], 2)),
            [None, Some(5.0), Some(5.0), Some(3.0)]
        );
        assert_eq!(
            numbers(&run("ta.lowest", &[4.0, 2.0, 3.0, 5.0], 2)),
            [None, Some(2.0), Some(2.0), Some(3.0)]
        );
        assert_eq!(
            numbers(&run("ta.max", &[1.0, 3.0, 2.0], 0)),
            [Some(1.0), Some(3.0), Some(3.0)]
        );
        assert_eq!(
            numbers(&run("ta.min", &[3.0, 1.0, 2.0], 0)),
            [Some(3.0), Some(1.0), Some(1.0)]
        );
        let mut state = State::default();
        evaluate("ta.max", &float(5.0), &RuntimeValue::Na, 0, &mut state);
        let kept = evaluate(
            "ta.max",
            &RuntimeValue::Na,
            &RuntimeValue::Na,
            0,
            &mut state,
        );
        assert_eq!(kept, float(5.0), "na does not change a running extreme");
    }

    #[test]
    fn change_momentum_and_rate_of_change() {
        assert_eq!(
            numbers(&run("ta.change", &[1.0, 4.0, 9.0], 1)),
            [None, Some(3.0), Some(5.0)]
        );
        assert_eq!(
            numbers(&run("ta.change", &[1.0, 4.0, 9.0], 0)),
            [None, Some(3.0), Some(5.0)],
            "0 reads as 1"
        );
        assert_eq!(
            numbers(&run("ta.change", &[1.0, 4.0, 9.0], 2)),
            [None, None, Some(8.0)]
        );
        assert_eq!(
            numbers(&run("ta.mom", &[10.0, 12.0, 15.0], 2)),
            [None, None, Some(5.0)]
        );
        let roc = numbers(&run("ta.roc", &[10.0, 12.0, 15.0], 2));
        assert!(close(roc[2], 50.0));
        assert_eq!(
            numbers(&run("ta.roc", &[0.0, 1.0], 1)),
            [None, None],
            "division by zero is na"
        );
    }

    #[test]
    fn rising_and_falling_need_every_step_to_agree() {
        let rising = run("ta.rising", &[1.0, 2.0, 3.0, 3.0, 4.0], 2);
        assert_eq!(
            rising,
            [false, false, true, false, false]
                .map(RuntimeValue::Bool)
                .to_vec()
        );
        let falling = run("ta.falling", &[5.0, 4.0, 3.0, 4.0], 2);
        assert_eq!(falling[2], RuntimeValue::Bool(true));
        assert_eq!(falling[3], RuntimeValue::Bool(false));
        assert_eq!(
            run("ta.rising", &[1.0, 2.0], 0)[1],
            RuntimeValue::Bool(false)
        );
    }

    #[test]
    fn weighted_and_dispersion_statistics_over_a_window() {
        let wma = numbers(&run("ta.wma", &[1.0, 2.0, 3.0], 3));
        assert!(close(wma[2], (1.0 + 4.0 + 9.0) / 6.0));
        let stdev = numbers(&run(
            "ta.stdev",
            &[2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0],
            8,
        ));
        assert!(close(stdev[7], 2.0), "population standard deviation");
        let variance = numbers(&run(
            "ta.variance",
            &[2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0],
            8,
        ));
        assert!(close(variance[7], 4.0));
        assert!(close(
            numbers(&run("ta.dev", &[1.0, 2.0, 3.0, 4.0], 4))[3],
            1.0
        ));
        assert!(close(
            numbers(&run("ta.median", &[5.0, 1.0, 3.0], 3))[2],
            3.0
        ));
        assert!(close(
            numbers(&run("ta.median", &[4.0, 1.0, 3.0, 2.0], 4))[3],
            2.5
        ));
        assert!(close(
            numbers(&run("ta.range", &[2.0, 9.0, 5.0], 3))[2],
            7.0
        ));
        assert!(close(
            numbers(&run("math.sum", &[1.0, 2.0, 3.0, 4.0], 3))[3],
            9.0
        ));
        assert!(close(
            numbers(&run("ta.swma", &[1.0, 2.0, 3.0, 4.0], 0))[3],
            (1.0 + 4.0 + 6.0 + 4.0) / 6.0
        ));
    }

    #[test]
    fn rsi_is_the_simple_average_variant_and_saturates() {
        let up = numbers(&run("ta.rsi", &[1.0, 2.0, 3.0, 4.0], 3));
        assert_eq!(up[3], Some(100.0), "no losses: 100");
        assert_eq!(up[2], None);
        let mixed = numbers(&run("ta.rsi", &[10.0, 11.0, 10.5, 11.5], 3));
        // gains: 1 + 1 = 2, losses: 0.5 → gain/loss = 4 → 100 − 100/5 = 80.
        assert!(close(mixed[3], 80.0));
        assert_eq!(numbers(&run("ta.rsi", &[1.0, 2.0], 0)), [None, None]);
    }

    #[test]
    fn cross_compares_this_bar_to_the_last() {
        let mut state = State::default();
        let mut feed =
            |name: &str, a: f64, b: f64| evaluate(name, &float(a), &float(b), 0, &mut state);
        assert_eq!(
            feed("ta.crossover", 1.0, 2.0),
            RuntimeValue::Bool(false),
            "no previous bar"
        );
        assert_eq!(feed("ta.crossover", 3.0, 2.0), RuntimeValue::Bool(true));
        assert_eq!(
            feed("ta.crossover", 4.0, 2.0),
            RuntimeValue::Bool(false),
            "already above"
        );
        assert_eq!(feed("ta.crossunder", 1.0, 2.0), RuntimeValue::Bool(true));
        assert_eq!(
            feed("ta.cross", 3.0, 2.0),
            RuntimeValue::Bool(true),
            "either way"
        );
        assert_eq!(feed("ta.cross", 3.5, 2.0), RuntimeValue::Bool(false));
        // Touching counts as the earlier side: 2 <= 2 then 3 > 2 crosses over.
        assert_eq!(feed("ta.crossunder", 2.0, 2.0), RuntimeValue::Bool(false));
        assert_eq!(feed("ta.crossover", 3.0, 2.0), RuntimeValue::Bool(true));
        // A pair with an `na` never crosses.
        let mut state = State::default();
        evaluate("ta.cross", &float(1.0), &float(2.0), 0, &mut state);
        let na = evaluate("ta.cross", &RuntimeValue::Na, &float(2.0), 0, &mut state);
        assert_eq!(na, RuntimeValue::Bool(false));
    }

    #[test]
    fn macd_needs_slow_history_then_a_signal() {
        let series: Vec<f64> = (1..=40).map(f64::from).collect();
        let mut state = State::default();
        let mut results = Vec::new();
        for value in &series {
            results.push(evaluate(
                "ta.macd",
                &float(*value),
                &RuntimeValue::Na,
                0,
                &mut state,
            ));
        }
        let parts = |result: &RuntimeValue| match result {
            RuntimeValue::Tuple(parts) => numbers(parts),
            other => panic!("expected a tuple, got {other:?}"),
        };
        assert_eq!(
            parts(&results[24]),
            [None, None, None],
            "slow EMA not ready"
        );
        let first = parts(&results[25]);
        assert!(
            first[0].is_some() && first[1].is_none(),
            "line but no signal yet"
        );
        let later = parts(&results[39]);
        assert!(later.iter().all(Option::is_some), "{later:?}");
        assert!(
            close(later[2], later[0].unwrap() - later[1].unwrap()),
            "histogram = line − signal"
        );
        assert!(
            later[0].unwrap() > 0.0,
            "a rising series has a positive MACD line"
        );
    }

    #[test]
    fn oscillators_cmo_and_cci() {
        assert_eq!(numbers(&run("ta.cmo", &[1.0, 2.0, 3.0], 2))[2], Some(100.0));
        assert_eq!(
            numbers(&run("ta.cmo", &[1.0, 1.0, 1.0], 2))[2],
            Some(0.0),
            "no movement"
        );
        assert!(close(
            numbers(&run("ta.cmo", &[1.0, 3.0, 2.0], 2))[2],
            100.0 * (2.0 - 1.0) / 3.0
        ));
        let cci = numbers(&run("ta.cci", &[1.0, 2.0, 3.0], 3));
        assert!(close(cci[2], (3.0 - 2.0) / (0.015 * (2.0 / 3.0))));
        assert_eq!(
            numbers(&run("ta.cci", &[5.0, 5.0, 5.0], 3))[2],
            Some(0.0),
            "flat: no deviation"
        );
    }

    #[test]
    fn extreme_bars_count_back_and_prefer_the_newest_on_a_tie() {
        let highest = run("ta.highestbars", &[1.0, 5.0, 3.0, 2.0], 3);
        assert_eq!(highest[3], RuntimeValue::Int(-2), "5 is two bars back");
        let tie = run("ta.highestbars", &[5.0, 1.0, 5.0], 3);
        assert_eq!(tie[2], RuntimeValue::Int(0), "the newest of equal highs");
        let lowest = run("ta.lowestbars", &[4.0, 1.0, 2.0], 3);
        assert_eq!(lowest[2], RuntimeValue::Int(-1));
        assert_eq!(highest[1], RuntimeValue::Na);
    }

    #[test]
    fn percent_rank_counts_prior_values_at_or_below() {
        let result = numbers(&run("ta.percentrank", &[1.0, 2.0, 3.0, 2.5], 3));
        assert_eq!(result[2], None, "needs length prior values");
        // Prior three: 1, 2, 3 vs current 2.5 → two are ≤ 2.5.
        assert!(close(result[3], 2.0 / 3.0 * 100.0));
        assert_eq!(numbers(&run("ta.percentrank", &[1.0], 0)), [None]);
    }

    #[test]
    fn linreg_fits_a_line_and_projects_with_the_offset() {
        let line = [2.0, 4.0, 6.0, 8.0];
        let mut state = State::default();
        let mut out = Vec::new();
        for value in line {
            out.push(evaluate(
                "ta.linreg",
                &float(value),
                &RuntimeValue::Int(0),
                4,
                &mut state,
            ));
        }
        assert!(
            close(out[3].number(), 8.0),
            "a perfect line reads back its last value"
        );
        let mut state = State::default();
        let mut back = RuntimeValue::Na;
        for value in line {
            back = evaluate(
                "ta.linreg",
                &float(value),
                &RuntimeValue::Int(1),
                4,
                &mut state,
            );
        }
        assert!(close(back.number(), 6.0), "offset 1 reads one bar back");
        let mut state = State::default();
        let mut ahead = RuntimeValue::Na;
        for value in line {
            ahead = evaluate(
                "ta.linreg",
                &float(value),
                &RuntimeValue::Int(-1),
                4,
                &mut state,
            );
        }
        assert!(
            close(ahead.number(), 10.0),
            "a negative offset projects forward"
        );
    }

    #[test]
    fn hma_is_a_weighted_average_of_the_difference_of_two_wmas() {
        let series: Vec<f64> = (1..=20).map(f64::from).collect();
        let result = numbers(&run("ta.hma", &series, 9));
        assert!(
            result[..8].iter().all(Option::is_none),
            "needs length + sqrt(length) − 1 bars"
        );
        // On a straight line the Hull average has no lag at all: it reads the line itself.
        assert!(close(result[19], 20.0), "{:?}", result[19]);
        assert_eq!(numbers(&run("ta.hma", &series, 1)), vec![None; 20]);
    }

    #[test]
    fn unknown_names_are_na_and_the_supported_list_is_honest() {
        let mut state = State::default();
        assert_eq!(
            evaluate("ta.nope", &float(1.0), &RuntimeValue::Na, 3, &mut state),
            RuntimeValue::Na
        );
        assert_eq!(state.results.len(), 1, "every evaluation is recorded");
        assert_eq!(state.inputs.len(), 1);
        assert!(is_supported("ta.sma") && is_supported("ta.linreg") && is_supported("math.sum"));
        assert!(!is_supported("ta.sar"));
        // Every advertised name evaluates to something other than the unknown-name fallback path
        // for a long enough series.
        let series: Vec<f64> = (1..=60)
            .map(|n| f64::from(n % 7) + f64::from(n) * 0.1)
            .collect();
        for name in SUPPORTED {
            let mut state = State::default();
            let mut last = RuntimeValue::Na;
            for value in &series {
                last = evaluate(name, &float(*value), &float(2.0), 5, &mut state);
            }
            assert!(
                last != RuntimeValue::Na
                    || matches!(name, "ta.cross" | "ta.crossover" | "ta.crossunder"),
                "{name} never produced a value"
            );
        }
    }

    #[test]
    fn true_range_is_the_widest_of_the_range_and_the_gaps() {
        let bar = |high: f64, low: f64| Kline {
            open_time: 0,
            close_time: 0,
            open: low,
            high,
            low,
            close: high,
            volume: 0.0,
        };
        assert_eq!(
            true_range(&bar(10.0, 8.0), None),
            2.0,
            "the first bar has only its range"
        );
        assert_eq!(
            true_range(&bar(10.0, 8.0), Some(7.0)),
            3.0,
            "a gap up from the last close"
        );
        assert_eq!(true_range(&bar(10.0, 8.0), Some(12.0)), 4.0, "a gap down");
        assert_eq!(
            true_range(&bar(10.0, 8.0), Some(9.0)),
            2.0,
            "inside the range"
        );
    }
}
