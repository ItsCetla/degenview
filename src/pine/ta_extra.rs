//! The rest of the `ta.*` namespace: windowed statistics that need no bar data (`alma`, `cog`,
//! `mode`, `percentile_*`, `valuewhen`), the ones that read the bar's high/low/close/volume
//! (`mfi`, `vwma`, `stoch`, `wpr`, `kc`, `kcw`, `bbw`, `tsi`) and the cumulative volume variables
//! (`ta.obv`, `ta.accdist`, …). Formulas follow the Pine v6 reference.
//!
//! Every function is called once per bar per call site and remembers what it needs in the call
//! site's [`State`], so a realtime tick that is rolled back rolls these back with it.

use super::runtime_value::RuntimeValue;
use super::state::RuntimeState;
use super::ta::{self, State};
use crate::model::Kline;

/// The `ta.*` functions this module computes (parameters in `ta_call::parameters`).
pub const FUNCTIONS: [&str; 14] = [
    "ta.alma",
    "ta.cog",
    "ta.mode",
    "ta.percentile_nearest_rank",
    "ta.percentile_linear_interpolation",
    "ta.valuewhen",
    "ta.mfi",
    "ta.vwma",
    "ta.stoch",
    "ta.wpr",
    "ta.kc",
    "ta.kcw",
    "ta.bbw",
    "ta.tsi",
];

/// The `ta.*` variables [`cumulative`] computes. A variable's position is where its state lives.
pub const VARIABLES: [&str; 8] = [
    "ta.obv",
    "ta.accdist",
    "ta.iii",
    "ta.nvi",
    "ta.pvi",
    "ta.pvt",
    "ta.wad",
    "ta.wvad",
];

/// A series a function derives from its inputs (`ta.kc`'s two averages, `ta.tsi`'s four) with the
/// last result of its own moving average.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Derived {
    values: Vec<Option<f64>>,
    last: Option<f64>,
}

impl Derived {
    /// The EMA of everything pushed so far, seeded with the simple average of the first `length`
    /// values like `ta.ema`.
    fn ema(&mut self, value: Option<f64>, length: usize) -> Option<f64> {
        self.values.push(value);
        let result = match (self.last, value) {
            _ if length == 0 => None,
            (Some(prior), Some(x)) => {
                let alpha = 2.0 / (length as f64 + 1.0);
                Some(alpha * x + (1.0 - alpha) * prior)
            }
            (Some(_), None) => None,
            (None, _) => self
                .window(length)
                .map(|w| w.iter().sum::<f64>() / length as f64),
        };
        if result.is_some() {
            self.last = result;
        } else if value.is_none() {
            // A gap breaks the average; it reseeds once `length` values are present again.
            self.last = None;
        }
        result
    }

    fn window(&self, length: usize) -> Option<Vec<f64>> {
        let start = self.values.len().checked_sub(length)?;
        self.values[start..].iter().copied().collect()
    }
}

fn derived(state: &mut State, slot: usize) -> &mut Derived {
    if state.derived.len() <= slot {
        state.derived.resize_with(slot + 1, Derived::default);
    }
    &mut state.derived[slot]
}

fn count(length: i64) -> usize {
    usize::try_from(length).unwrap_or(0)
}

/// `na` for a missing or non-finite result (0/0 in a ratio), the value otherwise.
fn out(value: Option<f64>) -> RuntimeValue {
    value
        .filter(|value| value.is_finite())
        .map_or(RuntimeValue::Na, RuntimeValue::Float)
}

/// The last `length` values of the source, or None while the window is short or has an `na`.
fn window(inputs: &[RuntimeValue], length: usize) -> Option<Vec<f64>> {
    if length == 0 || length > inputs.len() {
        return None;
    }
    inputs[inputs.len() - length..]
        .iter()
        .map(RuntimeValue::number)
        .collect()
}

fn last_n(values: &[f64], length: usize) -> Option<&[f64]> {
    (length > 0 && length <= values.len()).then(|| &values[values.len() - length..])
}

/// `ta.alma(series, length, offset, sigma, floor)`: the Arnaud Legoux moving average.
pub fn alma(
    source: &RuntimeValue,
    length: i64,
    offset: f64,
    sigma: f64,
    floor: bool,
    state: &mut State,
) -> RuntimeValue {
    state.inputs.push(source.clone());
    let length = count(length);
    let Some(values) = window(&state.inputs, length) else {
        return RuntimeValue::Na;
    };
    let mut m = offset * (length as f64 - 1.0);
    if floor {
        m = m.floor();
    }
    let s = length as f64 / sigma;
    let (mut norm, mut sum) = (0.0, 0.0);
    for i in 0..length {
        let weight = (-((i as f64 - m).powi(2)) / (2.0 * s * s)).exp();
        norm += weight;
        sum += values[length - 1 - i] * weight;
    }
    out((norm != 0.0).then(|| sum / norm))
}

/// `ta.cog(source, length)`: the center of gravity oscillator.
pub fn cog(source: &RuntimeValue, length: i64, state: &mut State) -> RuntimeValue {
    state.inputs.push(source.clone());
    let length = count(length);
    let Some(values) = window(&state.inputs, length) else {
        return RuntimeValue::Na;
    };
    let (mut weighted, mut total) = (0.0, 0.0);
    for i in 0..length {
        let value = values[length - 1 - i];
        weighted += value * (i as f64 + 1.0);
        total += value;
    }
    out((total != 0.0).then(|| -weighted / total))
}

/// `ta.mode(source, length)`: the most frequent value of the window, the smallest on a tie.
pub fn mode(source: &RuntimeValue, length: i64, state: &mut State) -> RuntimeValue {
    state.inputs.push(source.clone());
    let Some(mut values) = window(&state.inputs, count(length)) else {
        return RuntimeValue::Na;
    };
    values.sort_by(f64::total_cmp);
    let (mut best, mut best_run) = (values[0], 0);
    let mut start = 0;
    while start < values.len() {
        let mut end = start;
        while end < values.len() && values[end] == values[start] {
            end += 1;
        }
        if end - start > best_run {
            best_run = end - start;
            best = values[start];
        }
        start = end;
    }
    out(Some(best))
}

/// `ta.percentile_nearest_rank` / `ta.percentile_linear_interpolation` over the last `length` values.
pub fn percentile(
    source: &RuntimeValue,
    length: i64,
    percentage: f64,
    interpolate: bool,
    state: &mut State,
) -> RuntimeValue {
    state.inputs.push(source.clone());
    let Some(mut values) = window(&state.inputs, count(length)) else {
        return RuntimeValue::Na;
    };
    if !(0.0..=100.0).contains(&percentage) {
        return RuntimeValue::Na;
    }
    values.sort_by(f64::total_cmp);
    let n = values.len();
    if interpolate {
        let position = percentage / 100.0 * (n as f64 - 1.0);
        let below = position.floor() as usize;
        let above = position.ceil() as usize;
        out(Some(
            values[below] + (values[above] - values[below]) * (position - below as f64),
        ))
    } else {
        let rank = ((percentage / 100.0 * n as f64).ceil() as usize).clamp(1, n);
        out(Some(values[rank - 1]))
    }
}

/// `ta.valuewhen(condition, source, occurrence)`: the source on the bar the condition was last
/// true, `occurrence` true bars back (0 is the most recent).
pub fn valuewhen(
    condition: &RuntimeValue,
    source: &RuntimeValue,
    occurrence: i64,
    state: &mut State,
) -> RuntimeValue {
    if condition.bool() == Some(true) {
        state.seconds.push(source.clone());
    }
    let Ok(back) = usize::try_from(occurrence) else {
        return RuntimeValue::Na;
    };
    state
        .seconds
        .len()
        .checked_sub(back + 1)
        .map_or(RuntimeValue::Na, |index| state.seconds[index].clone())
}

/// `ta.mfi(series, length)`: the money flow index.
pub fn mfi(source: &RuntimeValue, length: i64, bar: &Kline, state: &mut State) -> RuntimeValue {
    let previous = state.inputs.last().and_then(RuntimeValue::number);
    state.inputs.push(source.clone());
    let flow = match (source.number(), previous) {
        (Some(now), Some(before)) => {
            let up = if now - before <= 0.0 { 0.0 } else { now };
            let down = if now - before >= 0.0 { 0.0 } else { now };
            Some((bar.volume * up, bar.volume * down))
        }
        _ => None,
    };
    state
        .results
        .push(flow.map_or(RuntimeValue::Na, |(up, down)| {
            RuntimeValue::Tuple(vec![RuntimeValue::Float(up), RuntimeValue::Float(down)])
        }));
    let length = count(length);
    let Some(start) = state
        .results
        .len()
        .checked_sub(length)
        .filter(|_| length > 0)
    else {
        return RuntimeValue::Na;
    };
    let (mut upper, mut lower) = (0.0, 0.0);
    for entry in &state.results[start..] {
        let RuntimeValue::Tuple(pair) = entry else {
            return RuntimeValue::Na;
        };
        upper += pair[0].number().unwrap_or(0.0);
        lower += pair[1].number().unwrap_or(0.0);
    }
    out(Some(100.0 - 100.0 / (1.0 + upper / lower)))
}

/// `ta.vwma(source, length)`: the volume-weighted moving average.
pub fn vwma(source: &RuntimeValue, length: i64, bar: &Kline, state: &mut State) -> RuntimeValue {
    state.inputs.push(source.clone());
    state.seconds.push(RuntimeValue::Float(bar.volume));
    let length = count(length);
    let (Some(prices), Some(volumes)) = (
        window(&state.inputs, length),
        window(&state.seconds, length),
    ) else {
        return RuntimeValue::Na;
    };
    let weighted: f64 = prices.iter().zip(&volumes).map(|(p, v)| p * v).sum();
    let volume: f64 = volumes.iter().sum();
    out((volume != 0.0).then(|| weighted / volume))
}

/// The highest high and lowest low of the last `length` bars, from the bars this call site saw.
fn range(state: &State, length: usize) -> Option<(f64, f64)> {
    let highs = last_n(&state.highs, length)?;
    let lows = last_n(&state.lows, length)?;
    // A gap (`na`) anywhere in the window makes the range unknown, as in `ta.highest`.
    if highs.iter().chain(lows).any(|value| value.is_nan()) {
        return None;
    }
    Some((
        highs.iter().copied().fold(f64::NEG_INFINITY, f64::max),
        lows.iter().copied().fold(f64::INFINITY, f64::min),
    ))
}

/// `ta.stoch(source, high, low, length)`: where the source sits in the high-low range, 0 to 100.
pub fn stoch(
    source: &RuntimeValue,
    high: &RuntimeValue,
    low: &RuntimeValue,
    length: i64,
    state: &mut State,
) -> RuntimeValue {
    let (Some(source), Some(high), Some(low)) = (source.number(), high.number(), low.number())
    else {
        // A gap in the inputs poisons the window: keep the history aligned with `na` bars.
        state.highs.push(f64::NAN);
        state.lows.push(f64::NAN);
        return RuntimeValue::Na;
    };
    state.highs.push(high);
    state.lows.push(low);
    let Some((highest, lowest)) = range(state, count(length)) else {
        return RuntimeValue::Na;
    };
    out(
        (highest.is_finite() && lowest.is_finite() && highest != lowest)
            .then(|| 100.0 * (source - lowest) / (highest - lowest)),
    )
}

/// `ta.wpr(length)`: Williams %R, from the bar's own high, low and close.
pub fn wpr(length: i64, bar: &Kline, state: &mut State) -> RuntimeValue {
    state.highs.push(bar.high);
    state.lows.push(bar.low);
    let Some((highest, lowest)) = range(state, count(length)) else {
        return RuntimeValue::Na;
    };
    out((highest != lowest).then(|| 100.0 * (bar.close - highest) / (highest - lowest)))
}

/// `ta.kc(series, length, mult, useTrueRange)` as `[basis, upper, lower]`.
pub fn keltner(
    source: &RuntimeValue,
    length: i64,
    multiplier: f64,
    use_true_range: bool,
    bar: &Kline,
    state: &mut State,
) -> (Option<f64>, Option<f64>, Option<f64>) {
    let length = count(length);
    let previous_close = state.closes.last().copied();
    state.closes.push(bar.close);
    let span = if use_true_range {
        ta::true_range(bar, previous_close)
    } else {
        bar.high - bar.low
    };
    let basis = derived(state, 0).ema(source.number(), length);
    let average_range = derived(state, 1).ema(Some(span), length);
    match (basis, average_range) {
        (Some(basis), Some(average)) => (
            Some(basis),
            Some(basis + average * multiplier),
            Some(basis - average * multiplier),
        ),
        _ => (None, None, None),
    }
}

/// `ta.kcw`: the Keltner channel's width relative to its basis.
pub fn keltner_width(
    source: &RuntimeValue,
    length: i64,
    multiplier: f64,
    use_true_range: bool,
    bar: &Kline,
    state: &mut State,
) -> RuntimeValue {
    let (basis, upper, lower) = keltner(source, length, multiplier, use_true_range, bar, state);
    match (basis, upper, lower) {
        (Some(basis), Some(upper), Some(lower)) if basis != 0.0 => {
            out(Some((upper - lower) / basis))
        }
        _ => RuntimeValue::Na,
    }
}

/// `ta.bbw(series, length, mult)`: the Bollinger band width relative to the basis.
pub fn bollinger_width(
    source: &RuntimeValue,
    length: i64,
    multiplier: f64,
    state: &mut State,
) -> RuntimeValue {
    let bands = ta::evaluate_bb(source, length, multiplier, state);
    let RuntimeValue::Tuple(parts) = bands else {
        return RuntimeValue::Na;
    };
    match (parts[0].number(), parts[1].number(), parts[2].number()) {
        (Some(basis), Some(upper), Some(lower)) if basis != 0.0 => {
            out(Some((upper - lower) / basis))
        }
        _ => RuntimeValue::Na,
    }
}

/// `ta.tsi(source, short_length, long_length)`: the true strength index, in -1 to 1.
pub fn tsi(source: &RuntimeValue, short: i64, long: i64, state: &mut State) -> RuntimeValue {
    let previous = state.inputs.last().and_then(RuntimeValue::number);
    state.inputs.push(source.clone());
    let change = source
        .number()
        .zip(previous)
        .map(|(now, before)| now - before);
    let (short, long) = (count(short), count(long));
    let momentum = derived(state, 0)
        .ema(change, long)
        .and_then(|v| derived(state, 1).ema(Some(v), short));
    let magnitude = derived(state, 2)
        .ema(change.map(f64::abs), long)
        .and_then(|v| derived(state, 3).ema(Some(v), short));
    match (momentum, magnitude) {
        (Some(m), Some(a)) if a != 0.0 => out(Some(m / a)),
        _ => RuntimeValue::Na,
    }
}

/// The cumulative volume variables a script reads without calling: `ta.obv`, `ta.accdist`, `ta.iii`,
/// `ta.nvi`, `ta.pvi`, `ta.pvt`, `ta.wad` and `ta.wvad`. `None` when `name` is not one of them.
///
/// `state` remembers how many bars it has folded in, so it catches up from the committed history
/// on a bar where the script did not read the variable, and a repeated read is free.
pub fn cumulative(
    name: &str,
    current: &Kline,
    runtime: &RuntimeState,
    state: &mut State,
) -> Option<RuntimeValue> {
    if !VARIABLES.contains(&name) {
        return None;
    }
    let series = |key: &str| -> Vec<f64> {
        runtime
            .histories
            .get(key)
            .map(|values| {
                values
                    .iter()
                    .map(|v| v.number().unwrap_or(f64::NAN))
                    .collect()
            })
            .unwrap_or_default()
    };
    let (open, high, low, close, volume) = (
        series("open"),
        series("high"),
        series("low"),
        series("close"),
        series("volume"),
    );
    let bars = close.len();
    // `results` holds one value per folded bar; the bar being drawn is folded last and then dropped
    // again so it is recomputed once its close is final.
    let folded = state.results.len().min(bars);
    state.results.truncate(folded);
    for index in folded..=bars {
        let bar = if index < bars {
            Kline {
                open_time: 0,
                close_time: 0,
                open: open[index],
                high: high[index],
                low: low[index],
                close: close[index],
                volume: volume[index],
            }
        } else {
            current.clone()
        };
        let before = index.checked_sub(1).map(|i| Kline {
            open_time: 0,
            close_time: 0,
            open: open[i],
            high: high[i],
            low: low[i],
            close: close[i],
            volume: volume[i],
        });
        let prior = state.results.last().and_then(RuntimeValue::number);
        let next = fold(name, &bar, before.as_ref(), prior);
        if index < bars {
            state.results.push(out(Some(next)));
        } else {
            return Some(out(Some(next)));
        }
    }
    None
}

fn fold(name: &str, bar: &Kline, before: Option<&Kline>, prior: Option<f64>) -> f64 {
    let total = prior.unwrap_or(0.0);
    let span = bar.high - bar.low;
    let change = before.map(|b| bar.close - b.close);
    match name {
        "ta.obv" => {
            let direction = change.map_or(0.0, |c| match c {
                c if c > 0.0 => 1.0,
                c if c < 0.0 => -1.0,
                _ => 0.0,
            });
            total + direction * bar.volume
        }
        "ta.accdist" => {
            if (bar.close == bar.high && bar.close == bar.low) || span == 0.0 {
                total
            } else {
                total + (2.0 * bar.close - bar.low - bar.high) / span * bar.volume
            }
        }
        "ta.iii" => {
            let denominator = span * bar.volume;
            if denominator == 0.0 {
                total
            } else {
                total + (2.0 * bar.close - bar.high - bar.low) / denominator
            }
        }
        "ta.pvt" => match before {
            Some(b) if b.close != 0.0 => total + (bar.close - b.close) / b.close * bar.volume,
            _ => total,
        },
        "ta.nvi" | "ta.pvi" => {
            let level = prior.filter(|p| *p != 0.0).unwrap_or(1.0);
            let Some(b) = before.filter(|b| b.close != 0.0) else {
                return level;
            };
            let moves = if name == "ta.nvi" {
                bar.volume < b.volume
            } else {
                bar.volume > b.volume
            };
            if moves {
                level + (bar.close - b.close) / b.close * level
            } else {
                level
            }
        }
        "ta.wad" => {
            let Some(b) = before else { return total };
            let true_high = bar.high.max(b.close);
            let true_low = bar.low.min(b.close);
            let gain = match bar.close - b.close {
                m if m > 0.0 => bar.close - true_low,
                m if m < 0.0 => bar.close - true_high,
                _ => 0.0,
            };
            total + gain
        }
        // Not cumulative: the volume-weighted position of the close within the bar.
        _ => {
            if span == 0.0 {
                0.0
            } else {
                (bar.close - bar.open) / span * bar.volume
            }
        }
    }
}
