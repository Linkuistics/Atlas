//! L6 composition edges from Docker Compose files (PR-11).
//!
//! ## Intent
//!
//! [`composition_edges_from_compose`] produces `bundled-into` and
//! `deployed-with` edges deterministically from every
//! `kind: compose-orchestration` component in the workspace.
//!
//! ## Algorithm (per design §4 PR-11)
//!
//! For each compose-orchestration component:
//!
//! 1. Find the compose file on disk (`docker-compose.yml` et al.).
//! 2. Parse it via
//!    [`atlas_analyzers::compose_classifier::parse_compose`].
//! 3. For each service:
//!    - **`image:` declared** — matched against a live `kind: docker-image`
//!      component by id leaf; falls back to a synthesised `external-<slug>` id
//!      when no local docker-image matches. Emits `bundled-into` from the
//!      resolved source to the orchestration component.
//!    - **`build:` declared** — resolve the Dockerfile path; emit a
//!      `bundled-into` from the inferred source component (same
//!      path-prefix lookup as `l6_composition.rs`) to the
//!      orchestration component.
//! 4. Between every pair of services in the same compose file →
//!    `deployed-with` (symmetric, lifecycle: deploy).
//!
//! ## External-component id scheme
//!
//! External images that have no corresponding `docker-image` component
//! in the workspace are represented by an id of the form
//! `external-<slug>` where `<slug>` is a simplified form of the image
//! reference (colons and slashes replaced with hyphens) truncated to
//! 64 characters. The prefix `external-` keeps the id valid (component
//! ids must start with a letter or digit; the `external-` prefix
//! guarantees that).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use atlas_analyzers::compose_classifier::{parse_compose, ComposeShape};
use atlas_index::ComponentEntry;
use component_ontology::{Edge, EdgeKind, EvidenceGrade, LifecycleScope};

use crate::db::AtlasDatabase;
use crate::l1_queries::file_content;
use crate::l4_tree::all_components;
use crate::roots::best_root_for;

/// Canonical compose filenames probed in preference order.
const COMPOSE_FILENAMES: &[&str] = &[
    "docker-compose.yml",
    "docker-compose.yaml",
    "compose.yml",
    "compose.yaml",
];

/// Build every deterministic composition edge implied by Compose files in
/// the current workspace.  Returns an empty `Vec` when there are no
/// `kind: compose-orchestration` components.
///
/// The result is **not** canonicalised — the caller (L6) feeds it through
/// [`crate::l6_edges::canonicalise_edges`] alongside the Dockerfile batch
/// and the LLM batch.
pub fn composition_edges_from_compose(db: &AtlasDatabase) -> Vec<Edge> {
    let components = all_components(db);
    let live: Vec<&ComponentEntry> = components.iter().filter(|c| !c.deleted).collect();
    let workspace = db.workspace();
    let roots = workspace.roots(db as &dyn salsa::Database).clone();

    // Pre-compute segment dirs for path-prefix resolution of `build:`
    // contexts — same table as in `l6_composition.rs`.
    let segment_dirs: Vec<(String, PathBuf)> = build_component_segment_dirs(&live, &roots);

    // Build a lookup table of docker-image components so we can match
    // `image:` references without a full linear scan per service.
    let docker_images: Vec<DockerImageEntry> = live
        .iter()
        .filter(|c| c.kind == "docker-image")
        .map(|c| DockerImageEntry {
            id: c.id.as_str().to_string(),
            leaf: component_id_leaf(c.id.as_str()).to_string(),
        })
        .collect();

    let mut out: Vec<Edge> = Vec::new();

    for compose_component in &live {
        if compose_component.kind != "compose-orchestration" {
            continue;
        }
        let Some(compose_dir) = absolute_component_dir(compose_component, &roots) else {
            continue;
        };

        // Find and parse the compose file.
        let Some((filename, shape)) = load_compose_shape(db, &compose_dir) else {
            continue;
        };

        let orchestration_id = compose_component.id.as_str().to_string();

        emit_edges_for_compose(
            &shape,
            &orchestration_id,
            &compose_dir,
            &filename,
            &docker_images,
            &segment_dirs,
            &mut out,
        );
    }

    out
}

/// Emit all edges for one compose file.
fn emit_edges_for_compose(
    shape: &ComposeShape,
    orchestration_id: &str,
    compose_dir: &Path,
    filename: &str,
    docker_images: &[DockerImageEntry],
    segment_dirs: &[(String, PathBuf)],
    out: &mut Vec<Edge>,
) {
    // Collect the source ids that are bundled into this orchestration so
    // we can emit `deployed-with` between every pair afterwards.
    let mut bundled_source_ids: BTreeSet<String> = BTreeSet::new();

    for service in &shape.services {
        let svc_name = &service.name;

        // ── image: ────────────────────────────────────────────────────
        if let Some(image_ref) = &service.image {
            let source_id = resolve_image_to_component(image_ref, docker_images);
            if source_id != orchestration_id {
                let edge = Edge {
                    kind: EdgeKind::BundledInto,
                    lifecycle: LifecycleScope::Deploy,
                    participants: vec![source_id.clone(), orchestration_id.to_string()],
                    evidence_grade: EvidenceGrade::Strong,
                    evidence_fields: vec![format!("{}:services.{}.image", filename, svc_name)],
                    rationale: format!(
                        "Compose service `{svc_name}` uses image `{image_ref}`; \
                         source component `{source_id}` bundled into \
                         orchestration `{orchestration_id}`"
                    ),
                };
                out.push(edge);
                bundled_source_ids.insert(source_id);
            }
        }

        // ── build: ────────────────────────────────────────────────────
        if let Some(build_context) = &service.build_context {
            // Resolve the build context to an absolute dir.
            let ctx_path = PathBuf::from(build_context);
            let abs_ctx = if ctx_path.is_absolute() {
                ctx_path
            } else {
                compose_dir.join(&ctx_path)
            };
            let abs_ctx = normalise_path(&abs_ctx);

            // Determine which Dockerfile lives in the build context.
            let dockerfile_rel = service.build_dockerfile.as_deref().unwrap_or("Dockerfile");
            let dockerfile_abs = abs_ctx.join(dockerfile_rel);
            let dockerfile_dir = dockerfile_abs.parent().unwrap_or(&abs_ctx).to_path_buf();

            // Resolve the Dockerfile directory to a component id via
            // the same path-prefix lookup as `l6_composition.rs`.
            if let Some(source_id) =
                path_prefix_lookup(&normalise_path(&dockerfile_dir), segment_dirs)
            {
                if source_id != orchestration_id {
                    let edge = Edge {
                        kind: EdgeKind::BundledInto,
                        lifecycle: LifecycleScope::Deploy,
                        participants: vec![source_id.clone(), orchestration_id.to_string()],
                        evidence_grade: EvidenceGrade::Strong,
                        evidence_fields: vec![format!("{}:services.{}.build", filename, svc_name)],
                        rationale: format!(
                            "Compose service `{svc_name}` builds from `{build_context}`; \
                             source component `{source_id}` bundled into \
                             orchestration `{orchestration_id}`"
                        ),
                    };
                    out.push(edge);
                    bundled_source_ids.insert(source_id);
                }
            }
        }
    }

    // `deployed-with`: every unordered pair of distinct service source
    // components bundled into this orchestration. The BTreeSet's natural
    // lex ordering ensures each pair is emitted exactly once with a < b,
    // satisfying the symmetric-edge canonicalisation rule up front.
    let sorted_sources: Vec<String> = bundled_source_ids.into_iter().collect();
    for i in 0..sorted_sources.len() {
        for j in (i + 1)..sorted_sources.len() {
            let a = &sorted_sources[i];
            let b = &sorted_sources[j];
            let edge = Edge {
                kind: EdgeKind::DeployedWith,
                lifecycle: LifecycleScope::Deploy,
                participants: vec![a.clone(), b.clone()],
                evidence_grade: EvidenceGrade::Strong,
                evidence_fields: vec![format!("{}:co-services", filename)],
                rationale: format!(
                    "Compose file `{filename}` (orchestration `{orchestration_id}`) \
                     co-deploys source components `{a}` and `{b}`"
                ),
            };
            out.push(edge);
        }
    }
}

// ─── image resolution ────────────────────────────────────────────────────────

/// Docker-image component entry for the image-reference lookup table.
struct DockerImageEntry {
    id: String,
    /// Leaf segment of the component id (after the last `/`).
    leaf: String,
}

/// Resolve a Docker image reference (e.g. `myrepo/web:1`, `postgres:15`)
/// to a component id.
///
/// Resolution strategy:
///
/// 1. **Local docker-image match by leaf** — if there is exactly one
///    `kind: docker-image` component whose id leaf matches the image
///    name (the portion before the `:` tag), return that component's
///    id. This handles the common case where the compose file references
///    an image that is built locally.
/// 2. **External-component fallback** — synthesise an `external-<slug>`
///    id. This is the safe option for public images like `postgres:15`
///    that have no counterpart in the workspace.
fn resolve_image_to_component(image_ref: &str, docker_images: &[DockerImageEntry]) -> String {
    // The image name is the portion before the `:` tag (or `@` digest).
    let base = image_ref
        .split(':')
        .next()
        .and_then(|s| s.split('@').next())
        .unwrap_or(image_ref);
    // Just the leaf of the image path (e.g. `web` from `myrepo/web`).
    let image_leaf = base.rsplit('/').next().unwrap_or(base);

    let mut matched: Option<&str> = None;
    let mut collision = false;
    for entry in docker_images {
        if entry.leaf == image_leaf {
            if matched.is_some() {
                collision = true;
                break;
            }
            matched = Some(&entry.id);
        }
    }
    if !collision {
        if let Some(id) = matched {
            return id.to_string();
        }
    }

    // Fallback: external-component.
    external_component_id(image_ref)
}

/// Synthesise an external-component id from an image reference.
///
/// The id is `external-<slug>` where `<slug>` is the image reference
/// with non-alphanumeric characters (except hyphens) replaced by
/// hyphens, truncated to 64 characters.  Leading/trailing hyphens are
/// stripped so the result is a valid component id segment.
fn external_component_id(image_ref: &str) -> String {
    let slug: String = image_ref
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .chars()
        .take(64)
        .collect();
    // Ensure the slug is non-empty even for a pathological input.
    let slug = if slug.is_empty() { "image" } else { &slug };
    format!("external-{slug}")
}

// ─── path utilities ──────────────────────────────────────────────────────────

/// Longest-prefix lookup: find the component whose segment dir is the
/// longest prefix of `abs_path`. Returns `None` when no segment dir
/// is a prefix.
fn path_prefix_lookup(abs_path: &Path, segment_dirs: &[(String, PathBuf)]) -> Option<String> {
    let mut best: Option<(&str, usize)> = None;
    for (id, dir) in segment_dirs {
        if abs_path.starts_with(dir) {
            let depth = dir.components().count();
            match best {
                Some((_, best_depth)) if best_depth >= depth => {}
                _ => best = Some((id.as_str(), depth)),
            }
        }
    }
    best.map(|(id, _)| id.to_string())
}

/// Extract the leaf segment of a slash-delimited component id.
fn component_id_leaf(id: &str) -> &str {
    match id.rfind('/') {
        Some(idx) => &id[idx + 1..],
        None => id,
    }
}

/// Build the `(component_id, absolute_segment_dir)` lookup table.
fn build_component_segment_dirs(
    components: &[&ComponentEntry],
    roots: &[PathBuf],
) -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    for c in components {
        for seg in &c.path_segments {
            let abs = absolute_under_any_root(&seg.path, roots);
            out.push((c.id.as_str().to_string(), abs));
        }
    }
    out
}

fn absolute_under_any_root(rel: &Path, roots: &[PathBuf]) -> PathBuf {
    if rel.is_absolute() {
        return rel.to_path_buf();
    }
    if let Some(root) = best_root_for(roots, rel) {
        return root.join(rel);
    }
    if let Some(first) = roots.first() {
        return first.join(rel);
    }
    rel.to_path_buf()
}

fn absolute_component_dir(component: &ComponentEntry, roots: &[PathBuf]) -> Option<PathBuf> {
    let seg = component.path_segments.first()?;
    Some(absolute_under_any_root(&seg.path, roots))
}

/// Load a compose file from `dir`, trying canonical filenames in order.
/// Returns `(filename, parsed-shape)` or `None` when nothing is found
/// or nothing parses successfully.
fn load_compose_shape(db: &AtlasDatabase, dir: &Path) -> Option<(String, ComposeShape)> {
    for name in COMPOSE_FILENAMES {
        let path = dir.join(name);
        if let Some(bytes) = file_content(db, &path) {
            if let Ok(text) = std::str::from_utf8(&bytes) {
                let shape = parse_compose(text);
                if !shape.services.is_empty() {
                    return Some(((*name).to_string(), shape));
                }
            }
        }
    }
    None
}

/// Resolve `..` and `.` components in a path without touching the
/// filesystem (mirrors the equivalent in `l6_composition.rs`).
fn normalise_path(path: &Path) -> PathBuf {
    use std::path::Component as C;
    let mut out: Vec<C> = Vec::new();
    for c in path.components() {
        match c {
            C::ParentDir => {
                let popped = out
                    .last()
                    .map(|c| matches!(c, C::Normal(_)))
                    .unwrap_or(false);
                if popped {
                    out.pop();
                } else {
                    out.push(c);
                }
            }
            C::CurDir => {}
            other => out.push(other),
        }
    }
    let mut buf = PathBuf::new();
    for c in out {
        buf.push(c.as_os_str());
    }
    buf
}

// ─── tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn external_component_id_converts_image_ref() {
        assert_eq!(external_component_id("postgres:15"), "external-postgres-15");
        assert_eq!(
            external_component_id("myrepo/web:1"),
            "external-myrepo-web-1"
        );
    }

    #[test]
    fn external_component_id_handles_registry_prefix() {
        let id = external_component_id("ghcr.io/foo/bar:latest");
        assert!(id.starts_with("external-"), "got: {id}");
        assert!(!id.contains('/'), "slashes must be replaced: {id}");
    }

    #[test]
    fn resolve_image_to_component_matches_docker_image_by_leaf() {
        let entries = vec![DockerImageEntry {
            id: "ws/web-image".to_string(),
            leaf: "web-image".to_string(),
        }];
        // Image ref `myrepo/web-image:1` → leaf `web-image` → matched.
        let got = resolve_image_to_component("myrepo/web-image:1", &entries);
        assert_eq!(got, "ws/web-image");
    }

    #[test]
    fn resolve_image_to_component_falls_back_to_external_on_collision() {
        let entries = vec![
            DockerImageEntry {
                id: "a/web".to_string(),
                leaf: "web".to_string(),
            },
            DockerImageEntry {
                id: "b/web".to_string(),
                leaf: "web".to_string(),
            },
        ];
        let got = resolve_image_to_component("myrepo/web:1", &entries);
        assert!(got.starts_with("external-"), "got: {got}");
    }

    #[test]
    fn resolve_image_to_component_external_for_unknown_image() {
        let got = resolve_image_to_component("postgres:15", &[]);
        assert_eq!(got, "external-postgres-15");
    }

    #[test]
    fn path_prefix_lookup_returns_longest_match() {
        let segs = vec![
            ("ws/app".to_string(), PathBuf::from("/ws/app")),
            ("ws/app/sub".to_string(), PathBuf::from("/ws/app/sub")),
        ];
        // The path `/ws/app/sub/file.rs` should match `/ws/app/sub`
        // (deeper = longer prefix).
        let got = path_prefix_lookup(Path::new("/ws/app/sub/file.rs"), &segs);
        assert_eq!(got.as_deref(), Some("ws/app/sub"));
    }

    #[test]
    fn normalise_path_resolves_double_dots() {
        let got = normalise_path(Path::new("/ws/compose/../app"));
        assert_eq!(got, PathBuf::from("/ws/app"));
    }
}
