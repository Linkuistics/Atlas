//! Per-field strict-string deserialization adapter.
//!
//! Use via `#[serde(deserialize_with = "deserialize_string_strict")]` on
//! string fields whose values could be misparsed under YAML's
//! implicit-typing rules: `component_id: NO` → bool `false` (the
//! Norway problem), `version: 1.10` → float `1.1` (lost trailing zero),
//! unquoted dates that coerce to YAML 1.1 timestamp values, etc.
//!
//! The adapter accepts only YAML scalars deserialized as `String`. Any
//! other YAML shape (bool, number, null, sequence, mapping, tagged
//! value) is rejected with an error string that names the failure mode
//! so Lane A's retry prompt can mention what to fix (memory
//! `feedback_yaml_canonical_interchange`).

use serde::{de, Deserialize, Deserializer};
use serde_yaml::Value;

/// Strict-string deserializer. Rejects any YAML value that was not
/// emitted as a quoted (or otherwise unambiguously string-shaped)
/// scalar. The rejection message names the implicit-typing failure mode
/// so the retry path's prompt revision can ask the LLM to quote the
/// value.
pub fn deserialize_string_strict<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    match value {
        Value::String(s) => Ok(s),
        Value::Bool(b) => Err(de::Error::custom(format!(
            "expected quoted string, got YAML bool {b} \
             (Norway-problem coercion? quote the value)"
        ))),
        Value::Number(n) => Err(de::Error::custom(format!(
            "expected quoted string, got YAML number {n} \
             (implicit numeric? quote the value)"
        ))),
        Value::Null => Err(de::Error::custom(
            "expected quoted string, got YAML null (quote the value)",
        )),
        Value::Sequence(_) => Err(de::Error::custom(
            "expected quoted string, got YAML sequence",
        )),
        Value::Mapping(_) => Err(de::Error::custom(
            "expected quoted string, got YAML mapping",
        )),
        Value::Tagged(_) => Err(de::Error::custom(
            "expected quoted string, got YAML tagged value",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Debug, Deserialize)]
    struct Wrap {
        #[serde(deserialize_with = "deserialize_string_strict")]
        id: String,
    }

    #[test]
    fn accepts_quoted_string() {
        let w: Wrap = serde_yaml::from_str(r#"id: "atlas-cli""#).unwrap();
        assert_eq!(w.id, "atlas-cli");
    }

    #[test]
    fn accepts_unambiguous_unquoted_string() {
        // Unquoted strings that don't collide with YAML's reserved
        // scalars are still strings.
        let w: Wrap = serde_yaml::from_str("id: atlas-cli").unwrap();
        assert_eq!(w.id, "atlas-cli");
    }

    #[test]
    fn rejects_unquoted_yaml_1_2_bool() {
        // serde_yaml 0.9 follows YAML 1.2 (strict), in which only
        // lowercase `true` / `false` are coerced to bool — not `NO` /
        // `yes` / `on` (those remain strings). The Norway-problem
        // framing in `feedback_yaml_canonical_interchange` is
        // historically YAML 1.1; the YAML 1.2 residual hazard is
        // `id: true` slipping through as a bool. The adapter rejects.
        let err = serde_yaml::from_str::<Wrap>("id: true").unwrap_err();
        assert!(
            err.to_string().contains("Norway-problem"),
            "error must name the failure mode for actionable LLM feedback; got: {err}"
        );
    }

    #[test]
    fn rejects_unquoted_version_shaped_number() {
        let err = serde_yaml::from_str::<Wrap>("id: 1.10").unwrap_err();
        assert!(
            err.to_string().contains("implicit numeric"),
            "error must name the failure mode for actionable LLM feedback; got: {err}"
        );
    }

    #[test]
    fn rejects_unquoted_integer() {
        let err = serde_yaml::from_str::<Wrap>("id: 123").unwrap_err();
        assert!(
            err.to_string().contains("implicit numeric"),
            "error must name the failure mode for actionable LLM feedback; got: {err}"
        );
    }

    #[test]
    fn yaml_1_2_keeps_no_as_string_naturally() {
        // YAML 1.2 reads `NO` as a string (no Norway coercion). The
        // strict adapter accepts it as a String — no rejection needed.
        let w: Wrap = serde_yaml::from_str("id: NO").unwrap();
        assert_eq!(w.id, "NO");
    }

    #[test]
    fn accepts_quoted_true() {
        // Quoted `true` is unambiguously a string.
        let w: Wrap = serde_yaml::from_str(r#"id: "true""#).unwrap();
        assert_eq!(w.id, "true");
    }

    #[test]
    fn rejects_null() {
        let err = serde_yaml::from_str::<Wrap>("id: null").unwrap_err();
        assert!(err.to_string().contains("YAML null"));
    }

    #[test]
    fn rejects_sequence_in_string_position() {
        let err = serde_yaml::from_str::<Wrap>("id:\n  - a\n  - b\n").unwrap_err();
        assert!(err.to_string().contains("YAML sequence"));
    }
}
