//! `map.*`, ported from `PineRuntimeSession+Maps.swift`: `map.new<K, V>()` and the operations a map
//! takes, also as methods (`zones.get(k)`). Pure over the run's `RuntimeState`; the session binds
//! the arguments named by `parameters(name)` and calls `call`.

use super::array_builtins::{Bound, new_array};
use super::containers::{Map, MapKey};
use super::runtime_value::{RefKind, RuntimeValue};
use super::state::RuntimeState;
use super::token::{Diagnostic, DiagnosticCategory, SourceRange};

/// Parameter names per operation (by member name, after `map.`), in positional order.
pub fn parameters(name: &str) -> &'static [&'static str] {
    match name.strip_prefix("map.").unwrap_or(name) {
        "put" => &["id", "key", "value"],
        "get" | "contains" | "remove" => &["id", "key"],
        "put_all" => &["id", "id2"],
        _ => &["id"],
    }
}

/// Whether `name` is a `map.*` function this module handles.
pub fn handles(name: &str) -> bool {
    name == "map.new"
        || name
            .strip_prefix("map.")
            .is_some_and(|member| MEMBERS.contains(&member))
}

const MEMBERS: [&str; 10] = [
    "put", "get", "contains", "remove", "size", "clear", "keys", "values", "copy", "put_all",
];

fn runtime_error(code: &str, message: String, range: SourceRange) -> Diagnostic {
    Diagnostic::error(code, DiagnosticCategory::Runtime, message, range)
}

fn new_map(state: &mut RuntimeState, map: Map) -> RuntimeValue {
    state.next_reference += 1;
    let id = state.next_reference;
    state.maps.insert(id, map);
    RuntimeValue::Ref(RefKind::Map, id)
}

fn key_of(
    value: Option<&RuntimeValue>,
    name: &str,
    range: SourceRange,
) -> Result<MapKey, Diagnostic> {
    value.and_then(MapKey::from_value).ok_or_else(|| {
        runtime_error(
            "PINE4027",
            format!("{name} needs a number, bool, string or color as its key."),
            range,
        )
    })
}

pub fn call(
    name: &str,
    bound: &Bound,
    state: &mut RuntimeState,
    range: SourceRange,
) -> Result<RuntimeValue, Diagnostic> {
    if name == "map.new" {
        return Ok(new_map(state, Map::default()));
    }
    let member = name.strip_prefix("map.").unwrap_or(name);
    let Some(RuntimeValue::Ref(RefKind::Map, id)) = bound.get("id") else {
        return Err(runtime_error(
            "PINE4026",
            format!("{name} requires a map; got na."),
            range,
        ));
    };
    let id = *id;
    // The map leaves the table while one operation runs on it, so `map.get` in a loop does not copy it.
    let Some(mut map) = state.maps.remove(&id) else {
        return Err(runtime_error(
            "PINE4026",
            format!("{name} requires a map; got na."),
            range,
        ));
    };
    let outcome = apply(member, name, bound, &mut map, id, state, range);
    state.maps.insert(id, map);
    outcome
}

fn apply(
    member: &str,
    name: &str,
    bound: &Bound,
    map: &mut Map,
    id: usize,
    state: &mut RuntimeState,
    range: SourceRange,
) -> Result<RuntimeValue, Diagnostic> {
    Ok(match member {
        "put" => {
            let key = key_of(bound.get("key"), name, range)?;
            map.put(bound.get("value").cloned().unwrap_or(RuntimeValue::Na), key);
            RuntimeValue::Void
        }
        "get" => map
            .value(&key_of(bound.get("key"), name, range)?)
            .cloned()
            .unwrap_or(RuntimeValue::Na),
        "contains" => RuntimeValue::Bool(map.contains(&key_of(bound.get("key"), name, range)?)),
        "remove" => map
            .remove(&key_of(bound.get("key"), name, range)?)
            .unwrap_or(RuntimeValue::Na),
        "size" => RuntimeValue::Int(map.count() as i64),
        "clear" => {
            map.clear();
            RuntimeValue::Void
        }
        "keys" => new_array(state, map.keys()),
        "values" => new_array(state, map.values()),
        "copy" => new_map(state, map.clone()),
        "put_all" => {
            let Some(RuntimeValue::Ref(RefKind::Map, other_id)) = bound.get("id2") else {
                return Err(runtime_error(
                    "PINE4026",
                    "map.put_all requires a map; got na.".to_owned(),
                    range,
                ));
            };
            // `m.put_all(m)` reads the map that is out of the table right now.
            let other = if *other_id == id {
                Some(map.clone())
            } else {
                state.maps.get(other_id).cloned()
            }
            .ok_or_else(|| {
                runtime_error(
                    "PINE4026",
                    "map.put_all requires a map; got na.".to_owned(),
                    range,
                )
            })?;
            for (key, value) in other.pairs() {
                if let Some(key) = MapKey::from_value(&key) {
                    map.put(value, key);
                }
            }
            RuntimeValue::Void
        }
        _ => {
            return Err(runtime_error(
                "PINE4007",
                format!("Unknown or unsupported function '{name}'."),
                range,
            ));
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use RuntimeValue::{Bool, Float, Int, Na};

    fn s(text: &str) -> RuntimeValue {
        RuntimeValue::String(text.to_owned())
    }

    fn run(
        state: &mut RuntimeState,
        name: &str,
        arguments: &[RuntimeValue],
    ) -> Result<RuntimeValue, Diagnostic> {
        let bound: Bound = parameters(name)
            .iter()
            .zip(arguments)
            .map(|(parameter, value)| ((*parameter).to_owned(), value.clone()))
            .collect();
        call(name, &bound, state, SourceRange::ZERO)
    }

    fn ok(state: &mut RuntimeState, name: &str, arguments: &[RuntimeValue]) -> RuntimeValue {
        run(state, name, arguments).unwrap_or_else(|e| panic!("{name}: {}", e.message))
    }

    fn items(state: &RuntimeState, array: &RuntimeValue) -> Vec<RuntimeValue> {
        match array {
            RuntimeValue::Ref(RefKind::Array, id) => state.arrays[id].clone(),
            other => panic!("not an array: {other:?}"),
        }
    }

    #[test]
    fn put_get_contains_and_remove() {
        let mut state = RuntimeState::default();
        let map = ok(&mut state, "map.new", &[]);
        assert_eq!(
            ok(&mut state, "map.put", &[map.clone(), s("a"), Int(1)]),
            RuntimeValue::Void
        );
        ok(&mut state, "map.put", &[map.clone(), s("b"), Float(2.5)]);
        assert_eq!(ok(&mut state, "map.get", &[map.clone(), s("a")]), Int(1));
        assert_eq!(
            ok(&mut state, "map.get", &[map.clone(), s("zzz")]),
            Na,
            "a missing key is na"
        );
        assert_eq!(
            ok(&mut state, "map.contains", &[map.clone(), s("b")]),
            Bool(true)
        );
        assert_eq!(
            ok(&mut state, "map.size", std::slice::from_ref(&map)),
            Int(2)
        );
        ok(&mut state, "map.put", &[map.clone(), s("a"), Int(9)]);
        assert_eq!(
            ok(&mut state, "map.get", &[map.clone(), s("a")]),
            Int(9),
            "put replaces"
        );
        assert_eq!(
            ok(&mut state, "map.size", std::slice::from_ref(&map)),
            Int(2),
            "and does not grow"
        );
        assert_eq!(ok(&mut state, "map.remove", &[map.clone(), s("a")]), Int(9));
        assert_eq!(
            ok(&mut state, "map.remove", &[map.clone(), s("a")]),
            Na,
            "already gone"
        );
        assert_eq!(
            ok(&mut state, "map.contains", &[map.clone(), s("a")]),
            Bool(false)
        );
        ok(&mut state, "map.clear", std::slice::from_ref(&map));
        assert_eq!(ok(&mut state, "map.size", &[map]), Int(0));
    }

    #[test]
    fn keys_and_values_come_back_in_insertion_order() {
        let mut state = RuntimeState::default();
        let map = ok(&mut state, "map.new", &[]);
        for (key, value) in [("z", 1), ("a", 2), ("m", 3)] {
            ok(&mut state, "map.put", &[map.clone(), s(key), Int(value)]);
        }
        let keys = ok(&mut state, "map.keys", std::slice::from_ref(&map));
        assert_eq!(items(&state, &keys), [s("z"), s("a"), s("m")]);
        let values = ok(&mut state, "map.values", std::slice::from_ref(&map));
        assert_eq!(items(&state, &values), [Int(1), Int(2), Int(3)]);
        ok(&mut state, "map.put", &[map.clone(), s("z"), Int(7)]);
        let again = ok(&mut state, "map.keys", &[map]);
        assert_eq!(
            items(&state, &again),
            [s("z"), s("a"), s("m")],
            "replacing keeps the slot"
        );
    }

    #[test]
    fn keys_may_be_numbers_bools_strings_or_colors() {
        let mut state = RuntimeState::default();
        let map = ok(&mut state, "map.new", &[]);
        for key in [
            Int(1),
            Float(2.5),
            Bool(true),
            s("k"),
            RuntimeValue::Color(0xFF00_00FF),
        ] {
            ok(&mut state, "map.put", &[map.clone(), key.clone(), s("v")]);
            assert_eq!(ok(&mut state, "map.get", &[map.clone(), key]), s("v"));
        }
        assert_eq!(
            ok(&mut state, "map.get", &[map.clone(), Float(1.0)]),
            s("v"),
            "1.0 is the key 1"
        );
        let error = run(&mut state, "map.put", &[map.clone(), Na, Int(1)]).unwrap_err();
        assert_eq!(error.code, "PINE4027");
        assert!(
            error
                .message
                .contains("map.put needs a number, bool, string or color")
        );
        assert_eq!(
            run(&mut state, "map.get", &[map, Na]).unwrap_err().code,
            "PINE4027"
        );
    }

    #[test]
    fn copies_are_independent_and_put_all_merges() {
        let mut state = RuntimeState::default();
        let a = ok(&mut state, "map.new", &[]);
        ok(&mut state, "map.put", &[a.clone(), s("x"), Int(1)]);
        let copy = ok(&mut state, "map.copy", std::slice::from_ref(&a));
        ok(&mut state, "map.put", &[copy.clone(), s("y"), Int(2)]);
        assert_eq!(
            ok(&mut state, "map.size", std::slice::from_ref(&a)),
            Int(1),
            "the original is untouched"
        );
        assert_eq!(
            ok(&mut state, "map.size", std::slice::from_ref(&copy)),
            Int(2)
        );
        ok(&mut state, "map.put", &[a.clone(), s("x"), Int(100)]);
        ok(&mut state, "map.put_all", &[a.clone(), copy.clone()]);
        assert_eq!(
            ok(&mut state, "map.get", &[a.clone(), s("x")]),
            Int(1),
            "the merged map wins"
        );
        assert_eq!(ok(&mut state, "map.get", &[a, s("y")]), Int(2));
        assert_eq!(
            run(&mut state, "map.put_all", &[copy, Na])
                .unwrap_err()
                .code,
            "PINE4026"
        );
    }

    #[test]
    fn a_non_map_or_unknown_member_is_an_error() {
        let mut state = RuntimeState::default();
        let error = run(&mut state, "map.get", &[Na, s("k")]).unwrap_err();
        assert_eq!(
            (error.code.as_str(), error.message.as_str()),
            ("PINE4026", "map.get requires a map; got na.")
        );
        let map = ok(&mut state, "map.new", &[]);
        assert_eq!(
            run(&mut state, "map.nope", &[map]).unwrap_err().code,
            "PINE4007"
        );
        assert!(
            handles("map.new")
                && handles("map.put_all")
                && !handles("map.nope")
                && !handles("array.get")
        );
        assert_eq!(parameters("map.put"), ["id", "key", "value"]);
        assert_eq!(parameters("map.size"), ["id"]);
    }
}
