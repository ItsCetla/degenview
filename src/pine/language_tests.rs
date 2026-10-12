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
