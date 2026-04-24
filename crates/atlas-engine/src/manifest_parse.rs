//! Minimal readers for the manifest shapes the deterministic
//! classifier looks at. `Cargo.toml` is parsed with the `toml` crate
//! and `package.json` with `serde_json` — in both cases a full parse
//! is cheaper and more reliable than hand-rolled scanning, and a
//! malformed document degrades to the default "all false" shape so
//! the classifier falls back to the LLM.

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
}

pub fn parse_cargo_toml(contents: &str) -> CargoTomlShape {
    let Ok(table) = contents.parse::<toml::Table>() else {
        return CargoTomlShape::default();
    };
    CargoTomlShape {
        has_lib_section: table.get("lib").is_some_and(toml::Value::is_table),
        has_bin_section: table.get("bin").is_some_and(toml::Value::is_array),
        has_workspace_section: table.get("workspace").is_some_and(toml::Value::is_table),
    }
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
}
