//! Dotted JSON path resolver with array-index support.
//!
//! Paths are dot-separated segments. Array indexing is supported inline:
//!   `"result.XXBTZUSD.c[0]"` → result → XXBTZUSD → c → [0]
//!   `"results[0].c"`          → results → [0] → c
//!   `"data[0].value"`         → data → [0] → value
//!   `"[0].value"`             → [0] → value   (leading bracket, no key)

use serde_json::Value;

/// Navigate a dotted path within a JSON Value.
/// Each dot-separated segment may carry a trailing `[N]` array index.
/// A segment that starts with `[N]` (no leading key) indexes the current value directly.
/// Returns `None` on any missing key, out-of-bounds index, or wrong type.
pub fn resolve<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    let mut current = value;
    for segment in path.split('.') {
        if segment.is_empty() { continue; }
        if let Some(bracket_pos) = segment.find('[') {
            let key     = &segment[..bracket_pos];
            let rest    = &segment[bracket_pos + 1..];
            let idx_str = rest.trim_end_matches(']');
            let idx: usize = idx_str.parse().ok()?;
            if !key.is_empty() {
                current = current.get(key)?;
            }
            current = current.get(idx)?;
        } else {
            current = current.get(segment)?;
        }
    }
    Some(current)
}

/// Resolve a dotted path and coerce the leaf to f64.
/// If `parse_string` is true, interprets the leaf as a JSON string and parses it.
pub fn resolve_f64(value: &Value, path: &str, parse_string: bool) -> Option<f64> {
    let v = resolve(value, path)?;
    if parse_string {
        v.as_str()?.parse::<f64>().ok()
    } else {
        v.as_f64()
    }
}

/// Split a path into `(property_key, rest)`.
///
/// `property_key` is the leading bare identifier — no `.` or `[`.
/// `rest` is everything after the first delimiter, including any leading `[` if the
/// first delimiter was `[`.
///
/// Examples:
/// - `"current.temperature_2m"` → `("current", "temperature_2m")`
/// - `"results[0].c"`           → `("results", "[0].c")`
/// - `"data[0].value"`          → `("data", "[0].value")`
/// - `"value"`                  → `("value", "")`
/// - `"[0].c"`                  → `("", "[0].c")`
pub fn split_first(path: &str) -> (&str, &str) {
    let dot_pos     = path.find('.');
    let bracket_pos = path.find('[');
    match (dot_pos, bracket_pos) {
        (None,    None)                => (path, ""),
        (Some(d), None)                => (&path[..d], &path[d + 1..]),
        (None,    Some(b))             => (&path[..b], &path[b..]),
        (Some(d), Some(b)) if b < d    => (&path[..b], &path[b..]),
        (Some(d), _)                   => (&path[..d], &path[d + 1..]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn plain_nested_path() {
        let v = json!({"current": {"temperature_2m": 7.0}});
        assert_eq!(resolve_f64(&v, "current.temperature_2m", false), Some(7.0));
    }

    #[test]
    fn array_suffix() {
        // result.XXBTZUSD.c[0] where c is ["103000", ...]
        let v = json!({"result": {"XXBTZUSD": {"c": ["103000", "102900"]}}});
        let top = v.get("result").unwrap();
        assert_eq!(resolve_f64(top, "XXBTZUSD.c[0]", true), Some(103000.0));
    }

    #[test]
    fn array_prefix_in_segment() {
        // results[0].c — top-level is the "results" value (already extracted)
        let v = json!([{"c": 685.99, "v": 83292447.0}]);
        assert_eq!(resolve_f64(&v, "[0].c", false), Some(685.99));
    }

    #[test]
    fn array_prefix_string() {
        // data[0].value — string value
        let v = json!([{"date": "2026-02-23", "value": "71.9"}]);
        assert_eq!(resolve_f64(&v, "[0].value", true), Some(71.9));
    }

    #[test]
    fn missing_key() {
        let v = json!({"a": {"b": 1.0}});
        assert_eq!(resolve_f64(&v, "a.c", false), None);
    }

    #[test]
    fn out_of_bounds_index() {
        let v = json!({"c": ["x"]});
        assert_eq!(resolve_f64(&v, "c[5]", true), None);
    }

    #[test]
    fn split_first_dot() {
        assert_eq!(split_first("current.temperature_2m"), ("current", "temperature_2m"));
    }

    #[test]
    fn split_first_bracket() {
        assert_eq!(split_first("results[0].c"), ("results", "[0].c"));
        assert_eq!(split_first("data[0].value"), ("data", "[0].value"));
    }

    #[test]
    fn split_first_plain() {
        assert_eq!(split_first("value"), ("value", ""));
    }
}
