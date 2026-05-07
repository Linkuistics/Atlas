//! Contract content-sha canonicalisation algorithms.
//!
//! Implements the two branches of the canonicalisation spec
//! `docs/superpowers/specs/2026-05-06-contract-content-sha-canonicalisation.md`:
//!
//! - **§2.1 Code-derived contracts.** [`code_derived_content_sha`] —
//!   `sha256(bytes[span.0..span.1])`. Used for Rust-binding-derived
//!   `data-format` contracts and Rust `library-api` contracts. The
//!   algorithm is intentionally a literal byte-range hash in Phase 1;
//!   Phase 2 swaps in a per-language AST canonicaliser at the cost of
//!   a `SurfacesFile.schema_version` bump.
//!
//! - **§2.2 Schema-derived contracts.** [`canonicalise_yaml`] +
//!   [`schema_derived_content_sha`] — parse with `serde_yaml`, walk the
//!   tree, rebuild every `Mapping` as a `BTreeMap<String, Value>`
//!   (sorted keys), preserve array element order, emit via
//!   `serde_yaml::to_string`, sha256 the bytes. The same machinery
//!   computes the [`SurfacesFile`]-level fingerprint via
//!   [`compute_surfaces_fingerprint`] (the file with its `fingerprint`
//!   field zeroed, canonically serialised, then sha256'd).
//!
//! Phase 1 emits two real contract kinds (Rust-binding `data-format`,
//! Rust `library-api`) plus exactly one test-only `wire-protocol`
//! fixture exercising the schema-derived branch. The schema-derived
//! algorithm lands here so Phase 2 has nothing to invent — only new
//! analyser parsers to slot in.

use atlas_index::SurfacesFile;
use serde_yaml::{Mapping, Value};
use sha2::{Digest, Sha256};

/// SHA-256 hex (64-character lowercase) of the file-byte-range
/// `bytes[span.0..span.1]`. Half-open range; out-of-bounds spans
/// produce an empty digest hash. Used to compute a binding's
/// `content_sha` and a code-derived contract's `fingerprint` (in
/// Phase 1 those are byte-equal — see §4 of the canonicalisation
/// spec).
pub fn code_derived_content_sha(bytes: &[u8], span: (usize, usize)) -> String {
    let (start, end) = span;
    let slice: &[u8] = if start <= end && end <= bytes.len() {
        &bytes[start..end]
    } else {
        // Out-of-bounds: degrade to hashing nothing rather than
        // panicking. Callers compute spans themselves; an invalid
        // span is a bug we should diagnose at the call site, not
        // fail the whole pipeline here.
        b""
    };
    let digest: [u8; 32] = Sha256::digest(slice).into();
    hex_lower(&digest)
}

/// Canonicalise a YAML document for the §2.2 schema-derived branch.
/// Sorts mapping keys lexicographically, preserves array order, and
/// re-emits via `serde_yaml::to_string`. Comments are dropped (serde_yaml
/// does not preserve them on round-trip).
///
/// Returns the canonical YAML string. The bytes of this string are
/// what [`schema_derived_content_sha`] hashes.
pub fn canonicalise_yaml(input: &str) -> Result<String, serde_yaml::Error> {
    let value: Value = serde_yaml::from_str(input)?;
    let canonical = sort_value_keys(value);
    serde_yaml::to_string(&canonical)
}

/// SHA-256 hex of the canonical YAML form of the input document.
/// Combines [`canonicalise_yaml`] with `sha256` for callers that only
/// want the digest.
pub fn schema_derived_content_sha(yaml: &str) -> Result<String, serde_yaml::Error> {
    let canonical = canonicalise_yaml(yaml)?;
    let digest: [u8; 32] = Sha256::digest(canonical.as_bytes()).into();
    Ok(hex_lower(&digest))
}

/// Compute the aggregate [`SurfacesFile::fingerprint`] using the
/// schema-derived canonicaliser (§2.2) over the file's canonical YAML
/// form **with the `fingerprint` field zeroed**. Encapsulating this
/// in one helper gives PR-11 a single call site to cite when threading
/// participant-surface shas.
///
/// The convention (zero the field, canonicalise, hash) avoids the
/// chicken-and-egg of "the fingerprint contributes to the bytes that
/// produce the fingerprint" — the field is a pure projection of every
/// other field, computable in one pass.
pub fn compute_surfaces_fingerprint(file: &SurfacesFile) -> String {
    // Clone-then-zero so the caller's input is untouched. The clone
    // is shallow at the field level (Vec/String moves), and PR-7's
    // call sites build a single `SurfacesFile` per component, not a
    // hot loop — the cost is negligible.
    let mut canonical = file.clone();
    canonical.fingerprint = String::new();
    let yaml = serde_yaml::to_string(&canonical)
        .expect("SurfacesFile must serialise — every field is plain serde");
    // The wire form already has stable key ordering (struct fields
    // serialise in declaration order; inner Vec preserves order).
    // For consistency with the schema-derived branch we still pipe
    // through the canonicaliser — this guarantees that *future*
    // additions to SurfacesFile (or to inner types) don't drift away
    // from the spec's "sorted-keys" rule.
    let canonicalised = canonicalise_yaml(&yaml)
        .expect("SurfacesFile YAML must canonicalise — it is well-formed serde output");
    let digest: [u8; 32] = Sha256::digest(canonicalised.as_bytes()).into();
    hex_lower(&digest)
}

/// Recursively sort every mapping in `value` by key. Arrays are
/// preserved in declaration order (semantic per §2.2). Non-string
/// keys (rare in YAML, but legal) are stringified through their
/// `serde_yaml` `Display` form so the final rebuild is total.
fn sort_value_keys(value: Value) -> Value {
    match value {
        Value::Mapping(map) => {
            // Collect into BTreeMap keyed by stringified key form so
            // the canonical ordering is lex-by-key. This mirrors the
            // spec's "rebuild every Mapping as BTreeMap<String, Value>"
            // rule precisely.
            let mut sorted: std::collections::BTreeMap<String, Value> =
                std::collections::BTreeMap::new();
            for (k, v) in map {
                let key_str = match &k {
                    Value::String(s) => s.clone(),
                    other => serde_yaml::to_string(other)
                        .unwrap_or_default()
                        .trim()
                        .to_string(),
                };
                sorted.insert(key_str, sort_value_keys(v));
            }
            let mut rebuilt = Mapping::new();
            for (k, v) in sorted {
                rebuilt.insert(Value::String(k), v);
            }
            Value::Mapping(rebuilt)
        }
        Value::Sequence(seq) => {
            // Array order is semantic per §2.2; recurse into elements
            // but do not reorder.
            Value::Sequence(seq.into_iter().map(sort_value_keys).collect())
        }
        other => other,
    }
}

/// 64-character lowercase hex render of a 32-byte digest.
fn hex_lower(digest: &[u8; 32]) -> String {
    let mut hex = String::with_capacity(64);
    use std::fmt::Write;
    for b in digest {
        write!(&mut hex, "{b:02x}").expect("writing to String never fails");
    }
    hex
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_derived_sha_is_64_char_hex() {
        let bytes = b"pub struct Foo { x: u32 }";
        let sha = code_derived_content_sha(bytes, (0, bytes.len()));
        assert_eq!(sha.len(), 64);
        assert!(sha
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
    }

    #[test]
    fn code_derived_sha_changes_when_bytes_inside_span_change() {
        // Whitespace inside the span affects the sha (per §2.1).
        let s1 = b"pub struct Foo { x: u32 }";
        let s2 = b"pub struct Foo {  x: u32 }"; // extra space inside
        let sha1 = code_derived_content_sha(s1, (0, s1.len()));
        let sha2 = code_derived_content_sha(s2, (0, s2.len()));
        assert_ne!(sha1, sha2);
    }

    #[test]
    fn code_derived_sha_unchanged_when_bytes_outside_span_change() {
        // Whitespace outside the span does not affect the sha (per §2.1).
        // The span covers only the struct definition; the leading
        // doc-comment is outside the span.
        let prefix = b"/// doc comment\n";
        let body = b"pub struct Foo { x: u32 }";
        let mut s1 = Vec::from(prefix.as_slice());
        s1.extend_from_slice(body);
        let span1 = (prefix.len(), prefix.len() + body.len());

        let prefix2 = b"/// CHANGED doc comment with more text\n";
        let mut s2 = Vec::from(prefix2.as_slice());
        s2.extend_from_slice(body);
        let span2 = (prefix2.len(), prefix2.len() + body.len());

        let sha1 = code_derived_content_sha(&s1, span1);
        let sha2 = code_derived_content_sha(&s2, span2);
        assert_eq!(
            sha1, sha2,
            "whitespace outside the span must not change the sha (spec §2.1)"
        );
    }

    #[test]
    fn code_derived_sha_handles_out_of_bounds_span_safely() {
        let bytes = b"abc";
        let sha = code_derived_content_sha(bytes, (10, 20));
        // Empty hash — sha256 of zero bytes — is the well-known constant.
        let empty_sha: [u8; 32] = Sha256::digest(b"").into();
        let mut hex = String::new();
        use std::fmt::Write;
        for b in empty_sha {
            write!(&mut hex, "{b:02x}").unwrap();
        }
        assert_eq!(sha, hex);
    }

    #[test]
    fn canonicalise_yaml_sorts_top_level_mapping_keys() {
        let input = "b: 2\na: 1\n";
        let output = canonicalise_yaml(input).unwrap();
        // After canonicalisation, `a` must precede `b`.
        let a_pos = output.find('a').unwrap();
        let b_pos = output.find('b').unwrap();
        assert!(a_pos < b_pos, "got:\n{output}");
    }

    #[test]
    fn canonicalise_yaml_recurses_into_nested_mappings() {
        let input = "outer:\n  z: 1\n  a: 2\n";
        let output = canonicalise_yaml(input).unwrap();
        // Inside `outer`, `a` precedes `z`.
        let a_pos = output.find("a:").unwrap();
        let z_pos = output.find("z:").unwrap();
        assert!(a_pos < z_pos, "got:\n{output}");
    }

    #[test]
    fn canonicalise_yaml_preserves_array_order() {
        // §2.2: array order is semantic; the canonicaliser must not
        // reorder.
        let input = "items:\n  - third\n  - first\n  - second\n";
        let output = canonicalise_yaml(input).unwrap();
        let third = output.find("third").unwrap();
        let first = output.find("first").unwrap();
        let second = output.find("second").unwrap();
        assert!(third < first && first < second, "got:\n{output}");
    }

    #[test]
    fn schema_derived_branch_worked_example_from_spec_section_2_2() {
        // Two YAML schemas equivalent up to cosmetic edits — the
        // verbatim example from spec §2.2. Both must produce the same
        // content sha.
        let v1 = r#"
type: object
properties:
  name: {type: string}
  age: {type: integer}
required: [name, age]
"#;
        let v2 = r#"
required:
  - name
  - age
properties:
  age:
    type: integer
  name:
    type: string
type: object
"#;
        let sha1 = schema_derived_content_sha(v1).unwrap();
        let sha2 = schema_derived_content_sha(v2).unwrap();
        assert_eq!(
            sha1, sha2,
            "spec §2.2 worked example: cosmetically-different YAML schemas must canonicalise to the same content sha"
        );
    }

    #[test]
    fn schema_derived_branch_distinguishes_array_reorderings() {
        // §2.2: array order is semantic. `[name, age]` and `[age, name]`
        // must produce different shas.
        let v1 = "required: [name, age]\n";
        let v2 = "required: [age, name]\n";
        let sha1 = schema_derived_content_sha(v1).unwrap();
        let sha2 = schema_derived_content_sha(v2).unwrap();
        assert_ne!(sha1, sha2, "array element order is semantic per spec §2.2");
    }

    #[test]
    fn schema_derived_branch_strips_comments() {
        // Comments are not preserved by serde_yaml's to_string, so two
        // YAMLs differing only in comments produce equal canonical
        // bytes.
        let v1 = "x: 1\n";
        let v2 = "# comment\nx: 1\n";
        let sha1 = schema_derived_content_sha(v1).unwrap();
        let sha2 = schema_derived_content_sha(v2).unwrap();
        assert_eq!(sha1, sha2);
    }

    #[test]
    fn compute_surfaces_fingerprint_zeroes_the_fingerprint_field() {
        use atlas_index::{Binding, Contract, ContractKind, SurfacesFile, SURFACES_SCHEMA_VERSION};
        use component_ontology::ComponentId;
        use std::path::PathBuf;

        let binding = Binding {
            language: "rust".into(),
            symbol: "Foo".into(),
            file: PathBuf::from("src/lib.rs"),
            span: (0, 10),
            content_sha: "0".repeat(64),
        };
        let contract = Contract {
            id: "demo/foo-shape".into(),
            kind: ContractKind::DataFormat,
            fingerprint: "1".repeat(64),
            definition_binding: binding.clone(),
            description: String::new(),
        };
        let mut file = SurfacesFile {
            schema_version: SURFACES_SCHEMA_VERSION,
            component_id: ComponentId::parse("demo/comp").unwrap(),
            // Pre-existing value must be ignored.
            fingerprint: "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff".into(),
            contracts_defined: vec![contract.clone()],
            contracts_implemented: Vec::new(),
            contracts_consumed: Vec::new(),
            library_apis: Vec::new(),
        };
        let fp_before = compute_surfaces_fingerprint(&file);
        // Mutate the fingerprint to a different sentinel; the function
        // must still produce the same hash because it zeroes the field.
        file.fingerprint = "deadbeef".repeat(8);
        let fp_after = compute_surfaces_fingerprint(&file);
        assert_eq!(
            fp_before, fp_after,
            "compute_surfaces_fingerprint must ignore the existing fingerprint field"
        );
        // And the result is 64-char hex.
        assert_eq!(fp_before.len(), 64);
    }

    #[test]
    fn compute_surfaces_fingerprint_changes_when_contract_changes() {
        use atlas_index::{Binding, Contract, ContractKind, SurfacesFile, SURFACES_SCHEMA_VERSION};
        use component_ontology::ComponentId;
        use std::path::PathBuf;

        let binding = Binding {
            language: "rust".into(),
            symbol: "Foo".into(),
            file: PathBuf::from("src/lib.rs"),
            span: (0, 10),
            content_sha: "0".repeat(64),
        };
        let mk = |sha: &str| SurfacesFile {
            schema_version: SURFACES_SCHEMA_VERSION,
            component_id: ComponentId::parse("demo/comp").unwrap(),
            fingerprint: String::new(),
            contracts_defined: vec![Contract {
                id: "demo/foo-shape".into(),
                kind: ContractKind::DataFormat,
                fingerprint: sha.to_string(),
                definition_binding: binding.clone(),
                description: String::new(),
            }],
            contracts_implemented: Vec::new(),
            contracts_consumed: Vec::new(),
            library_apis: Vec::new(),
        };
        let a = compute_surfaces_fingerprint(&mk("a".repeat(64).as_str()));
        let b = compute_surfaces_fingerprint(&mk("b".repeat(64).as_str()));
        assert_ne!(
            a, b,
            "changing a contract's fingerprint must change the surfaces fingerprint"
        );
    }
}
