//! Small runtime support types: the result, execution rejection, security keys/providers, dataset
//! identity and the flattened library linkage. Ported from `Pine/Runtime/*`.

use std::collections::{HashMap, HashSet};

use super::ast::{EnumMember, Expression, Parameter, Statement, TypeField};
use super::program::{CompiledProgram, Import};
use super::runtime_types::RuntimeFunction;
use crate::model::Kline;
use crate::pine::Diagnostic;

pub struct RuntimeResult {
    pub output: super::program::VisualOutput,
    pub diagnostics: Vec<Diagnostic>,
    pub bar_states: Vec<super::runtime_types::BarFlags>,
}

/// Why `execute` refused a bar. Not a script failure: the feed repeated or reordered itself.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExecutionRejection {
    /// The bar opened before one already executed.
    StaleBar,
    /// The bar is the one most recently committed.
    DuplicateBar,
}

/// A series `request.security` asks for: a symbol as the script wrote it, at a bar length in seconds.
#[derive(Clone, Debug)]
pub struct SecurityKey {
    pub symbol: String,
    pub interval: f64,
    /// `calc_bars_count` of a `request.security_lower_tf` call. 0 is every bar.
    pub recent_bars: usize,
}

impl PartialEq for SecurityKey {
    fn eq(&self, other: &Self) -> bool {
        self.symbol == other.symbol
            && self.interval.to_bits() == other.interval.to_bits()
            && self.recent_bars == other.recent_bars
    }
}

impl Eq for SecurityKey {}

impl std::hash::Hash for SecurityKey {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.symbol.hash(state);
        self.interval.to_bits().hash(state);
        self.recent_bars.hash(state);
    }
}

/// Supplies the candles `request.security` reads from a series the chart does not carry. The engine
/// never fetches anything; whoever runs it decides what exists.
pub trait SecurityDataProvider {
    /// Closed candles of `key`, oldest first, or None when the provider has nothing for it.
    fn candles(&self, key: &SecurityKey) -> Option<Vec<Kline>>;
}

/// A provider that holds no data and remembers what it was asked for.
pub struct SecurityRecorder {
    recorded: std::sync::Mutex<Vec<SecurityKey>>,
}

impl SecurityRecorder {
    pub fn new() -> Self {
        Self {
            recorded: std::sync::Mutex::new(Vec::new()),
        }
    }

    /// The series asked for, in first-asked order, without repeats.
    pub fn keys(&self) -> Vec<SecurityKey> {
        self.recorded.lock().unwrap().clone()
    }
}

impl SecurityDataProvider for SecurityRecorder {
    fn candles(&self, key: &SecurityKey) -> Option<Vec<Kline>> {
        let mut recorded = self.recorded.lock().unwrap();
        if !recorded.contains(key) {
            recorded.push(key.clone());
        }
        Some(Vec::new())
    }
}

/// What a set of committed bars belongs to. Series from one key are never reused for another.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct DatasetKey {
    /// `"<source>:<ticker>"`, matching `syminfo.tickerid`.
    pub symbol_key: String,
    /// The chart timeframe's label, e.g. `"1h"`.
    pub timeframe: String,
}

/// A bar's identity: dataset plus open time. Duplicate transport messages for one bar share it.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct BarId {
    pub dataset: DatasetKey,
    pub open_time: i64,
}

/// What the imported libraries contribute to a run, flattened out of the compiled import graph.
#[derive(Clone, Debug, Default)]
pub struct LibraryLinkage {
    /// Scope (a library path, or `""` for the script) -> import alias -> library path.
    pub aliases: HashMap<String, HashMap<String, String>>,
    /// Library path -> the names it exports.
    pub exports: HashMap<String, HashSet<String>>,
    /// Library path -> its top-level constants (evaluated on first use).
    pub constants: HashMap<String, HashMap<String, Expression>>,
    pub functions: HashMap<String, RuntimeFunction>,
    pub methods: HashMap<String, Vec<RuntimeFunction>>,
    pub types: HashMap<String, Vec<TypeField>>,
    pub enums: HashMap<String, Vec<EnumMember>>,
}

impl LibraryLinkage {
    pub fn new(program: &CompiledProgram) -> Self {
        let mut linkage = Self::default();
        linkage.aliases.insert(
            String::new(),
            program
                .imports
                .iter()
                .map(|entry| (entry.alias.clone(), entry.path.clone()))
                .collect(),
        );
        let mut seen = HashSet::new();
        linkage.visit(&program.imports, &mut seen);
        linkage
    }

    fn visit(&mut self, imports: &[Import], seen: &mut HashSet<String>) {
        for entry in imports {
            let Some(library) = &entry.library else {
                continue;
            };
            if !seen.insert(entry.path.clone()) {
                continue;
            }
            self.aliases.insert(
                entry.path.clone(),
                library
                    .imports
                    .iter()
                    .map(|inner| (inner.alias.clone(), inner.path.clone()))
                    .collect(),
            );
            self.register(library, &entry.path);
            self.visit(&library.imports, seen);
        }
    }

    fn register(&mut self, library: &CompiledProgram, key: &str) {
        self.exports
            .insert(key.to_owned(), library.exported_names.clone());
        let declared_types: HashSet<&str> = library
            .statements
            .iter()
            .filter_map(|statement| match statement {
                Statement::TypeDeclaration { name, .. }
                | Statement::EnumDeclaration { name, .. } => Some(name.as_str()),
                _ => None,
            })
            .collect();
        for statement in &library.statements {
            match statement {
                Statement::TypeDeclaration { name, fields, .. } => {
                    self.types.insert(format!("{key}::{name}"), fields.clone());
                }
                Statement::EnumDeclaration { name, members, .. } => {
                    self.enums.insert(format!("{key}::{name}"), members.clone());
                }
                Statement::Declaration {
                    name,
                    mode: super::ast::DeclarationMode::Ordinary,
                    value,
                    ..
                } => {
                    self.constants
                        .entry(key.to_owned())
                        .or_default()
                        .insert(name.clone(), value.clone());
                }
                Statement::Function {
                    name,
                    parameters,
                    body,
                    ..
                } => {
                    let mut function = RuntimeFunction::new(
                        parameters
                            .iter()
                            .map(|p| self.canonical(p, key, &declared_types))
                            .collect(),
                        body.clone(),
                    );
                    function.scope = Some(key.to_owned());
                    function.is_exported = library.exported_names.contains(name);
                    self.functions
                        .insert(format!("{key}::{name}"), function.clone());
                    if library.method_names.contains(name) {
                        self.methods.entry(name.clone()).or_default().push(function);
                    }
                }
                _ => {}
            }
        }
    }

    /// A parameter whose type name is spelled as the importing code spells it, canonicalized.
    pub fn canonical(
        &self,
        parameter: &Parameter,
        scope: &str,
        declared: &HashSet<&str>,
    ) -> Parameter {
        let mut result = parameter.clone();
        if let Some(name) = &parameter.type_name {
            if let Some((alias, rest)) = name.split_once('.') {
                if let Some(path) = self.aliases.get(scope).and_then(|m| m.get(alias)) {
                    result.type_name = Some(format!("{path}::{rest}"));
                }
            } else if declared.contains(name.as_str()) && !scope.is_empty() {
                result.type_name = Some(format!("{scope}::{name}"));
            }
        }
        result
    }

    /// `alias.rest` as seen from `scope`: the library path and what follows the alias.
    pub fn resolve(&self, name: &str, scope: &str) -> Option<(String, String)> {
        let (alias, rest) = name.split_once('.')?;
        let path = self.aliases.get(scope)?.get(alias)?;
        Some((path.clone(), rest.to_owned()))
    }
}
