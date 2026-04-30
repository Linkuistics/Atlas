//! Metric computation and differential evaluation.
//!
//! Per-run metrics compare a tool output to a hand-authored golden:
//! component coverage, spurious rate, kind accuracy, edge
//! precision/recall, identifier stability across a no-op rerun.
//!
//! The differential report compares two tool runs (typically a blessed
//! baseline and a fresh run) and enumerates what changed: components
//! added, removed, modified; edges added, removed; identifier changes.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use atlas_index::{ComponentEntry, ComponentsFile};
use component_ontology::{Edge, EdgeKind, LifecycleScope, RelatedComponentsFile};
use serde::{Deserialize, Serialize};

pub const DEFAULT_OVERLAP_THRESHOLD: f32 = 0.5;

/// Fraction of the tool-output path's components that are shared with
/// the golden path. Symmetric: max of the two jaccard-style ratios so
/// that "golden has one big segment, tool split it in three" still
/// matches when the shared prefix is substantial.
pub fn path_set_overlap(golden: &[PathBuf], tool: &[PathBuf]) -> f32 {
    if golden.is_empty() || tool.is_empty() {
        return 0.0;
    }
    let golden_set: BTreeSet<&PathBuf> = golden.iter().collect();
    let tool_set: BTreeSet<&PathBuf> = tool.iter().collect();
    let intersection = golden_set.intersection(&tool_set).count() as f32;
    let union = golden_set.union(&tool_set).count() as f32;
    if union == 0.0 {
        return 0.0;
    }
    intersection / union
}

#[derive(Debug, Clone, Copy)]
pub struct OverlapThreshold(pub f32);

impl Default for OverlapThreshold {
    fn default() -> Self {
        OverlapThreshold(DEFAULT_OVERLAP_THRESHOLD)
    }
}

/// Coverage = |{golden components with an overlap-matching tool
/// component}| / |golden components|. Overlap is path-set-based; a tool
/// component matches a golden component when their path-segment sets
/// jaccard-overlap by at least `threshold`.
pub fn component_coverage(
    golden: &ComponentsFile,
    tool: &ComponentsFile,
    threshold: OverlapThreshold,
) -> f32 {
    let golden_components: Vec<&ComponentEntry> =
        golden.components.iter().filter(|c| !c.deleted).collect();
    if golden_components.is_empty() {
        return 1.0;
    }
    let matched = golden_components
        .iter()
        .filter(|g| best_overlap(g, tool) >= threshold.0)
        .count();
    matched as f32 / golden_components.len() as f32
}

/// Spurious rate = |{tool components with no golden match}| / |tool
/// components|. Uses the same threshold as `component_coverage`.
pub fn spurious_rate(
    golden: &ComponentsFile,
    tool: &ComponentsFile,
    threshold: OverlapThreshold,
) -> f32 {
    let tool_components: Vec<&ComponentEntry> =
        tool.components.iter().filter(|c| !c.deleted).collect();
    if tool_components.is_empty() {
        return 0.0;
    }
    let spurious = tool_components
        .iter()
        .filter(|t| best_overlap_symmetric(t, golden) < threshold.0)
        .count();
    spurious as f32 / tool_components.len() as f32
}

fn best_overlap(component: &ComponentEntry, other_file: &ComponentsFile) -> f32 {
    let paths: Vec<PathBuf> = component
        .path_segments
        .iter()
        .map(|p| p.path.clone())
        .collect();
    other_file
        .components
        .iter()
        .filter(|c| !c.deleted)
        .map(|o| {
            let other_paths: Vec<PathBuf> =
                o.path_segments.iter().map(|p| p.path.clone()).collect();
            path_set_overlap(&paths, &other_paths)
        })
        .fold(0.0_f32, f32::max)
}

fn best_overlap_symmetric(component: &ComponentEntry, other_file: &ComponentsFile) -> f32 {
    best_overlap(component, other_file)
}

/// Fraction of matched (overlap ≥ threshold) component pairs where the
/// tool's `kind` equals the golden's. Unmatched golden entries are
/// excluded from the denominator — kind accuracy is a conditional
/// metric scoped to pairs that both sides agree exist.
pub fn kind_accuracy(
    golden: &ComponentsFile,
    tool: &ComponentsFile,
    threshold: OverlapThreshold,
) -> f32 {
    let mut denom = 0usize;
    let mut agree = 0usize;
    for g in &golden.components {
        if g.deleted {
            continue;
        }
        if let Some(matched) = best_matching_component(g, tool, threshold) {
            denom += 1;
            if matched.kind == g.kind {
                agree += 1;
            }
        }
    }
    if denom == 0 {
        return 1.0;
    }
    agree as f32 / denom as f32
}

fn best_matching_component<'a>(
    target: &ComponentEntry,
    pool: &'a ComponentsFile,
    threshold: OverlapThreshold,
) -> Option<&'a ComponentEntry> {
    let target_paths: Vec<PathBuf> = target
        .path_segments
        .iter()
        .map(|p| p.path.clone())
        .collect();
    let mut best: Option<(f32, &ComponentEntry)> = None;
    for candidate in &pool.components {
        if candidate.deleted {
            continue;
        }
        let paths: Vec<PathBuf> = candidate
            .path_segments
            .iter()
            .map(|p| p.path.clone())
            .collect();
        let overlap = path_set_overlap(&target_paths, &paths);
        if overlap >= threshold.0 {
            match best {
                Some((bo, _)) if bo >= overlap => {}
                _ => best = Some((overlap, candidate)),
            }
        }
    }
    best.map(|(_, c)| c)
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct PrecisionRecall {
    pub precision: f32,
    pub recall: f32,
}

/// Edge precision/recall, optionally filtered by kind or lifecycle.
/// Precision = |golden ∩ tool| / |tool|; recall = |golden ∩ tool| /
/// |golden|. Two edges match when their canonical keys are equal
/// (kind + lifecycle + participant set with directed-order semantics).
pub fn edge_precision_recall(
    golden: &RelatedComponentsFile,
    tool: &RelatedComponentsFile,
    kind_filter: Option<EdgeKind>,
    lifecycle_filter: Option<LifecycleScope>,
) -> PrecisionRecall {
    let g = filter_edges(&golden.edges, kind_filter, lifecycle_filter);
    let t = filter_edges(&tool.edges, kind_filter, lifecycle_filter);

    let golden_keys: BTreeSet<String> = g.iter().map(|e| edge_key(e)).collect();
    let tool_keys: BTreeSet<String> = t.iter().map(|e| edge_key(e)).collect();

    let intersection = golden_keys.intersection(&tool_keys).count() as f32;
    let precision = if tool_keys.is_empty() {
        1.0
    } else {
        intersection / tool_keys.len() as f32
    };
    let recall = if golden_keys.is_empty() {
        1.0
    } else {
        intersection / golden_keys.len() as f32
    };
    PrecisionRecall { precision, recall }
}

fn filter_edges(
    edges: &[Edge],
    kind_filter: Option<EdgeKind>,
    lifecycle_filter: Option<LifecycleScope>,
) -> Vec<&Edge> {
    edges
        .iter()
        .filter(|e| kind_filter.is_none_or(|k| e.kind == k))
        .filter(|e| lifecycle_filter.is_none_or(|l| e.lifecycle == l))
        .collect()
}

/// Identifier stability across two runs: fraction of prior-run ids also
/// present in the later run (same id -> same component). Near 1.0 is
/// the stability guarantee §10.3 criterion 2 asks for.
pub fn identifier_stability(run_a: &ComponentsFile, run_b: &ComponentsFile) -> f32 {
    let a: BTreeSet<&str> = run_a
        .components
        .iter()
        .filter(|c| !c.deleted)
        .map(|c| c.id.as_str())
        .collect();
    let b: BTreeSet<&str> = run_b
        .components
        .iter()
        .filter(|c| !c.deleted)
        .map(|c| c.id.as_str())
        .collect();
    if a.is_empty() {
        return 1.0;
    }
    let shared = a.intersection(&b).count() as f32;
    shared / a.len() as f32
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MetricSummary {
    pub component_coverage: f32,
    pub spurious_rate: f32,
    pub kind_accuracy: f32,
    pub edge_precision: f32,
    pub edge_recall: f32,
    pub identifier_stability: Option<f32>,
}

/// Convenience: compute every per-run metric from a (golden, tool)
/// pair. Identifier stability is left `None` — the caller supplies it
/// when a prior run is available.
pub fn metric_summary(
    golden_components: &ComponentsFile,
    tool_components: &ComponentsFile,
    golden_related: &RelatedComponentsFile,
    tool_related: &RelatedComponentsFile,
    threshold: OverlapThreshold,
) -> MetricSummary {
    let edge = edge_precision_recall(golden_related, tool_related, None, None);
    MetricSummary {
        component_coverage: component_coverage(golden_components, tool_components, threshold),
        spurious_rate: spurious_rate(golden_components, tool_components, threshold),
        kind_accuracy: kind_accuracy(golden_components, tool_components, threshold),
        edge_precision: edge.precision,
        edge_recall: edge.recall,
        identifier_stability: None,
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DifferentialReport {
    pub added_components: Vec<String>,
    pub removed_components: Vec<String>,
    pub modified_components: Vec<ComponentDelta>,
    pub added_edges: Vec<String>,
    pub removed_edges: Vec<String>,
    pub identifier_changes: Vec<IdentifierChange>,
}

impl DifferentialReport {
    pub fn is_empty(&self) -> bool {
        self.added_components.is_empty()
            && self.removed_components.is_empty()
            && self.modified_components.is_empty()
            && self.added_edges.is_empty()
            && self.removed_edges.is_empty()
            && self.identifier_changes.is_empty()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentDelta {
    pub id: String,
    pub fields: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdentifierChange {
    pub prior_id: String,
    pub new_id: String,
    pub shared_path: PathBuf,
}

/// Compare two tool runs end-to-end. Both arguments are expected to be
/// directories that contain the three generated YAMLs; any missing file
/// is treated as empty.
pub fn run_diff(baseline_dir: &Path, candidate_dir: &Path) -> anyhow::Result<DifferentialReport> {
    let baseline = load_triple(baseline_dir)?;
    let candidate = load_triple(candidate_dir)?;
    Ok(diff_triple(&baseline, &candidate))
}

struct LoadedTriple {
    components: ComponentsFile,
    related: RelatedComponentsFile,
}

fn load_triple(dir: &Path) -> anyhow::Result<LoadedTriple> {
    let components = atlas_index::load_or_default_components(&dir.join("components.yaml"))?;
    let related = component_ontology::load_or_default(&dir.join("related-components.yaml"))?;
    Ok(LoadedTriple {
        components,
        related,
    })
}

fn diff_triple(baseline: &LoadedTriple, candidate: &LoadedTriple) -> DifferentialReport {
    let baseline_by_id: BTreeMap<&str, &ComponentEntry> = baseline
        .components
        .components
        .iter()
        .filter(|c| !c.deleted)
        .map(|c| (c.id.as_str(), c))
        .collect();
    let candidate_by_id: BTreeMap<&str, &ComponentEntry> = candidate
        .components
        .components
        .iter()
        .filter(|c| !c.deleted)
        .map(|c| (c.id.as_str(), c))
        .collect();

    let mut added_components: Vec<String> = candidate_by_id
        .keys()
        .filter(|k| !baseline_by_id.contains_key(*k))
        .map(|s| (*s).to_string())
        .collect();
    added_components.sort();

    let mut removed_components: Vec<String> = baseline_by_id
        .keys()
        .filter(|k| !candidate_by_id.contains_key(*k))
        .map(|s| (*s).to_string())
        .collect();
    removed_components.sort();

    let mut modified_components: Vec<ComponentDelta> = Vec::new();
    for (id, base) in &baseline_by_id {
        if let Some(new) = candidate_by_id.get(id) {
            let fields = component_field_delta(base, new);
            if !fields.is_empty() {
                modified_components.push(ComponentDelta {
                    id: (*id).into(),
                    fields,
                });
            }
        }
    }
    modified_components.sort_by(|a, b| a.id.cmp(&b.id));

    let baseline_edge_keys: BTreeSet<String> =
        baseline.related.edges.iter().map(edge_key).collect();
    let candidate_edge_keys: BTreeSet<String> =
        candidate.related.edges.iter().map(edge_key).collect();

    let mut added_edges: Vec<String> = candidate_edge_keys
        .difference(&baseline_edge_keys)
        .cloned()
        .collect();
    added_edges.sort();
    let mut removed_edges: Vec<String> = baseline_edge_keys
        .difference(&candidate_edge_keys)
        .cloned()
        .collect();
    removed_edges.sort();

    let identifier_changes = detect_identifier_changes(
        &baseline.components.components,
        &candidate.components.components,
    );

    DifferentialReport {
        added_components,
        removed_components,
        modified_components,
        added_edges,
        removed_edges,
        identifier_changes,
    }
}

fn component_field_delta(a: &ComponentEntry, b: &ComponentEntry) -> Vec<String> {
    let mut fields = Vec::new();
    if a.kind != b.kind {
        fields.push("kind".into());
    }
    if a.language != b.language {
        fields.push("language".into());
    }
    if a.build_system != b.build_system {
        fields.push("build_system".into());
    }
    if a.role != b.role {
        fields.push("role".into());
    }
    if a.parent != b.parent {
        fields.push("parent".into());
    }
    if a.lifecycle_roles != b.lifecycle_roles {
        fields.push("lifecycle_roles".into());
    }
    if a.evidence_grade != b.evidence_grade {
        fields.push("evidence_grade".into());
    }
    if a.manifests != b.manifests {
        fields.push("manifests".into());
    }
    if a.path_segments != b.path_segments {
        fields.push("path_segments".into());
    }
    fields
}

fn edge_key(edge: &Edge) -> String {
    let (kind, lifecycle, parts) = edge.canonical_key();
    format!(
        "{}|{}|{}",
        kind.as_str(),
        lifecycle.as_str(),
        parts.join(",")
    )
}

fn detect_identifier_changes(
    baseline: &[ComponentEntry],
    candidate: &[ComponentEntry],
) -> Vec<IdentifierChange> {
    // Identifier change = same path_segments content_sha, different id.
    let mut by_sha: BTreeMap<&str, &ComponentEntry> = BTreeMap::new();
    for c in baseline.iter().filter(|c| !c.deleted) {
        for seg in &c.path_segments {
            by_sha.insert(seg.content_sha.as_str(), c);
        }
    }
    let mut changes = Vec::new();
    for c in candidate.iter().filter(|c| !c.deleted) {
        for seg in &c.path_segments {
            if let Some(prior) = by_sha.get(seg.content_sha.as_str()) {
                if prior.id != c.id {
                    changes.push(IdentifierChange {
                        prior_id: prior.id.clone(),
                        new_id: c.id.clone(),
                        shared_path: seg.path.clone(),
                    });
                }
            }
        }
    }
    changes.sort_by(|a, b| a.prior_id.cmp(&b.prior_id));
    changes.dedup_by(|a, b| a.prior_id == b.prior_id && a.new_id == b.new_id);
    changes
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use atlas_index::{
        CacheFingerprints, ComponentEntry, ComponentsFile, PathSegment, COMPONENTS_SCHEMA_VERSION,
    };
    use component_ontology::{
        Edge, EdgeKind, EvidenceGrade, LifecycleScope, RelatedComponentsFile,
    };
    use tempfile::TempDir;

    use super::*;

    fn component(id: &str, path: &str) -> ComponentEntry {
        ComponentEntry {
            id: id.into(),
            parent: None,
            kind: "rust-library".into(),
            lifecycle_roles: vec![],
            language: Some("rust".into()),
            build_system: Some("cargo".into()),
            role: None,
            path_segments: vec![PathSegment {
                path: PathBuf::from(path),
                content_sha: format!("sha-{path}"),
            }],
            manifests: vec![],
            doc_anchors: vec![],
            evidence_grade: EvidenceGrade::Strong,
            evidence_fields: vec!["manifest".into()],
            rationale: "r".into(),
            deleted: false,
        }
    }

    fn file_of(components: Vec<ComponentEntry>) -> ComponentsFile {
        ComponentsFile {
            schema_version: COMPONENTS_SCHEMA_VERSION,
            root: PathBuf::from("."),
            generated_at: String::new(),
            cache_fingerprints: CacheFingerprints::default(),
            components,
        }
    }

    fn edge(kind: EdgeKind, lifecycle: LifecycleScope, a: &str, b: &str) -> Edge {
        Edge {
            kind,
            lifecycle,
            participants: vec![a.into(), b.into()],
            evidence_grade: EvidenceGrade::Strong,
            evidence_fields: vec!["field".into()],
            rationale: "r".into(),
        }
    }

    #[test]
    fn component_coverage_of_identical_files_is_one() {
        let g = file_of(vec![component("a", "pkg-a"), component("b", "pkg-b")]);
        let t = g.clone();
        assert!((component_coverage(&g, &t, OverlapThreshold::default()) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn component_coverage_on_disjoint_is_zero() {
        let g = file_of(vec![component("a", "pkg-a")]);
        let t = file_of(vec![component("b", "pkg-b")]);
        assert_eq!(component_coverage(&g, &t, OverlapThreshold::default()), 0.0);
    }

    #[test]
    fn component_coverage_on_empty_golden_is_one() {
        let g = file_of(vec![]);
        let t = file_of(vec![component("a", "pkg-a")]);
        assert_eq!(component_coverage(&g, &t, OverlapThreshold::default()), 1.0);
    }

    #[test]
    fn spurious_rate_of_identical_files_is_zero() {
        let g = file_of(vec![component("a", "pkg-a")]);
        let t = g.clone();
        assert_eq!(spurious_rate(&g, &t, OverlapThreshold::default()), 0.0);
    }

    #[test]
    fn spurious_rate_flags_unmatched_tool_components() {
        let g = file_of(vec![component("a", "pkg-a")]);
        let t = file_of(vec![component("a", "pkg-a"), component("b", "pkg-b")]);
        let rate = spurious_rate(&g, &t, OverlapThreshold::default());
        assert!((rate - 0.5).abs() < 1e-6, "got {rate}");
    }

    #[test]
    fn kind_accuracy_of_identical_files_is_one() {
        let g = file_of(vec![component("a", "pkg-a")]);
        let t = g.clone();
        assert_eq!(kind_accuracy(&g, &t, OverlapThreshold::default()), 1.0);
    }

    #[test]
    fn kind_accuracy_penalises_mismatched_kind() {
        let g = file_of(vec![component("a", "pkg-a")]);
        let mut tool_c = component("a", "pkg-a");
        tool_c.kind = "rust-binary".into();
        let t = file_of(vec![tool_c]);
        assert_eq!(kind_accuracy(&g, &t, OverlapThreshold::default()), 0.0);
    }

    #[test]
    fn edge_precision_recall_on_identical_edges_is_one_one() {
        let mut golden = RelatedComponentsFile::default();
        golden
            .add_edge(edge(EdgeKind::DependsOn, LifecycleScope::Build, "a", "b"))
            .unwrap();
        let tool = golden.clone();
        let pr = edge_precision_recall(&golden, &tool, None, None);
        assert!((pr.precision - 1.0).abs() < 1e-6);
        assert!((pr.recall - 1.0).abs() < 1e-6);
    }

    #[test]
    fn edge_precision_recall_on_extra_tool_edge_drops_precision() {
        let mut golden = RelatedComponentsFile::default();
        golden
            .add_edge(edge(EdgeKind::DependsOn, LifecycleScope::Build, "a", "b"))
            .unwrap();
        let mut tool = golden.clone();
        tool.add_edge(edge(EdgeKind::DependsOn, LifecycleScope::Build, "a", "c"))
            .unwrap();
        let pr = edge_precision_recall(&golden, &tool, None, None);
        assert!(
            (pr.precision - 0.5).abs() < 1e-6,
            "precision={}",
            pr.precision
        );
        assert!((pr.recall - 1.0).abs() < 1e-6, "recall={}", pr.recall);
    }

    #[test]
    fn edge_precision_recall_filters_by_kind() {
        let mut golden = RelatedComponentsFile::default();
        golden
            .add_edge(edge(EdgeKind::DependsOn, LifecycleScope::Build, "a", "b"))
            .unwrap();
        golden
            .add_edge(edge(EdgeKind::Tests, LifecycleScope::Test, "a", "b"))
            .unwrap();
        let mut tool = RelatedComponentsFile::default();
        tool.add_edge(edge(EdgeKind::DependsOn, LifecycleScope::Build, "a", "b"))
            .unwrap();
        let pr = edge_precision_recall(&golden, &tool, Some(EdgeKind::Tests), None);
        assert_eq!(
            pr.precision, 1.0,
            "no tool edges of this kind => precision 1"
        );
        assert_eq!(pr.recall, 0.0, "one golden edge of this kind, missed");
    }

    #[test]
    fn identifier_stability_of_identical_runs_is_one() {
        let a = file_of(vec![component("a", "pkg-a"), component("b", "pkg-b")]);
        let b = a.clone();
        assert_eq!(identifier_stability(&a, &b), 1.0);
    }

    #[test]
    fn identifier_stability_penalises_missing_id_in_new_run() {
        let a = file_of(vec![component("a", "pkg-a"), component("b", "pkg-b")]);
        let b = file_of(vec![component("a", "pkg-a")]);
        assert_eq!(identifier_stability(&a, &b), 0.5);
    }

    fn write_triple(
        dir: &std::path::Path,
        components: &ComponentsFile,
        related: &RelatedComponentsFile,
    ) {
        atlas_index::save_components_atomic(&dir.join("components.yaml"), components).unwrap();
        component_ontology::save_atomic(&dir.join("related-components.yaml"), related).unwrap();
    }

    #[test]
    fn run_diff_of_same_dir_is_empty() {
        let base = TempDir::new().unwrap();
        let candidate = TempDir::new().unwrap();
        let components = file_of(vec![component("a", "pkg-a")]);
        let related = RelatedComponentsFile::default();
        write_triple(base.path(), &components, &related);
        write_triple(candidate.path(), &components, &related);

        let report = run_diff(base.path(), candidate.path()).unwrap();
        assert!(report.is_empty(), "unexpected deltas: {report:?}");
    }

    #[test]
    fn run_diff_flags_added_component() {
        let base = TempDir::new().unwrap();
        let candidate = TempDir::new().unwrap();
        let before = file_of(vec![component("a", "pkg-a")]);
        let after = file_of(vec![component("a", "pkg-a"), component("b", "pkg-b")]);
        let related = RelatedComponentsFile::default();
        write_triple(base.path(), &before, &related);
        write_triple(candidate.path(), &after, &related);

        let report = run_diff(base.path(), candidate.path()).unwrap();
        assert_eq!(report.added_components, vec!["b".to_string()]);
        assert!(report.removed_components.is_empty());
    }

    #[test]
    fn run_diff_flags_modified_kind() {
        let base = TempDir::new().unwrap();
        let candidate = TempDir::new().unwrap();
        let before = file_of(vec![component("a", "pkg-a")]);
        let mut mutated = component("a", "pkg-a");
        mutated.kind = "rust-binary".into();
        let after = file_of(vec![mutated]);
        let related = RelatedComponentsFile::default();
        write_triple(base.path(), &before, &related);
        write_triple(candidate.path(), &after, &related);

        let report = run_diff(base.path(), candidate.path()).unwrap();
        assert_eq!(report.modified_components.len(), 1);
        assert!(report.modified_components[0]
            .fields
            .contains(&"kind".to_string()));
    }

    #[test]
    fn run_diff_detects_identifier_change_via_shared_content_sha() {
        let base = TempDir::new().unwrap();
        let candidate = TempDir::new().unwrap();
        // Same content_sha, different id — the id rename.
        let mut before_c = component("old-id", "pkg");
        before_c.path_segments[0].content_sha = "shared".into();
        let before = file_of(vec![before_c]);
        let mut after_c = component("new-id", "pkg");
        after_c.path_segments[0].content_sha = "shared".into();
        let after = file_of(vec![after_c]);
        let related = RelatedComponentsFile::default();
        write_triple(base.path(), &before, &related);
        write_triple(candidate.path(), &after, &related);

        let report = run_diff(base.path(), candidate.path()).unwrap();
        assert_eq!(report.identifier_changes.len(), 1);
        assert_eq!(report.identifier_changes[0].prior_id, "old-id");
        assert_eq!(report.identifier_changes[0].new_id, "new-id");
    }
}
