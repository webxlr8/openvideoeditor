//! Literal formatting rules shared by the printer and the typings (docs/spec/projection-format.md §Values).

use serde_json::{Number, Value};

use ove_core::time::{MICROS_PER_SECOND, Micros};

/// Seconds at microsecond precision as the shortest plain decimal: `2`, `2.5`, `0.000001`, `-1.25`.
pub fn micros(us: Micros) -> String {
    let neg = us < 0;
    let a = us.unsigned_abs();
    let whole = a / MICROS_PER_SECOND as u64;
    let frac = a % MICROS_PER_SECOND as u64;
    let mut s = if frac == 0 {
        whole.to_string()
    } else {
        let f = format!("{frac:06}");
        format!("{whole}.{}", f.trim_end_matches('0'))
    };
    if neg {
        s.insert(0, '-');
    }
    s
}

/// A JSON number as a plain decimal (never exponent notation).
pub fn number(n: &Number) -> String {
    if let Some(i) = n.as_i64() {
        i.to_string()
    } else if let Some(u) = n.as_u64() {
        u.to_string()
    } else {
        let f = n.as_f64().unwrap_or(0.0);
        if f == f.trunc() && f.abs() < 9.0e15 { format!("{}", f as i64) } else { format!("{f}") }
    }
}

fn is_ident(k: &str) -> bool {
    let mut c = k.chars();
    matches!(c.next(), Some(ch) if ch.is_ascii_alphabetic() || ch == '_' || ch == '$')
        && c.all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '$')
}

/// A JSON value as a JavaScript literal: objects with bare identifier keys, strings as JSON strings.
pub fn literal(v: &Value) -> String {
    match v {
        Value::Null => "null".into(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => number(n),
        Value::String(s) => serde_json::to_string(s).unwrap(),
        Value::Array(a) => format!("[{}]", a.iter().map(literal).collect::<Vec<_>>().join(", ")),
        Value::Object(o) if o.is_empty() => "{}".into(),
        Value::Object(o) => format!(
            "{{{}}}",
            o.iter()
                .map(|(k, v)| {
                    let key = if is_ident(k) { k.clone() } else { serde_json::to_string(k).unwrap() };
                    format!("{key}: {}", literal(v))
                })
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

/// True when a string can be written as a quoted JSX attribute and read back unchanged.
pub fn quotable(s: &str) -> bool {
    !s.chars().any(|c| c == '"' || c == '\\' || c.is_control())
}

/// An attribute value: quoted string when safe, otherwise braced literal.
pub fn attr_value(v: &Value) -> String {
    match v {
        Value::String(s) if quotable(s) => format!("\"{s}\""),
        Value::Object(_) => format!("{{{}}}", literal(v)),
        _ => format!("{{{}}}", literal(v)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn numbers_never_use_exponents() {
        assert_eq!(micros(1), "0.000001");
        assert_eq!(micros(2_500_000), "2.5");
        assert_eq!(micros(-1_250_000), "-1.25");
        assert_eq!(micros(3_000_000), "3");
        assert_eq!(literal(&json!(1e-7)), "0.0000001");
        assert_eq!(literal(&json!(100.0)), "100");
        assert_eq!(literal(&json!(0.1)), "0.1");
    }

    #[test]
    fn strings_and_objects() {
        assert_eq!(attr_value(&json!("Hello")), "\"Hello\"");
        assert_eq!(attr_value(&json!("C:\\new")), "{\"C:\\\\new\"}");
        assert_eq!(attr_value(&json!("a\nb")), "{\"a\\nb\"}");
        assert_eq!(attr_value(&json!({"x": 0, "scale": 1.1})), "{{x: 0, scale: 1.1}}");
        assert_eq!(literal(&json!({"a-b": [1, true, null]})), "{\"a-b\": [1, true, null]}");
    }
}
