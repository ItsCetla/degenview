//! End-to-end checks of language features that real scripts lean on: each test compiles a small
//! original script, runs it over a few bars and reads the variables the last bar left behind.

use std::collections::HashMap;

use crate::model::Kline;
use crate::pine::compiler::{self, NoLibraries};
use crate::pine::program::SymbolInfo;
use crate::pine::runtime_value::RuntimeValue;
use crate::pine::session::Session;
use crate::pine::token::Limits;

fn bars(count: usize) -> Vec<Kline> {
    (0..count)
        .map(|index| {
            let price = 100.0 + index as f64;
            Kline {
                open_time: index as i64 * 60_000,
                close_time: (index as i64 + 1) * 60_000,
                open: price,
                high: price + 2.0,
                low: price - 2.0,
                close: price + 1.0,
                volume: 10.0,
            }
        })
        .collect()
}

/// Compiles and runs `body` (after the version and declaration lines) over `count` bars.
fn run(body: &str, count: usize) -> Session {
    let source = format!("//@version=6\nindicator(\"T\")\n{body}");
    let program = compiler::compile(&source, Limits::default(), &NoLibraries);
    assert!(program.is_valid(), "{:?}", program.diagnostics);
    let mut session = Session::new(
        program,
        HashMap::new(),
        Limits::default(),
        None,
        SymbolInfo::default(),
        None,
        None,
    );
    if let Err(error) = session.evaluate(&bars(count)) {
        panic!("{error:?}");
    }
    session
}

fn value(session: &Session, name: &str) -> RuntimeValue {
    session
        .working
        .variables
        .get(name)
        .cloned()
        .unwrap_or_else(|| panic!("no variable {name}"))
}

#[test]
fn a_bool_field_drives_a_condition() {
    let session = run(
        "type Zone\n    float top\n    bool active\n\
         z = Zone.new(5.0, true)\n\
         a = z.active ? 1 : 0\n\
         b = 0\n\
         if z.active\n    b := 2\n\
         c = z.top",
        2,
    );
    assert_eq!(value(&session, "a"), RuntimeValue::Int(1));
    assert_eq!(value(&session, "b"), RuntimeValue::Int(2));
    assert_eq!(value(&session, "c"), RuntimeValue::Float(5.0));
}

#[test]
fn a_dotted_field_can_be_assigned_and_updated() {
    let session = run(
        "type Counter\n    int hits\n    bool on\n\
         var c = Counter.new(0, true)\n\
         c.hits += 1\n\
         c.on := c.hits < 3\n\
         total = c.hits\n\
         still = c.on",
        5,
    );
    assert_eq!(value(&session, "total"), RuntimeValue::Int(5));
    assert_eq!(value(&session, "still"), RuntimeValue::Bool(false));
}

#[test]
fn fields_of_nested_objects_read_and_write_through_the_chain() {
    let session = run(
        "type Inner\n    float level\n\
         type Outer\n    Inner inner\n\
         var o = Outer.new(Inner.new(1.0))\n\
         o.inner.level := o.inner.level + 2.0\n\
         seen = o.inner.level",
        1,
    );
    assert_eq!(value(&session, "seen"), RuntimeValue::Float(3.0));
}

#[test]
fn a_method_runs_on_its_object_in_both_call_styles() {
    let session = run(
        "type Box2\n    float w\n    float h\n\
         method area(Box2 this) =>\n    this.w * this.h\n\
         method grow(Box2 this, float by) =>\n    this.w += by\n    this\n\
         b = Box2.new(2.0, 3.0)\n\
         first = b.area()\n\
         grown = b.grow(1.0)\n\
         again = area(b)\n\
         width = b.w",
        1,
    );
    assert_eq!(value(&session, "first"), RuntimeValue::Float(6.0));
    assert_eq!(value(&session, "again"), RuntimeValue::Float(9.0));
    assert_eq!(value(&session, "width"), RuntimeValue::Float(3.0));
}

#[test]
fn method_overloads_are_chosen_by_the_receiver_type() {
    let session = run(
        "type Dog\n    int legs\n\
         type Bird\n    int legs\n\
         method describe(Dog this) =>\n    \"dog\"\n\
         method describe(Bird this) =>\n    \"bird\"\n\
         method describe(int this) =>\n    \"int\"\n\
         method describe(string this) =>\n    \"string\"\n\
         d = Dog.new(4).describe()\n\
         b = describe(Bird.new(2))\n\
         i = (3).describe()\n\
         s = \"x\".describe()",
        1,
    );
    assert_eq!(value(&session, "d"), RuntimeValue::String("dog".into()));
    assert_eq!(value(&session, "b"), RuntimeValue::String("bird".into()));
    assert_eq!(value(&session, "i"), RuntimeValue::String("int".into()));
    assert_eq!(value(&session, "s"), RuntimeValue::String("string".into()));
}

#[test]
fn a_method_on_a_variable_of_a_builtin_type() {
    let session = run(
        "method twice(float this) =>\n    this * 2.0\n\
         x = close\n\
         y = x.twice()",
        3,
    );
    assert_eq!(value(&session, "y"), RuntimeValue::Float(2.0 * 103.0));
}

fn close_to(session: &Session, name: &str, expected: f64) {
    let RuntimeValue::Float(actual) = value(session, name) else {
        panic!("{name} is {:?}", value(session, name));
    };
    assert!(
        (actual - expected).abs() < 1e-9,
        "{name}: {actual} != {expected}"
    );
}

#[test]
fn windowed_statistics_follow_the_reference_formulas() {
    // Five bars: close 101…105, high close+1, low close-3, volume 10.
    let session = run(
        "alma = ta.alma(close, 3, 0.85, 6)\n\
         nearest = ta.percentile_nearest_rank(close, 5, 90)\n\
         linear = ta.percentile_linear_interpolation(close, 5, 90)\n\
         median = ta.percentile_linear_interpolation(close, 5, 50)\n\
         common = ta.mode(close % 2 == 0 ? 1.0 : 2.0, 5)\n\
         gravity = ta.cog(close, 2)\n\
         weighted = ta.vwma(close, 3)\n\
         every = ta.valuewhen(bar_index % 2 == 0, close, 0)\n\
         before = ta.valuewhen(bar_index % 2 == 0, close, 1)",
        5,
    );
    close_to(&session, "alma", 103.314_326_399_761_74);
    close_to(&session, "nearest", 105.0);
    close_to(&session, "linear", 104.6);
    close_to(&session, "median", 103.0);
    // close 101…105: odd values 101, 103, 105 map to 2.0 (three of them), even ones to 1.0.
    close_to(&session, "common", 2.0);
    // -(105 * 1 + 104 * 2) / (105 + 104)
    close_to(&session, "gravity", -(105.0 + 208.0) / 209.0);
    close_to(&session, "weighted", 104.0);
    close_to(&session, "every", 105.0);
    close_to(&session, "before", 103.0);
}

#[test]
fn range_indicators_read_the_bar_range() {
    let session = run(
        "stoch = ta.stoch(close, high, low, 3)\n\
         wpr = ta.wpr(3)\n\
         [basis, upper, lower] = ta.kc(close, 3, 2)\n\
         width = ta.kcw(close, 3, 2)\n\
         strength = ta.tsi(close, 2, 3)\n\
         flow = ta.mfi(close, 3)",
        5,
    );
    // Highs 104…106 → highest 106; lows 98+2…: lowest 100; close 105.
    close_to(&session, "stoch", 100.0 * 5.0 / 6.0);
    close_to(&session, "wpr", -100.0 / 6.0);
    // EMA(3) of close 101…105 seeds at 102 and ends at 104; the true range is 4 on every bar.
    close_to(&session, "basis", 104.0);
    close_to(&session, "upper", 112.0);
    close_to(&session, "lower", 96.0);
    close_to(&session, "width", 16.0 / 104.0);
    // Every change is +1, so the doubly smoothed momentum equals the smoothed magnitude.
    close_to(&session, "strength", 1.0);
    // Only rising closes: no money flows out.
    close_to(&session, "flow", 100.0);
}

#[test]
fn cumulative_volume_variables_catch_up_on_skipped_bars() {
    // Read only on the last bar: the state must still fold in the bars before it.
    let session = run(
        "var float seen_obv = na\n\
         var float seen_acc = na\n\
         var float seen_wvad = na\n\
         if barstate.islast\n    seen_obv := ta.obv\n    seen_acc := ta.accdist\n    seen_wvad := ta.wvad",
        5,
    );
    close_to(&session, "seen_obv", 40.0);
    // (2 * close - low - high) / (high - low) * volume is 2 / 4 * 10 = 5 on every bar, five bars.
    close_to(&session, "seen_acc", 25.0);
    // (close - open) / (high - low) * volume = 1 / 4 * 10 on the last bar.
    close_to(&session, "seen_wvad", 2.5);
}

/// The codes of every diagnostic the compiler reports for `body`, errors and warnings alike.
fn diagnostic_codes(body: &str) -> Vec<String> {
    let source = format!("//@version=6\nindicator(\"T\")\n{body}");
    compiler::compile(&source, Limits::default(), &NoLibraries)
        .diagnostics
        .into_iter()
        .map(|d| d.code)
        .collect()
}

#[test]
fn stateless_builtins_return_what_the_app_can_know() {
    let session = run(
        "max_bars_back(close, 500)\n\
         pair = ticker.new(\"BINANCE\", \"BTCUSDT\")\n\
         plain = ticker.standard(\"BINANCE:ETHUSDT\")\n\
         kept = ticker.inherit(\"A:B\", \"C:D\")\n\
         var box slot = box(na)\n\
         empty = na(slot)\n\
         same = request.currency_rate(\"USD\", \"USD\")\n\
         other = request.currency_rate(\"USD\", \"EUR\")\n\
         seed = request.seed(\"repo\", \"SYM\", close)\n\
         log.info(\"hello {0}\", close)",
        2,
    );
    assert_eq!(
        value(&session, "pair"),
        RuntimeValue::String("BINANCE:BTCUSDT".into())
    );
    assert_eq!(
        value(&session, "plain"),
        RuntimeValue::String("BINANCE:ETHUSDT".into())
    );
    assert_eq!(value(&session, "kept"), RuntimeValue::String("C:D".into()));
    assert_eq!(value(&session, "empty"), RuntimeValue::Bool(true));
    assert_eq!(value(&session, "same"), RuntimeValue::Float(1.0));
    assert_eq!(value(&session, "other"), RuntimeValue::Na);
    assert_eq!(value(&session, "seed"), RuntimeValue::Na);
}

#[test]
fn what_cannot_be_honoured_is_warned_about() {
    let codes = diagnostic_codes(
        "a = request.seed(\"repo\", \"SYM\", close)\n\
         b = request.economic(\"US\", \"GDP\")\n\
         c = ticker.heikinashi(syminfo.tickerid)\n\
         log.error(\"x\")\n\
         plot(close)",
    );
    assert_eq!(
        codes.iter().filter(|c| *c == "PINE7005").count(),
        3,
        "{codes:?}"
    );
    assert!(codes.contains(&"PINE7002".to_owned()), "{codes:?}");
    assert!(!codes.contains(&"PINE9003".to_owned()), "{codes:?}");
    // A feed that returns an object has no stand-in and stays a compile error.
    let footprint = diagnostic_codes("f = request.footprint(1)\nplot(close)");
    assert!(footprint.contains(&"PINE9003".to_owned()), "{footprint:?}");
}

#[test]
fn a_parameter_may_reuse_a_builtin_function_name_with_a_warning() {
    let codes = diagnostic_codes("f(color fill) =>\n    fill\nplot(close)");
    assert_eq!(codes, ["PINE7006"]);
    // Builtin variables stay reserved.
    let reserved = diagnostic_codes("f(float close) =>\n    close\nplot(close)");
    assert!(reserved.contains(&"CE10190".to_owned()), "{reserved:?}");
}

#[test]
fn int_truncates_toward_zero() {
    let session = run(
        "up = int(2.9)\n\
         down = int(-2.9)\n\
         whole = int(7)\n\
         gone = int(na)\n\
         bucket = math.min(5, int((close - 100.0) / 2.0))",
        4,
    );
    assert_eq!(value(&session, "up"), RuntimeValue::Int(2));
    assert_eq!(value(&session, "down"), RuntimeValue::Int(-2));
    assert_eq!(value(&session, "whole"), RuntimeValue::Int(7));
    assert_eq!(value(&session, "gone"), RuntimeValue::Na);
    // close is 104 on the last bar: (104 - 100) / 2 = 2.
    assert_eq!(value(&session, "bucket"), RuntimeValue::Int(2));
}

#[test]
fn a_string_may_run_over_several_lines() {
    let session = run(
        "tip = \"first line\n second line\"\n\
         after = 1 + 2\n\
         plot(close, title = tip)",
        1,
    );
    assert_eq!(
        value(&session, "tip"),
        RuntimeValue::String("first line\n second line".to_owned())
    );
    assert_eq!(value(&session, "after"), RuntimeValue::Int(3));
    // A quote that is never closed still reports where it started.
    let codes = diagnostic_codes("broken = \"never closed\nplot(close)\nx = 1\ny = 2");
    assert!(codes.contains(&"PINE1003".to_owned()), "{codes:?}");
}

#[test]
fn a_function_declared_twice_is_picked_by_its_argument_types() {
    let session = run(
        "label_of(float value) =>\n    \"number\"\n\
         label_of(string text, bool loud = false) =>\n    loud ? \"SHOUT\" : \"text\"\n\
         label_of(float lower, float upper) =>\n    \"range\"\n\
         a = label_of(1.5)\n\
         b = label_of(\"hi\")\n\
         c = label_of(\"hi\", true)\n\
         d = label_of(1.0, 2.0)\n\
         e = label_of(text = \"named\")",
        1,
    );
    let text = |name: &str| value(&session, name);
    assert_eq!(text("a"), RuntimeValue::String("number".into()));
    assert_eq!(text("b"), RuntimeValue::String("text".into()));
    assert_eq!(text("c"), RuntimeValue::String("SHOUT".into()));
    assert_eq!(text("d"), RuntimeValue::String("range".into()));
    assert_eq!(text("e"), RuntimeValue::String("text".into()));
    // The same signature twice is still an error.
    let codes = diagnostic_codes("f(float x) =>\n    1\nf(float y) =>\n    2\nplot(close)");
    assert!(codes.contains(&"PINE3024".to_owned()), "{codes:?}");
}

#[test]
fn var_inside_a_function_initialises_once_per_call_site() {
    let session = run(
        "counter() =>\n    var int n = 10\n    n += 1\n    n\n\
         first = counter()\n\
         again = counter()",
        3,
    );
    // Each call site keeps its own `n`: 10 + one increment per bar.
    assert_eq!(value(&session, "first"), RuntimeValue::Int(13));
    assert_eq!(value(&session, "again"), RuntimeValue::Int(13));
}

#[test]
fn history_offsets_that_reach_no_bar_are_na() {
    let session = run(
        "gone = close[na]\n\
         far = close[100]\n\
         back = close[1]\n\
         offset = bar_index > 1 ? 2 : na\n\
         dynamic = close[offset]",
        4,
    );
    assert_eq!(value(&session, "gone"), RuntimeValue::Na);
    assert_eq!(value(&session, "far"), RuntimeValue::Na);
    assert_eq!(value(&session, "back"), RuntimeValue::Float(103.0));
    assert_eq!(value(&session, "dynamic"), RuntimeValue::Float(102.0));
}

#[test]
fn drawing_collections_list_what_is_alive() {
    let session = run(
        "var box first = box.new(0, 2.0, 1, 1.0)\n\
         if bar_index == 2\n    box.new(2, 3.0, 3, 2.0)\n\
         if bar_index == 3\n    box.delete(first)\n\
         count = array.size(box.all)\n\
         none = array.size(table.all)",
        4,
    );
    assert_eq!(value(&session, "count"), RuntimeValue::Int(1));
    assert_eq!(value(&session, "none"), RuntimeValue::Int(0));
}

#[test]
fn an_unknown_member_of_a_variable_namespace_is_an_error_not_a_string() {
    let source = "//@version=6\nindicator(\"T\")\nx = ta.nonexistent\nplot(close)";
    let program = compiler::compile(source, Limits::default(), &NoLibraries);
    let mut session = Session::new(
        program,
        HashMap::new(),
        Limits::default(),
        None,
        SymbolInfo::default(),
        None,
        None,
    );
    let Err(crate::pine::session::RunError::Script(diagnostic)) = session.evaluate(&bars(2)) else {
        panic!("expected a script error");
    };
    assert_eq!(diagnostic.code, "PINE4008");
    assert!(diagnostic.message.contains("ta.nonexistent"));
    // Named constants of the other namespaces still read as themselves.
    let session = run("style = label.style_label_up\nplot(close)", 1);
    assert_eq!(
        value(&session, "style"),
        RuntimeValue::String("label.style_label_up".into())
    );
}

#[test]
fn a_method_can_take_an_enum_receiver() {
    let session = run(
        "enum Side\n    long\n    short\n\
         method sign(Side this) =>\n    this == Side.long ? 1 : -1\n\
         up = Side.long.sign()\n\
         down = sign(Side.short)",
        1,
    );
    assert_eq!(value(&session, "up"), RuntimeValue::Int(1));
    assert_eq!(value(&session, "down"), RuntimeValue::Int(-1));
}

#[test]
fn named_time_zones_follow_daylight_saving_time() {
    // The first bar opens at the epoch, 00:00 UTC: 19:00 the day before in New York (standard time).
    let session = run(
        "ny = hour(time, \"America/New_York\")\n\
         tokyo = hour(time, \"Asia/Tokyo\")\n\
         plain = hour(time, \"UTC-5\")\n\
         open930 = timestamp(\"America/New_York\", 2024, 7, 15, 9, 30)\n\
         shown = str.format_time(open930, \"HH:mm\", \"America/New_York\")",
        1,
    );
    assert_eq!(value(&session, "ny"), RuntimeValue::Int(19));
    assert_eq!(value(&session, "tokyo"), RuntimeValue::Int(9));
    assert_eq!(value(&session, "plain"), RuntimeValue::Int(19));
    // 09:30 on a July day in New York is 13:30 UTC.
    assert_eq!(
        value(&session, "open930"),
        RuntimeValue::Int(1_721_050_200_000)
    );
    assert_eq!(
        value(&session, "shown"),
        RuntimeValue::String("09:30".into())
    );
}

#[test]
fn a_zone_the_table_lacks_is_warned_about() {
    let codes = diagnostic_codes(
        "z = input.string(\"Asia/Jerusalem\", \"zone\")\nok = hour(time, \"Europe/Paris\")\nplot(close)",
    );
    assert_eq!(codes, ["PINE7007"]);
}

#[test]
fn ratios_with_nothing_to_divide_are_na_not_nan() {
    // A flat market: no money flow in either direction, and a zero-width range.
    let session = run(
        "flat = 100.0\n\
         flow = ta.mfi(flat, 3)\n\
         pos = ta.stoch(flat, flat, flat, 3)\n\
         plain = ta.wpr(3)",
        5,
    );
    assert_eq!(value(&session, "flow"), RuntimeValue::Na);
    assert_eq!(value(&session, "pos"), RuntimeValue::Na);
    // The bars themselves still span 4 points, so %R is defined.
    assert!(matches!(value(&session, "plain"), RuntimeValue::Float(_)));
}
