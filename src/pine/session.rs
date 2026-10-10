//! Interprets a compiled Pine program bar by bar. Ported from `PineRuntimeSession.swift` +
//! `+Expressions`/`+Statements`/`+Calls`/`+Market`. Builtin handlers live in their own modules
//! (`scalar_builtins`, `array_builtins`, `strategy_builtins`, `ta`, `math`, `strings`, …).

use std::collections::{HashMap, HashSet};

use super::ast::{
    AssignmentOperator, BinaryOperator, DeclarationMode, Expression, Literal, Statement, SwitchArm,
    UnaryOperator,
};
use super::broker::Broker;
use super::calendar;
use super::operators;
use super::program::{
    BarEvent, BarPhase, CompiledProgram, InputValue, ScriptType, SymbolInfo, VisualOutput,
};
use super::runtime_support::{
    ExecutionRejection, LibraryLinkage, RuntimeResult, SecurityDataProvider,
};
use super::runtime_types::{BarFlags, Flow, RuntimeContext, RuntimeFunction};
use super::runtime_value::{RefKind, RuntimeValue};
use super::state::{AlertEvent, RuntimeState, SecuritySite};
use super::traversal::nested_blocks;
use super::{
    alert_builtins, array_builtins, drawings, map_builtins, market_values, math, matrix_builtins,
    number_format, strategy_builtins, strings, ta, visuals,
};
use crate::model::Kline;
use crate::pine::{Diagnostic, DiagnosticCategory, Limits, SourceRange};

const INTERNAL_PREFIX: &str = "\u{0}";
const ALERT_LIMIT: usize = 200;
const DEFAULT_MINTICK: f64 = 0.01;

pub struct Session {
    pub program: CompiledProgram,
    pub inputs: HashMap<String, InputValue>,
    pub limits: Limits,
    pub symbol: SymbolInfo,
    pub functions: HashMap<String, RuntimeFunction>,
    pub methods: HashMap<String, Vec<RuntimeFunction>>,
    pub types: HashMap<String, Vec<super::ast::TypeField>>,
    pub enums: HashMap<String, Vec<super::ast::EnumMember>>,
    pub input_variables: HashMap<usize, String>,
    pub linkage: LibraryLinkage,
    pub security_data: Option<Box<dyn SecurityDataProvider>>,
    pub security_series: HashMap<super::runtime_support::SecurityKey, Option<Vec<Kline>>>,
    pub library_constants: HashMap<String, RuntimeValue>,
    pub constants_in_progress: HashSet<String>,
    pub committed: RuntimeState,
    pub working: RuntimeState,
    pub intrabar: HashMap<String, RuntimeValue>,
    pub last_open_time: Option<i64>,
    pub security_globals: Option<HashMap<String, RuntimeValue>>,
    pub last_bar_index: i64,
    pub last_bar_time: Option<i64>,
    pub first_bar_time: Option<i64>,
    pub visible_range: Option<(i64, i64)>,
    pub last_committed_open_time: Option<i64>,
    pub once_per_bar_ledger: HashSet<usize>,
    pub emitted_alerts: Vec<AlertEvent>,
    pub mintick: f64,
    pub bar_seconds: f64,
    /// What each `ta.*` call site remembers between bars, by site key.
    ta_states: HashMap<usize, ta::State>,
    supplied_mintick: Option<f64>,
}

struct Step {
    flow: Flow,
    value: RuntimeValue,
}

impl Step {
    fn normal(value: RuntimeValue) -> Self {
        Self {
            flow: Flow::Normal,
            value,
        }
    }
}

#[derive(Debug)]
pub enum RunError {
    Rejection(ExecutionRejection),
    Script(Diagnostic),
}

impl From<Diagnostic> for RunError {
    fn from(diagnostic: Diagnostic) -> Self {
        Self::Script(diagnostic)
    }
}

impl Session {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        program: CompiledProgram,
        inputs: HashMap<String, InputValue>,
        limits: Limits,
        mintick: Option<f64>,
        symbol: SymbolInfo,
        security_data: Option<Box<dyn SecurityDataProvider>>,
        visible_range: Option<(i64, i64)>,
    ) -> Self {
        let supplied_mintick = mintick.filter(|m| *m > 0.0 && m.is_finite());
        let mut functions = HashMap::new();
        let mut methods: HashMap<String, Vec<RuntimeFunction>> = HashMap::new();
        let mut input_variables = HashMap::new();
        let mut types = HashMap::new();
        let mut enums = HashMap::new();
        let linkage = LibraryLinkage::new(&program);
        for statement in &program.statements {
            match statement {
                Statement::TypeDeclaration { name, fields, .. } => {
                    types.insert(name.clone(), fields.clone());
                }
                Statement::EnumDeclaration { name, members, .. } => {
                    enums.insert(name.clone(), members.clone());
                }
                Statement::Function {
                    name,
                    parameters,
                    body,
                    ..
                } => {
                    let function = RuntimeFunction::new(
                        parameters
                            .iter()
                            .map(|p| linkage.canonical(p, "", &HashSet::new()))
                            .collect(),
                        body.clone(),
                    );
                    functions.insert(name.clone(), function.clone());
                    if program.method_names.contains(name) {
                        methods.entry(name.clone()).or_default().push(function);
                    }
                }
                Statement::Declaration {
                    name,
                    value: Expression::Call { site, .. },
                    ..
                } => {
                    input_variables.insert(*site, name.clone());
                }
                _ => {}
            }
        }
        let mut inputs = inputs;
        for input in &program.input_schema.inputs {
            inputs
                .entry(input.id.clone())
                .or_insert_with(|| input.default_value.clone());
        }
        let mut session = Self {
            program,
            inputs,
            limits,
            symbol,
            functions: functions
                .into_iter()
                .chain(linkage.functions.clone())
                .collect(),
            methods: {
                let mut merged = methods;
                for (name, funcs) in &linkage.methods {
                    merged
                        .entry(name.clone())
                        .or_default()
                        .extend(funcs.clone());
                }
                merged
            },
            types: {
                let mut merged = types;
                for (name, fields) in &linkage.types {
                    merged.entry(name.clone()).or_insert_with(|| fields.clone());
                }
                merged
            },
            enums: {
                let mut merged = enums;
                for (name, members) in &linkage.enums {
                    merged
                        .entry(name.clone())
                        .or_insert_with(|| members.clone());
                }
                merged
            },
            input_variables,
            linkage,
            security_data,
            security_series: HashMap::new(),
            library_constants: HashMap::new(),
            constants_in_progress: HashSet::new(),
            committed: RuntimeState::default(),
            working: RuntimeState::default(),
            intrabar: HashMap::new(),
            last_open_time: None,
            security_globals: None,
            last_bar_index: -1,
            last_bar_time: None,
            first_bar_time: None,
            visible_range,
            last_committed_open_time: None,
            once_per_bar_ledger: HashSet::new(),
            emitted_alerts: Vec::new(),
            mintick: supplied_mintick.unwrap_or(DEFAULT_MINTICK),
            bar_seconds: 0.0,
            ta_states: HashMap::new(),
            supplied_mintick,
        };
        session.committed = session.fresh_state();
        session.working = session.committed.clone();
        session
    }

    fn fresh_state(&self) -> RuntimeState {
        RuntimeState {
            broker: Broker::new(
                self.program
                    .declaration
                    .strategy
                    .clone()
                    .unwrap_or_default(),
            ),
            ..RuntimeState::default()
        }
    }

    pub fn reset(&mut self, inputs: Option<HashMap<String, InputValue>>) {
        if let Some(inputs) = inputs {
            self.inputs = inputs;
        }
        self.committed = self.fresh_state();
        self.working = self.committed.clone();
        self.intrabar.clear();
        self.last_open_time = None;
        self.last_committed_open_time = None;
        self.once_per_bar_ledger.clear();
        self.emitted_alerts.clear();
        self.bar_seconds = 0.0;
        self.last_bar_index = -1;
        self.last_bar_time = None;
        self.first_bar_time = None;
        self.mintick = self.supplied_mintick.unwrap_or(DEFAULT_MINTICK);
    }

    // MARK: Running

    pub fn evaluate(&mut self, bars: &[Kline]) -> Result<RuntimeResult, RunError> {
        let states = self.load_history(bars, false, None)?;
        Ok(RuntimeResult {
            output: self.output(),
            diagnostics: Vec::new(),
            bar_states: states,
        })
    }

    pub fn load_history(
        &mut self,
        bars: &[Kline],
        precedes_live_bar: bool,
        seed: Option<f64>,
    ) -> Result<Vec<BarFlags>, RunError> {
        self.reset(None);
        self.infer_mintick(bars);
        if let Some(seed) = seed.filter(|s| *s > 0.0) {
            self.bar_seconds = seed;
        }
        if bars.len() > 1 {
            self.bar_seconds =
                (bars[bars.len() - 1].open_time - bars[bars.len() - 2].open_time) as f64 / 1000.0;
        }
        self.last_bar_index = bars.len() as i64 - 1 + (precedes_live_bar as i64);
        self.last_bar_time = bars.last().map(|bar| {
            bar.open_time
                + (if precedes_live_bar {
                    (self.bar_seconds * 1000.0) as i64
                } else {
                    0
                })
        });
        self.first_bar_time = bars.first().map(|bar| bar.open_time);
        let mut states = Vec::with_capacity(bars.len());
        for (index, bar) in bars.iter().enumerate() {
            let is_last_history = index == bars.len() - 1;
            let flags = self.execute(
                BarEvent {
                    candle: bar.clone(),
                    phase: BarPhase::Historical,
                },
                is_last_history && !precedes_live_bar,
                Some(is_last_history),
            )?;
            states.push(flags);
        }
        Ok(states)
    }

    pub fn inferred_mintick(bars: &[Kline]) -> f64 {
        let mut decimals = 0i32;
        for bar in bars.iter().rev().take(200) {
            for price in [bar.open, bar.high, bar.low, bar.close] {
                if !price.is_finite() {
                    continue;
                }
                while decimals < 8 {
                    let scaled = price * 10f64.powi(decimals);
                    if (scaled - scaled.round()).abs() <= (1e-6f64).max(scaled.abs() * 1e-12) {
                        break;
                    }
                    decimals += 1;
                }
            }
        }
        10f64.powi(-decimals)
    }

    /// Recompute `mintick` from the bars when the caller did not supply one. `load_history` runs
    /// this before its execute loop; the host's `run_script`/`run_script_alerts` call it directly
    /// because they drive `execute` themselves.
    pub fn infer_mintick(&mut self, bars: &[Kline]) {
        if self.supplied_mintick.is_none() {
            self.mintick = Self::inferred_mintick(bars);
        }
    }

    #[allow(clippy::result_large_err)]
    pub fn execute(
        &mut self,
        event: BarEvent,
        is_last: bool,
        is_last_history: Option<bool>,
    ) -> Result<BarFlags, RunError> {
        let open_time = event.candle.open_time;
        if let Some(committed_time) = self.last_committed_open_time
            && open_time <= committed_time
        {
            return Err(RunError::Rejection(if open_time < committed_time {
                ExecutionRejection::StaleBar
            } else {
                ExecutionRejection::DuplicateBar
            }));
        }
        if let Some(previous) = self.last_open_time
            && open_time < previous
        {
            return Err(RunError::Rejection(ExecutionRejection::StaleBar));
        }
        let is_new = Some(open_time) != self.last_open_time;
        if is_new {
            self.intrabar.clear();
            self.once_per_bar_ledger.clear();
        }
        self.emitted_alerts.clear();
        if is_new && let Some(previous) = self.last_open_time {
            self.bar_seconds = (open_time - previous) as f64 / 1000.0;
        }
        let confirmed = event.phase.is_confirmed();
        let realtime = event.phase.is_realtime();
        self.working = self.committed.clone();
        self.working.bar_index = self.committed.bar_index + 1;
        if self.working.bar_index >= self.last_bar_index {
            self.last_bar_index = self.working.bar_index;
            self.last_bar_time = Some(open_time);
        }
        self.working.instructions = 0;
        if !is_new {
            for (key, value) in self.intrabar.clone() {
                self.working.variables.insert(key, value);
            }
        }
        let flags = BarFlags {
            is_first: self.working.bar_index == 0,
            is_last,
            is_history: !realtime,
            is_realtime: realtime,
            is_new,
            is_confirmed: confirmed,
            is_last_confirmed_history: !realtime && (is_last_history.unwrap_or(is_last)),
        };
        let candle = event.candle;
        let mut context = RuntimeContext::new(candle.clone(), flags);
        if self.is_strategy() {
            self.working.broker.process(
                candle.clone(),
                self.working.bar_index as usize,
                self.mintick,
            );
        }
        self.run(&self.program.statements.clone(), &mut context)
            .map_err(RunError::Script)?;
        if self.is_strategy() {
            self.finish_strategy_bar(candle.clone());
        }
        if confirmed {
            self.commit_histories(candle, true);
            self.committed = self.working.clone();
            self.intrabar.clear();
            self.last_committed_open_time = Some(open_time);
        }
        self.last_open_time = Some(open_time);
        Ok(flags)
    }

    fn is_strategy(&self) -> bool {
        self.program.declaration.script_type == ScriptType::Strategy
    }

    fn finish_strategy_bar(&mut self, candle: Kline) {
        let close = candle.close;
        if self
            .program
            .declaration
            .strategy
            .as_ref()
            .is_some_and(|s| s.process_orders_on_close)
        {
            self.working.broker.fill_market_orders_at_close(
                candle,
                self.working.bar_index as usize,
                self.mintick,
            );
        }
        self.working.broker.record_equity(close);
    }

    pub fn emit(&mut self, alert: AlertEvent) {
        self.emitted_alerts.push(alert);
    }

    pub fn output(&self) -> VisualOutput {
        let w = &self.working;
        let linefills: Vec<_> = w
            .linefills
            .values()
            .filter(|lf| w.lines.contains_key(&lf.line1) && w.lines.contains_key(&lf.line2))
            .cloned()
            .collect();
        VisualOutput {
            overlay: self.program.declaration.overlay,
            bar_count: (w.bar_index + 1) as usize,
            plots: sorted_by_id(&w.plots),
            hlines: sorted_by_id(&w.hlines),
            markers: sorted_by_id(&w.markers),
            backgrounds: sorted_by_id(&w.backgrounds),
            bar_colors: sorted_by_id(&w.bar_colors),
            fills: sorted_by_id(&w.fills),
            lines: sorted_by_id(&w.lines),
            labels: sorted_by_id(&w.labels),
            boxes: sorted_by_id(&w.boxes),
            linefills: {
                let mut v: Vec<_> = linefills.into_iter().collect();
                v.sort_by_key(|lf| lf.id);
                v
            },
            polylines: sorted_by_id(&w.polylines),
            tables: sorted_by_id(&w.tables),
            candles: sorted_by_id(&w.candles),
            alerts: w.alerts.clone(),
            strategy: if self.is_strategy() {
                Some(w.broker.report())
            } else {
                None
            },
            behind_chart: self.program.declaration.overlay
                && self.program.declaration.behind_chart.unwrap_or(true),
        }
    }

    // MARK: Expressions

    pub(crate) fn eval(
        &mut self,
        expression: &Expression,
        context: &mut RuntimeContext,
    ) -> Result<RuntimeValue, Diagnostic> {
        self.budget()?;
        match expression {
            Expression::Literal(value, _) => Ok(match value {
                Literal::Int(n) => RuntimeValue::Int(*n),
                Literal::Float(n) => RuntimeValue::Float(*n),
                Literal::Bool(b) => RuntimeValue::Bool(*b),
                Literal::String(s) => RuntimeValue::String(s.clone()),
                Literal::Color(c) => RuntimeValue::Color(*c),
                Literal::Na => RuntimeValue::Na,
            }),
            Expression::Identifier(name, range) => self.resolve_identifier(name, *range, context),
            Expression::Unary { op, operand, range } => {
                self.eval_unary(*op, operand, *range, context)
            }
            Expression::Binary {
                lhs,
                op,
                rhs,
                range,
            } => self.eval_binary(lhs, *op, rhs, *range, context),
            Expression::Ternary {
                condition,
                when_true,
                when_false,
                range,
            } => {
                let test = Self::truth(self.eval(condition, context)?).ok_or_else(|| {
                    Diagnostic::error(
                        "PINE4005",
                        DiagnosticCategory::Runtime,
                        "Ternary condition must be bool.",
                        *range,
                    )
                })?;
                self.eval(if test { when_true } else { when_false }, context)
            }
            Expression::History {
                base,
                offset,
                range,
            } => self.eval_history(base, offset, *range, context),
            Expression::Tuple(expressions, _) => {
                let mut values = Vec::with_capacity(expressions.len());
                for e in expressions {
                    values.push(self.eval(e, context)?);
                }
                Ok(RuntimeValue::Tuple(values))
            }
            Expression::MethodCall {
                receiver,
                name,
                arguments,
                site,
                range,
            } => self.method_call_on_value(receiver, name, arguments, *site, *range, context),
            Expression::Member { base, name, range } => {
                let value = self.eval(base, context)?;
                self.read_field(name, value, *range)
            }
            Expression::Statement(statement, _) => {
                let (_, value) = self.run(std::slice::from_ref(statement.as_ref()), context)?;
                Ok(if value == RuntimeValue::Void {
                    RuntimeValue::Na
                } else {
                    value
                })
            }
            Expression::Call {
                name,
                arguments,
                site,
                range,
            } => self.call(name, arguments, *site, *range, context),
        }
    }

    fn resolve_identifier(
        &mut self,
        name: &str,
        range: SourceRange,
        context: &mut RuntimeContext,
    ) -> Result<RuntimeValue, Diagnostic> {
        if let Some(value) = self.working.variables.get(name) {
            return Ok(value.clone());
        }
        if let Some(value) = self.security_globals.as_ref().and_then(|g| g.get(name)) {
            return Ok(value.clone());
        }
        if let Some(value) = self.market(name, context) {
            return Ok(value);
        }
        if let Some(flag) = context.flags.value(name) {
            return Ok(RuntimeValue::Bool(flag));
        }
        if let Some(color) = super::builtins::color(name) {
            return Ok(RuntimeValue::Color(color));
        }
        if let Some(constant) = super::builtins::constant(name) {
            return Ok(constant);
        }
        // A library constant or enum member (via an import alias).
        if let Some(value) = self.library_identifier(name, range, context)? {
            return Ok(value);
        }
        // A dotted name that matches nothing is a named constant (`size.small`) and stands for itself.
        if name.contains('.') && super::compiler::is_named_constant(name) {
            return Ok(RuntimeValue::String(name.to_owned()));
        }
        if let Some((enum_name, member)) = name.split_once('.')
            && let Some(members) = self.enums.get(enum_name)
        {
            if !members.iter().any(|m| m.name == member) {
                return Err(Diagnostic::error(
                    "PINE4025",
                    DiagnosticCategory::Runtime,
                    format!("Enum '{enum_name}' has no member '{member}'."),
                    range,
                ));
            }
            return Ok(RuntimeValue::String(name.to_owned()));
        }
        // Field paths and library constants are stubbed for now; treat as a named constant.
        if name.contains('.') {
            return Ok(RuntimeValue::String(name.to_owned()));
        }
        Err(Diagnostic::error(
            "PINE4008",
            DiagnosticCategory::Runtime,
            format!("Undefined variable '{name}'."),
            range,
        ))
    }

    fn market(&self, name: &str, context: &RuntimeContext) -> Option<RuntimeValue> {
        let bar = &context.bar;
        match name {
            "open" => Some(RuntimeValue::Float(bar.open)),
            "high" => Some(RuntimeValue::Float(bar.high)),
            "low" => Some(RuntimeValue::Float(bar.low)),
            "close" => Some(RuntimeValue::Float(bar.close)),
            "volume" => Some(RuntimeValue::Float(bar.volume)),
            "hl2" => Some(RuntimeValue::Float((bar.high + bar.low) / 2.0)),
            "hlc3" => Some(RuntimeValue::Float((bar.high + bar.low + bar.close) / 3.0)),
            "ohlc4" => Some(RuntimeValue::Float(
                (bar.open + bar.high + bar.low + bar.close) / 4.0,
            )),
            "hlcc4" => Some(RuntimeValue::Float(
                (bar.high + bar.low + 2.0 * bar.close) / 4.0,
            )),
            "time" => Some(RuntimeValue::Int(bar.open_time)),
            "time_close" => Some(RuntimeValue::Int(
                bar.open_time + (self.bar_seconds * 1000.0) as i64,
            )),
            "bar_index" => Some(RuntimeValue::Int(self.working.bar_index)),
            "last_bar_index" => Some(RuntimeValue::Int(self.last_bar_index)),
            "last_bar_time" => self
                .last_bar_time
                .map(RuntimeValue::Int)
                .or(Some(RuntimeValue::Na)),
            "year" | "month" | "dayofmonth" | "hour" | "minute" | "second" | "dayofweek" => {
                let c = calendar::components(bar.open_time);
                Some(RuntimeValue::Int(match name {
                    "year" => c.year,
                    "month" => c.month,
                    "dayofmonth" => c.day,
                    "hour" => c.hour,
                    "minute" => c.minute,
                    "second" => c.second,
                    "dayofweek" => c.weekday,
                    _ => unreachable!(),
                }))
            }
            _ => {
                if let Some(value) = market_values::read(
                    name,
                    &market_values::MarketContext {
                        state: &self.working,
                        bar: &context.bar,
                        symbol: &self.symbol,
                        bar_seconds: self.bar_seconds,
                        visible_range: self.visible_range,
                        first_bar_time: self.first_bar_time,
                        last_bar_time: self.last_bar_time,
                    },
                ) {
                    return Some(value);
                }
                self.symbol_value(name)
            }
        }
    }

    fn symbol_value(&self, name: &str) -> Option<RuntimeValue> {
        match name {
            "syminfo.mintick" => Some(RuntimeValue::Float(self.mintick)),
            "syminfo.ticker" => Some(RuntimeValue::String(self.symbol.ticker.clone())),
            "syminfo.tickerid" => Some(RuntimeValue::String(self.symbol.ticker_id.clone())),
            "syminfo.currency" => Some(RuntimeValue::String(self.symbol.currency.clone())),
            "syminfo.root" => Some(RuntimeValue::String(self.symbol.ticker.clone())),
            "syminfo.pointvalue" => Some(RuntimeValue::Float(1.0)),
            "syminfo.type" => Some(RuntimeValue::String(self.symbol.kind.clone())),
            "syminfo.basecurrency" => self
                .symbol
                .base_currency
                .clone()
                .map(RuntimeValue::String)
                .or(Some(RuntimeValue::Na)),
            _ => None,
        }
    }

    fn eval_unary(
        &mut self,
        op: UnaryOperator,
        operand: &Expression,
        range: SourceRange,
        context: &mut RuntimeContext,
    ) -> Result<RuntimeValue, Diagnostic> {
        let value = self.eval(operand, context)?;
        match op {
            UnaryOperator::Negate => Ok(operators::negate(&value)),
            UnaryOperator::Plus => Ok(value),
            UnaryOperator::Not => {
                let b = Self::truth(value).ok_or_else(|| {
                    Diagnostic::error(
                        "PINE4002",
                        DiagnosticCategory::Runtime,
                        "not requires bool.",
                        range,
                    )
                })?;
                Ok(RuntimeValue::Bool(!b))
            }
        }
    }

    fn eval_binary(
        &mut self,
        left: &Expression,
        op: BinaryOperator,
        right: &Expression,
        range: SourceRange,
        context: &mut RuntimeContext,
    ) -> Result<RuntimeValue, Diagnostic> {
        let lhs = self.eval(left, context)?;
        match op {
            BinaryOperator::And => self.eval_logical(true, lhs, right, range, context),
            BinaryOperator::Or => self.eval_logical(false, lhs, right, range, context),
            _ => Ok(operators::apply(op, &lhs, &self.eval(right, context)?)),
        }
    }

    fn eval_logical(
        &mut self,
        is_and: bool,
        lhs: RuntimeValue,
        right: &Expression,
        range: SourceRange,
        context: &mut RuntimeContext,
    ) -> Result<RuntimeValue, Diagnostic> {
        let word = if is_and { "and" } else { "or" };
        let code = if is_and { "PINE4003" } else { "PINE4004" };
        let error = |r: SourceRange| {
            Diagnostic::error(
                code,
                DiagnosticCategory::Runtime,
                format!("{word} requires bool operands."),
                r,
            )
        };
        let left = Self::truth(lhs).ok_or_else(|| error(range))?;
        if left != is_and {
            return Ok(RuntimeValue::Bool(left));
        }
        let rhs = Self::truth(self.eval(right, context)?).ok_or_else(|| error(range))?;
        Ok(RuntimeValue::Bool(rhs))
    }

    fn eval_history(
        &mut self,
        base: &Expression,
        offset: &Expression,
        range: SourceRange,
        context: &mut RuntimeContext,
    ) -> Result<RuntimeValue, Diagnostic> {
        let n = self.eval(offset, context)?.number().ok_or_else(|| {
            Diagnostic::error(
                "PINE4006",
                DiagnosticCategory::Runtime,
                "History offset must be a number.",
                range,
            )
        })?;
        if !n.is_finite() {
            return Err(Diagnostic::error(
                "PINE4006",
                DiagnosticCategory::Runtime,
                "History offset must be a number.",
                range,
            ));
        }
        let Some(i) = RuntimeValue::integral(n).int_value() else {
            if n < 0.0 {
                return Err(Diagnostic::error(
                    "PINE4006",
                    DiagnosticCategory::Runtime,
                    "History offset cannot be negative.",
                    range,
                ));
            }
            return Ok(RuntimeValue::Na);
        };
        if i < 0 {
            return Err(Diagnostic::error(
                "PINE4006",
                DiagnosticCategory::Runtime,
                "History offset cannot be negative.",
                range,
            ));
        }
        if i == 0 {
            return self.eval(base, context);
        }
        let key = match base {
            Expression::Identifier(name, _) => name.clone(),
            _ => {
                let key = format!(
                    "{INTERNAL_PREFIX}expr:{}",
                    self.site_key(range.start.offset, context)
                );
                let value = self.eval(base, context)?;
                self.working.expression_values.insert(key.clone(), value);
                key
            }
        };
        let history = self
            .working
            .histories
            .get(&key)
            .cloned()
            .unwrap_or_default();
        let i = i as usize;
        Ok(if i <= history.len() {
            history[history.len() - i].clone()
        } else {
            RuntimeValue::Na
        })
    }

    // MARK: Statements

    fn run(
        &mut self,
        statements: &[Statement],
        context: &mut RuntimeContext,
    ) -> Result<(Flow, RuntimeValue), Diagnostic> {
        let mut last = RuntimeValue::Void;
        for statement in statements {
            self.budget()?;
            let step = self.run_statement(statement, context)?;
            if step.value != RuntimeValue::Void {
                last = step.value;
            }
            if step.flow != Flow::Normal {
                return Ok((step.flow, last));
            }
        }
        Ok((Flow::Normal, last))
    }

    fn run_statement(
        &mut self,
        statement: &Statement,
        context: &mut RuntimeContext,
    ) -> Result<Step, Diagnostic> {
        match statement {
            Statement::Declaration {
                name, mode, value, ..
            } => self.run_declaration(name, *mode, value, context),
            Statement::Assignment {
                name, op, value, ..
            } => self.run_assignment(name, *op, value, context),
            Statement::Expression(expression) => Ok(Step::normal(self.eval(expression, context)?)),
            Statement::Conditional {
                condition,
                when_true,
                when_false,
                ..
            } => self.run_conditional(condition, when_true, when_false, context),
            Statement::ForRange {
                variable,
                from,
                to,
                step,
                body,
                range,
            } => self.run_for_range(variable, from, to, step.as_ref(), body, *range, context),
            Statement::ForIn {
                index,
                value,
                collection,
                body,
                range,
            } => self.run_for_in(index.as_deref(), value, collection, body, *range, context),
            Statement::WhileLoop {
                condition, body, ..
            } => self.run_while(condition, body, context),
            Statement::Switch {
                subject,
                arms,
                range,
            } => self.run_switch(subject.as_ref(), arms, *range, context),
            Statement::TupleDeclaration { names, value, .. } => {
                self.run_tuple_declaration(names, value, context)
            }
            Statement::LoopControl(control, _) => Ok(Step {
                flow: if *control == super::ast::LoopControl::Break {
                    Flow::Break
                } else {
                    Flow::Continue
                },
                value: RuntimeValue::Void,
            }),
            Statement::FieldAssignment {
                target,
                op,
                value,
                range,
            } => self.run_field_assignment(target, *op, value, *range, context),
            Statement::Function { .. }
            | Statement::TypeDeclaration { .. }
            | Statement::EnumDeclaration { .. } => Ok(Step {
                flow: Flow::Normal,
                value: RuntimeValue::Void,
            }),
        }
    }

    fn run_declaration(
        &mut self,
        name: &str,
        mode: DeclarationMode,
        expression: &Expression,
        context: &mut RuntimeContext,
    ) -> Result<Step, Diagnostic> {
        if mode == DeclarationMode::Variable
            && let Some(existing) = self.working.variables.get(name)
        {
            return Ok(Step::normal(existing.clone()));
        }
        if mode == DeclarationMode::Intrabar
            && let Some(value) = self
                .intrabar
                .get(name)
                .or_else(|| self.working.variables.get(name))
        {
            let value = value.clone();
            self.working
                .variables
                .insert(name.to_owned(), value.clone());
            self.intrabar.insert(name.to_owned(), value.clone());
            context.record(mode, name);
            return Ok(Step::normal(value));
        }
        let value = self.eval(expression, context)?;
        self.working
            .variables
            .insert(name.to_owned(), value.clone());
        context.record(mode, name);
        if mode == DeclarationMode::Intrabar {
            self.intrabar.insert(name.to_owned(), value.clone());
        }
        Ok(Step::normal(value))
    }

    fn run_assignment(
        &mut self,
        name: &str,
        op: AssignmentOperator,
        expression: &Expression,
        context: &mut RuntimeContext,
    ) -> Result<Step, Diagnostic> {
        let rhs = self.eval(expression, context)?;
        let old = self
            .working
            .variables
            .get(name)
            .cloned()
            .unwrap_or(RuntimeValue::Na);
        let value = operators::apply_assignment(op, &old, &rhs);
        self.working
            .variables
            .insert(name.to_owned(), value.clone());
        if context.intrabar_names.contains(name) || self.intrabar.contains_key(name) {
            self.intrabar.insert(name.to_owned(), value.clone());
        }
        Ok(Step::normal(value))
    }

    fn run_tuple_declaration(
        &mut self,
        names: &[String],
        expression: &Expression,
        context: &mut RuntimeContext,
    ) -> Result<Step, Diagnostic> {
        let value = self.eval(expression, context)?;
        let items = match &value {
            RuntimeValue::Tuple(items) => items.clone(),
            _ => Vec::new(),
        };
        for (i, name) in names.iter().enumerate() {
            if name != "_" {
                self.working.variables.insert(
                    name.clone(),
                    items.get(i).cloned().unwrap_or(RuntimeValue::Na),
                );
            }
        }
        Ok(Step::normal(value))
    }

    fn run_conditional(
        &mut self,
        condition: &Expression,
        when_true: &[Statement],
        when_false: &[Statement],
        context: &mut RuntimeContext,
    ) -> Result<Step, Diagnostic> {
        let value = self.eval(condition, context)?;
        let test = self.require_bool(
            value,
            "if condition must be bool; numeric-to-bool coercion is not allowed in v6.",
            condition.range(),
        )?;
        let (flow, value) = self.run(if test { when_true } else { when_false }, context)?;
        Ok(Step { flow, value })
    }

    #[allow(clippy::too_many_arguments)]
    fn run_for_range(
        &mut self,
        variable: &str,
        from: &Expression,
        to: &Expression,
        step_expression: Option<&Expression>,
        body: &[Statement],
        range: SourceRange,
        context: &mut RuntimeContext,
    ) -> Result<Step, Diagnostic> {
        let start_value = self.eval(from, context)?;
        let Some(start) = start_value.number() else {
            return Ok(Step {
                flow: Flow::Normal,
                value: RuntimeValue::Void,
            });
        };
        let Some(stop) = self.eval(to, context)?.number() else {
            return Ok(Step {
                flow: Flow::Normal,
                value: RuntimeValue::Void,
            });
        };
        let mut stride = 1.0;
        let mut integral = start_value.is_int();
        if let Some(step_expression) = step_expression {
            let step_value = self.eval(step_expression, context)?;
            let Some(size) = step_value.number() else {
                return Err(Diagnostic::error(
                    "PINE4014",
                    DiagnosticCategory::Runtime,
                    "for loop step must be a non-zero number.",
                    range,
                ));
            };
            if size == 0.0 || !size.is_finite() {
                return Err(Diagnostic::error(
                    "PINE4014",
                    DiagnosticCategory::Runtime,
                    "for loop step must be a non-zero number.",
                    range,
                ));
            }
            stride = size.abs();
            if !step_value.is_int() {
                integral = false;
            }
        }
        let direction = if stop >= start { 1.0 } else { -1.0 };
        let mut last = RuntimeValue::Void;
        let mut i = start;
        while if direction > 0.0 {
            i <= stop
        } else {
            i >= stop
        } {
            self.budget()?;
            self.working.variables.insert(
                variable.to_owned(),
                if integral {
                    RuntimeValue::integral(i)
                } else {
                    RuntimeValue::Float(i)
                },
            );
            let (flow, value) = self.run(body, context)?;
            last = value;
            if flow == Flow::Break {
                break;
            }
            i += direction * stride;
        }
        Ok(Step {
            flow: Flow::Normal,
            value: last,
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn run_for_in(
        &mut self,
        index_name: Option<&str>,
        value_name: &str,
        collection: &Expression,
        body: &[Statement],
        range: SourceRange,
        context: &mut RuntimeContext,
    ) -> Result<Step, Diagnostic> {
        let target = self.eval(collection, context)?;
        if target == RuntimeValue::Na {
            return Ok(Step {
                flow: Flow::Normal,
                value: RuntimeValue::Void,
            });
        }
        let RuntimeValue::Ref(RefKind::Array, id) = target else {
            return Err(Diagnostic::error(
                "PINE4011",
                DiagnosticCategory::Runtime,
                "for...in requires an array or a map.",
                range,
            ));
        };
        let items = self.working.arrays.get(&id).cloned().unwrap_or_default();
        let mut last = RuntimeValue::Void;
        for (offset, item) in items.into_iter().enumerate() {
            self.budget()?;
            if let Some(index_name) = index_name {
                self.working
                    .variables
                    .insert(index_name.to_owned(), RuntimeValue::Int(offset as i64));
            }
            self.working.variables.insert(value_name.to_owned(), item);
            let (flow, value) = self.run(body, context)?;
            last = value;
            if flow == Flow::Break {
                break;
            }
        }
        Ok(Step {
            flow: Flow::Normal,
            value: last,
        })
    }

    fn run_while(
        &mut self,
        condition: &Expression,
        body: &[Statement],
        context: &mut RuntimeContext,
    ) -> Result<Step, Diagnostic> {
        let mut last = RuntimeValue::Void;
        loop {
            self.budget()?;
            let value = self.eval(condition, context)?;
            let test =
                self.require_bool(value, "while condition must be bool.", condition.range())?;
            if !test {
                break;
            }
            let (flow, value) = self.run(body, context)?;
            last = value;
            if flow == Flow::Break {
                break;
            }
        }
        Ok(Step {
            flow: Flow::Normal,
            value: last,
        })
    }

    fn run_switch(
        &mut self,
        subject: Option<&Expression>,
        arms: &[SwitchArm],
        range: SourceRange,
        context: &mut RuntimeContext,
    ) -> Result<Step, Diagnostic> {
        let target = match subject {
            Some(subject) => Some(self.eval(subject, context)?),
            None => None,
        };
        let Some(chosen) = self.choose_arm(arms, target.as_ref(), range, context)? else {
            return Ok(Step {
                flow: Flow::Normal,
                value: RuntimeValue::Na,
            });
        };
        let (flow, value) = self.run(&arms[chosen].body, context)?;
        Ok(Step { flow, value })
    }

    fn choose_arm(
        &mut self,
        arms: &[SwitchArm],
        target: Option<&RuntimeValue>,
        range: SourceRange,
        context: &mut RuntimeContext,
    ) -> Result<Option<usize>, Diagnostic> {
        let mut fallback = None;
        for (index, arm) in arms.iter().enumerate() {
            let Some(condition) = &arm.condition else {
                fallback = fallback.or(Some(index));
                continue;
            };
            let value = self.eval(condition, context)?;
            let matches = if let Some(target) = target {
                operators::apply(BinaryOperator::Equal, target, &value) == RuntimeValue::Bool(true)
            } else {
                self.require_bool(value, "switch arm condition must be bool.", range)?
            };
            if matches {
                return Ok(Some(index));
            }
        }
        Ok(fallback)
    }

    fn require_bool(
        &self,
        value: RuntimeValue,
        message: &str,
        range: SourceRange,
    ) -> Result<bool, Diagnostic> {
        Self::truth(value).ok_or_else(|| {
            Diagnostic::error("PINE4001", DiagnosticCategory::Runtime, message, range)
        })
    }

    pub fn truth(value: RuntimeValue) -> Option<bool> {
        match value {
            RuntimeValue::Bool(b) => Some(b),
            RuntimeValue::Na => Some(false),
            _ => None,
        }
    }

    // MARK: Calls

    pub(crate) fn call(
        &mut self,
        name: &str,
        arguments: &[super::ast::Argument],
        site: usize,
        range: SourceRange,
        context: &mut RuntimeContext,
    ) -> Result<RuntimeValue, Diagnostic> {
        // A script-defined (or library) type constructor: `Type.new(...)` / `alias.Type.new(...)`.
        if let Some(object) = self.constructor_call(name, arguments, range, context)? {
            return Ok(object);
        }
        // User function (own, or a library's via an import alias).
        if let Some(function) = self.scoped_function(name, range, context)?
            && self.methods.get(name).is_none_or(|m| m.len() <= 1)
        {
            return self.invoke(function, name, arguments, site, range, context);
        }
        // The declaration and input builtins are no-ops here (their values are read at compile time).
        match name {
            "indicator" | "strategy" | "library" => return Ok(RuntimeValue::Na),
            _ if name.starts_with("input.") || name == "input" => {
                return self.input_value(site, range, context);
            }
            _ => {}
        }
        if let Some(value) = self.scalar_call(name, arguments, range, context)? {
            return Ok(value);
        }
        if let Some(value) = self.collection_call(name, arguments, range, context)? {
            return Ok(value);
        }
        if math::SUPPORTED.contains(&name) {
            let values = self.eval_all(arguments, context)?;
            return Ok(math::call(name, &values, self.mintick));
        }
        if name.starts_with("str.") {
            let values = self.eval_all(arguments, context)?;
            return strings::call(name, &values, range, self.mintick);
        }
        if matches!(name, "ta.sar" | "ta.supertrend") {
            let values = self.eval_all(arguments, context)?;
            let key = self.site_key(site, context);
            let state = self.ta_states.entry(key).or_default();
            return Ok(ta::evaluate_price(name, &values, &context.bar, state));
        }
        if name == "ta.cum" {
            let source = self.arg_value(arguments, 0, Some("source"), context)?;
            let key = self.site_key(site, context);
            let state = self.ta_states.entry(key).or_default();
            return Ok(ta::evaluate_cum(&source, state));
        }
        if name == "ta.correlation" {
            let first = self.arg_value(arguments, 0, Some("source1"), context)?;
            let second = self.arg_value(arguments, 1, Some("source2"), context)?;
            let length = self.arg_value(arguments, 2, Some("length"), context)?;
            let key = self.site_key(site, context);
            let state = self.ta_states.entry(key).or_default();
            return Ok(ta::evaluate_correlation(
                &first,
                &second,
                length.int_value().unwrap_or(0),
                state,
            ));
        }
        if name == "ta.atr" {
            let length = self.arg_value(arguments, 0, Some("length"), context)?;
            let key = self.site_key(site, context);
            let state = self.ta_states.entry(key).or_default();
            return Ok(ta::evaluate_atr(
                state,
                &context.bar,
                length.int_value().unwrap_or(14),
            ));
        }
        if name == "ta.bb" {
            let source = self.arg_value(arguments, 0, Some("series"), context)?;
            let length = self.arg_value(arguments, 1, Some("length"), context)?;
            let multiplier = self.arg_value(arguments, 2, Some("mult"), context)?;
            let key = self.site_key(site, context);
            let state = self.ta_states.entry(key).or_default();
            return Ok(ta::evaluate_bb(
                &source,
                length.int_value().unwrap_or(0),
                multiplier.number().unwrap_or(2.0),
                state,
            ));
        }
        if matches!(name, "ta.pivothigh" | "ta.pivotlow") {
            let high = name == "ta.pivothigh";
            // `(source, leftbars, rightbars)`, or `(leftbars, rightbars)` reading the bar's own
            // high or low.
            let positional = arguments.iter().filter(|a| a.name.is_none()).count();
            let (source, left, right) = if positional >= 3
                || arguments
                    .iter()
                    .any(|a| a.name.as_deref() == Some("source"))
            {
                (
                    self.arg_value(arguments, 0, Some("source"), context)?,
                    self.arg_value(arguments, 1, Some("leftbars"), context)?,
                    self.arg_value(arguments, 2, Some("rightbars"), context)?,
                )
            } else {
                let own = if high {
                    context.bar.high
                } else {
                    context.bar.low
                };
                (
                    RuntimeValue::Float(own),
                    self.arg_value(arguments, 0, Some("leftbars"), context)?,
                    self.arg_value(arguments, 1, Some("rightbars"), context)?,
                )
            };
            let key = self.site_key(site, context);
            let state = self.ta_states.entry(key).or_default();
            return Ok(ta::evaluate_pivot(
                &source,
                left.int_value().unwrap_or(0),
                right.int_value().unwrap_or(0),
                high,
                state,
            ));
        }
        if ta::is_supported(name) {
            let source = self.arg_value(arguments, 0, None, context)?;
            let second = self.arg_value(arguments, 1, None, context)?;
            let length = second.int_value().unwrap_or(0);
            let key = self.site_key(site, context);
            let state = self.ta_states.entry(key).or_default();
            return Ok(ta::evaluate(name, &source, &second, length, state));
        }
        if drawings::handles(name) {
            let args = self.eval_args(arguments, context)?;
            return drawings::call(
                name,
                &args,
                &mut self.working,
                &context.bar,
                self.bar_seconds,
                &drawings::DrawingLimits::default(),
                range,
            );
        }
        if visuals::handles(name) {
            let args = self.eval_args(arguments, context)?;
            let key = self.site_key(site, context);
            return visuals::call(name, &args, key, &mut self.working, range);
        }
        if strategy_builtins::handles(name) {
            let args = self.eval_args(arguments, context)?;
            let is_strategy =
                self.program.declaration.script_type == super::program::ScriptType::Strategy;
            return strategy_builtins::call(name, &args, &mut self.working, is_strategy, range);
        }
        if alert_builtins::handles(name) {
            let args = self.eval_args(arguments, context)?;
            let key = self.site_key(site, context);
            if let Some(event) = alert_builtins::call(
                name,
                &args,
                key,
                &mut self.working,
                &context.flags,
                &mut self.once_per_bar_ledger,
                &self.symbol,
                self.mintick,
                self.bar_seconds,
                &context.bar,
                range,
            )? {
                self.emit(event);
            }
            return Ok(RuntimeValue::Void);
        }
        if name == "request.security" {
            return self.security_call(arguments, site, range, context);
        }
        if name == "request.security_lower_tf" {
            return self.security_lower_tf_call(arguments, site, range, context);
        }
        // `levels.push(x)`: the lexer reads `name.member` as one dotted name, so a variable (or
        // parameter) holding an array, map or matrix reaches here as a call to `levels.push`.
        if let Some((receiver, member)) = name.rsplit_once('.')
            && !receiver.contains('.')
        {
            let expression = Expression::Identifier(receiver.to_owned(), range);
            if let Ok(RuntimeValue::Ref(..)) = self.eval(&expression, context) {
                return self.method_call_on_value(
                    &expression,
                    member,
                    arguments,
                    site,
                    range,
                    context,
                );
            }
        }
        // Any name still unhandled is genuinely unknown.
        Err(Diagnostic::error(
            "PINE4007",
            DiagnosticCategory::Runtime,
            format!("Unknown or unsupported function '{name}'."),
            range,
        ))
    }

    /// The value an `input.*` call stands for: the user's override, or the declared default, of
    /// the variable it is assigned to. A call that isn't a plain declaration reads `na`.
    fn input_value(
        &mut self,
        site: usize,
        range: SourceRange,
        context: &mut RuntimeContext,
    ) -> Result<RuntimeValue, Diagnostic> {
        let Some(variable) = self.input_variables.get(&site) else {
            return Ok(RuntimeValue::Na);
        };
        Ok(match self.inputs.get(variable).cloned() {
            Some(InputValue::Int(value)) => RuntimeValue::Int(value),
            Some(InputValue::Float(value)) => RuntimeValue::Float(value),
            Some(InputValue::Bool(value)) => RuntimeValue::Bool(value),
            Some(InputValue::String(value)) => RuntimeValue::String(value),
            Some(InputValue::Color(value)) => RuntimeValue::Color(value),
            Some(InputValue::Source(name)) => self.resolve_identifier(&name, range, context)?,
            None => RuntimeValue::Na,
        })
    }

    fn arg_value(
        &mut self,
        arguments: &[super::ast::Argument],
        index: usize,
        key: Option<&str>,
        context: &mut RuntimeContext,
    ) -> Result<RuntimeValue, Diagnostic> {
        if let Some(key) = key
            && let Some(found) = arguments.iter().find(|a| a.name.as_deref() == Some(key))
        {
            return self.eval(&found.value, context);
        }
        if index < arguments.len() {
            return self.eval(&arguments[index].value, context);
        }
        Ok(RuntimeValue::Na)
    }

    fn eval_all(
        &mut self,
        arguments: &[super::ast::Argument],
        context: &mut RuntimeContext,
    ) -> Result<Vec<RuntimeValue>, Diagnostic> {
        arguments
            .iter()
            .map(|argument| self.eval(&argument.value, context))
            .collect()
    }

    /// Evaluates arguments as source-ordered `drawings::Arg`s (name + value), for the handlers
    /// that bind overloads themselves.
    fn eval_args(
        &mut self,
        arguments: &[super::ast::Argument],
        context: &mut RuntimeContext,
    ) -> Result<Vec<drawings::Arg>, Diagnostic> {
        arguments
            .iter()
            .map(|argument| {
                Ok(drawings::Arg {
                    name: argument.name.clone(),
                    value: self.eval(&argument.value, context)?,
                })
            })
            .collect()
    }

    /// Binds arguments by parameter name list (positional first, then named, then `na`), the
    /// builtin-handler analogue of `bind_parameters`.
    fn bind_named(
        &mut self,
        arguments: &[super::ast::Argument],
        parameters: &[&str],
        context: &mut RuntimeContext,
    ) -> Result<HashMap<String, RuntimeValue>, Diagnostic> {
        let positional: Vec<&super::ast::Argument> = arguments
            .iter()
            .filter(|argument| argument.name.is_none())
            .collect();
        let mut bound = HashMap::new();
        for (index, name) in parameters.iter().enumerate() {
            if let Some(named) = arguments
                .iter()
                .find(|argument| argument.name.as_deref() == Some(*name))
            {
                bound.insert((*name).to_owned(), self.eval(&named.value, context)?);
            } else if index < positional.len() {
                bound.insert(
                    (*name).to_owned(),
                    self.eval(&positional[index].value, context)?,
                );
            } else {
                bound.insert((*name).to_owned(), RuntimeValue::Na);
            }
        }
        Ok(bound)
    }

    /// Runs one `array.*`/`map.*`/`matrix.*` call, or `None` when the name is none of them.
    fn collection_call(
        &mut self,
        name: &str,
        arguments: &[super::ast::Argument],
        range: SourceRange,
        context: &mut RuntimeContext,
    ) -> Result<Option<RuntimeValue>, Diagnostic> {
        let parameters = if array_builtins::handles(name) {
            array_builtins::parameters(name)
        } else if map_builtins::handles(name) {
            map_builtins::parameters(name)
        } else if matrix_builtins::handles(name) {
            matrix_builtins::parameters(name)
        } else {
            return Ok(None);
        };
        let bound = self.bind_named(arguments, parameters, context)?;
        let value = if array_builtins::handles(name) {
            let all = self.eval_all(arguments, context)?;
            array_builtins::call(name, &bound, &all, &mut self.working, self.mintick, range)?
        } else if map_builtins::handles(name) {
            map_builtins::call(name, &bound, &mut self.working, range)?
        } else {
            matrix_builtins::call(name, &bound, &mut self.working, range)?
        };
        Ok(Some(value))
    }

    fn scalar_call(
        &mut self,
        name: &str,
        arguments: &[super::ast::Argument],
        range: SourceRange,
        context: &mut RuntimeContext,
    ) -> Result<Option<RuntimeValue>, Diagnostic> {
        match name {
            "na" => Ok(Some(RuntimeValue::Bool(
                self.arg_value(arguments, 0, None, context)? == RuntimeValue::Na,
            ))),
            "nz" => {
                let v = self.arg_value(arguments, 0, None, context)?;
                if v != RuntimeValue::Na {
                    return Ok(Some(v));
                }
                let replacement = self.arg_value(arguments, 1, None, context)?;
                Ok(Some(if replacement == RuntimeValue::Na {
                    RuntimeValue::Float(0.0)
                } else {
                    replacement
                }))
            }
            "int" => Ok(Some(
                self.arg_value(arguments, 0, None, context)?
                    .int_value()
                    .map(RuntimeValue::Int)
                    .unwrap_or(RuntimeValue::Na),
            )),
            "float" => Ok(Some(
                self.arg_value(arguments, 0, None, context)?
                    .number()
                    .map(RuntimeValue::Float)
                    .unwrap_or(RuntimeValue::Na),
            )),
            "bool" => {
                let v = self.arg_value(arguments, 0, None, context)?;
                Ok(Some(match &v {
                    RuntimeValue::Bool(_) => v,
                    _ => RuntimeValue::Bool(v.number().is_some_and(|n| n != 0.0)),
                }))
            }
            "string" | "color" => {
                let v = self.arg_value(arguments, 0, None, context)?;
                let keeps = matches!(
                    (&v, name),
                    (RuntimeValue::String(_), "string") | (RuntimeValue::Color(_), "color")
                );
                Ok(Some(if keeps { v } else { RuntimeValue::Na }))
            }
            "color.new" => {
                let c = self.arg_value(arguments, 0, None, context)?;
                let transparency = self.arg_value(arguments, 1, None, context)?.number();
                Ok(Some(match (&c, transparency) {
                    (RuntimeValue::Color(c), Some(t)) => {
                        RuntimeValue::Color(super::builtins::with_transparency(*c, t))
                    }
                    _ => RuntimeValue::Na,
                }))
            }
            "color.rgb" => {
                let r = self
                    .arg_value(arguments, 0, None, context)?
                    .number()
                    .unwrap_or(0.0);
                let g = self
                    .arg_value(arguments, 1, None, context)?
                    .number()
                    .unwrap_or(0.0);
                let b = self
                    .arg_value(arguments, 2, None, context)?
                    .number()
                    .unwrap_or(0.0);
                let transparency = self
                    .arg_value(arguments, 3, None, context)?
                    .number()
                    .unwrap_or(0.0);
                Ok(Some(RuntimeValue::Color(super::builtins::rgb(
                    r,
                    g,
                    b,
                    transparency,
                ))))
            }
            "timestamp" => {
                let mut positional = Vec::new();
                let mut named = HashMap::new();
                for argument in arguments {
                    let v = self.eval(&argument.value, context)?;
                    if let Some(label) = &argument.name {
                        named.insert(label.clone(), v);
                    } else {
                        positional.push(v);
                    }
                }
                Ok(Some(
                    super::timestamp::evaluate(&positional, &named)
                        .map(RuntimeValue::Int)
                        .unwrap_or(RuntimeValue::Na),
                ))
            }
            _ if name.starts_with("math.") => {
                let values: Vec<RuntimeValue> = arguments
                    .iter()
                    .map(|a| self.eval(&a.value, context))
                    .collect::<Result<_, _>>()?;
                Ok(Some(super::scalar_builtins::math(
                    name,
                    &values,
                    self.mintick,
                )))
            }
            _ if name.starts_with("str.") && name != "str.tostring" => {
                let values: Vec<RuntimeValue> = arguments
                    .iter()
                    .map(|a| self.eval(&a.value, context))
                    .collect::<Result<_, _>>()?;
                Ok(Some(super::scalar_builtins::string(
                    name,
                    &values,
                    range,
                    self.mintick,
                )?))
            }
            "str.tostring" => {
                let v = self.arg_value(arguments, 0, None, context)?;
                let pattern = self
                    .arg_value(arguments, 1, Some("format"), context)?
                    .text_value()
                    .map(str::to_owned);
                Ok(Some(RuntimeValue::String(super::scalar_builtins::format(
                    &v,
                    pattern.as_deref(),
                    self.mintick,
                ))))
            }
            _ => Ok(None),
        }
    }

    fn method_call_on_value(
        &mut self,
        receiver: &Expression,
        member: &str,
        arguments: &[super::ast::Argument],
        _site: usize,
        range: SourceRange,
        context: &mut RuntimeContext,
    ) -> Result<RuntimeValue, Diagnostic> {
        let value = self.eval(receiver, context)?;
        let prefix = match &value {
            RuntimeValue::Ref(RefKind::Array, _) => "array",
            RuntimeValue::Ref(RefKind::Map, _) => "map",
            RuntimeValue::Ref(RefKind::Matrix, _) => "matrix",
            _ => {
                return Err(Diagnostic::error(
                    "PINE4007",
                    DiagnosticCategory::Runtime,
                    format!("Unknown or unsupported method '{member}'."),
                    range,
                ));
            }
        };
        let full = format!("{prefix}.{member}");
        let parameters = match prefix {
            "array" => array_builtins::parameters(&full),
            "map" => map_builtins::parameters(&full),
            _ => matrix_builtins::parameters(&full),
        };
        let mut bound = HashMap::new();
        bound.insert("id".to_owned(), value.clone());
        let positional: Vec<&super::ast::Argument> = arguments
            .iter()
            .filter(|argument| argument.name.is_none())
            .collect();
        let mut position = 0;
        for parameter in parameters {
            if *parameter == "id" {
                continue;
            }
            if let Some(named) = arguments
                .iter()
                .find(|argument| argument.name.as_deref() == Some(*parameter))
            {
                bound.insert((*parameter).to_owned(), self.eval(&named.value, context)?);
            } else if position < positional.len() {
                bound.insert(
                    (*parameter).to_owned(),
                    self.eval(&positional[position].value, context)?,
                );
                position += 1;
            } else {
                bound.insert((*parameter).to_owned(), RuntimeValue::Na);
            }
        }
        let result = match prefix {
            "array" => {
                array_builtins::call(&full, &bound, &[], &mut self.working, self.mintick, range)?
            }
            "map" => map_builtins::call(&full, &bound, &mut self.working, range)?,
            _ => matrix_builtins::call(&full, &bound, &mut self.working, range)?,
        };
        Ok(result)
    }

    fn read_field(
        &self,
        name: &str,
        base: RuntimeValue,
        range: SourceRange,
    ) -> Result<RuntimeValue, Diagnostic> {
        if let RuntimeValue::Ref(RefKind::Object, id) = base
            && let Some(object) = self.working.instances.get(&id)
            && let Some(value) = object.fields.get(name)
        {
            return Ok(value.clone());
        }
        Err(Diagnostic::error(
            "PINE4010",
            DiagnosticCategory::Runtime,
            format!("Field '{name}' not found on this value."),
            range,
        ))
    }

    fn run_field_assignment(
        &mut self,
        target: &Expression,
        op: AssignmentOperator,
        value: &Expression,
        range: SourceRange,
        context: &mut RuntimeContext,
    ) -> Result<Step, Diagnostic> {
        let Expression::Member { base, name, .. } = target else {
            return Err(Diagnostic::error(
                "PINE4007",
                DiagnosticCategory::Runtime,
                "Assignment to this target is not yet supported.",
                range,
            ));
        };
        let RuntimeValue::Ref(RefKind::Object, id) = self.eval(base, context)? else {
            return Err(Diagnostic::error(
                "PINE4007",
                DiagnosticCategory::Runtime,
                "Field assignment requires an object.",
                range,
            ));
        };
        let rhs = self.eval(value, context)?;
        let assigned = match op {
            AssignmentOperator::Reassign => rhs,
            AssignmentOperator::Compound(_) => {
                let old = self.read_field(name, RuntimeValue::Ref(RefKind::Object, id), range)?;
                operators::apply_assignment(op, &old, &rhs)
            }
        };
        if let Some(object) = self.working.instances.get_mut(&id) {
            object.fields.insert(name.clone(), assigned);
        }
        Ok(Step::normal(RuntimeValue::Void))
    }

    pub(crate) fn site_key(&self, site: usize, context: &RuntimeContext) -> usize {
        if context.site_prefix == 0 {
            site
        } else {
            context
                .site_prefix
                .wrapping_mul(1_000_003)
                .wrapping_add(site)
        }
    }

    fn invoke(
        &mut self,
        function: RuntimeFunction,
        call_name: &str,
        arguments: &[super::ast::Argument],
        site: usize,
        range: SourceRange,
        context: &mut RuntimeContext,
    ) -> Result<RuntimeValue, Diagnostic> {
        if context.depth >= self.limits.call_depth {
            return Err(Diagnostic::error(
                "PINE8005",
                DiagnosticCategory::Resource,
                format!("Call depth limit exceeded in '{call_name}'."),
                range,
            ));
        }
        let bound = self.bind_parameters(&function, arguments, context)?;
        let prefix = self.site_key(site, context);
        let stash_key = |local: &str| format!("{INTERNAL_PREFIX}fn:{prefix}:{local}");

        let saved: Vec<(String, Option<RuntimeValue>)> = function
            .locals
            .iter()
            .map(|local| (local.clone(), self.working.variables.get(local).cloned()))
            .collect();
        for local in &function.locals {
            self.working.variables.remove(local);
        }
        for local in &function.persistent_locals {
            let value = self
                .working
                .variables
                .get(&stash_key(local))
                .cloned()
                .unwrap_or(RuntimeValue::Na);
            self.working.variables.insert(local.clone(), value);
        }
        for (parameter, value) in bound {
            self.working.variables.insert(parameter, value);
        }

        let caller_prefix = context.site_prefix;
        let caller_scope = context.scope.clone();
        let caller_locals = context.locals.clone();
        context.site_prefix = prefix;
        context.depth += 1;
        context.scope = function.scope.clone();
        context.locals = if function.scope.is_none() {
            None
        } else {
            Some(function.locals.clone())
        };
        let result = self.run(&function.body, context);
        context.site_prefix = caller_prefix;
        context.depth -= 1;
        context.scope = caller_scope;
        context.locals = caller_locals;

        let (_, value) = result?;
        for local in &function.persistent_locals {
            let value = self
                .working
                .variables
                .get(local)
                .cloned()
                .unwrap_or(RuntimeValue::Na);
            self.working.variables.insert(stash_key(local), value);
        }
        for (local, previous) in saved {
            if let Some(previous) = previous {
                self.working.variables.insert(local, previous);
            } else {
                self.working.variables.remove(&local);
            }
        }
        Ok(value)
    }

    fn bind_parameters(
        &mut self,
        function: &RuntimeFunction,
        arguments: &[super::ast::Argument],
        context: &mut RuntimeContext,
    ) -> Result<HashMap<String, RuntimeValue>, Diagnostic> {
        let positional: Vec<&super::ast::Argument> =
            arguments.iter().filter(|a| a.name.is_none()).collect();
        let mut bound = HashMap::new();
        for (i, parameter) in function.parameters.iter().enumerate() {
            if let Some(named) = arguments
                .iter()
                .find(|a| a.name.as_deref() == Some(&parameter.name))
            {
                bound.insert(parameter.name.clone(), self.eval(&named.value, context)?);
            } else if i < positional.len() {
                bound.insert(
                    parameter.name.clone(),
                    self.eval(&positional[i].value, context)?,
                );
            } else if let Some(fallback) = &parameter.default_value {
                bound.insert(parameter.name.clone(), self.eval(fallback, context)?);
            } else {
                bound.insert(parameter.name.clone(), RuntimeValue::Na);
            }
        }
        Ok(bound)
    }

    // MARK: Bookkeeping

    pub(crate) fn commit_histories(&mut self, bar: Kline, releasing_committed: bool) {
        let mut series: HashMap<String, RuntimeValue> = HashMap::new();
        series.insert("open".to_owned(), RuntimeValue::Float(bar.open));
        series.insert("high".to_owned(), RuntimeValue::Float(bar.high));
        series.insert("low".to_owned(), RuntimeValue::Float(bar.low));
        series.insert("close".to_owned(), RuntimeValue::Float(bar.close));
        series.insert("volume".to_owned(), RuntimeValue::Float(bar.volume));
        series.insert(
            "hl2".to_owned(),
            RuntimeValue::Float((bar.high + bar.low) / 2.0),
        );
        series.insert(
            "hlc3".to_owned(),
            RuntimeValue::Float((bar.high + bar.low + bar.close) / 3.0),
        );
        series.insert(
            "ohlc4".to_owned(),
            RuntimeValue::Float((bar.open + bar.high + bar.low + bar.close) / 4.0),
        );
        series.insert("time".to_owned(), RuntimeValue::Int(bar.open_time));
        series.insert(
            "time_close".to_owned(),
            RuntimeValue::Int(bar.open_time + (self.bar_seconds * 1000.0) as i64),
        );
        series.insert(
            "bar_index".to_owned(),
            RuntimeValue::Int(self.working.bar_index),
        );

        let mut histories = std::mem::take(&mut self.working.histories);
        if releasing_committed {
            self.committed.histories.clear();
        }
        for (name, value) in series.into_iter().chain(self.working.variables.clone()) {
            if !name.starts_with(INTERNAL_PREFIX) {
                histories.entry(name).or_default().push(value);
            }
        }
        let expression_keys: HashSet<String> = histories
            .keys()
            .filter(|k| k.starts_with(INTERNAL_PREFIX))
            .cloned()
            .chain(self.working.expression_values.keys().cloned())
            .collect();
        for key in expression_keys {
            let value = self
                .working
                .expression_values
                .get(&key)
                .cloned()
                .unwrap_or(RuntimeValue::Na);
            histories.entry(key).or_default().push(value);
        }
        self.working.histories = histories;
        self.working.expression_values.clear();
    }

    fn budget(&mut self) -> Result<(), Diagnostic> {
        self.working.instructions += 1;
        if self.working.instructions > self.limits.instructions_per_bar {
            return Err(Diagnostic::error(
                "PINE8004",
                DiagnosticCategory::Resource,
                "Per-bar instruction limit exceeded.",
                SourceRange::ZERO,
            ));
        }
        Ok(())
    }

    fn allocate(&mut self) -> usize {
        self.working.next_reference += 1;
        self.working.next_reference
    }
}

fn sorted_by_id<T: Clone>(map: &HashMap<usize, T>) -> Vec<T> {
    let mut entries: Vec<_> = map.iter().collect();
    entries.sort_by_key(|(id, _)| **id);
    entries.into_iter().map(|(_, v)| v.clone()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pine::Limits;

    fn bar() -> Kline {
        Kline {
            open_time: 1_000,
            close_time: 2_000,
            open: 100.0,
            high: 110.0,
            low: 90.0,
            close: 105.0,
            volume: 1.0,
        }
    }

    #[test]
    fn evaluates_a_declaration() {
        let program = super::super::compiler::compile(
            "//@version=6\nindicator(\"T\")\nx = 1 + 2\n",
            Limits::default(),
            &super::super::compiler::NoLibraries,
        );
        let mut session = Session::new(
            program,
            HashMap::new(),
            Limits::default(),
            None,
            SymbolInfo::default(),
            None,
            None,
        );
        let result = session.evaluate(std::slice::from_ref(&bar())).unwrap();
        assert_eq!(result.bar_states.len(), 1);
        assert_eq!(
            session.working.variables.get("x"),
            Some(&RuntimeValue::Int(3))
        );
    }

    #[test]
    fn evaluates_control_flow() {
        let program = super::super::compiler::compile(
            "//@version=6\nindicator(\"T\")\nx = 0\nif close > open\n    x := 1\n",
            Limits::default(),
            &super::super::compiler::NoLibraries,
        );
        let mut session = Session::new(
            program,
            HashMap::new(),
            Limits::default(),
            None,
            SymbolInfo::default(),
            None,
            None,
        );
        session.evaluate(std::slice::from_ref(&bar())).unwrap();
        assert_eq!(
            session.working.variables.get("x"),
            Some(&RuntimeValue::Int(1))
        );
    }
}
