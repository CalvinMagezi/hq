//! Checks a tool call's arguments against the tool's own parameter schema, and fixes the slips a
//! model makes most (a number sent as a string, one item sent where a list was wanted).
//!
//! This is opt-in per tool (`HqTool::checks_arguments`). Many tools read their arguments with
//! helpers that quietly fall back to a default, so a wrong key does nothing and the model never
//! learns why. A tool that opts in gets a specific message instead: what is missing, which
//! argument name does not exist and which one was probably meant, and the call shape.
//!
//! Offline and deterministic: the schema and the arguments are all it reads.

use serde_json::{Map, Value};

use crate::tool_rank::nearest;

/// Argument names the runtime adds (`_hq_connector`, `_hq_caller_session`, ...). They are not the
/// model's and are never checked.
const RESERVED_PREFIX: &str = "_hq_";

/// The arguments with safe type slips fixed, and what is still wrong with them.
#[derive(Debug, Clone, PartialEq)]
pub struct Checked {
    pub args: Value,
    pub problems: Vec<String>,
}

/// Check `args` against `parameters` (a JSON Schema object). A schema with no `properties` is
/// not checked, since there is nothing to check against.
pub fn check(parameters: &Value, args: Value) -> Checked {
    let Some(props) = parameters.get("properties").and_then(Value::as_object) else {
        return Checked {
            args,
            problems: Vec::new(),
        };
    };
    let mut problems = Vec::new();
    let mut map: Map<String, Value> = match args {
        Value::Object(m) => m,
        Value::Null => Map::new(),
        other => {
            return Checked {
                args: other,
                problems: vec!["the arguments must be an object of named values".to_string()],
            };
        }
    };

    let open = parameters
        .get("additionalProperties")
        .is_some_and(|v| v != &Value::Bool(false));
    let prop_names: Vec<&str> = props.keys().map(String::as_str).collect();
    let required: Vec<&str> = parameters
        .get("required")
        .and_then(Value::as_array)
        .map(|r| r.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let missing: Vec<&str> = required
        .iter()
        .copied()
        .filter(|name| map.get(*name).is_none_or(Value::is_null))
        .collect();
    let unknown: Vec<String> = map
        .keys()
        .filter(|k| !k.starts_with(RESERVED_PREFIX) && !props.contains_key(*k) && !open)
        .cloned()
        .collect();

    for key in &unknown {
        // Offer the closest name the caller has not already used. When the spelling is nowhere
        // near but exactly one argument was sent that does not exist and exactly one required
        // argument is missing, the two are almost certainly the same thing (`file` for `path`).
        let free: Vec<&str> = prop_names
            .iter()
            .copied()
            .filter(|n| !map.contains_key(*n))
            .collect();
        let guess = nearest(&free, key, 1)
            .first()
            .map(|&i| free[i])
            .or_else(|| (unknown.len() == 1 && missing.len() == 1).then(|| missing[0]));
        problems.push(match guess {
            Some(g) => format!("`{key}` is not an argument (did you mean `{g}`?)"),
            None => format!("`{key}` is not an argument"),
        });
    }

    for name in &missing {
        problems.push(format!("`{name}` is required"));
    }

    for (name, schema) in props {
        let Some(value) = map.get_mut(name) else {
            continue;
        };
        if value.is_null() {
            continue;
        }
        if let Some(problem) = fit(name, schema, value) {
            problems.push(problem);
        }
    }

    Checked {
        args: Value::Object(map),
        problems,
    }
}

/// The JSON Schema type names a property allows.
fn allowed_types(schema: &Value) -> Vec<&str> {
    match schema.get("type") {
        Some(Value::String(t)) => vec![t.as_str()],
        Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).collect(),
        _ => Vec::new(),
    }
}

fn type_of(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "a list",
        Value::Object(_) => "an object",
    }
}

fn matches(ty: &str, v: &Value) -> bool {
    match ty {
        "string" => v.is_string(),
        "integer" => v.is_i64() || v.is_u64(),
        "number" => v.is_number(),
        "boolean" => v.is_boolean(),
        "array" => v.is_array(),
        "object" => v.is_object(),
        "null" => v.is_null(),
        _ => true,
    }
}

/// Make `value` fit `schema` when the fix cannot change its meaning, otherwise say why it does
/// not fit. Also checks an `enum`.
fn fit(name: &str, schema: &Value, value: &mut Value) -> Option<String> {
    let types = allowed_types(schema);
    if !types.is_empty()
        && !types.iter().any(|t| matches(t, value))
        && !coerce(&types, schema, value)
    {
        return Some(format!(
            "`{name}` should be {}, not {}",
            describe(&types),
            type_of(value)
        ));
    }
    if let (Some(options), Some(text)) = (
        schema.get("enum").and_then(Value::as_array),
        value.as_str().map(str::to_string),
    ) && !options.iter().any(|o| o.as_str() == Some(text.as_str()))
    {
        let names: Vec<&str> = options.iter().filter_map(Value::as_str).collect();
        let guess = nearest(&names, &text, 1).first().map(|&i| names[i]);
        return Some(match guess {
            Some(g) => format!(
                "`{name}` must be one of {} (did you mean `{g}`?)",
                names.join(", ")
            ),
            None => format!("`{name}` must be one of {}", names.join(", ")),
        });
    }
    None
}

fn describe(types: &[&str]) -> String {
    let words: Vec<&str> = types
        .iter()
        .filter(|t| **t != "null")
        .map(|t| match *t {
            "string" => "a string",
            "integer" => "a whole number",
            "number" => "a number",
            "boolean" => "true or false",
            "array" => "a list",
            "object" => "an object",
            other => other,
        })
        .collect();
    words.join(" or ")
}

/// The slips that cannot change what was meant: `"5"` for 5, `"true"` for true, 5 for `"5"`, and
/// one item where a list of them was wanted. Returns whether the value now fits.
fn coerce(types: &[&str], schema: &Value, value: &mut Value) -> bool {
    for ty in types {
        let fixed = match (*ty, &*value) {
            ("integer", Value::String(s)) => s.trim().parse::<i64>().ok().map(Value::from),
            ("number", Value::String(s)) => s
                .trim()
                .parse::<f64>()
                .ok()
                .and_then(serde_json::Number::from_f64)
                .map(Value::Number),
            ("boolean", Value::String(s)) => match s.trim().to_lowercase().as_str() {
                "true" => Some(Value::Bool(true)),
                "false" => Some(Value::Bool(false)),
                _ => None,
            },
            ("string", Value::Number(n)) => Some(Value::String(n.to_string())),
            ("string", Value::Bool(b)) => Some(Value::String(b.to_string())),
            ("array", v) if !v.is_array() && !v.is_object() => {
                // Only when the single item is what the list holds.
                let item_ok = schema
                    .get("items")
                    .map(|items| {
                        let t = allowed_types(items);
                        t.is_empty() || t.iter().any(|t| matches(t, v))
                    })
                    .unwrap_or(true);
                item_ok.then(|| Value::Array(vec![v.clone()]))
            }
            _ => None,
        };
        if let Some(f) = fixed {
            *value = f;
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {"type": "string"},
                "limit": {"type": "integer"},
                "recursive": {"type": "boolean"},
                "tags": {"type": "array", "items": {"type": "string"}},
                "mode": {"type": "string", "enum": ["read", "write", "append"]},
            },
            "required": ["path"],
        })
    }

    fn problems(args: Value) -> Vec<String> {
        check(&schema(), args).problems
    }

    #[test]
    fn a_correct_call_passes_unchanged() {
        let args = json!({"path": "a.md", "limit": 5, "mode": "read"});
        let c = check(&schema(), args.clone());
        assert!(c.problems.is_empty(), "{:?}", c.problems);
        assert_eq!(c.args, args);
    }

    #[test]
    fn a_missing_required_argument_is_named() {
        assert_eq!(problems(json!({})), ["`path` is required"]);
        assert_eq!(problems(json!({"path": null})), ["`path` is required"]);
    }

    #[test]
    fn an_unknown_argument_points_at_the_closest_real_one() {
        let p = problems(json!({"path": "a", "limt": 3}));
        assert_eq!(p, ["`limt` is not an argument (did you mean `limit`?)"]);
        // Nothing like it by spelling, but it is the only unknown name and `path` is the only
        // missing required one, so it is offered.
        let p = problems(json!({"file": "a.md"}));
        assert_eq!(
            p,
            [
                "`file` is not an argument (did you mean `path`?)",
                "`path` is required"
            ]
        );
        // With two unknown names the pairing would be a guess, so none is made.
        let p = problems(json!({"file": "a.md", "bogus": 1}));
        assert!(p.iter().all(|m| !m.contains("did you mean")), "{p:?}");
    }

    #[test]
    fn an_argument_already_supplied_is_not_offered_as_the_fix() {
        let p = problems(json!({"path": "a", "pth": "b"}));
        assert_eq!(p.len(), 1);
        assert!(!p[0].contains("did you mean `path`"), "{p:?}");
    }

    #[test]
    fn harmless_type_slips_are_fixed_not_reported() {
        let c = check(
            &schema(),
            json!({"path": 7, "limit": "12", "recursive": "true", "tags": "draft"}),
        );
        assert!(c.problems.is_empty(), "{:?}", c.problems);
        assert_eq!(
            c.args,
            json!({"path": "7", "limit": 12, "recursive": true, "tags": ["draft"]})
        );
    }

    #[test]
    fn a_value_that_cannot_be_fixed_says_what_was_wanted() {
        let p = problems(json!({"path": "a", "limit": "lots", "recursive": "maybe"}));
        assert!(
            p.contains(&"`limit` should be a whole number, not a string".to_string()),
            "{p:?}"
        );
        assert!(
            p.contains(&"`recursive` should be true or false, not a string".to_string()),
            "{p:?}"
        );
        let p = problems(json!({"path": {"a": 1}}));
        assert_eq!(p, ["`path` should be a string, not an object"]);
    }

    #[test]
    fn an_enum_value_that_is_not_allowed_lists_the_options() {
        let p = problems(json!({"path": "a", "mode": "writ"}));
        assert_eq!(
            p,
            ["`mode` must be one of read, write, append (did you mean `write`?)"]
        );
    }

    #[test]
    fn runtime_arguments_are_never_checked() {
        let c = check(
            &schema(),
            json!({"path": "a", "_hq_connector": "x", "_hq_caller_session": "s"}),
        );
        assert!(c.problems.is_empty(), "{:?}", c.problems);
        assert_eq!(c.args["_hq_connector"], "x");
    }

    #[test]
    fn a_schema_that_allows_extra_arguments_or_has_no_properties_is_not_policed() {
        let open = json!({"type": "object", "properties": {"a": {"type": "string"}}, "additionalProperties": true});
        assert!(check(&open, json!({"b": 1})).problems.is_empty());
        let none = json!({"type": "object"});
        assert!(check(&none, json!({"anything": 1})).problems.is_empty());
        assert!(
            check(&json!({}), json!("not an object"))
                .problems
                .is_empty()
        );
    }

    #[test]
    fn arguments_that_are_not_an_object_are_reported() {
        let p = problems(json!("just a string"));
        assert_eq!(p, ["the arguments must be an object of named values"]);
    }
}
