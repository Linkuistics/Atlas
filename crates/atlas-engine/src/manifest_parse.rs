//! Minimal readers for the manifest shapes the deterministic
//! classifier looks at. `Cargo.toml` is parsed with the `toml` crate
//! and `package.json` with `serde_json` — in both cases a full parse
//! is cheaper and more reliable than hand-rolled scanning, and a
//! malformed document degrades to the default "all false" shape so
//! the classifier falls back to the LLM.
//!
//! [`extract_mix_exs_path_deps`] extracts Elixir `mix.exs` path-deps
//! via a pragmatic regex; preserved for future use (not currently called).

use std::path::PathBuf;

/// Facts lifted from a `Cargo.toml`. True for each table that exists
/// at the document root. Cargo's own spec defines these as top-level
/// tables (`[lib]`, `[[bin]]`, `[workspace]`), so a proper TOML parse
/// is the authoritative way to detect them — a hand-rolled line
/// scanner gets fooled by multiline strings, quoted keys, and
/// comment-in-string edge cases.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CargoTomlShape {
    pub has_lib_section: bool,
    pub has_bin_section: bool,
    pub has_workspace_section: bool,
    pub has_package_section: bool,
}

pub fn parse_cargo_toml(contents: &str) -> CargoTomlShape {
    let Ok(table) = contents.parse::<toml::Table>() else {
        return CargoTomlShape::default();
    };
    CargoTomlShape {
        has_lib_section: table.get("lib").is_some_and(toml::Value::is_table),
        has_bin_section: table.get("bin").is_some_and(toml::Value::is_array),
        has_workspace_section: table.get("workspace").is_some_and(toml::Value::is_table),
        has_package_section: table.get("package").is_some_and(toml::Value::is_table),
    }
}

/// Returns every path-dep target declared in a `Cargo.toml` manifest,
/// as paths relative to the manifest's parent directory (callers
/// canonicalise). Includes `[dependencies]`, `[dev-dependencies]`,
/// `[build-dependencies]`, and `[workspace.dependencies]`. Skips git
/// deps, registry deps, version-only specs.
///
/// On a malformed manifest the function returns an empty Vec rather
/// than erroring — the same degrade-to-default policy `parse_cargo_toml`
/// uses. Callers that walk path-deps should treat an opaque parse
/// failure as an empty result rather than aborting the entire walk.
///
/// Per-target tables (`[target.'cfg(...)'.dependencies]`) are not
/// walked in Phase 1 — Atlas vNext analysers do not yet cross
/// platform-conditional boundaries. Phase 2 may extend this.
pub fn extract_path_deps(contents: &str) -> Vec<PathBuf> {
    let Ok(table) = contents.parse::<toml::Table>() else {
        return Vec::new();
    };
    let mut out: Vec<PathBuf> = Vec::new();

    // Top-level dependency tables.
    for key in ["dependencies", "dev-dependencies", "build-dependencies"] {
        collect_path_deps_from_table(table.get(key), &mut out);
    }

    // Workspace inheritance: `[workspace.dependencies]` is a Cargo-2024
    // affordance that lifts member deps into the workspace root. PR-4
    // walks through these too because the path-dep edges they carry are
    // semantically identical to `[dependencies]` edges.
    if let Some(ws) = table.get("workspace").and_then(toml::Value::as_table) {
        collect_path_deps_from_table(ws.get("dependencies"), &mut out);
    }

    out
}

/// Walks a single dependency-table-shaped TOML value, pushing every
/// `path = "..."` value as a `PathBuf` onto `out`. Non-table specs and
/// path-less entries (registry / git deps) are skipped silently.
fn collect_path_deps_from_table(block: Option<&toml::Value>, out: &mut Vec<PathBuf>) {
    let Some(deps) = block.and_then(toml::Value::as_table) else {
        return;
    };
    for (_name, spec) in deps {
        let Some(spec_table) = spec.as_table() else {
            continue;
        };
        if let Some(toml::Value::String(p)) = spec_table.get("path") {
            out.push(PathBuf::from(p));
        }
    }
}

/// Returns every path-dep target declared in a `pyproject.toml`
/// manifest, as paths relative to the manifest's parent directory
/// (callers canonicalise). Recognised forms:
///
/// - `[tool.poetry.dependencies]` `name = { path = "..." }` —
///   Poetry's path-dep convention.
/// - `[tool.poetry.dev-dependencies]` and
///   `[tool.poetry.group.<name>.dependencies]` — Poetry's dev /
///   group dependency tables.
/// - `[tool.uv.sources]` `name = { path = "..." }` — uv's source map
///   layered on PEP-621 projects.
///
/// Phase 2 PR-3 introduces this for the cross-tree fixed-point walk
/// (mirroring the Cargo path-dep pattern). PEP 621's
/// `[project.dependencies]` itself is a string-array form that does
/// not standardise local-path dependencies, so it is intentionally
/// not consulted here; tools layered on top (uv, Hatch) carry the
/// path information in `tool.*` tables instead.
///
/// On a malformed manifest the function returns an empty Vec rather
/// than erroring — the same degrade-to-default policy `parse_cargo_toml`
/// uses.
pub fn extract_pyproject_path_deps(contents: &str) -> Vec<PathBuf> {
    let Ok(table) = contents.parse::<toml::Table>() else {
        return Vec::new();
    };
    let mut out: Vec<PathBuf> = Vec::new();
    if let Some(tool) = table.get("tool").and_then(toml::Value::as_table) {
        if let Some(poetry) = tool.get("poetry").and_then(toml::Value::as_table) {
            collect_path_deps_from_table(poetry.get("dependencies"), &mut out);
            collect_path_deps_from_table(poetry.get("dev-dependencies"), &mut out);
            if let Some(group_table) = poetry.get("group").and_then(toml::Value::as_table) {
                for (_group_name, group_value) in group_table {
                    if let Some(group_inner) = group_value.as_table() {
                        collect_path_deps_from_table(group_inner.get("dependencies"), &mut out);
                    }
                }
            }
        }
        if let Some(uv) = tool.get("uv").and_then(toml::Value::as_table) {
            collect_path_deps_from_table(uv.get("sources"), &mut out);
        }
    }
    out
}

/// Returns every `<ProjectReference Include="...">` path declared in a
/// `*.csproj` XML manifest, as paths relative to the manifest's parent
/// directory (callers canonicalise). This drives the cross-tree
/// path-dep fixed-point walk introduced in PR-6 for C# projects.
///
/// **`<PackageReference>` is intentionally excluded.** A
/// `<PackageReference Include="Microsoft.Extensions.Hosting"
/// Version="8.0.0" />` resolves through NuGet, not as a local source
/// tree. Including NuGet packages in the workspace-local path-dep walk
/// would require a NuGet resolver (Phase 3 scope). Only
/// `<ProjectReference>` carries a real relative filesystem path and
/// therefore participates here.
///
/// Parsing is deliberately minimal: a regex-based scanner extracts the
/// `Include="..."` value from `<ProjectReference ...>` tags. This is
/// the same Wave-3 pattern used by PR-8's `mix.exs` parser and avoids
/// pulling in an XML-parser dependency. Malformed XML or missing
/// attributes degrade to an empty Vec (same policy as
/// `extract_pyproject_path_deps`).
pub fn extract_csproj_path_deps(contents: &str) -> Vec<PathBuf> {
    // Match <ProjectReference Include="some/path/Foo.csproj" ...>
    // Case-insensitive for the element name; path value is captured.
    let re = regex::Regex::new(r#"(?i)<ProjectReference\s[^>]*Include\s*=\s*"([^"]+)""#);
    let Ok(re) = re else {
        return Vec::new();
    };
    re.captures_iter(contents)
        .filter_map(|cap| cap.get(1).map(|m| PathBuf::from(m.as_str())))
        .collect()
}

/// Returns every path-dep target declared in a `pubspec.yaml` manifest,
/// as paths relative to the manifest's parent directory (callers
/// canonicalise). Recognised form:
///
/// ```yaml
/// dependencies:
///   lib_a:
///     path: ../lib_a
///   http: ^0.13.0
/// ```
///
/// Both `dependencies:` and `dev_dependencies:` tables are walked.
/// Version-string, SDK, and git forms are skipped.
///
/// Phase 2 PR-7 introduces this for the cross-tree fixed-point walk
/// (mirroring the Cargo and Python path-dep patterns).
///
/// On a malformed manifest the function returns an empty Vec rather
/// than erroring — the same degrade-to-default policy `parse_cargo_toml`
/// uses.
pub fn extract_pubspec_path_deps(contents: &str) -> Vec<PathBuf> {
    let Ok(value): Result<serde_yaml::Value, _> = serde_yaml::from_str(contents) else {
        return Vec::new();
    };
    let Some(mapping) = value.as_mapping() else {
        return Vec::new();
    };
    let mut out: Vec<PathBuf> = Vec::new();
    for key in ["dependencies", "dev_dependencies"] {
        let Some(deps) = mapping.get(key) else {
            continue;
        };
        let Some(deps_map) = deps.as_mapping() else {
            continue;
        };
        for (_dep_name, dep_spec) in deps_map {
            let Some(spec_map) = dep_spec.as_mapping() else {
                continue;
            };
            if let Some(path_val) = spec_map.get("path") {
                if let Some(path_str) = path_val.as_str() {
                    out.push(PathBuf::from(path_str));
                }
            }
        }
    }
    out
}

/// Returns every path-dep target declared in an `info.rkt` manifest,
/// as paths relative to the manifest's parent directory (callers
/// canonicalise). Extracts string literals from the `deps` list that
/// look like paths (start with `.` or `/`). Registry package names
/// (bare strings without path separators) are excluded.
///
/// The Racket `info.rkt` s-expression syntax is parsed with the same
/// minimal reader shipped in `atlas-racket-analyzer::lib`. This
/// engine-side copy avoids taking a dep on the analyser crate (dep
/// arrows go from analyser → engine-types, not the other way). The
/// approach is a simple byte scan: find the `(define deps ...)` form
/// and collect string literal content.
///
/// On a malformed manifest the function returns an empty Vec.
pub fn extract_info_rkt_path_deps(contents: &str) -> Vec<PathBuf> {
    let bytes = contents.as_bytes();
    let len = bytes.len();
    let mut out: Vec<PathBuf> = Vec::new();

    // Scan for the `define deps` token sequence (preceded by `(`).
    // We look for the literal bytes `(define deps` in the file, then
    // extract string literals until the matching close paren.
    let mut i = 0;
    while i < len {
        // Skip line comment.
        if bytes[i] == b';' {
            while i < len && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        // Skip block comment `#| ... |#`.
        if i + 1 < len && bytes[i] == b'#' && bytes[i + 1] == b'|' {
            i += 2;
            while i + 1 < len {
                if bytes[i] == b'|' && bytes[i + 1] == b'#' {
                    i += 2;
                    break;
                }
                i += 1;
            }
            continue;
        }
        // Skip `#lang ...` lines.
        if bytes[i] == b'#' {
            while i < len && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        // Look for `(define deps`.
        if bytes[i] == b'(' {
            let rest = &contents[i..];
            // Quick substring test before paying for a full parse.
            if rest.starts_with("(define deps") {
                // Verify the char after `deps` is a delimiter.
                let after = rest.get("(define deps".len()..=("(define deps".len()));
                let is_delim = after.is_some_and(|s| {
                    s.chars()
                        .next()
                        .is_some_and(|c| c.is_whitespace() || c == ')')
                });
                if is_delim {
                    // Collect all string literals inside this form.
                    let mut depth: i32 = 0;
                    let mut j = i;
                    while j < len {
                        match bytes[j] {
                            b'(' | b'[' | b'{' => {
                                depth += 1;
                                j += 1;
                            }
                            b')' | b']' | b'}' => {
                                depth -= 1;
                                if depth == 0 {
                                    j += 1;
                                    break;
                                }
                                j += 1;
                            }
                            b'"' => {
                                j += 1;
                                let str_start = j;
                                while j < len {
                                    match bytes[j] {
                                        b'\\' => j += 2,
                                        b'"' => {
                                            let s = &contents[str_start..j];
                                            if s.starts_with('.') || s.starts_with('/') {
                                                out.push(PathBuf::from(s));
                                            }
                                            j += 1;
                                            break;
                                        }
                                        _ => j += 1,
                                    }
                                }
                            }
                            b';' => {
                                while j < len && bytes[j] != b'\n' {
                                    j += 1;
                                }
                            }
                            _ => j += 1,
                        }
                    }
                    i = j;
                    continue;
                }
            }
        }
        i += 1;
    }
    out
}

/// Facts lifted from a `package.json` by serde_json. Missing fields
/// degrade to `false`; malformed JSON degrades to an all-false shape,
/// which sends the classifier down the LLM fallback path.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PackageJsonShape {
    pub has_main: bool,
    pub has_exports: bool,
    pub has_bin: bool,
}

pub fn parse_package_json(contents: &str) -> PackageJsonShape {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(contents) else {
        return PackageJsonShape::default();
    };
    let Some(object) = value.as_object() else {
        return PackageJsonShape::default();
    };
    PackageJsonShape {
        has_main: object.get("main").is_some(),
        has_exports: object.get("exports").is_some(),
        has_bin: object.get("bin").is_some(),
    }
}

/// Extract path-dep targets from a `mix.exs` file. Recognises the
/// canonical form:
///
/// ```text
/// defp deps do
///   [
///     {:foo, path: "../foo"},
///     {:bar, "~> 1.0"},
///   ]
/// end
/// ```
///
/// `mix.exs` is Elixir source code, so a proper parse would require
/// an Elixir parser. A pragmatic regex (per §4 PR-8: "pragmatic
/// regex; mix.exs is Elixir code") covers the overwhelming majority
/// of real-world `deps/0` bodies. A regex cannot handle all edge
/// cases (nested brackets, dynamic path construction).
///
/// On a malformed manifest (or one with no path-deps) returns an
/// empty Vec — the same degrade-to-default policy used by
/// [`extract_path_deps`].
///
/// Preserved for future use; not currently called anywhere in the
/// engine post-Phase-5 PR-2 (the cross-tree fixed-point walk that
/// consumed this output was removed in that PR).
pub fn extract_mix_exs_path_deps(contents: &str) -> Vec<PathBuf> {
    let re = match regex::Regex::new(r#"path:\s*"([^"]+)""#) {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };
    re.captures_iter(contents)
        .map(|cap| PathBuf::from(&cap[1]))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cargo_toml_detects_lib_section() {
        let shape = parse_cargo_toml(
            "[package]\nname = \"x\"\nversion = \"0.1.0\"\n\n[lib]\npath = \"src/lib.rs\"\n",
        );
        assert!(shape.has_lib_section);
        assert!(!shape.has_bin_section);
        assert!(!shape.has_workspace_section);
    }

    #[test]
    fn cargo_toml_detects_bin_and_workspace_sections() {
        let shape = parse_cargo_toml(
            "[workspace]\nmembers = []\n[[bin]]\nname = \"tool\"\npath = \"src/main.rs\"\n",
        );
        assert!(shape.has_bin_section);
        assert!(shape.has_workspace_section);
    }

    #[test]
    fn cargo_toml_ignores_trailing_comment_on_header_line() {
        let shape = parse_cargo_toml(
            "[package]\nname = \"x\"\nversion = \"0.1.0\"\n[lib] # library crate\n",
        );
        assert!(shape.has_lib_section);
    }

    #[test]
    fn cargo_toml_does_not_match_header_inside_a_value() {
        let shape = parse_cargo_toml(
            "[package]\nname = \"x\"\nversion = \"0.1.0\"\ndescription = \"Says [lib] a lot\"\n",
        );
        assert!(!shape.has_lib_section);
    }

    #[test]
    fn cargo_toml_does_not_match_header_inside_multiline_string() {
        // A line that starts with `[lib]` inside a multi-line string
        // literal is the classic fragility that tripped hand-rolled
        // scanning. The toml parser reads this as a description value,
        // not a section header.
        let shape = parse_cargo_toml(
            r#"[package]
name = "x"
version = "0.1.0"
description = """
An example:
[lib]
path = "src/lib.rs"
"""
"#,
        );
        assert!(!shape.has_lib_section);
    }

    #[test]
    fn cargo_toml_malformed_input_degrades_to_default() {
        let shape = parse_cargo_toml("this is not valid toml at all ][");
        assert_eq!(shape, CargoTomlShape::default());
    }

    #[test]
    fn cargo_toml_bin_as_single_table_does_not_count_as_array_of_bins() {
        // `[bin]` (single table) is not the array-of-tables form
        // `[[bin]]` that Cargo expects; detecting it as a bin section
        // would be wrong.
        let shape = parse_cargo_toml(
            "[package]\nname = \"x\"\nversion = \"0.1.0\"\n[bin]\nname = \"tool\"\n",
        );
        assert!(!shape.has_bin_section);
    }

    #[test]
    fn package_json_detects_main_and_bin() {
        let shape = parse_package_json("{\"main\":\"index.js\",\"bin\":\"cli.js\"}");
        assert!(shape.has_main);
        assert!(shape.has_bin);
        assert!(!shape.has_exports);
    }

    #[test]
    fn package_json_malformed_input_degrades_to_default() {
        let shape = parse_package_json("{ not valid json");
        assert_eq!(shape, PackageJsonShape::default());
    }

    #[test]
    fn extract_path_deps_returns_empty_for_no_deps() {
        let deps = extract_path_deps(
            "[package]\nname = \"x\"\nversion = \"0.1.0\"\n\n[lib]\npath = \"src/lib.rs\"\n",
        );
        assert!(deps.is_empty());
    }

    #[test]
    fn extract_path_deps_returns_empty_for_registry_only_deps() {
        let deps = extract_path_deps(
            "[package]\nname = \"x\"\nversion = \"0.1.0\"\n[dependencies]\nserde = \"1\"\nanyhow = { version = \"1\" }\n",
        );
        assert!(deps.is_empty());
    }

    #[test]
    fn extract_path_deps_picks_up_path_in_dependencies() {
        let deps = extract_path_deps(
            "[package]\nname = \"x\"\nversion = \"0.1.0\"\n[dependencies]\nsibling = { path = \"../sibling\" }\nserde = \"1\"\n",
        );
        assert_eq!(deps, vec![PathBuf::from("../sibling")]);
    }

    #[test]
    fn extract_path_deps_picks_up_path_in_dev_and_build_dependencies() {
        let deps = extract_path_deps(
            "[package]\nname = \"x\"\nversion = \"0.1.0\"\n[dev-dependencies]\ntest-utils = { path = \"../test-utils\" }\n[build-dependencies]\nbuild-helpers = { path = \"../build-helpers\" }\n",
        );
        let mut sorted = deps.clone();
        sorted.sort();
        assert_eq!(
            sorted,
            vec![
                PathBuf::from("../build-helpers"),
                PathBuf::from("../test-utils"),
            ]
        );
    }

    #[test]
    fn extract_path_deps_picks_up_workspace_dependencies() {
        let deps = extract_path_deps(
            "[workspace]\nmembers = [\"a\"]\n\n[workspace.dependencies]\nshared = { path = \"shared\" }\n",
        );
        assert_eq!(deps, vec![PathBuf::from("shared")]);
    }

    #[test]
    fn extract_path_deps_skips_git_and_version_specs() {
        let deps = extract_path_deps(
            "[package]\nname = \"x\"\nversion = \"0.1.0\"\n[dependencies]\nfoo = { git = \"https://example.com/foo.git\" }\nbar = { version = \"1\", features = [\"derive\"] }\n",
        );
        assert!(deps.is_empty());
    }

    #[test]
    fn extract_path_deps_malformed_input_degrades_to_empty() {
        let deps = extract_path_deps("this is not valid toml at all ][");
        assert!(deps.is_empty());
    }

    #[test]
    fn extract_info_rkt_path_deps_picks_up_relative_path() {
        let info = "#lang info\n(define deps '(\"base\" \"../sibling-pkg\"))\n";
        let deps = extract_info_rkt_path_deps(info);
        assert_eq!(deps, vec![PathBuf::from("../sibling-pkg")]);
    }

    #[test]
    fn extract_info_rkt_path_deps_excludes_registry_packages() {
        let info = "#lang info\n(define deps '(\"base\" \"rackunit-lib\"))\n";
        let deps = extract_info_rkt_path_deps(info);
        assert!(deps.is_empty());
    }

    #[test]
    fn extract_info_rkt_path_deps_multiple_paths() {
        let info = "#lang info\n(define deps '(\"base\" \"../a\" \"../b\"))\n";
        let deps = extract_info_rkt_path_deps(info);
        assert_eq!(deps, vec![PathBuf::from("../a"), PathBuf::from("../b")]);
    }

    #[test]
    fn extract_info_rkt_path_deps_no_deps_field() {
        let info = "#lang info\n(define name \"my-pkg\")\n";
        let deps = extract_info_rkt_path_deps(info);
        assert!(deps.is_empty());
    }
}
