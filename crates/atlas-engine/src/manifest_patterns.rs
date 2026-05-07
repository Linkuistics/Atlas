//! Recognition of manifest filenames (e.g. `Cargo.toml`,
//! `package.json`). Centralised here so adding a new ecosystem touches
//! one place.

use std::path::Path;

/// Exact basenames that identify a manifest across many ecosystems.
const EXACT_MANIFEST_BASENAMES: &[&str] = &[
    "Cargo.toml",
    "package.json",
    // tsconfig.json is recognised as a manifest so the Phase 2 PR-1
    // TS/JS classifier sees it via `Target.manifests`. It is not the
    // primary manifest of a TypeScript package (that is `package.json`)
    // but its presence flips the kind from `javascript-package` to
    // `typescript-package`.
    "tsconfig.json",
    "pyproject.toml",
    // pubspec.yaml is the Dart/Flutter manifest. Recognised so the
    // Phase 2 PR-7 `dart-classifier` can inspect it via
    // `Target.manifests`. The root-expansion walker also reads it
    // for path-dep discovery (see `root_expansion.rs`).
    "pubspec.yaml",
    "go.mod",
    "setup.py",
    "Gemfile",
    "pom.xml",
    "build.gradle",
    "CMakeLists.txt",
    "Dockerfile",
    "flake.nix",
    "shard.yml",
    "mix.exs",
    "composer.json",
    "deno.json",
];

/// Return `true` when the file's basename matches any known manifest
/// pattern. Matching is by exact basename, plus:
///
/// - `*.nix` suffix rule (subsumes `flake.nix`; explicit entry kept above
///   for reader clarity).
/// - C# manifests: `*.csproj` (project files) and `*.sln` (solution
///   files). Both are recognised as component boundaries by the
///   `csharp-classifier` (PR-6).
/// - Docker Compose filename patterns: `docker-compose.yml`,
///   `docker-compose.yaml`, `docker-compose.*.yml`,
///   `docker-compose.*.yaml`, `compose.yml`, `compose.yaml`,
///   `compose.*.yml`, `compose.*.yaml`. Recognised here so the engine's
///   L1 manifest walk pre-loads them into the candidate's
///   `Target.manifests` for the `compose-classifier` (PR-11).
pub fn is_manifest_file(path: &Path) -> bool {
    let Some(basename) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    if EXACT_MANIFEST_BASENAMES.contains(&basename) {
        return true;
    }
    // *.nix subsumes flake.nix (kept explicitly above for
    // self-documentation).
    if basename.ends_with(".nix") {
        return true;
    }
    // C# manifests: *.csproj (project files) and *.sln (solution
    // files). Both are recognised as component boundaries by the
    // `csharp-classifier` (PR-6).
    if basename.ends_with(".csproj") || basename.ends_with(".sln") {
        return true;
    }
    is_compose_manifest_basename(basename)
}

/// Return `true` when `basename` matches one of the four canonical Docker
/// Compose filename patterns:
///
/// - `docker-compose.yml` / `docker-compose.yaml`
/// - `docker-compose.<override>.yml` / `docker-compose.<override>.yaml`
/// - `compose.yml` / `compose.yaml`
/// - `compose.<override>.yml` / `compose.<override>.yaml`
///
/// The function is pub(crate) so `compose_classifier.rs` can reuse it
/// without duplication.
pub(crate) fn is_compose_manifest_basename(basename: &str) -> bool {
    // Fast exact-match for the two canonical "bare" names first.
    if matches!(
        basename,
        "docker-compose.yml" | "docker-compose.yaml" | "compose.yml" | "compose.yaml"
    ) {
        return true;
    }
    // Override forms: `docker-compose.<something>.yml|yaml` and
    // `compose.<something>.yml|yaml`.  The <something> must be non-empty
    // so we do not accidentally match `docker-compose..yml`.
    for prefix in ["docker-compose.", "compose."] {
        if let Some(rest) = basename.strip_prefix(prefix) {
            if let Some(inner) = rest
                .strip_suffix(".yml")
                .or_else(|| rest.strip_suffix(".yaml"))
            {
                // `inner` is the override segment; must be non-empty and
                // must not contain a further `.` at the boundaries that
                // would make it look like the bare form again.
                if !inner.is_empty() {
                    return true;
                }
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn recognises_rust_manifest() {
        assert!(is_manifest_file(&PathBuf::from("foo/Cargo.toml")));
    }

    #[test]
    fn recognises_nix_files_by_suffix() {
        assert!(is_manifest_file(&PathBuf::from("flake.nix")));
        assert!(is_manifest_file(&PathBuf::from("shell.nix")));
        assert!(is_manifest_file(&PathBuf::from("default.nix")));
    }

    #[test]
    fn does_not_recognise_source_files() {
        assert!(!is_manifest_file(&PathBuf::from("src/lib.rs")));
        assert!(!is_manifest_file(&PathBuf::from("README.md")));
    }

    #[test]
    fn recognises_canonical_compose_files() {
        for name in &[
            "docker-compose.yml",
            "docker-compose.yaml",
            "compose.yml",
            "compose.yaml",
        ] {
            assert!(
                is_manifest_file(&PathBuf::from(name)),
                "{name} should be recognised as a manifest"
            );
        }
    }

    #[test]
    fn recognises_override_compose_files() {
        for name in &[
            "docker-compose.override.yml",
            "docker-compose.prod.yaml",
            "compose.dev.yml",
            "compose.ci.yaml",
        ] {
            assert!(
                is_manifest_file(&PathBuf::from(name)),
                "{name} should be recognised as a manifest"
            );
        }
    }

    #[test]
    fn does_not_recognise_malformed_compose_names() {
        // Double-dot edge cases and non-compose names.
        assert!(!is_manifest_file(&PathBuf::from("docker-compose..yml")));
        assert!(!is_manifest_file(&PathBuf::from("not-compose.yml")));
        assert!(!is_manifest_file(&PathBuf::from("compose")));
    }

    #[test]
    fn recognises_csharp_project_file() {
        assert!(is_manifest_file(&PathBuf::from("MyApp.csproj")));
        assert!(is_manifest_file(&PathBuf::from("src/MyApp.csproj")));
    }

    #[test]
    fn recognises_csharp_solution_file() {
        assert!(is_manifest_file(&PathBuf::from("MySolution.sln")));
        assert!(is_manifest_file(&PathBuf::from("workspace/MySolution.sln")));
    }
}
