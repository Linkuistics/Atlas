//! L7 graph-structural analysis — aggregates L6's proposed edges into an
//! [`EdgeGraph`] and computes four signals on it:
//!
//! - [`sccs`] — strongly-connected components of the directed projection
//!   (Tarjan via `petgraph`).
//! - [`cliques`] — maximal cliques of size ≥ `min_k` on the undirected
//!   projection (Bron–Kerbosch, unpivoted; graphs are small in practice).
//! - [`seam_density`] — per-component ratio of internal edges to external
//!   edges, where "internal" means both endpoints live under `id` in the
//!   L4 tree.
//! - [`modularity_hint`] — brute-force bisection over the internal
//!   subgraph; returns a hint only when a clean partition exists.
//!
//! ## Why brute-force bisection
//!
//! Design §4.1 L7 says "Louvain or a simpler spectral bisection — pick
//! whichever is easiest to implement correctly; document the choice".
//! Brute force on subgraphs with ≤ 12 nodes is O(4096·m), trivially
//! correct, and matches the real input distribution: a component's
//! internal subgraph is bounded by its child count, which is small.
//! A larger internal subgraph returns [`None`] — a modularity hint is
//! always advisory, never required.
//!
//! ## No Salsa tracking
//!
//! L7 queries are plain functions taking `&AtlasDatabase` (matching the
//! L5/L6 pattern). They indirectly depend on [`crate::all_proposed_edges`]
//! which is memoised via the LLM response cache; repeated L7 calls in a
//! revision compute on a cached edge set, and the aggregation itself is
//! cheap enough that adding another cache layer would be premature.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use component_ontology::Edge;
use petgraph::algo::tarjan_scc;
use petgraph::graphmap::DiGraphMap;

use crate::db::AtlasDatabase;
use crate::l4_tree::all_components;
use crate::l6_edges::all_proposed_edges;

/// Upper bound on internal-subgraph size for which [`modularity_hint`]
/// enumerates bipartitions. 12 keeps the worst case under 4k iterations
/// and still covers every realistic child-count.
const MODULARITY_BRUTE_FORCE_LIMIT: usize = 12;

/// Bisection quality threshold: the cross-partition edge count must be
/// at most this fraction of total internal edges for [`modularity_hint`]
/// to surface the partition.
const MODULARITY_ACCEPT_CROSS_RATIO: f32 = 0.2;

/// Aggregated view of L6's edge proposals. Holds the full component-id
/// set (so isolated components appear even when no edge touches them)
/// and the canonical edge list.
#[derive(Debug, Clone, Default)]
pub struct EdgeGraph {
    /// All known component ids, sorted. Includes ids with zero edges.
    pub nodes: Vec<String>,
    /// All canonicalised edges from [`all_proposed_edges`].
    pub edges: Vec<Edge>,
}

/// One strongly-connected component of the directed projection. Only
/// non-trivial SCCs (size > 1) are reported — singletons are noise.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scc {
    pub members: Vec<String>,
}

/// One maximal clique of size ≥ `min_k` on the undirected projection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Clique {
    pub members: Vec<String>,
}

/// Proposed internal partition of a component's descendants for L8 to
/// consider. `cross_edges` over `total_internal_edges` is the quality
/// signal: lower is better.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModularityHint {
    pub partition_a: Vec<String>,
    pub partition_b: Vec<String>,
    pub cross_edges: usize,
    pub total_internal_edges: usize,
}

// ---------------------------------------------------------------------
// Database-level entry points
// ---------------------------------------------------------------------

/// Aggregate edges from [`all_proposed_edges`] plus every live id from
/// [`all_components`] into an [`EdgeGraph`].
pub fn edge_graph(db: &AtlasDatabase) -> Arc<EdgeGraph> {
    let components = all_components(db);
    let nodes: Vec<String> = components
        .iter()
        .filter(|c| !c.deleted)
        .map(|c| c.id.clone())
        .collect();
    let edges: Vec<Edge> = (*all_proposed_edges(db)).clone();
    Arc::new(EdgeGraph { nodes, edges })
}

/// Non-trivial strongly-connected components of the directed edges.
pub fn sccs(db: &AtlasDatabase) -> Arc<Vec<Scc>> {
    let graph = edge_graph(db);
    Arc::new(sccs_of(&graph))
}

/// Maximal cliques of size ≥ `min_k` on the undirected projection.
pub fn cliques(db: &AtlasDatabase, min_k: u32) -> Arc<Vec<Clique>> {
    let graph = edge_graph(db);
    Arc::new(cliques_of(&graph, min_k))
}

/// Ratio of internal edges to external edges for the given component
/// id. "Internal" means both participants live at or under `id` in the
/// L4 tree. Returns [`f32::INFINITY`] if every touching edge is
/// internal (no seams) and `0.0` if there are no touching edges at all.
pub fn seam_density(db: &AtlasDatabase, id: String) -> f32 {
    let graph = edge_graph(db);
    let components = all_components(db);
    let inside = inside_set(&components, &id);
    if inside.is_empty() {
        return 0.0;
    }
    let (internal, external) = count_internal_external(&graph.edges, &inside);
    seam_density_raw(internal, external)
}

/// If the internal subgraph of `id` admits a clean bipartition, return
/// it; else [`None`]. Subgraphs larger than
/// [`MODULARITY_BRUTE_FORCE_LIMIT`] nodes always return [`None`] — the
/// hint is advisory, and a larger component is handled by the LLM
/// escalation path in L8.
pub fn modularity_hint(db: &AtlasDatabase, id: String) -> Option<ModularityHint> {
    let graph = edge_graph(db);
    let components = all_components(db);
    let inside = inside_set(&components, &id);
    // Exclude `id` itself: we're partitioning its descendants, not the
    // parent. A hint over `{id, descendants}` would conflate the parent
    // with a proposed sub-child.
    let mut descendants: Vec<String> = inside.iter().filter(|n| **n != id).cloned().collect();
    descendants.sort();
    find_bisection(&descendants, &graph.edges)
}

// ---------------------------------------------------------------------
// Pure algorithmic helpers. Exposed at module scope so unit tests can
// pass hand-built EdgeGraphs without standing up a full database.
// ---------------------------------------------------------------------

/// Tarjan's SCC on the directed projection. Symmetric-kind edges are
/// excluded — they carry no directional information and would force
/// every symmetric pair into a spurious SCC.
pub fn sccs_of(graph: &EdgeGraph) -> Vec<Scc> {
    let mut g: DiGraphMap<&str, ()> = DiGraphMap::new();
    for node in &graph.nodes {
        g.add_node(node.as_str());
    }
    for edge in &graph.edges {
        if !edge.kind.is_directed() {
            continue;
        }
        g.add_edge(
            edge.participants[0].as_str(),
            edge.participants[1].as_str(),
            (),
        );
    }

    let mut out: Vec<Scc> = Vec::new();
    for scc in tarjan_scc(&g) {
        if scc.len() < 2 {
            continue;
        }
        let mut members: Vec<String> = scc.iter().map(|s| s.to_string()).collect();
        members.sort();
        out.push(Scc { members });
    }
    out.sort_by(|a, b| a.members.cmp(&b.members));
    out
}

/// Bron–Kerbosch on the undirected projection. Directed and symmetric
/// edges alike contribute one undirected edge per pair.
pub fn cliques_of(graph: &EdgeGraph, min_k: u32) -> Vec<Clique> {
    let adj = undirected_adjacency(graph);
    let mut cliques: Vec<Vec<String>> = Vec::new();
    let p: BTreeSet<String> = adj.keys().cloned().collect();
    bron_kerbosch(&BTreeSet::new(), &p, &BTreeSet::new(), &adj, &mut cliques);

    let mut out: Vec<Clique> = cliques
        .into_iter()
        .filter(|c| c.len() as u32 >= min_k)
        .map(|members| Clique { members })
        .collect();
    out.sort_by(|a, b| {
        b.members
            .len()
            .cmp(&a.members.len())
            .then(a.members.cmp(&b.members))
    });
    out
}

fn undirected_adjacency(graph: &EdgeGraph) -> BTreeMap<String, BTreeSet<String>> {
    let mut adj: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for n in &graph.nodes {
        adj.entry(n.clone()).or_default();
    }
    for edge in &graph.edges {
        let a = &edge.participants[0];
        let b = &edge.participants[1];
        adj.entry(a.clone()).or_default().insert(b.clone());
        adj.entry(b.clone()).or_default().insert(a.clone());
    }
    adj
}

fn bron_kerbosch(
    r: &BTreeSet<String>,
    p: &BTreeSet<String>,
    x: &BTreeSet<String>,
    adj: &BTreeMap<String, BTreeSet<String>>,
    out: &mut Vec<Vec<String>>,
) {
    if p.is_empty() && x.is_empty() {
        let mut members: Vec<String> = r.iter().cloned().collect();
        members.sort();
        if !members.is_empty() {
            out.push(members);
        }
        return;
    }
    let mut p = p.clone();
    let mut x = x.clone();
    let candidates: Vec<String> = p.iter().cloned().collect();
    for v in candidates {
        let neighbors = adj.get(&v).cloned().unwrap_or_default();
        let mut new_r = r.clone();
        new_r.insert(v.clone());
        let new_p: BTreeSet<String> = p.intersection(&neighbors).cloned().collect();
        let new_x: BTreeSet<String> = x.intersection(&neighbors).cloned().collect();
        bron_kerbosch(&new_r, &new_p, &new_x, adj, out);
        p.remove(&v);
        x.insert(v);
    }
}

/// Ids at or under `id` in the parent/child tree, including `id` itself.
fn inside_set(components: &[atlas_index::ComponentEntry], id: &str) -> BTreeSet<String> {
    let mut inside: BTreeSet<String> = BTreeSet::new();
    if !components.iter().any(|c| c.id == id) {
        return inside;
    }
    inside.insert(id.to_string());
    // Iteratively expand until no new descendants appear. The tree is
    // a DAG (L4 enforces acyclicity) so this terminates.
    loop {
        let before = inside.len();
        for c in components {
            if let Some(parent) = &c.parent {
                if inside.contains(parent) && !inside.contains(&c.id) {
                    inside.insert(c.id.clone());
                }
            }
        }
        if inside.len() == before {
            break;
        }
    }
    inside
}

fn count_internal_external(edges: &[Edge], inside: &BTreeSet<String>) -> (usize, usize) {
    let mut internal = 0usize;
    let mut external = 0usize;
    for e in edges {
        let a_in = inside.contains(&e.participants[0]);
        let b_in = inside.contains(&e.participants[1]);
        match (a_in, b_in) {
            (true, true) => internal += 1,
            (true, false) | (false, true) => external += 1,
            _ => {}
        }
    }
    (internal, external)
}

fn seam_density_raw(internal: usize, external: usize) -> f32 {
    if internal == 0 && external == 0 {
        return 0.0;
    }
    if external == 0 {
        return f32::INFINITY;
    }
    internal as f32 / external as f32
}

/// Brute-force minimum-cut bisection over `nodes`. Returns the
/// partition that minimises cross edges, subject to both sides having
/// at least 2 members and the cross ratio being below
/// [`MODULARITY_ACCEPT_CROSS_RATIO`].
fn find_bisection(descendants: &[String], all_edges: &[Edge]) -> Option<ModularityHint> {
    if descendants.len() < 4 || descendants.len() > MODULARITY_BRUTE_FORCE_LIMIT {
        return None;
    }
    let descendant_set: BTreeSet<String> = descendants.iter().cloned().collect();
    let internal: Vec<&Edge> = all_edges
        .iter()
        .filter(|e| {
            descendant_set.contains(&e.participants[0])
                && descendant_set.contains(&e.participants[1])
        })
        .collect();
    if internal.is_empty() {
        return None;
    }
    let n = descendants.len();
    let total = internal.len();

    let mut best: Option<(u32, usize)> = None; // (mask, cross)
    let upper = 1u32 << n;
    for mask in 1..(upper - 1) {
        let count_a = mask.count_ones() as usize;
        let count_b = n - count_a;
        if count_a < 2 || count_b < 2 {
            continue;
        }
        // Avoid counting each bipartition twice by canonicalising on
        // "partition A contains descendant 0".
        if mask & 1 == 0 {
            continue;
        }
        let cross = count_cross(mask, descendants, &internal);
        match best {
            None => best = Some((mask, cross)),
            Some((_, current)) if cross < current => best = Some((mask, cross)),
            _ => {}
        }
    }

    let (mask, cross) = best?;
    if cross as f32 / total as f32 > MODULARITY_ACCEPT_CROSS_RATIO {
        return None;
    }
    let mut partition_a: Vec<String> = Vec::new();
    let mut partition_b: Vec<String> = Vec::new();
    for (i, id) in descendants.iter().enumerate() {
        if (mask >> i) & 1 == 1 {
            partition_a.push(id.clone());
        } else {
            partition_b.push(id.clone());
        }
    }
    partition_a.sort();
    partition_b.sort();
    Some(ModularityHint {
        partition_a,
        partition_b,
        cross_edges: cross,
        total_internal_edges: total,
    })
}

fn count_cross(mask: u32, descendants: &[String], edges: &[&Edge]) -> usize {
    let index_of = |id: &String| -> Option<usize> { descendants.iter().position(|n| n == id) };
    edges
        .iter()
        .filter(|e| {
            let Some(ai) = index_of(&e.participants[0]) else {
                return false;
            };
            let Some(bi) = index_of(&e.participants[1]) else {
                return false;
            };
            let a_in_a = (mask >> ai) & 1 == 1;
            let b_in_a = (mask >> bi) & 1 == 1;
            a_in_a != b_in_a
        })
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use component_ontology::{EdgeKind, EvidenceGrade, LifecycleScope};

    fn directed_edge(kind: EdgeKind, from: &str, to: &str) -> Edge {
        Edge {
            kind,
            lifecycle: LifecycleScope::Build,
            participants: vec![from.into(), to.into()],
            evidence_grade: EvidenceGrade::Strong,
            evidence_fields: vec!["test".into()],
            rationale: "test".into(),
        }
    }

    fn symmetric_edge(a: &str, b: &str) -> Edge {
        // co-implements is symmetric; store with sorted participants
        // per Edge::validate.
        let (first, second) = if a <= b { (a, b) } else { (b, a) };
        Edge {
            kind: EdgeKind::CoImplements,
            lifecycle: LifecycleScope::Design,
            participants: vec![first.into(), second.into()],
            evidence_grade: EvidenceGrade::Medium,
            evidence_fields: vec!["test".into()],
            rationale: "test".into(),
        }
    }

    // ---------------------------------------------------------------
    // SCC tests
    // ---------------------------------------------------------------

    #[test]
    fn three_node_cycle_produces_one_scc_of_size_three() {
        let graph = EdgeGraph {
            nodes: vec!["A".into(), "B".into(), "C".into()],
            edges: vec![
                directed_edge(EdgeKind::DependsOn, "A", "B"),
                directed_edge(EdgeKind::DependsOn, "B", "C"),
                directed_edge(EdgeKind::DependsOn, "C", "A"),
            ],
        };
        let out = sccs_of(&graph);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].members, vec!["A", "B", "C"]);
    }

    #[test]
    fn linear_directed_chain_has_no_scc() {
        // Singletons are filtered out; a DAG produces no SCCs in our
        // report.
        let graph = EdgeGraph {
            nodes: vec!["A".into(), "B".into(), "C".into()],
            edges: vec![
                directed_edge(EdgeKind::DependsOn, "A", "B"),
                directed_edge(EdgeKind::DependsOn, "B", "C"),
            ],
        };
        let out = sccs_of(&graph);
        assert!(
            out.is_empty(),
            "DAG must produce no non-trivial SCC, got {out:?}"
        );
    }

    #[test]
    fn symmetric_edges_do_not_introduce_false_sccs() {
        // Two nodes connected only by a symmetric (co-implements) edge:
        // no directed cycle → no SCC.
        let graph = EdgeGraph {
            nodes: vec!["A".into(), "B".into()],
            edges: vec![symmetric_edge("A", "B")],
        };
        let out = sccs_of(&graph);
        assert!(out.is_empty());
    }

    #[test]
    fn two_disjoint_cycles_produce_two_separate_sccs() {
        let graph = EdgeGraph {
            nodes: vec!["A".into(), "B".into(), "C".into(), "D".into()],
            edges: vec![
                directed_edge(EdgeKind::DependsOn, "A", "B"),
                directed_edge(EdgeKind::DependsOn, "B", "A"),
                directed_edge(EdgeKind::Calls, "C", "D"),
                directed_edge(EdgeKind::Calls, "D", "C"),
            ],
        };
        let out = sccs_of(&graph);
        assert_eq!(out.len(), 2);
    }

    // ---------------------------------------------------------------
    // Clique tests
    // ---------------------------------------------------------------

    #[test]
    fn k4_produces_one_clique_of_size_four() {
        // K4: every pair connected. Use symmetric edges (co-implements)
        // so Edge::validate is happy with sorted participants.
        let nodes = vec!["A", "B", "C", "D"];
        let mut edges = Vec::new();
        for i in 0..nodes.len() {
            for j in (i + 1)..nodes.len() {
                edges.push(symmetric_edge(nodes[i], nodes[j]));
            }
        }
        let graph = EdgeGraph {
            nodes: nodes.into_iter().map(String::from).collect(),
            edges,
        };
        let out = cliques_of(&graph, 3);
        assert_eq!(out.len(), 1, "expected exactly one maximal clique");
        assert_eq!(out[0].members, vec!["A", "B", "C", "D"]);
    }

    #[test]
    fn isolated_nodes_produce_no_cliques_at_min_k_two() {
        let graph = EdgeGraph {
            nodes: vec!["A".into(), "B".into(), "C".into()],
            edges: Vec::new(),
        };
        let out = cliques_of(&graph, 2);
        assert!(out.is_empty(), "no edges → no cliques ≥ 2, got {out:?}");
    }

    #[test]
    fn directed_edges_contribute_to_undirected_clique_projection() {
        // A,B,C mutually directed: projects to K3 undirected.
        let graph = EdgeGraph {
            nodes: vec!["A".into(), "B".into(), "C".into()],
            edges: vec![
                directed_edge(EdgeKind::DependsOn, "A", "B"),
                directed_edge(EdgeKind::DependsOn, "B", "C"),
                directed_edge(EdgeKind::DependsOn, "A", "C"),
            ],
        };
        let out = cliques_of(&graph, 3);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].members, vec!["A", "B", "C"]);
    }

    // ---------------------------------------------------------------
    // Seam-density / bisection tests
    // ---------------------------------------------------------------

    #[test]
    fn seam_density_is_zero_when_all_edges_are_external() {
        // One "inside" node and one "outside" node, connected.
        let inside: BTreeSet<String> = ["A".to_string()].into_iter().collect();
        let edges = vec![directed_edge(EdgeKind::DependsOn, "A", "X")];
        let (internal, external) = count_internal_external(&edges, &inside);
        assert_eq!((internal, external), (0, 1));
        assert_eq!(seam_density_raw(internal, external), 0.0);
    }

    #[test]
    fn seam_density_reports_infinity_when_no_edges_leave() {
        let inside: BTreeSet<String> = ["A".to_string(), "B".to_string()].into_iter().collect();
        let edges = vec![directed_edge(EdgeKind::DependsOn, "A", "B")];
        let (internal, external) = count_internal_external(&edges, &inside);
        assert_eq!((internal, external), (1, 0));
        assert_eq!(seam_density_raw(internal, external), f32::INFINITY);
    }

    #[test]
    fn seam_density_balances_internal_over_external() {
        let inside: BTreeSet<String> = ["A".into(), "B".into()].into_iter().collect();
        let edges = vec![
            directed_edge(EdgeKind::DependsOn, "A", "B"), // internal
            directed_edge(EdgeKind::DependsOn, "A", "X"), // external
            directed_edge(EdgeKind::DependsOn, "B", "Y"), // external
        ];
        let (internal, external) = count_internal_external(&edges, &inside);
        assert_eq!((internal, external), (1, 2));
        assert!((seam_density_raw(internal, external) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn two_dense_blocks_with_single_bridge_yield_modularity_hint() {
        // Internal subgraph: {A,B,C} as K3, {D,E,F} as K3, single
        // edge B↔D bridges the two blocks. Descendants of "parent".
        let descendants = vec![
            "A".to_string(),
            "B".to_string(),
            "C".to_string(),
            "D".to_string(),
            "E".to_string(),
            "F".to_string(),
        ];
        let edges = vec![
            // Block 1
            symmetric_edge("A", "B"),
            symmetric_edge("A", "C"),
            symmetric_edge("B", "C"),
            // Block 2
            symmetric_edge("D", "E"),
            symmetric_edge("D", "F"),
            symmetric_edge("E", "F"),
            // Single bridge
            symmetric_edge("B", "D"),
        ];
        let hint = find_bisection(&descendants, &edges).expect("expected bisection");
        assert_eq!(hint.cross_edges, 1);
        assert_eq!(hint.total_internal_edges, 7);
        // The partition must separate {A,B,C} from {D,E,F} (in either
        // order). Check that every block's members cluster together.
        let a: BTreeSet<String> = hint.partition_a.iter().cloned().collect();
        let b: BTreeSet<String> = hint.partition_b.iter().cloned().collect();
        let block1: BTreeSet<String> = ["A", "B", "C"].into_iter().map(String::from).collect();
        let block2: BTreeSet<String> = ["D", "E", "F"].into_iter().map(String::from).collect();
        assert!(
            (a == block1 && b == block2) || (a == block2 && b == block1),
            "partitions must separate the two blocks: got a={a:?}, b={b:?}"
        );
    }

    #[test]
    fn densely_connected_single_block_returns_no_modularity_hint() {
        // K4: every pair connected. No sensible bisection — every cut
        // crosses many edges.
        let descendants: Vec<String> = ["A", "B", "C", "D"].into_iter().map(String::from).collect();
        let mut edges = Vec::new();
        for i in 0..descendants.len() {
            for j in (i + 1)..descendants.len() {
                edges.push(symmetric_edge(&descendants[i], &descendants[j]));
            }
        }
        assert!(
            find_bisection(&descendants, &edges).is_none(),
            "K4 must not produce a modularity hint"
        );
    }

    #[test]
    fn too_few_descendants_yields_no_hint() {
        // n=3 is below the 4-node minimum for a meaningful bisection.
        let descendants: Vec<String> = ["A", "B", "C"].into_iter().map(String::from).collect();
        let edges = vec![symmetric_edge("A", "B"), symmetric_edge("B", "C")];
        assert!(find_bisection(&descendants, &edges).is_none());
    }

    #[test]
    fn too_many_descendants_yields_no_hint() {
        let descendants: Vec<String> = (0..(MODULARITY_BRUTE_FORCE_LIMIT + 1))
            .map(|i| format!("N{i}"))
            .collect();
        let edges = vec![symmetric_edge(&descendants[0], &descendants[1])];
        assert!(
            find_bisection(&descendants, &edges).is_none(),
            "input exceeding brute-force limit must skip bisection"
        );
    }

    // ---------------------------------------------------------------
    // inside_set: tree descendant walk
    // ---------------------------------------------------------------

    fn entry(id: &str, parent: Option<&str>) -> atlas_index::ComponentEntry {
        atlas_index::ComponentEntry {
            id: id.into(),
            parent: parent.map(String::from),
            kind: "rust-library".into(),
            lifecycle_roles: Vec::new(),
            language: None,
            build_system: None,
            role: None,
            path_segments: Vec::new(),
            manifests: Vec::new(),
            doc_anchors: Vec::new(),
            evidence_grade: EvidenceGrade::Strong,
            evidence_fields: Vec::new(),
            rationale: String::new(),
            deleted: false,
        }
    }

    #[test]
    fn inside_set_collects_id_and_all_descendants() {
        let comps = vec![
            entry("root", None),
            entry("child-1", Some("root")),
            entry("child-2", Some("root")),
            entry("grandchild", Some("child-1")),
            entry("other", None),
        ];
        let inside = inside_set(&comps, "root");
        let expected: BTreeSet<String> = ["root", "child-1", "child-2", "grandchild"]
            .into_iter()
            .map(String::from)
            .collect();
        assert_eq!(inside, expected);
    }

    #[test]
    fn inside_set_returns_empty_for_unknown_id() {
        let comps = vec![entry("root", None)];
        assert!(inside_set(&comps, "nonexistent").is_empty());
    }

    #[test]
    fn inside_set_returns_singleton_for_leaf() {
        let comps = vec![entry("a", None), entry("b", None)];
        let inside = inside_set(&comps, "a");
        let expected: BTreeSet<String> = ["a".to_string()].into_iter().collect();
        assert_eq!(inside, expected);
    }
}
