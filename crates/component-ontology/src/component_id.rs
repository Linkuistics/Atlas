//! Typed component identifier. Path-derived: a non-empty sequence of
//! kebab-case slug segments joined by `/`. Owns the segment-validation
//! rules previously implicit in `atlas-engine::identifiers::slugify_segment`.

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ComponentIdError {
    #[error("component id must not be empty")]
    Empty,
    #[error("component id segment {index} is empty (id was {input:?})")]
    EmptySegment { index: usize, input: String },
    #[error("component id segment {segment:?} contains non-slug character {ch:?}")]
    InvalidChar { segment: String, ch: char },
    #[error("component id segment {segment:?} has leading or trailing dash")]
    EdgeDash { segment: String },
    #[error("component id segment {segment:?} contains a run of dashes")]
    DashRun { segment: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Ord, PartialOrd)]
pub struct ComponentId(String);

impl ComponentId {
    pub fn parse(s: &str) -> Result<Self, ComponentIdError> {
        if s.is_empty() {
            return Err(ComponentIdError::Empty);
        }
        for (i, seg) in s.split('/').enumerate() {
            validate_segment(seg, i, s)?;
        }
        Ok(ComponentId(s.to_string()))
    }

    pub fn from_segments<I, S>(segments: I) -> Result<Self, ComponentIdError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let joined: String = segments
            .into_iter()
            .map(|s| s.as_ref().to_string())
            .collect::<Vec<_>>()
            .join("/");
        Self::parse(&joined)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn segments(&self) -> impl Iterator<Item = &str> {
        self.0.split('/')
    }

    pub fn leaf(&self) -> &str {
        self.0
            .rsplit('/')
            .next()
            .expect("non-empty by construction")
    }

    pub fn parent(&self) -> Option<ComponentId> {
        self.0
            .rfind('/')
            .map(|i| ComponentId(self.0[..i].to_string()))
    }

    pub fn child(&self, leaf: &str) -> Result<ComponentId, ComponentIdError> {
        let full = format!("{}/{leaf}", self.0);
        validate_segment(leaf, /* index */ 1, &full)?;
        Ok(ComponentId(full))
    }

    pub fn is_descendant_of(&self, ancestor: &ComponentId) -> bool {
        self.0
            .strip_prefix(&ancestor.0)
            .map(|rest| rest.starts_with('/'))
            .unwrap_or(false)
    }

    /// First path segment.
    pub fn root(&self) -> &str {
        self.0.split('/').next().expect("non-empty by construction")
    }
}

fn validate_segment(seg: &str, index: usize, full: &str) -> Result<(), ComponentIdError> {
    if seg.is_empty() {
        return Err(ComponentIdError::EmptySegment {
            index,
            input: full.to_string(),
        });
    }
    if seg.starts_with('-') || seg.ends_with('-') {
        return Err(ComponentIdError::EdgeDash {
            segment: seg.to_string(),
        });
    }
    let mut prev_dash = false;
    for ch in seg.chars() {
        let ok = ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-';
        if !ok {
            return Err(ComponentIdError::InvalidChar {
                segment: seg.to_string(),
                ch,
            });
        }
        if ch == '-' {
            if prev_dash {
                return Err(ComponentIdError::DashRun {
                    segment: seg.to_string(),
                });
            }
            prev_dash = true;
        } else {
            prev_dash = false;
        }
    }
    Ok(())
}

impl fmt::Display for ComponentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Serialize for ComponentId {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(s)
    }
}

impl<'de> Deserialize<'de> for ComponentId {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        ComponentId::parse(&s).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_single_segment() {
        let id = ComponentId::parse("knowledge-graph").unwrap();
        assert_eq!(id.as_str(), "knowledge-graph");
        assert_eq!(id.leaf(), "knowledge-graph");
        assert_eq!(id.root(), "knowledge-graph");
        assert!(id.parent().is_none());
        assert_eq!(id.segments().collect::<Vec<_>>(), vec!["knowledge-graph"]);
    }

    #[test]
    fn parses_multi_segment() {
        let id = ComponentId::parse("ravel-lite/knowledge-graph").unwrap();
        assert_eq!(id.as_str(), "ravel-lite/knowledge-graph");
        assert_eq!(id.leaf(), "knowledge-graph");
        assert_eq!(id.root(), "ravel-lite");
        assert_eq!(id.parent().unwrap().as_str(), "ravel-lite");
    }

    #[test]
    fn parses_three_segments() {
        let id = ComponentId::parse("atlas/atlas-cli/mycli").unwrap();
        assert_eq!(id.parent().unwrap().as_str(), "atlas/atlas-cli");
        assert_eq!(id.leaf(), "mycli");
    }

    #[test]
    fn rejects_empty() {
        assert_eq!(ComponentId::parse(""), Err(ComponentIdError::Empty));
    }

    #[test]
    fn rejects_empty_segment() {
        assert!(matches!(
            ComponentId::parse("a//b"),
            Err(ComponentIdError::EmptySegment { .. })
        ));
        assert!(matches!(
            ComponentId::parse("/a"),
            Err(ComponentIdError::EmptySegment { .. })
        ));
        assert!(matches!(
            ComponentId::parse("a/"),
            Err(ComponentIdError::EmptySegment { .. })
        ));
    }

    #[test]
    fn rejects_uppercase() {
        assert!(matches!(
            ComponentId::parse("Atlas"),
            Err(ComponentIdError::InvalidChar { .. })
        ));
    }

    #[test]
    fn rejects_underscore() {
        assert!(matches!(
            ComponentId::parse("foo_bar"),
            Err(ComponentIdError::InvalidChar { .. })
        ));
    }

    #[test]
    fn rejects_edge_dashes() {
        assert!(matches!(
            ComponentId::parse("-foo"),
            Err(ComponentIdError::EdgeDash { .. })
        ));
        assert!(matches!(
            ComponentId::parse("foo-"),
            Err(ComponentIdError::EdgeDash { .. })
        ));
    }

    #[test]
    fn rejects_dash_runs() {
        assert!(matches!(
            ComponentId::parse("foo--bar"),
            Err(ComponentIdError::DashRun { .. })
        ));
    }

    #[test]
    fn child_appends_segment() {
        let parent = ComponentId::parse("atlas").unwrap();
        let child = parent.child("atlas-cli").unwrap();
        assert_eq!(child.as_str(), "atlas/atlas-cli");
    }

    #[test]
    fn child_rejects_invalid_leaf() {
        let parent = ComponentId::parse("atlas").unwrap();
        assert!(parent.child("Bad").is_err());
        assert!(parent.child("a/b").is_err()); // slash in leaf
    }

    #[test]
    fn is_descendant_of_strict() {
        let p = ComponentId::parse("atlas").unwrap();
        let c = ComponentId::parse("atlas/atlas-cli").unwrap();
        let unrelated = ComponentId::parse("atlas-x").unwrap();
        assert!(c.is_descendant_of(&p));
        assert!(!p.is_descendant_of(&c));
        assert!(!p.is_descendant_of(&p));
        // "atlas-x" must not be considered a descendant of "atlas"
        assert!(!unrelated.is_descendant_of(&p));
    }

    #[test]
    fn from_segments_joins() {
        let id = ComponentId::from_segments(["atlas", "atlas-cli", "mycli"]).unwrap();
        assert_eq!(id.as_str(), "atlas/atlas-cli/mycli");
    }

    #[test]
    fn yaml_round_trip() {
        let id = ComponentId::parse("ravel-lite/knowledge-graph").unwrap();
        let yaml = serde_yaml::to_string(&id).unwrap();
        let parsed: ComponentId = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(parsed, id);
    }

    #[test]
    fn yaml_rejects_invalid_string() {
        let err = serde_yaml::from_str::<ComponentId>("Bad/Name").unwrap_err();
        assert!(err.to_string().contains("component id"));
    }

    #[test]
    fn parses_single_char_segment() {
        let id = ComponentId::parse("a").unwrap();
        assert_eq!(id.as_str(), "a");
    }

    #[test]
    fn parses_all_digit_segment() {
        let id = ComponentId::parse("123").unwrap();
        assert_eq!(id.as_str(), "123");
    }

    #[test]
    fn child_chains() {
        let p = ComponentId::parse("a").unwrap();
        let chained = p.child("b").unwrap().child("c").unwrap();
        assert_eq!(chained.as_str(), "a/b/c");
    }

    #[test]
    fn from_segments_empty_iterator_is_empty_error() {
        let r: Result<ComponentId, _> = ComponentId::from_segments::<[&str; 0], &str>([]);
        assert_eq!(r, Err(ComponentIdError::Empty));
    }
}
