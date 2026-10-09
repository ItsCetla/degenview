//! Per-bar runtime scaffolding: bar-state flags, flow control, the evaluation context and a
//! user-defined function. Ported from `PineRuntimeContext.swift` + `PineRuntimeFunction.swift`.

use std::collections::HashSet;

use super::ast::{DeclarationMode, Expression, Parameter, Statement};
use super::containers::Object;
use super::runtime_value::{RefKind, RuntimeValue};
use super::traversal::nested_blocks;
use crate::model::Kline;
use crate::pine::ValueType;

/// How a block of statements ended.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Flow {
    Normal,
    Break,
    Continue,
}

/// `barstate.*` for one bar.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct BarFlags {
    pub is_first: bool,
    pub is_last: bool,
    pub is_history: bool,
    pub is_realtime: bool,
    pub is_new: bool,
    pub is_confirmed: bool,
    pub is_last_confirmed_history: bool,
}

impl BarFlags {
    /// Every `barstate.*` name `value` resolves.
    pub fn value(&self, name: &str) -> Option<bool> {
        match name {
            "barstate.isfirst" => Some(self.is_first),
            "barstate.islast" => Some(self.is_last),
            "barstate.ishistory" => Some(self.is_history),
            "barstate.isrealtime" => Some(self.is_realtime),
            "barstate.isnew" => Some(self.is_new),
            "barstate.isconfirmed" => Some(self.is_confirmed),
            "barstate.islastconfirmedhistory" => Some(self.is_last_confirmed_history),
            _ => None,
        }
    }
}

/// Per-bar, per-call evaluation context.
#[derive(Clone, Debug)]
pub struct RuntimeContext {
    pub bar: Kline,
    pub flags: BarFlags,
    /// Names declared `varip` in this bar's run, which persist across realtime ticks.
    pub intrabar_names: HashSet<String>,
    /// Non-zero inside a user function: the caller's call-site key, so builtins called in the
    /// body keep separate histories per call site.
    pub site_prefix: usize,
    pub depth: usize,
    /// The library whose code is running (`user/name/version`); None for the script itself.
    pub scope: Option<String>,
    /// Inside a library function: the names that are variables there.
    pub locals: Option<HashSet<String>>,
}

impl RuntimeContext {
    pub fn new(bar: Kline, flags: BarFlags) -> Self {
        Self {
            bar,
            flags,
            intrabar_names: HashSet::new(),
            site_prefix: 0,
            depth: 0,
            scope: None,
            locals: None,
        }
    }

    pub fn record(&mut self, mode: DeclarationMode, name: &str) {
        if mode == DeclarationMode::Intrabar {
            self.intrabar_names.insert(name.to_owned());
        } else {
            self.intrabar_names.remove(name);
        }
    }
}

/// A user-defined function, with the name sets needed to give each call a clean scope.
#[derive(Clone, Debug, PartialEq)]
pub struct RuntimeFunction {
    pub parameters: Vec<Parameter>,
    pub body: Vec<Statement>,
    /// Parameters and every name declared in the body; restored after each call.
    pub locals: HashSet<String>,
    /// `var`/`varip` locals, which persist per call site between calls.
    pub persistent_locals: HashSet<String>,
    /// The library that defines the function; its body runs in that library's scope.
    pub scope: Option<String>,
    /// Whether the importing script may call it (a library's helpers are private).
    pub is_exported: bool,
}

impl RuntimeFunction {
    pub fn new(parameters: Vec<Parameter>, body: Vec<Statement>) -> Self {
        let mut locals: HashSet<String> = parameters.iter().map(|p| p.name.clone()).collect();
        let mut persistent = HashSet::new();
        collect_declarations(&body, &mut locals, &mut persistent);
        Self {
            parameters,
            body,
            locals,
            persistent_locals: persistent,
            scope: None,
            is_exported: true,
        }
    }

    /// Whether `value` is something this function's first parameter accepts, which is how a
    /// method is picked by its receiver. An untyped parameter accepts anything.
    pub fn accepts_receiver(
        &self,
        value: &RuntimeValue,
        instances: &std::collections::HashMap<usize, Object>,
    ) -> bool {
        let Some(first) = self.parameters.first() else {
            return false;
        };
        if let Some(name) = &first.type_name {
            return match value {
                RuntimeValue::Ref(RefKind::Object, id) => instances
                    .get(id)
                    .is_some_and(|object| object.type_name == *name),
                RuntimeValue::String(text) => text.starts_with(&format!("{name}.")),
                _ => false,
            };
        }
        let Some(kind) = first.kind else {
            return true;
        };
        matches!(
            (kind, value),
            (ValueType::Int, RuntimeValue::Int(_))
                | (ValueType::Float, RuntimeValue::Int(_))
                | (ValueType::Float, RuntimeValue::Float(_))
                | (ValueType::Bool, RuntimeValue::Bool(_))
                | (ValueType::String, RuntimeValue::String(_))
                | (ValueType::Color, RuntimeValue::Color(_))
                | (ValueType::Time, RuntimeValue::Int(_))
                | (ValueType::Array, RuntimeValue::Ref(RefKind::Array, _))
                | (ValueType::Map, RuntimeValue::Ref(RefKind::Map, _))
                | (ValueType::Matrix, RuntimeValue::Ref(RefKind::Matrix, _))
                | (ValueType::Line, RuntimeValue::Ref(RefKind::Line, _))
                | (ValueType::Label, RuntimeValue::Ref(RefKind::Label, _))
                | (ValueType::Box, RuntimeValue::Ref(RefKind::Box, _))
                | (ValueType::Table, RuntimeValue::Ref(RefKind::Table, _))
                | (
                    ValueType::Object,
                    RuntimeValue::Ref(RefKind::Object | RefKind::Linefill | RefKind::Polyline, _)
                )
        )
    }
}

fn collect_declarations(
    statements: &[Statement],
    names: &mut HashSet<String>,
    persistent: &mut HashSet<String>,
) {
    for statement in statements {
        match statement {
            Statement::Declaration {
                name, mode, value, ..
            } => {
                names.insert(name.clone());
                if *mode != DeclarationMode::Ordinary {
                    persistent.insert(name.clone());
                }
                if let Expression::Statement(block, _) = value {
                    collect_declarations(std::slice::from_ref(block.as_ref()), names, persistent);
                }
            }
            Statement::ForRange { variable, .. } => {
                names.insert(variable.clone());
            }
            Statement::ForIn { index, value, .. } => {
                if let Some(index) = index {
                    names.insert(index.clone());
                }
                names.insert(value.clone());
            }
            Statement::TupleDeclaration {
                names: tuple_names, ..
            } => {
                names.extend(tuple_names.iter().cloned());
            }
            _ => {}
        }
        for block in nested_blocks(statement) {
            collect_declarations(block, names, persistent);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bar_flags_resolve_barstate_names() {
        let flags = BarFlags {
            is_first: true,
            is_confirmed: true,
            ..BarFlags::default()
        };
        assert_eq!(flags.value("barstate.isfirst"), Some(true));
        assert_eq!(flags.value("barstate.islast"), Some(false));
        assert_eq!(flags.value("barstate.nope"), None);
    }
}
