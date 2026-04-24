//! Recognition of manifest filenames (e.g. `Cargo.toml`,
//! `package.json`). Centralised here so adding a new ecosystem touches
//! one place.

use std::path::Path;

/// Exact basenames that identify a manifest across many ecosystems.
const EXACT_MANIFEST_BASENAMES: &[&str] = &[
    "Cargo.toml",
    "package.json",
    "pyproject.toml",
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
/// pattern. Matching is by exact basename, plus the suffix rule
/// `*.nix` (which subsumes `flake.nix` but the explicit entry is kept
/// above as self-documentation for readers).
pub fn is_manifest_file(path: &Path) -> bool {
    let Some(basename) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    if EXACT_MANIFEST_BASENAMES.contains(&basename) {
        return true;
    }
    basename.ends_with(".nix")
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
}
