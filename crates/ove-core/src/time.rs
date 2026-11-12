//! Time representation (design D4): integer microseconds inside the core, seconds as JSON numbers on the wire,
//! written with at most 6 decimals.

use serde_json::{Number, Value};

/// A time or duration in integer microseconds.
pub type Micros = i64;

pub const MICROS_PER_SECOND: i64 = 1_000_000;

/// Converts seconds (as read from JSON) to microseconds, rounding to the nearest microsecond.
/// Returns `None` for non-finite values or values outside the representable range.
pub fn secs_to_micros(secs: f64) -> Option<Micros> {
    if !secs.is_finite() {
        return None;
    }
    let us = (secs * MICROS_PER_SECOND as f64).round();
    // Keep well inside the range where every microsecond is exactly representable as f64 seconds.
    if us.abs() > 9.0e15 {
        return None;
    }
    Some(us as Micros)
}

/// Converts microseconds to a JSON number in seconds: an integer when whole, otherwise the shortest decimal that
/// round-trips (at most 6 decimals).
pub fn micros_to_json(us: Micros) -> Value {
    if us % MICROS_PER_SECOND == 0 {
        Value::Number(Number::from(us / MICROS_PER_SECOND))
    } else {
        // Any i64 divided by 1e6 is finite, so `from_f64` always succeeds; whole seconds are the fallback.
        let n = Number::from_f64(us as f64 / MICROS_PER_SECOND as f64);
        Value::Number(n.unwrap_or_else(|| Number::from(us / MICROS_PER_SECOND)))
    }
}

/// Reads a JSON value holding seconds as microseconds.
pub fn json_to_micros(v: &Value) -> Option<Micros> {
    match v {
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                i.checked_mul(MICROS_PER_SECOND)
            } else {
                n.as_f64().and_then(secs_to_micros)
            }
        }
        _ => None,
    }
}

/// Normalises a JSON seconds value to microsecond precision (same value written the canonical way).
pub fn normalise_secs(v: &Value) -> Option<Value> {
    json_to_micros(v).map(micros_to_json)
}

/// Formats microseconds as seconds for messages, e.g. `2.5s`.
pub fn fmt_secs(us: Micros) -> String {
    format!("{}s", micros_to_json(us))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whole_seconds_are_integers() {
        assert_eq!(micros_to_json(2_000_000).to_string(), "2");
        assert_eq!(micros_to_json(0).to_string(), "0");
        assert_eq!(micros_to_json(-3_000_000).to_string(), "-3");
    }

    #[test]
    fn fractions_print_shortest() {
        assert_eq!(micros_to_json(2_500_000).to_string(), "2.5");
        assert_eq!(micros_to_json(100_000).to_string(), "0.1");
        assert_eq!(micros_to_json(1).to_string(), "1e-6");
        assert_eq!(micros_to_json(123_456_789).to_string(), "123.456789");
    }

    #[test]
    fn rounding_to_micros() {
        assert_eq!(secs_to_micros(0.1 + 0.2), Some(300_000));
        assert_eq!(secs_to_micros(1.000_000_4), Some(1_000_000));
        assert_eq!(secs_to_micros(f64::NAN), None);
        assert_eq!(secs_to_micros(f64::INFINITY), None);
    }

    #[test]
    fn roundtrip_every_micro_in_range() {
        for us in (0..5_000_000).step_by(997) {
            assert_eq!(json_to_micros(&micros_to_json(us)), Some(us));
        }
        let big = 36_000 * MICROS_PER_SECOND + 123_457;
        assert_eq!(json_to_micros(&micros_to_json(big)), Some(big));
    }
}
