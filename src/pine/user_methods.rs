//! User-defined methods (`method area(Rect this) => …`): `rect.area()` and `area(rect)` run the
//! overload whose first parameter accepts the receiver's type. Builtin namespaces keep priority only
//! when no overload matches.

use super::ast::{Argument, Expression, Parameter};
use super::runtime_types::{RuntimeContext, RuntimeFunction};
use super::runtime_value::{RefKind, RuntimeValue};
use super::session::{INTERNAL_PREFIX, Session};
use super::token::{Diagnostic, SourceRange, ValueType};

impl Session {
    /// Runs the user method `name` on `receiver`; None when no overload accepts the receiver.
    pub(crate) fn call_user_method(
        &mut self,
        name: &str,
        receiver: RuntimeValue,
        arguments: &[Argument],
        site: usize,
        range: SourceRange,
        context: &mut RuntimeContext,
    ) -> Result<Option<RuntimeValue>, Diagnostic> {
        let Some(function) = self.method_overload(name, &receiver) else {
            return Ok(None);
        };
        // The receiver is already evaluated, so it travels to the parameter binder as a hidden variable.
        let hidden = format!("{INTERNAL_PREFIX}receiver");
        self.working.variables.insert(hidden.clone(), receiver);
        let mut all = vec![Argument {
            name: None,
            value: Expression::Identifier(hidden.clone(), range),
            name_range: None,
        }];
        all.extend_from_slice(arguments);
        let result = self.invoke(function, name, &all, site, range, context);
        self.working.variables.remove(&hidden);
        result.map(Some)
    }

    fn method_overload(&self, name: &str, receiver: &RuntimeValue) -> Option<RuntimeFunction> {
        self.methods
            .get(name)?
            .iter()
            .find(|function| {
                function
                    .parameters
                    .first()
                    .is_some_and(|parameter| self.accepts(parameter, receiver))
            })
            .cloned()
    }

    /// Whether a `this` parameter of this declared type takes `value`. An untyped parameter takes
    /// anything, and `na` fits every type.
    fn accepts(&self, parameter: &Parameter, value: &RuntimeValue) -> bool {
        if matches!(value, RuntimeValue::Na) {
            return true;
        }
        if let Some(declared) = &parameter.type_name {
            let RuntimeValue::Ref(RefKind::Object, id) = value else {
                return false;
            };
            return self
                .working
                .instances
                .get(id)
                .is_some_and(|object| same_type(declared, &object.type_name));
        }
        let Some(kind) = parameter.kind else {
            return true;
        };
        match (kind, value) {
            (ValueType::Int | ValueType::Time, RuntimeValue::Int(_))
            | (ValueType::Float, RuntimeValue::Int(_) | RuntimeValue::Float(_))
            | (ValueType::Bool, RuntimeValue::Bool(_))
            | (ValueType::String, RuntimeValue::String(_))
            | (ValueType::Color, RuntimeValue::Color(_)) => true,
            (kind, RuntimeValue::Ref(handle, _)) => matches!(
                (kind, handle),
                (ValueType::Line, RefKind::Line)
                    | (ValueType::Label, RefKind::Label)
                    | (ValueType::Box, RefKind::Box)
                    | (ValueType::Table, RefKind::Table)
                    | (ValueType::Array, RefKind::Array)
                    | (ValueType::Map, RefKind::Map)
                    | (ValueType::Matrix, RefKind::Matrix)
                    | (ValueType::Object, RefKind::Object)
            ),
            _ => false,
        }
    }
}

/// A library type is registered as `path::Name`; an annotation may name it with or without the path.
fn same_type(declared: &str, actual: &str) -> bool {
    let short = |name: &str| name.rsplit(['.', ':']).next().unwrap_or(name).to_owned();
    declared == actual || short(declared) == short(actual)
}
