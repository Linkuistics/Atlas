//! Path-dep root expansion to fixed point (PR-4 of Atlas vNext Phase 1).
//!
//! Atlas vNext is multi-root: a Ravel-Lite checkout that path-deps
//! `../atlas-contracts/...` should index both repositories as peer
//! roots so `components.yaml` can name components from each. PR-3
//! plumbed the `Vec<PathBuf>` shape; this module fills it.
//!
//! [`expand_roots`] takes the primary root (the directory `atlas
//! index` was invoked from) and walks the path-deps in every
//! reachable `Cargo.toml` under it. For each path-dep target whose
//! resolved path lies *outside* the primary root, the algorithm walks
//! up the directory tree looking for an enclosing Cargo workspace
//! manifest. If found, that workspace's directory is the new peer
//! root; otherwise the target's own crate directory is the peer root.
//! The walk iterates over every newly-discovered peer until no new
//! roots are added — a textbook fixed-point.
//!
//! Cycle handling: `crate-a → crate-b → crate-a` is silently
//! terminated by a `BTreeSet<PathBuf>` of visited roots. A `warning:`
//! line on stderr names both endpoints of the cycle. The CLI continues
//! — escalating to a hard error is reserved for Phase 2 if real-world
//! experience demands it (plan §6 risk row "Cross-tree path-dep
//! cycles").
//!
//! Phase 1 only handles `Cargo.toml`. `package.json`, `pyproject.toml`,
//! and friends are skipped — Phase 2 adds npm-workspace support.

use std::collections::{BTreeSet, VecDeque};
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::manifest_parse::{
    extract_csproj_path_deps, extract_mix_exs_path_deps, extract_path_deps,
    extract_pubspec_path_deps, extract_pyproject_path_deps,
};

/// Expand `primary` into the full set of roots reachable via Cargo
/// path-deps. The returned Vec always begins with the canonicalised
/// primary; peer roots follow in discovery order. Cycles terminate
/// silently after emitting a `warning:` line on stderr.
///
/// Walk semantics:
///
/// - Start a queue with `[primary']` and a visited set `{primary'}`
///   (where `primary'` is the canonicalised primary root).
/// - For each root popped from the queue, enumerate every `Cargo.toml`
///   under it (recursive walk, skipping `target/` and `.git/`).
/// - For each path-dep declared in those manifests, resolve the path
///   relative to its manifest's parent directory and canonicalise it.
///   A canonicalisation failure (target not yet checked out, broken
///   symlink) is silently skipped — that's data, not error.
/// - If the resolved path is inside any already-known root, skip — its
///   contents are covered by that root's L1 walk.
/// - Otherwise walk upwards looking for a `[workspace]` manifest. If
///   found, that's the new peer root; if not, the target's enclosing
///   crate directory is the new peer root.
/// - If the candidate is in `visited`, emit the cycle warning and
///   skip; otherwise add it to the result, the queue, and `visited`.
///
/// The recursion order does not matter for correctness; each
/// manifest's path-deps are sorted by manifest path before walking so
/// the discovery order — and the resulting `Vec` order — is stable
/// across runs.
pub fn expand_roots(primary: &Path) -> Result<Vec<PathBuf>> {
    let mut stderr = std::io::stderr();
    expand_roots_with_warnings(primary, &mut stderr)
}

/// Same contract as [`expand_roots`], but warning lines (cycle
/// detection) go to a caller-supplied writer instead of stderr.
/// Tests use this to assert on the exact warning text without spawning
/// a subprocess; production code should call [`expand_roots`].
pub fn expand_roots_with_warnings(
    primary: &Path,
    warnings: &mut dyn Write,
) -> Result<Vec<PathBuf>> {
    let primary_canonical = primary
        .canonicalize()
        .with_context(|| format!("failed to canonicalize primary root {}", primary.display()))?;

    let mut visited: BTreeSet<PathBuf> = BTreeSet::new();
    visited.insert(primary_canonical.clone());

    let mut result: Vec<PathBuf> = vec![primary_canonical.clone()];
    let mut queue: VecDeque<PathBuf> = VecDeque::from([primary_canonical]);

    while let Some(root) = queue.pop_front() {
        // Cargo + pyproject + mix.exs manifests: each contributes
        // path-deps that can route to peer roots. Phase 2 PR-3 added
        // the pyproject branch; PR-8 adds mix.exs. The cycle /
        // inside-known-root logic below is language-agnostic.
        let manifests = enumerate_path_dep_manifests(&root);
        for manifest in manifests {
            let Ok(contents) = std::fs::read_to_string(&manifest) else {
                continue;
            };
            let manifest_dir = match manifest.parent() {
                Some(p) => p.to_path_buf(),
                None => continue,
            };

            let basename = manifest
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default();
            let path_deps: Vec<PathBuf> = if basename == "Cargo.toml" {
                extract_path_deps(&contents)
            } else if basename == "pyproject.toml" {
                extract_pyproject_path_deps(&contents)
            } else if basename == "pubspec.yaml" {
                extract_pubspec_path_deps(&contents)
            } else if basename == "mix.exs" {
                extract_mix_exs_path_deps(&contents)
            } else if basename.ends_with(".csproj") {
                extract_csproj_path_deps(&contents)
            } else {
                continue;
            };
            for rel in path_deps {
                let target = manifest_dir.join(&rel);
                let Ok(target_canonical) = target.canonicalize() else {
                    // Target may not be checked out yet — that's not an
                    // error, just skip. Emitting a debug log here is
                    // overkill for Phase 1.
                    continue;
                };

                let candidate = enclosing_manifest_root(&target_canonical);
                let Ok(candidate_canonical) = candidate.canonicalize() else {
                    continue;
                };

                // Cycle detection: the candidate root is already in the
                // visited set. Two flavours, both indicate a cycle in
                // the path-dep graph:
                //
                //  - Exact-equality: the candidate is one of the roots
                //    we've already walked. e.g. `peer-a` path-deps to
                //    `peer-b/x`, then `peer-b` path-deps to a sibling
                //    that walks-up to peer-a. The cycle is between two
                //    discovered peers.
                //
                //  - Inside-known-root: the candidate is a descendant
                //    of an already-discovered root. e.g. peer-b
                //    path-deps to a workspace member of peer-a; the
                //    walk-up returns peer-a (already in `result`).
                //    The defence covers `is_inside_any` since
                //    `result` may include strict ancestors of the
                //    candidate.
                //
                // Both branches are silent skips structurally — no new
                // root added, no recursion. We emit the warning when
                // the candidate equals (or is enclosed by) a visited
                // root AND the manifest declaring the path-dep is on a
                // peer (not the primary's own walk into a known root,
                // which is benign and common).
                if visited.contains(&candidate_canonical) {
                    // The "the manifest is itself on a peer that's
                    // visited" check distinguishes a real cycle (peer
                    // X path-deps peer Y, peer Y path-deps back to X)
                    // from the benign case where a primary-rooted
                    // crate path-deps a sibling that walks up to the
                    // primary itself.
                    if is_inside_any(&manifest, &result[1..]) {
                        let _ = writeln!(
                            warnings,
                            "warning: path-dep cycle detected: {} <-> {}",
                            manifest.display(),
                            candidate_canonical.display()
                        );
                    }
                    continue;
                }

                // Defence in depth: the candidate is a strict
                // descendant of a known root (e.g. a workspace member
                // of the primary that a peer's path-dep aliased
                // through). Not a cycle, just already-covered — silent
                // skip.
                if is_inside_any(&candidate_canonical, &result) {
                    continue;
                }

                visited.insert(candidate_canonical.clone());
                result.push(candidate_canonical.clone());
                queue.push_back(candidate_canonical);
            }
        }
    }

    Ok(result)
}

/// Walk up from `start` looking for the enclosing Cargo workspace
/// manifest. Returns the workspace's directory if one is found;
/// otherwise the directory containing `start` itself if `start` is a
/// path to (or inside) a Cargo crate. The fall-through case — `start`
/// is some random directory with no Cargo manifest above or inside —
/// returns `start` unchanged; the caller already canonicalised so
/// every return value is absolute.
///
/// `start` may be either a directory or a file; in the file case the
/// walk begins at the file's parent.
fn enclosing_manifest_root(start: &Path) -> PathBuf {
    // If `start` is a file, work from its parent. Otherwise work from
    // `start` directly. (Path::is_dir / is_file follow symlinks; if
    // the caller already canonicalised, the answer is stable.)
    let mut cursor: PathBuf = if start.is_dir() {
        start.to_path_buf()
    } else {
        start
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| start.to_path_buf())
    };

    // The crate-root candidate is the innermost directory that
    // contains a `Cargo.toml`; the workspace-root candidate is the
    // innermost such directory whose `Cargo.toml` declares
    // `[workspace]`. Phase 1 returns the workspace if it exists, else
    // the crate.
    //
    // Innermost-wins matches Cargo's own resolution rule: when crate
    // `X` walks up looking for an enclosing workspace, the *first*
    // `[workspace]` ancestor binds. Two independently-versioned
    // workspaces under a common parent (e.g. a monorepo umbrella that
    // happens to also be a workspace) must therefore stay separate —
    // the inner workspace owns its members, the outer does not see
    // through.
    let mut crate_root: Option<PathBuf> = None;
    let mut workspace_root: Option<PathBuf> = None;
    // Phase 2 PR-3: a Python path-dep target may resolve to a
    // directory containing a `pyproject.toml` rather than a
    // `Cargo.toml`. Treat it as a "crate root" candidate (no workspace
    // shape — Python's analogue is the project root itself).
    let mut python_root: Option<PathBuf> = None;
    // Phase 2 PR-6: a C# path-dep target may resolve to a directory
    // containing a `*.csproj`. Treat it as a "crate root" candidate —
    // no solution-level workspace shape is walked here.
    let mut csharp_root: Option<PathBuf> = None;
    // Phase 2 PR-7: a Dart path-dep target may resolve to a directory
    // containing a `pubspec.yaml`. Treat it the same as a pyproject root.
    let mut dart_root: Option<PathBuf> = None;
    // Phase 2 PR-8: an Elixir path-dep target may resolve to a directory
    // containing a `mix.exs`. Treat it the same as a pyproject root.
    let mut elixir_root: Option<PathBuf> = None;

    loop {
        let manifest = cursor.join("Cargo.toml");
        if manifest.is_file() {
            if let Ok(contents) = std::fs::read_to_string(&manifest) {
                let shape = crate::manifest_parse::parse_cargo_toml(&contents);
                if shape.has_workspace_section && workspace_root.is_none() {
                    workspace_root = Some(cursor.clone());
                }
                if shape.has_package_section && crate_root.is_none() {
                    crate_root = Some(cursor.clone());
                }
            }
        }
        let pyproject = cursor.join("pyproject.toml");
        if pyproject.is_file() && python_root.is_none() {
            python_root = Some(cursor.clone());
        }
        // A directory containing any `*.csproj` is a C# project root.
        if csharp_root.is_none() {
            let has_csproj = std::fs::read_dir(&cursor)
                .ok()
                .map(|entries| {
                    entries.flatten().any(|e| {
                        e.file_name()
                            .to_str()
                            .map(|n| n.ends_with(".csproj"))
                            .unwrap_or(false)
                    })
                })
                .unwrap_or(false);
            if has_csproj {
                csharp_root = Some(cursor.clone());
            }
        }
        let pubspec = cursor.join("pubspec.yaml");
        if pubspec.is_file() && dart_root.is_none() {
            dart_root = Some(cursor.clone());
        }
        let mix_exs = cursor.join("mix.exs");
        if mix_exs.is_file() && elixir_root.is_none() {
            elixir_root = Some(cursor.clone());
        }
        match cursor.parent() {
            Some(parent) if parent != cursor => cursor = parent.to_path_buf(),
            _ => break,
        }
    }

    workspace_root
        .or(crate_root)
        .or(python_root)
        .or(csharp_root)
        .or(dart_root)
        .or(elixir_root)
        .unwrap_or_else(|| start.to_path_buf())
}

/// Returns `true` when `path` is the same as, or a descendant of, any
/// of the directories in `roots`. All inputs must already be
/// canonicalised; the comparison is a literal prefix check.
fn is_inside_any(path: &Path, roots: &[PathBuf]) -> bool {
    roots.iter().any(|root| path.starts_with(root))
}

/// Recursively enumerate every path-dep-bearing manifest under
/// `root`, skipping `target/`, `node_modules/`, and `.git/`.
/// Manifests are returned in lexicographic order so the path-dep walk
/// is deterministic across runs.
///
/// Phase 2 PR-3 broadened this to include `pyproject.toml` alongside
/// `Cargo.toml` so Python path-deps participate in the cross-tree
/// fixed-point walk. Phase 2 PR-7 adds `pubspec.yaml` for Dart;
/// Phase 2 PR-8 adds `mix.exs` for Elixir. Future PRs may extend
/// with `package.json` (npm `file:` deps), etc.
///
/// This is a fresh filesystem walk rather than a Salsa-tracked query
/// because [`expand_roots`] runs once before the database is seeded —
/// the tracked-query plumbing is not yet available. The walk uses
/// `walkdir` semantics via `std::fs::read_dir` to avoid pulling in a
/// new dependency; the output volume (one entry per manifest in the
/// tree) is small enough that the naive walk is comfortable.
fn enumerate_path_dep_manifests(root: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    walk(root, &mut out);
    out.sort();
    out
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_dir() {
            // Skip the obvious build-product directories. The full
            // gitignore-aware walk is overkill here — `target/` alone
            // can balloon a Rust workspace by 10x, and is the only
            // directory that materially matters for path-dep walking.
            if let Some(name) = path.file_name().and_then(|s| s.to_str()) {
                if matches!(
                    name,
                    "target"
                        | "node_modules"
                        | ".git"
                        | "__pycache__"
                        | ".venv"
                        | "venv"
                        | "_build"
                        | "deps"
                ) {
                    continue;
                }
            }
            walk(&path, out);
        } else if file_type.is_file() {
            if let Some(name) = path.file_name().and_then(|s| s.to_str()) {
                if matches!(
                    name,
                    "Cargo.toml" | "pyproject.toml" | "pubspec.yaml" | "mix.exs"
                ) || name.ends_with(".csproj")
                {
                    out.push(path);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write(path: &Path, contents: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, contents).unwrap();
    }

    #[test]
    fn enumerate_path_dep_manifests_finds_nested_manifests_in_lex_order() {
        let tmp = TempDir::new().unwrap();
        write(
            &tmp.path().join("Cargo.toml"),
            "[workspace]\nmembers = [\"a\", \"b\"]\n",
        );
        write(
            &tmp.path().join("a/Cargo.toml"),
            "[package]\nname = \"a\"\nversion = \"0.1.0\"\n",
        );
        write(
            &tmp.path().join("b/Cargo.toml"),
            "[package]\nname = \"b\"\nversion = \"0.1.0\"\n",
        );
        // target/ should be skipped:
        write(
            &tmp.path().join("target/zzz/Cargo.toml"),
            "[package]\nname = \"build-leftover\"\nversion = \"0.0.0\"\n",
        );

        let manifests = enumerate_path_dep_manifests(tmp.path());
        let names: Vec<_> = manifests
            .iter()
            .map(|p| {
                p.strip_prefix(tmp.path())
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/")
            })
            .collect();
        assert_eq!(names, vec!["Cargo.toml", "a/Cargo.toml", "b/Cargo.toml"]);
    }

    #[test]
    fn enclosing_manifest_root_finds_workspace_when_present() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        write(
            &root.join("Cargo.toml"),
            "[workspace]\nmembers = [\"crates/a\"]\n",
        );
        write(
            &root.join("crates/a/Cargo.toml"),
            "[package]\nname = \"a\"\nversion = \"0.1.0\"\n",
        );

        let result = enclosing_manifest_root(&root.join("crates/a"));
        assert_eq!(result, root);
    }

    #[test]
    fn enclosing_manifest_root_falls_back_to_crate_root_when_no_workspace() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        let crate_dir = root.join("standalone");
        write(
            &crate_dir.join("Cargo.toml"),
            "[package]\nname = \"standalone\"\nversion = \"0.1.0\"\n",
        );

        let result = enclosing_manifest_root(&crate_dir);
        assert_eq!(result, crate_dir);
    }
}
