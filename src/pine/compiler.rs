//! Source text → `CompiledProgram`: lex, parse, read the declaration, fold constants, collect
//! inputs, then run the structural and type checks. Ported from `PineCompiler*.swift` +
//! `PineLibraryLinker.swift`.

use std::collections::{HashMap, HashSet};

use super::alert_frequency::AlertFrequency;
use super::ast::{Argument, EnumMember, Expression, Statement};
use super::builtins;
use super::calendar;
use super::operators;
use super::outputs::{
    EnumMember, LineExtend, LineStyle, MarkerLocation, MarkerShape, Size, TablePosition,
};
use super::program::{
    CompiledProgram, DeclarationMetadata, Import, InputDefinition, InputSchema, InputValue,
    ScriptType,
};
use super::runtime_value::RuntimeValue;
use super::strategy::StrategySettings;
use super::timestamp;
use super::traversal::assigned_names;
use crate::pine::{Diagnostic, DiagnosticCategory, Lexer, Limits, Parser, SourceRange, ValueType};

/// Where an `import user/Library/version` finds its source. The compiler never reads files: the
/// app supplies the libraries it has, and tests supply their own.
pub trait LibraryResolver {
    fn source_for_library(&self, path: &str) -> Option<String>;
}

/// The default: no library can be found.
pub struct NoLibraries;

impl LibraryResolver for NoLibraries {
    fn source_for_library(&self, _path: &str) -> Option<String> {
        None
    }
}

const VERSION_ANNOTATION: &str = "//@version=";
const SUPPORTED_VERSION: &str = "6";

pub fn compile(source: &str, limits: Limits, libraries: &dyn LibraryResolver) -> CompiledProgram {
    compile_internal(source, limits, libraries, &[])
}

fn compile_internal(
    source: &str,
    limits: Limits,
    libraries: &dyn LibraryResolver,
    import_stack: &[String],
) -> CompiledProgram {
    let normalized = normalize_line_endings(source);
    let mut diagnostics = Vec::new();
    let version = declared_version(&normalized, &mut diagnostics);

    let lexed = Lexer::new(&normalized).lex();
    diagnostics.extend(lexed.diagnostics);

    let scanned = scan_imports(&normalized);
    diagnostics.extend(scanned.diagnostics);

    let import_aliases: HashSet<String> = scanned
        .declarations
        .iter()
        .map(|d| d.alias.clone())
        .collect();
    let mut parser = Parser::new(lexed.tokens, limits, import_aliases);
    let (statements, parse_diagnostics) = parser.parse();
    diagnostics.extend(parse_diagnostics);

    let linked = link_imports(&scanned.declarations, libraries, limits, import_stack);
    diagnostics.extend(linked.diagnostics);
    diagnostics.extend(validate_usage(&statements, &linked.imports));

    let environment = constant_environment(&statements);
    let metadata = declaration_metadata(
        &statements,
        version.as_deref(),
        &environment,
        &mut diagnostics,
    );
    if metadata.script_type != ScriptType::Library {
        diagnostics.extend(parser.export_ranges().iter().map(|range| {
            Diagnostic::error(
                "PINE3037",
                DiagnosticCategory::Semantic,
                "export is only allowed in a library().",
                *range,
            )
        }));
    }
    let mut schema = InputSchema::default();
    collect_inputs(&statements, &environment, &mut schema, &mut diagnostics);

    diagnostics.extend(structure_validate(
        &statements,
        parser.method_names(),
        &normalized,
    ));
    diagnostics.extend(super::alert_call::validate(&statements));
    let has_early_error = diagnostics.iter().any(|d| {
        matches!(
            d.category,
            DiagnosticCategory::Lexical | DiagnosticCategory::Syntax
        )
    });
    if !has_early_error {
        diagnostics.extend(super::type_checker::TypeChecker::check(&statements));
        diagnostics.extend(super::ignored_features::validate(&statements));
    }

    CompiledProgram {
        source: normalized,
        statements,
        declaration: metadata,
        input_schema: schema,
        diagnostics,
        method_names: parser.method_names().clone(),
        exported_names: parser.exported_names().clone(),
        imports: linked.imports,
    }
}

/// Text copied from browsers can contain CR-only or Unicode line separators.
pub fn normalize_line_endings(source: &str) -> String {
    source
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace('\u{2028}', "\n")
        .replace('\u{2029}', "\n")
}

fn declared_version(source: &str, diagnostics: &mut Vec<Diagnostic>) -> Option<String> {
    let versions: Vec<String> = source
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with(VERSION_ANNOTATION))
        .map(|line| line[VERSION_ANNOTATION.len()..].to_owned())
        .collect();
    let first = versions.first().cloned();
    match first {
        None => {
            diagnostics.push(Diagnostic::error(
                "PINE0001",
                DiagnosticCategory::Semantic,
                "Missing //@version=6 annotation.",
                SourceRange::ZERO,
            ));
            None
        }
        Some(version) => {
            if version != SUPPORTED_VERSION {
                diagnostics.push(Diagnostic::error(
                    "PINE0002",
                    DiagnosticCategory::Semantic,
                    format!(
                        "Pine Script version {version} is not supported; this runtime targets v6."
                    ),
                    SourceRange::ZERO,
                ));
            }
            Some(version)
        }
    }
}

// MARK: Declaration

struct ScriptDeclaration {
    script_type: ScriptType,
    name: String,
    arguments: Vec<Argument>,
    range: SourceRange,
}

fn script_declarations(statements: &[Statement]) -> Vec<ScriptDeclaration> {
    statements
        .iter()
        .filter_map(|statement| {
            let Statement::Expression(Expression::Call {
                name,
                arguments,
                range,
                ..
            }) = statement
            else {
                return None;
            };
            let script_type = match name.as_str() {
                "indicator" => ScriptType::Indicator,
                "strategy" => ScriptType::Strategy,
                "library" => ScriptType::Library,
                _ => return None,
            };
            Some(ScriptDeclaration {
                script_type,
                name: name.clone(),
                arguments: arguments.clone(),
                range: *range,
            })
        })
        .collect()
}

const COMMON_ARGUMENTS: [&str; 15] = [
    "title",
    "shorttitle",
    "overlay",
    "format",
    "precision",
    "max_bars_back",
    "max_lines_count",
    "max_labels_count",
    "max_boxes_count",
    "max_polylines_count",
    "behind_chart",
    "explicit_plot_zorder",
    "dynamic_requests",
    "scale",
    "backadjustment",
];

const STRATEGY_ARGUMENTS: [&str; 14] = [
    "initial_capital",
    "default_qty_type",
    "default_qty_value",
    "commission_type",
    "commission_value",
    "slippage",
    "pyramiding",
    "currency",
    "process_orders_on_close",
    "calc_on_order_fills",
    "calc_on_every_tick",
    "close_entries_rule",
    "margin_long",
    "margin_short",
];

fn declaration_metadata(
    statements: &[Statement],
    version: Option<&str>,
    environment: &HashMap<String, RuntimeValue>,
    diagnostics: &mut Vec<Diagnostic>,
) -> DeclarationMetadata {
    let declarations = script_declarations(statements);
    if declarations.len() != 1 {
        diagnostics.push(Diagnostic::error(
            "PINE3001",
            DiagnosticCategory::Semantic,
            "A script must contain exactly one indicator(), strategy(), or library() declaration.",
            declarations.first().map_or(SourceRange::ZERO, |d| d.range),
        ));
    }
    let mut metadata = DeclarationMetadata {
        script_type: declarations
            .first()
            .map_or(ScriptType::Indicator, |d| d.script_type),
        pine_version: version.and_then(|v| v.parse().ok()),
        ..DeclarationMetadata::default()
    };
    if metadata.script_type == ScriptType::Strategy {
        metadata.strategy = Some(StrategySettings::default());
    }
    if let Some(declaration) = declarations.first() {
        apply_declaration(declaration, environment, &mut metadata, diagnostics);
    }
    metadata
}

fn apply_declaration(
    declaration: &ScriptDeclaration,
    environment: &HashMap<String, RuntimeValue>,
    metadata: &mut DeclarationMetadata,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if let Some(first) = declaration.arguments.first() {
        if first.name.as_deref().is_none_or(|n| n == "title") {
            if let Some(RuntimeValue::String(title)) = constant_value(&first.value, environment) {
                metadata.title = title;
                return apply_remaining(declaration, environment, metadata, diagnostics);
            }
        }
    }
    diagnostics.push(Diagnostic::error(
        "PINE3002",
        DiagnosticCategory::Semantic,
        format!("{}() title must be a constant string.", declaration.name),
        declaration.range,
    ));
    apply_remaining(declaration, environment, metadata, diagnostics);
}

fn apply_remaining(
    declaration: &ScriptDeclaration,
    environment: &HashMap<String, RuntimeValue>,
    metadata: &mut DeclarationMetadata,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let supported: HashSet<&str> = {
        let mut set: HashSet<&str> = COMMON_ARGUMENTS.iter().copied().collect();
        if metadata.script_type == ScriptType::Strategy {
            set.extend(STRATEGY_ARGUMENTS);
        }
        set
    };
    for argument in &declaration.arguments {
        let Some(name) = argument.name.as_deref() else {
            continue;
        };
        if !supported.contains(name) {
            diagnostics.push(Diagnostic::error(
                "PINE9001",
                DiagnosticCategory::Unsupported,
                format!("Unsupported {}() argument '{name}'.", declaration.name),
                argument.name_range.unwrap_or(argument.value.range()),
            ));
            continue;
        }
        let Some(value) = constant_value(&argument.value, environment) else {
            continue;
        };
        apply_argument(name, value, metadata);
    }
}

fn apply_argument(name: &str, value: RuntimeValue, metadata: &mut DeclarationMetadata) {
    match (name, value) {
        ("shorttitle", RuntimeValue::String(v)) => metadata.short_title = Some(v),
        ("overlay", RuntimeValue::Bool(v)) => metadata.overlay = v,
        ("format", RuntimeValue::String(v)) => metadata.format = Some(v),
        ("precision", RuntimeValue::Int(v)) => metadata.precision = Some(v),
        ("max_bars_back", RuntimeValue::Int(v)) => metadata.max_bars_back = Some(v),
        ("max_lines_count", RuntimeValue::Int(v)) => metadata.max_lines_count = Some(v),
        ("max_labels_count", RuntimeValue::Int(v)) => metadata.max_labels_count = Some(v),
        ("max_boxes_count", RuntimeValue::Int(v)) => metadata.max_boxes_count = Some(v),
        ("max_polylines_count", RuntimeValue::Int(v)) => metadata.max_polylines_count = Some(v),
        ("behind_chart", RuntimeValue::Bool(v)) => metadata.behind_chart = Some(v),
        _ => apply_strategy_setting(name, value, metadata),
    }
}

fn apply_strategy_setting(name: &str, value: RuntimeValue, metadata: &mut DeclarationMetadata) {
    let Some(settings) = metadata.strategy.as_mut() else {
        return;
    };
    match (name, value) {
        ("initial_capital", _) => {
            if let Some(n) = value.number().filter(|n| *n > 0.0) {
                settings.initial_capital = n;
            }
        }
        ("default_qty_type", RuntimeValue::String(v)) => {
            settings.quantity_type = match v.as_str() {
                "strategy.cash" => super::strategy::QuantityType::Cash,
                "strategy.percent_of_equity" => super::strategy::QuantityType::PercentOfEquity,
                _ => super::strategy::QuantityType::Fixed,
            };
        }
        ("default_qty_value", _) => {
            if let Some(n) = value.number().filter(|n| *n >= 0.0) {
                settings.quantity_value = n;
            }
        }
        ("commission_type", RuntimeValue::String(v)) => {
            settings.commission_type = match v.as_str() {
                "strategy.commission.cash_per_order" => {
                    super::strategy::CommissionType::CashPerOrder
                }
                "strategy.commission.cash_per_contract" => {
                    super::strategy::CommissionType::CashPerContract
                }
                _ => super::strategy::CommissionType::Percent,
            };
        }
        ("commission_value", _) => {
            if let Some(n) = value.number().filter(|n| *n >= 0.0) {
                settings.commission_value = n;
            }
        }
        ("slippage", _) => {
            if let Some(n) = value.number().filter(|n| *n >= 0.0 && *n < 1e6) {
                settings.slippage = n as usize;
            }
        }
        ("pyramiding", _) => {
            if let Some(n) = value.number().filter(|n| *n >= 0.0 && *n < 1e6) {
                settings.pyramiding = n as usize;
            }
        }
        ("currency", RuntimeValue::String(v)) => settings.currency = Some(v),
        ("process_orders_on_close", RuntimeValue::Bool(v)) => settings.process_orders_on_close = v,
        ("calc_on_every_tick", RuntimeValue::Bool(v)) => settings.calc_on_every_tick = v,
        _ => {}
    }
}

// MARK: Constants

fn constant_environment(statements: &[Statement]) -> HashMap<String, RuntimeValue> {
    let reassigned = assigned_names(statements);
    let mut environment = HashMap::new();
    for statement in statements {
        let Statement::Declaration {
            name, mode, value, ..
        } = statement
        else {
            continue;
        };
        if *mode != super::ast::DeclarationMode::Ordinary || reassigned.contains(name) {
            continue;
        }
        if let Some(value) = constant_value(value, &environment) {
            environment.insert(name.clone(), value);
        }
    }
    environment
}

fn constant_value(
    expression: &Expression,
    environment: &HashMap<String, RuntimeValue>,
) -> Option<RuntimeValue> {
    match expression {
        Expression::Literal(value, _) => {
            let value = match value {
                super::ast::Literal::Int(n) => RuntimeValue::Int(*n),
                super::ast::Literal::Float(n) => RuntimeValue::Float(*n),
                super::ast::Literal::Bool(b) => RuntimeValue::Bool(*b),
                super::ast::Literal::String(s) => RuntimeValue::String(s.clone()),
                super::ast::Literal::Color(c) => RuntimeValue::Color(*c),
                super::ast::Literal::Na => return None,
            };
            Some(value)
        }
        Expression::Identifier(name, _) => {
            if let Some(value) = environment.get(name) {
                return Some(value.clone());
            }
            if let Some(value) = builtins::constant(name) {
                return Some(value);
            }
            if let Some(color) = builtins::color(name) {
                return Some(RuntimeValue::Color(color));
            }
            // A named constant is the string of its own name (`size.small`, `shape.circle`).
            if is_named_constant(name) {
                Some(RuntimeValue::String(name.clone()))
            } else {
                None
            }
        }
        Expression::Unary {
            op: super::ast::UnaryOperator::Negate,
            operand,
            ..
        } => match constant_value(operand, environment)? {
            RuntimeValue::Int(n) => Some(RuntimeValue::Int(0i64.wrapping_sub(n))),
            RuntimeValue::Float(n) => Some(RuntimeValue::Float(-n)),
            _ => None,
        },
        Expression::Binary { lhs, op, rhs, .. } => {
            if !op.is_arithmetic() {
                return None;
            }
            let a = constant_value(lhs, environment)?;
            let b = constant_value(rhs, environment)?;
            let value = operators::apply(*op, &a, &b);
            if value == RuntimeValue::Na {
                None
            } else {
                Some(value)
            }
        }
        Expression::Call {
            name, arguments, ..
        } if name == "timestamp" => constant_timestamp(arguments, environment),
        Expression::Call {
            name, arguments, ..
        } if name == "color.new" || name == "color.rgb" => {
            constant_color(expression, environment).map(RuntimeValue::Color)
        }
        _ => None,
    }
}

fn constant_timestamp(
    arguments: &[Argument],
    environment: &HashMap<String, RuntimeValue>,
) -> Option<RuntimeValue> {
    let mut positional = Vec::new();
    let mut named = HashMap::new();
    for argument in arguments {
        let value = constant_value(&argument.value, environment)?;
        if let Some(name) = &argument.name {
            named.insert(name.clone(), value);
        } else {
            positional.push(value);
        }
    }
    timestamp::evaluate(&positional, &named).map(RuntimeValue::Int)
}

fn constant_color(
    expression: &Expression,
    environment: &HashMap<String, RuntimeValue>,
) -> Option<u32> {
    match expression {
        Expression::Literal(super::ast::Literal::Color(rgba), _) => Some(*rgba),
        Expression::Identifier(_, _) => match constant_value(expression, environment)? {
            RuntimeValue::Color(rgba) => Some(rgba),
            _ => None,
        },
        Expression::Call {
            name, arguments, ..
        } if name == "color.new" => {
            if arguments.len() < 2 {
                return None;
            }
            let base = constant_color(&arguments[0].value, environment)?;
            let transparency = constant_number(&arguments[1].value, environment)?;
            Some(builtins::with_transparency(base, transparency))
        }
        Expression::Call {
            name, arguments, ..
        } if name == "color.rgb" => {
            let numbers: Vec<f64> = arguments
                .iter()
                .filter_map(|a| constant_number(&a.value, environment))
                .collect();
            if numbers.len() != arguments.len() || numbers.len() < 3 {
                return None;
            }
            Some(builtins::rgb(
                numbers[0],
                numbers[1],
                numbers[2],
                numbers.get(3).copied().unwrap_or(0.0),
            ))
        }
        _ => None,
    }
}

fn constant_number(
    expression: &Expression,
    environment: &HashMap<String, RuntimeValue>,
) -> Option<f64> {
    match expression {
        Expression::Literal(value, _) => match value {
            super::ast::Literal::Int(n) => Some(*n as f64),
            super::ast::Literal::Float(n) => Some(*n),
            _ => None,
        },
        Expression::Identifier(_, _) => constant_value(expression, environment)?.number(),
        Expression::Unary {
            op: super::ast::UnaryOperator::Negate,
            operand,
            ..
        } => constant_number(operand, environment).map(|n| -n),
        _ => None,
    }
}

// MARK: Inputs

fn collect_inputs(
    statements: &[Statement],
    environment: &HashMap<String, RuntimeValue>,
    schema: &mut InputSchema,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let enums = enum_declarations(statements);
    for statement in statements {
        if let Statement::Declaration { name, value, .. } = statement {
            if let Expression::Call {
                name: fn_name,
                arguments,
                range,
                ..
            } = value
            {
                if fn_name.starts_with("input.") || fn_name == "input" {
                    if fn_name == "input.enum" {
                        if let Some(input) = enum_input(name, arguments, &enums, environment) {
                            schema.inputs.push(input);
                        } else {
                            diagnostics.push(Diagnostic::error(
                                "PINE3010",
                                DiagnosticCategory::Semantic,
                                "input.enum requires a member of a declared enum as its default value.",
                                *range,
                            ));
                        }
                        continue;
                    }
                    let Some(default) = arguments
                        .iter()
                        .find(|a| a.name.is_none() || a.name.as_deref() == Some("defval"))
                    else {
                        diagnostics.push(Diagnostic::error(
                            "PINE3010",
                            DiagnosticCategory::Semantic,
                            format!("{fn_name} requires a constant default value."),
                            *range,
                        ));
                        continue;
                    };
                    let Some(default_value) = input_value(&default.value, fn_name, environment)
                    else {
                        diagnostics.push(Diagnostic::error(
                            "PINE3010",
                            DiagnosticCategory::Semantic,
                            format!("{fn_name} requires a constant default value."),
                            *range,
                        ));
                        continue;
                    };
                    schema.inputs.push(InputDefinition {
                        id: name.clone(),
                        kind: builtins::input_type(fn_name),
                        default_value,
                        title: input_title(arguments, environment),
                        tooltip: input_string(arguments, "tooltip", environment),
                        group: input_string(arguments, "group", environment),
                        inline: input_string(arguments, "inline", environment),
                        confirm: input_bool(arguments, "confirm"),
                        min_value: input_number(arguments, "minval", environment),
                        max_value: input_number(arguments, "maxval", environment),
                        step: input_number(arguments, "step", environment),
                        options: input_options(arguments, fn_name, environment),
                        option_titles: None,
                    });
                }
            }
        }
        if let Statement::Conditional {
            when_true,
            when_false,
            ..
        } = statement
        {
            collect_inputs(when_true, environment, schema, diagnostics);
            collect_inputs(when_false, environment, schema, diagnostics);
        }
    }
}

fn enum_declarations(statements: &[Statement]) -> HashMap<String, Vec<EnumMember>> {
    let mut enums = HashMap::new();
    for statement in statements {
        if let Statement::EnumDeclaration { name, members, .. } = statement {
            enums.insert(name.clone(), members.clone());
        }
    }
    enums
}

fn enum_input(
    variable: &str,
    args: &[Argument],
    enums: &HashMap<String, Vec<EnumMember>>,
    environment: &HashMap<String, RuntimeValue>,
) -> Option<InputDefinition> {
    let first = args
        .iter()
        .find(|a| a.name.is_none() || a.name.as_deref() == Some("defval"))?;
    let (initial_name, initial_member) = enum_member(&first.value, enums)?;
    let mut choices: Vec<(String, EnumMember)> = Vec::new();
    if let Some(listed) = args.iter().find(|a| a.name.as_deref() == Some("options")) {
        if let Expression::Tuple(values, _) = &listed.value {
            choices = values
                .iter()
                .filter_map(|v| enum_member(v, enums))
                .collect();
        }
    } else if let Some((prefix, _)) = initial_name.split_once('.') {
        if let Some(all) = enums.get(prefix) {
            choices = all
                .iter()
                .map(|m| (format!("{prefix}.{}", m.name), m.clone()))
                .collect();
        }
    }
    Some(InputDefinition {
        id: variable.to_owned(),
        kind: ValueType::String,
        default_value: InputValue::String(initial_name.clone()),
        title: input_title(args, environment),
        tooltip: input_string(args, "tooltip", environment),
        group: input_string(args, "group", environment),
        inline: input_string(args, "inline", environment),
        confirm: input_bool(args, "confirm"),
        min_value: None,
        max_value: None,
        step: None,
        options: Some(
            choices
                .iter()
                .map(|(n, _)| InputValue::String(n.clone()))
                .collect(),
        ),
        option_titles: Some(choices.iter().map(|(_, m)| m.title.clone()).collect()),
    })
}

fn enum_member(
    expression: &Expression,
    enums: &HashMap<String, Vec<EnumMember>>,
) -> Option<(String, EnumMember)> {
    let Expression::Identifier(full, _) = expression else {
        return None;
    };
    let (prefix, member_name) = full.split_once('.')?;
    let member = enums
        .get(prefix)?
        .iter()
        .find(|m| m.name == member_name)?
        .clone();
    Some((full.clone(), member))
}

fn input_string(
    args: &[Argument],
    key: &str,
    environment: &HashMap<String, RuntimeValue>,
) -> Option<String> {
    let argument = args.iter().find(|a| a.name.as_deref() == Some(key))?;
    match constant_value(&argument.value, environment)? {
        RuntimeValue::String(value) => Some(value),
        _ => None,
    }
}

fn input_number(
    args: &[Argument],
    key: &str,
    environment: &HashMap<String, RuntimeValue>,
) -> Option<f64> {
    let argument = args.iter().find(|a| a.name.as_deref() == Some(key))?;
    constant_value(&argument.value, environment)?.number()
}

fn input_bool(args: &[Argument], key: &str) -> bool {
    matches!(
        args.iter()
            .find(|a| a.name.as_deref() == Some(key))
            .map(|a| &a.value),
        Some(Expression::Literal(super::ast::Literal::Bool(true), _))
    )
}

fn input_title(args: &[Argument], environment: &HashMap<String, RuntimeValue>) -> Option<String> {
    if let Some(title) = input_string(args, "title", environment) {
        return Some(title);
    }
    let second = args.get(1).filter(|a| a.name.is_none())?;
    match constant_value(&second.value, environment)? {
        RuntimeValue::String(value) => Some(value),
        _ => None,
    }
}

fn input_options(
    args: &[Argument],
    function: &str,
    environment: &HashMap<String, RuntimeValue>,
) -> Option<Vec<InputValue>> {
    let positional: Vec<&Argument> = args.iter().filter(|a| a.name.is_none()).collect();
    let argument = args
        .iter()
        .find(|a| a.name.as_deref() == Some("options"))
        .or_else(|| positional.get(2).copied())?;
    let Expression::Tuple(values, _) = &argument.value else {
        return None;
    };
    let options: Vec<InputValue> = values
        .iter()
        .filter_map(|v| input_value(v, function, environment))
        .collect();
    (options.len() == values.len() && !options.is_empty()).then_some(options)
}

fn input_value(
    expression: &Expression,
    function: &str,
    environment: &HashMap<String, RuntimeValue>,
) -> Option<InputValue> {
    if let Expression::Identifier(name, _) = expression {
        if function == "input.source"
            || (function == "input"
                && super::builtin_types::identifier(name).kind() == Some(ValueType::Float))
        {
            return Some(InputValue::Source(name.clone()));
        }
    }
    match constant_value(expression, environment)? {
        RuntimeValue::Int(value) => Some(InputValue::Int(value)),
        RuntimeValue::Float(value) => Some(InputValue::Float(value)),
        RuntimeValue::Bool(value) => Some(InputValue::Bool(value)),
        RuntimeValue::String(value) => Some(InputValue::String(value)),
        RuntimeValue::Color(value) => Some(InputValue::Color(value)),
        _ => None,
    }
}

// MARK: Libraries

struct ImportDeclaration {
    path: String,
    alias: String,
    range: SourceRange,
}

struct ScanResult {
    declarations: Vec<ImportDeclaration>,
    diagnostics: Vec<Diagnostic>,
}

fn scan_imports(source: &str) -> ScanResult {
    let mut declarations = Vec::new();
    let mut diagnostics = Vec::new();
    let mut offset = 0usize;
    for (index, line) in source.split('\n').enumerate() {
        let line_len = line.encode_utf16().count();
        let range = SourceRange::new(
            crate::pine::SourcePosition::new(index + 1, 1, offset),
            crate::pine::SourcePosition::new(index + 1, line_len + 1, offset + line_len),
        );
        offset += line_len + 1;
        if !line.starts_with("import ") && !line.starts_with("import\t") {
            continue;
        }
        let Some((path, alias)) = parse_import_line(line) else {
            diagnostics.push(Diagnostic::error(
                "PINE3042",
                DiagnosticCategory::Semantic,
                "An import reads: import user/Library/version [as alias].",
                range,
            ));
            continue;
        };
        let parts: Vec<&str> = path.split('/').collect();
        if parts.len() != 3
            || parts.iter().any(|p| p.is_empty())
            || !parts[2].chars().all(|c| c.is_ascii_digit())
        {
            diagnostics.push(Diagnostic::error(
                "PINE3042",
                DiagnosticCategory::Semantic,
                format!(
                    "'{path}' is not a library path: expected user/Library/version, e.g. me/Math/1."
                ),
                range,
            ));
            continue;
        }
        let alias = alias.unwrap_or_else(|| parts[1].to_owned());
        if declarations.iter().any(|d| d.alias.as_str() == alias.as_str())
            || is_namespace(&alias)
            || RESERVED_WORDS.contains(&alias.as_str())
        {
            diagnostics.push(Diagnostic::error(
                "PINE3045",
                DiagnosticCategory::Semantic,
                format!("'{alias}' is already used or is a Pine namespace and cannot be an import alias."),
                range,
            ));
        } else {
            declarations.push(ImportDeclaration { path, alias, range });
        }
    }
    ScanResult {
        declarations,
        diagnostics,
    }
}

fn parse_import_line(line: &str) -> Option<(String, Option<String>)> {
    let rest = line
        .strip_prefix("import ")
        .or_else(|| line.strip_prefix("import\t"))?;
    let rest = rest.split("//").next().unwrap_or(rest).trim_end();
    let mut parts = rest.split_whitespace();
    let path = parts.next()?.to_owned();
    let alias = match parts.next() {
        None => None,
        Some("as") => Some(parts.next()?.to_owned()),
        Some(_) => return None,
    };
    if parts.next().is_some() {
        return None;
    }
    Some((path, alias))
}

const MAXIMUM_DEPTH: usize = 8;
const RESERVED_WORDS: [&str; 6] = ["import", "export", "method", "type", "enum", "as"];

fn is_namespace(alias: &str) -> bool {
    matches!(
        alias,
        "ta" | "math"
            | "str"
            | "array"
            | "matrix"
            | "map"
            | "color"
            | "line"
            | "label"
            | "box"
            | "table"
            | "strategy"
            | "syminfo"
            | "timeframe"
            | "session"
            | "chart"
            | "barstate"
            | "request"
            | "input"
            | "plot"
            | "hline"
            | "barcolor"
            | "bgcolor"
            | "fill"
            | "display"
            | "dayofweek"
            | "format"
            | "order"
            | "xloc"
            | "yloc"
            | "barmerge"
            | "scale"
            | "adjustment"
            | "backadjustment"
            | "settlement_as_close"
            | "font"
            | "text"
            | "currency"
    )
}

fn link_imports(
    declarations: &[ImportDeclaration],
    resolver: &dyn LibraryResolver,
    limits: Limits,
    stack: &[String],
) -> LinkResult {
    let mut imports = Vec::new();
    let mut diagnostics = Vec::new();
    for declaration in declarations {
        let fail = |code: &str,
                    message: String,
                    imports: &mut Vec<Import>,
                    diagnostics: &mut Vec<Diagnostic>| {
            diagnostics.push(Diagnostic::error(
                code,
                DiagnosticCategory::Semantic,
                message,
                declaration.range,
            ));
            imports.push(Import {
                path: declaration.path.clone(),
                alias: declaration.alias.clone(),
                range: declaration.range,
                library: None,
            });
        };
        if stack.contains(&declaration.path) {
            fail(
                "PINE3041",
                format!(
                    "Library '{}' imports itself, directly or through another library.",
                    declaration.path
                ),
                &mut imports,
                &mut diagnostics,
            );
            continue;
        }
        if stack.len() >= MAXIMUM_DEPTH {
            fail(
                "PINE3047",
                format!("Libraries import each other more than {MAXIMUM_DEPTH} levels deep."),
                &mut imports,
                &mut diagnostics,
            );
            continue;
        }
        let Some(source) = resolver.source_for_library(&declaration.path) else {
            fail(
                "PINE3040",
                format!("Library '{}' was not found.", declaration.path),
                &mut imports,
                &mut diagnostics,
            );
            continue;
        };
        let mut stack = stack.to_vec();
        stack.push(declaration.path.clone());
        let program = compile_internal(&source, limits, resolver, &stack);
        if program.declaration.script_type != ScriptType::Library {
            fail(
                "PINE3043",
                format!(
                    "'{}' is not a library: it does not declare library().",
                    declaration.path
                ),
                &mut imports,
                &mut diagnostics,
            );
            continue;
        }
        if let Some(problem) = program
            .diagnostics
            .iter()
            .find(|d| d.severity == crate::pine::DiagnosticSeverity::Error)
        {
            if problem.code == "PINE3041" || problem.code == "PINE3047" {
                fail(
                    &problem.code,
                    problem.message.clone(),
                    &mut imports,
                    &mut diagnostics,
                );
            } else {
                fail(
                    "PINE3043",
                    format!(
                        "Library '{}' has errors (line {}: {}).",
                        declaration.path, problem.range.start.line, problem.message
                    ),
                    &mut imports,
                    &mut diagnostics,
                );
            }
            continue;
        }
        imports.push(Import {
            path: declaration.path.clone(),
            alias: declaration.alias.clone(),
            range: declaration.range,
            library: Some(Box::new(program)),
        });
    }
    LinkResult {
        imports,
        diagnostics,
    }
}

struct LinkResult {
    imports: Vec<Import>,
    diagnostics: Vec<Diagnostic>,
}

fn validate_usage(statements: &[Statement], imports: &[Import]) -> Vec<Diagnostic> {
    let libraries: HashMap<&str, &CompiledProgram> = imports
        .iter()
        .filter_map(|entry| {
            entry
                .library
                .as_deref()
                .map(|library| (entry.alias.as_str(), library))
        })
        .collect();
    if libraries.is_empty() {
        return Vec::new();
    }
    let mut diagnostics = Vec::new();
    super::traversal::for_each_call(statements, &mut |name, range| {
        let Some((alias, rest)) = name.split_once('.') else {
            return;
        };
        let Some(library) = libraries.get(alias) else {
            return;
        };
        let member = rest.split('.').next().unwrap_or(rest);
        if !library.exported_names.contains(member) {
            diagnostics.push(Diagnostic::error(
                "PINE3044",
                DiagnosticCategory::Semantic,
                format!("'{alias}' does not export '{member}'."),
                range,
            ));
        }
    });
    diagnostics
}

// MARK: Named constants and structure (reduced until the symbol catalog + validator land)

fn is_named_constant(name: &str) -> bool {
    if builtins::color(name).is_some() || builtins::constant(name).is_some() {
        return true;
    }
    let sizes = [
        Size::Auto,
        Size::Tiny,
        Size::Small,
        Size::Normal,
        Size::Large,
        Size::Huge,
    ];
    if sizes.iter().any(|s| s.pine_name() == name) {
        return true;
    }
    let line_styles = [
        LineStyle::Solid,
        LineStyle::Dotted,
        LineStyle::Dashed,
        LineStyle::ArrowLeft,
        LineStyle::ArrowRight,
        LineStyle::ArrowBoth,
    ];
    if line_styles.iter().any(|s| s.pine_name() == name) {
        return true;
    }
    let extends = [
        LineExtend::None,
        LineExtend::Left,
        LineExtend::Right,
        LineExtend::Both,
    ];
    if extends.iter().any(|s| s.pine_name() == name) {
        return true;
    }
    let locations = [
        MarkerLocation::AboveBar,
        MarkerLocation::BelowBar,
        MarkerLocation::Top,
        MarkerLocation::Bottom,
        MarkerLocation::Absolute,
    ];
    if locations.iter().any(|s| s.pine_name() == name) {
        return true;
    }
    let positions = [
        TablePosition::TopLeft,
        TablePosition::TopCenter,
        TablePosition::TopRight,
        TablePosition::MiddleLeft,
        TablePosition::MiddleCenter,
        TablePosition::MiddleRight,
        TablePosition::BottomLeft,
        TablePosition::BottomCenter,
        TablePosition::BottomRight,
    ];
    if positions.iter().any(|s| s.pine_name() == name) {
        return true;
    }
    let shapes = [
        MarkerShape::Xcross,
        MarkerShape::Cross,
        MarkerShape::Circle,
        MarkerShape::TriangleUp,
        MarkerShape::TriangleDown,
        MarkerShape::Flag,
        MarkerShape::ArrowUp,
        MarkerShape::ArrowDown,
        MarkerShape::LabelUp,
        MarkerShape::LabelDown,
        MarkerShape::Square,
        MarkerShape::Diamond,
    ];
    if shapes.iter().any(|s| s.pine_name() == name) {
        return true;
    }
    AlertFrequency::PINE_NAMES.contains(&name)
}

/// Structure validation (PINE3020–3024, PINE3048, PINE9003) is deferred until `PineSymbolCatalog`
/// lands; it composes the runtime's dispatch tables. For now it reports nothing.
fn structure_validate(
    _statements: &[Statement],
    _methods: &HashSet<String>,
    _source: &str,
) -> Vec<Diagnostic> {
    Vec::new()
}
