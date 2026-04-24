//! Minimal readers for the manifest shapes the deterministic
//! classifier looks at. Full TOML/JSON parsing is avoided where simple
//! line-matching suffices, both to keep the dependency footprint tight
//! and because the classifier never needs to re-serialise what it
//! reads — a false negative merely delegates to the LLM fallback.

/// Facts lifted from a `Cargo.toml` by line-scanning. True for each
/// section that appears as a bracketed header at the start of a line
/// (optionally indented; comments and string-quoted headers are
/// ignored because Cargo forbids them).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CargoTomlShape {
    pub has_lib_section: bool,
    pub has_bin_section: bool,
    pub has_workspace_section: bool,
}

pub fn parse_cargo_toml(contents: &str) -> CargoTomlShape {
    let mut out = CargoTomlShape::default();
    for raw_line in contents.lines() {
        // Strip trailing comment. `#` is Cargo.toml's comment
        // character; string literals can contain it, but section
        // headers cannot, so trimming before the header-test is safe.
        let line = raw_line.split('#').next().unwrap_or("").trim();
        if line == "[lib]" {
            out.has_lib_section = true;
        } else if line == "[[bin]]" {
            out.has_bin_section = true;
        } else if line == "[workspace]" {
            out.has_workspace_section = true;
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cargo_toml_detects_lib_section() {
        let shape = parse_cargo_toml(
            "[package]\nname = \"x\"\n\n[lib]\npath = \"src/lib.rs\"\n",
        );
        assert!(shape.has_lib_section);
        assert!(!shape.has_bin_section);
        assert!(!shape.has_workspace_section);
    }

    #[test]
    fn cargo_toml_detects_bin_and_workspace_sections() {
        let shape = parse_cargo_toml("[[bin]]\nname = \"tool\"\n[workspace]\n");
        assert!(shape.has_bin_section);
        assert!(shape.has_workspace_section);
    }

    #[test]
    fn cargo_toml_ignores_trailing_comment_on_header_line() {
        let shape = parse_cargo_toml("[lib] # library crate\n");
        assert!(shape.has_lib_section);
    }

    #[test]
    fn cargo_toml_does_not_match_header_inside_a_value() {
        let shape = parse_cargo_toml("description = \"Says [lib] a lot\"\n");
        assert!(!shape.has_lib_section);
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
}
