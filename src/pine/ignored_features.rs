//! Warnings about TradingView features the engine accepts and then ignores, ported from
//! `PineIgnoredFeatureValidator.swift` + `PineIgnoredFeatures.swift`.

use std::collections::HashMap;

use super::ast::{Argument, Expression, Statement};
use super::traversal::{for_each_call_with_args, for_each_identifier};
use crate::pine::{Diagnostic, DiagnosticCategory, DiagnosticSeverity, SourceRange};

/// Arguments, by function. A declaration (`indicator`, `strategy`, `library`) is keyed by its name.
fn ignored_arguments() -> HashMap<&'static str, HashMap<&'static str, &'static str>> {
    let mut table: HashMap<&'static str, HashMap<&'static str, &'static str>> = HashMap::new();
    table.insert("indicator", declaration());
    table.insert("library", declaration());
    let mut strategy = declaration();
    strategy.insert(
        "margin_long",
        "There is no margin model, so no margin call ever closes a position.",
    );
    strategy.insert(
        "margin_short",
        "There is no margin model, so no margin call ever closes a position.",
    );
    table.insert("strategy", strategy);
    table.insert("strategy.entry", entry());
    table.insert("strategy.order", entry());
    table.insert("strategy.exit", exit());
    table.insert("strategy.close", close());
    table.insert("strategy.close_all", close());
    table.insert("plot", plot());
    table.insert("hline", hline());
    table.insert("bgcolor", color_series());
    table.insert("barcolor", color_series());
    table.insert("fill", fill());
    table.insert("plotcandle", plotcandle());
    table.insert("box.new", box_new());
    let mut request_security = request();
    request_security.insert("calc_bars_count", "The whole history is always requested.");
    table.insert("request.security", request_security);
    table.insert("request.security_lower_tf", request());
    table.insert("plotshape", marker());
    table.insert("plotchar", marker());
    table
}

fn declaration() -> HashMap<&'static str, &'static str> {
    HashMap::from([
        (
            "explicit_plot_zorder",
            "Plots and drawings are painted in the app's own order.",
        ),
        (
            "scale",
            "The chart has one value axis, so the scale is not changed.",
        ),
    ])
}

fn entry() -> HashMap<&'static str, &'static str> {
    HashMap::from([
        (
            "oca_name",
            "Orders are never grouped; each fills on its own.",
        ),
        (
            "oca_type",
            "Orders are never grouped; each fills on its own.",
        ),
        ("comment", "Order comments are not shown in the trade list."),
        ("alert_message", "Order fill alerts are not sent."),
    ])
}

fn exit() -> HashMap<&'static str, &'static str> {
    HashMap::from([
        ("comment", "Exit comments are not shown in the trade list."),
        (
            "comment_profit",
            "Exit comments are not shown in the trade list.",
        ),
        (
            "comment_loss",
            "Exit comments are not shown in the trade list.",
        ),
        (
            "comment_trailing",
            "Exit comments are not shown in the trade list.",
        ),
        ("alert_message", "Order fill alerts are not sent."),
        ("alert_profit", "Order fill alerts are not sent."),
        ("alert_loss", "Order fill alerts are not sent."),
        ("alert_trailing", "Order fill alerts are not sent."),
        (
            "oca_name",
            "Orders are never grouped; each fills on its own.",
        ),
    ])
}

fn close() -> HashMap<&'static str, &'static str> {
    HashMap::from([
        ("comment", "Order comments are not shown in the trade list."),
        ("alert_message", "Order fill alerts are not sent."),
        (
            "immediately",
            "The position closes on the next bar's open, as with any market order.",
        ),
    ])
}

fn plot() -> HashMap<&'static str, &'static str> {
    HashMap::from([
        (
            "offset",
            "The plot is not shifted; it is drawn on the bar that produced each value.",
        ),
        ("trackprice", "No price line is drawn across the chart."),
        ("join", "Isolated points are not connected."),
        ("editable", "Plot settings cannot be edited from the chart."),
        (
            "show_last",
            "The plot is drawn on every bar, not only the last ones.",
        ),
        ("format", "The value is formatted by the chart's own rules."),
        (
            "precision",
            "The value is formatted by the chart's own rules.",
        ),
        (
            "force_overlay",
            "The plot stays in the pane the script declares.",
        ),
        ("linestyle", "The line is always solid."),
    ])
}

fn hline() -> HashMap<&'static str, &'static str> {
    HashMap::from([
        ("linestyle", "The line is always solid."),
        (
            "linewidth",
            "The line is always drawn at the default width.",
        ),
        ("editable", "The line cannot be edited from the chart."),
    ])
}

fn color_series() -> HashMap<&'static str, &'static str> {
    HashMap::from([
        (
            "offset",
            "The colors are not shifted; each applies to the bar that produced it.",
        ),
        ("editable", "The color cannot be edited from the chart."),
        (
            "show_last",
            "The colors are applied to every bar, not only the last ones.",
        ),
        (
            "force_overlay",
            "The colors apply to the pane the script declares.",
        ),
    ])
}

fn fill() -> HashMap<&'static str, &'static str> {
    HashMap::from([
        ("fillgaps", "Gaps in the plots are not bridged."),
        ("editable", "The fill cannot be edited from the chart."),
        (
            "show_last",
            "The fill is drawn on every bar, not only the last ones.",
        ),
    ])
}

fn plotcandle() -> HashMap<&'static str, &'static str> {
    HashMap::from([
        (
            "offset",
            "The candles are not shifted; each is drawn on the bar that produced it.",
        ),
        ("editable", "The candles cannot be edited from the chart."),
        (
            "show_last",
            "The candles are drawn on every bar, not only the last ones.",
        ),
    ])
}

fn box_new() -> HashMap<&'static str, &'static str> {
    HashMap::from([
        ("text_wrap", "Box text is never wrapped."),
        ("text_font_family", "Box text uses the default font."),
        (
            "text_formatting",
            "Box text has no bold or italic formatting.",
        ),
    ])
}

fn request() -> HashMap<&'static str, &'static str> {
    HashMap::from([("currency", "Prices are not converted to another currency.")])
}

fn marker() -> HashMap<&'static str, &'static str> {
    HashMap::from([
        ("text", "The text next to the marker is not drawn."),
        ("textcolor", "The text next to the marker is not drawn."),
        (
            "offset",
            "The markers are not shifted; each is drawn on the bar that produced it.",
        ),
        (
            "transp",
            "Transparency is not applied; use `color.new(color, transparency)` instead.",
        ),
        ("editable", "The markers cannot be edited from the chart."),
        (
            "show_last",
            "The markers are drawn on every bar, not only the last ones.",
        ),
        (
            "force_overlay",
            "The markers stay in the pane the script declares.",
        ),
    ])
}

/// Whole functions that do nothing, by name prefix.
fn function_prefixes() -> [(&'static str, &'static str); 1] {
    [(
        "strategy.risk.",
        "Risk limits are never enforced, so a drawdown or loss limit does not stop the strategy.",
    )]
}

/// Argument values that make the engine draw something else, keyed function → argument → constant.
fn ignored_values()
-> HashMap<&'static str, HashMap<&'static str, HashMap<&'static str, &'static str>>> {
    let mut plot: HashMap<&'static str, HashMap<&'static str, &'static str>> = HashMap::new();
    let mut style = HashMap::new();
    style.insert(
        "plot.style_stepline_diamond",
        "It is drawn as a plain line.",
    );
    plot.insert("style", style);
    HashMap::from([("plot", plot)])
}

/// Pine variables this release does not implement (they evaluate to `na`).
fn unimplemented_variables() -> [&'static str; 13] {
    [
        "weekofyear",
        "session.isfirstbar",
        "session.isfirstbar_regular",
        "session.islastbar",
        "session.islastbar_regular",
        "session.ismarket",
        "session.ispremarket",
        "session.ispostmarket",
        "syminfo.session",
        "syminfo.mincontract",
        "syminfo.description",
        "syminfo.volumetype",
        "session.extended",
    ]
}

pub fn validate(statements: &[Statement]) -> Vec<Diagnostic> {
    let own: std::collections::HashSet<&str> = statements
        .iter()
        .filter_map(|statement| match statement {
            Statement::Function { name, .. } => Some(name.as_str()),
            _ => None,
        })
        .collect();
    let arguments = ignored_arguments();
    let values = ignored_values();
    let prefixes = function_prefixes();
    let unimplemented = unimplemented_variables();

    let mut diagnostics = Vec::new();
    for_each_call_with_args(statements, &mut |name, call_arguments, range| {
        if own.contains(name) {
            return;
        }
        if let Some((_, explanation)) = prefixes.iter().find(|(p, _)| name.starts_with(p)) {
            diagnostics.push(warning(
                "PINE7002",
                format!("{name}() is ignored. {explanation}"),
                spanning(name, range),
            ));
        }
        for argument in call_arguments {
            let Some(argument_name) = argument.name.as_deref() else {
                continue;
            };
            if let Some(explanation) = arguments
                .get(name)
                .and_then(|table| table.get(argument_name))
            {
                diagnostics.push(warning(
                    "PINE7001",
                    format!("{name}(): `{argument_name}` is ignored. {explanation}"),
                    argument.name_range.unwrap_or(argument.value.range()),
                ));
            }
            if let Expression::Identifier(constant, value_range) = &argument.value
                && let Some(explanation) = values
                    .get(name)
                    .and_then(|table| table.get(argument_name))
                    .and_then(|table| table.get(constant.as_str()))
            {
                diagnostics.push(warning(
                    "PINE7004",
                    format!(
                        "{name}(): `{argument_name} = {constant}` is not supported. {explanation}"
                    ),
                    spanning(constant, *value_range),
                ));
            }
        }
    });
    for_each_identifier(statements, &mut |name, range| {
        if unimplemented.contains(&name) && !own.contains(name) {
            diagnostics.push(warning(
                "PINE7003",
                format!(
                    "`{name}` is not supported. It is not modelled here, so it is always `na`."
                ),
                spanning(name, range),
            ));
        }
    });
    diagnostics.sort_by_key(|d| d.range.start.offset);
    diagnostics
}

/// The parser records only the first token of a dotted name; the underline should cover all of it.
fn spanning(name: &str, range: SourceRange) -> SourceRange {
    let length = name.encode_utf16().count();
    if range.end.offset - range.start.offset >= length {
        return range;
    }
    let mut end = range.start;
    end.column += length;
    end.offset += length;
    SourceRange::new(range.start, end)
}

fn warning(code: &str, message: String, range: SourceRange) -> Diagnostic {
    Diagnostic {
        code: code.to_owned(),
        severity: DiagnosticSeverity::Warning,
        category: DiagnosticCategory::Unsupported,
        message,
        range,
    }
}
