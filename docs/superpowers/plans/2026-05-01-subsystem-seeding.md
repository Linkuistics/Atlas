# Subsystem Seeding Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add hand-authored subsystem seeding to Atlas: a new override file (`subsystems.overrides.yaml`), a new projection (`subsystems.yaml`), and shared id namespace with components for edge participation.

**Architecture:** L9-only feature. New schemas in `atlas-contracts/crates/atlas-index`. New Salsa input on `Workspace` read by L9 alone. New projection function next to `components_yaml_snapshot`. Validation extends `validate-overrides` (pre-LLM stage) and adds a post-L4 cross-namespace collision check.

**Tech Stack:** Rust, Salsa 0.26, serde_yaml, globset (already transitive via `ignore`).

**Cross-repo note:** Tasks 1–4 operate in `~/Development/atlas-contracts/` (separate git repo). Tasks 5+ operate in `~/Development/Atlas/`. The two repos are connected by path deps (`path = "../atlas-contracts/crates/<crate>"`); changes to atlas-contracts are visible to Atlas immediately without a Cargo.lock bump.

**Spec:** `docs/superpowers/specs/2026-05-01-subsystem-seeding-design.md`.

---

## File Map

### atlas-contracts (separate repo)

- **Modify** `crates/atlas-index/src/schema.rs` — add `SUBSYSTEMS_OVERRIDES_SCHEMA_VERSION`, `SUBSYSTEMS_SCHEMA_VERSION`, `SubsystemOverride`, `SubsystemsOverridesFile`, `SubsystemEntry`, `MemberEvidence`, `SubsystemsFile` types.
- **Modify** `crates/atlas-index/src/yaml_io.rs` — add `load_subsystems_overrides`, `load_or_default_subsystems_overrides`, `save_subsystems_overrides_atomic`, `load_subsystems`, `load_or_default_subsystems`, `save_subsystems_atomic`.
- **Modify** `crates/atlas-index/src/lib.rs` — re-export new types and functions.
- **Modify** `crates/component-ontology/src/schema.rs` — add `validate_participant_namespace` free function.

### Atlas

- **Modify** `crates/atlas-engine/Cargo.toml` — add `globset` direct dependency.
- **Modify** `crates/atlas-engine/src/db.rs` — add `subsystems_overrides` Salsa input field, `set_subsystems_overrides` setter with skip-if-equal guard.
- **Create** `crates/atlas-engine/src/l9_subsystems.rs` — `subsystems_yaml_snapshot`, `MemberMatch` enum, glob/id resolution, empty-glob warning emission, cross-namespace collision detection.
- **Modify** `crates/atlas-engine/src/lib.rs` — add `mod l9_subsystems;` and re-export.
- **Modify** `crates/atlas-cli/src/validate.rs` — extend `validate_overrides` to accept a `&SubsystemsOverridesFile` and produce subsystem-shape issues alongside component-shape issues.
- **Modify** `crates/atlas-cli/src/main.rs` — load `subsystems.overrides.yaml` in the `validate-overrides` subcommand and pass it through.
- **Modify** `crates/atlas-cli/src/pipeline.rs` — load + set + save the new file; run cross-namespace check after L4.
- **Modify** `crates/atlas-cli/tests/pipeline_integration.rs` — add subsystem fixtures and assertions.
- **Create** `crates/atlas-cli/tests/subsystems_integration.rs` — dedicated rename-stability and collision-halt tests.
- **Modify** `README.md` — add "Pre-seeding subsystems" subsection in Usage.

---

## Phase 1 — atlas-contracts schemas

### Task 1: Subsystem override input schema

**Files:**
- Modify: `~/Development/atlas-contracts/crates/atlas-index/src/schema.rs`

- [ ] **Step 1: Write failing tests for `SubsystemsOverridesFile` round-trip**

Append to `~/Development/atlas-contracts/crates/atlas-index/src/schema.rs` inside `#[cfg(test)] mod tests`:

```rust
    #[test]
    fn subsystems_overrides_file_default_has_current_schema_version() {
        let f = SubsystemsOverridesFile::default();
        assert_eq!(f.schema_version, SUBSYSTEMS_OVERRIDES_SCHEMA_VERSION);
        assert!(f.subsystems.is_empty());
    }

    #[test]
    fn subsystem_override_round_trips_through_yaml() {
        let s = SubsystemOverride {
            id: "auth".into(),
            members: vec!["services/auth/*".into(), "identity-core".into()],
            role: Some("identity-and-authorisation".into()),
            lifecycle_roles: vec![LifecycleScope::Runtime],
            rationale: "owns all session/token surfaces".into(),
            evidence_grade: EvidenceGrade::Strong,
            evidence_fields: vec![],
        };
        let yaml = serde_yaml::to_string(&s).unwrap();
        let parsed: SubsystemOverride = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(parsed, s);
    }

    #[test]
    fn subsystems_overrides_file_round_trips_through_yaml() {
        let f = SubsystemsOverridesFile {
            schema_version: SUBSYSTEMS_OVERRIDES_SCHEMA_VERSION,
            subsystems: vec![SubsystemOverride {
                id: "auth".into(),
                members: vec!["libs/identity".into()],
                role: None,
                lifecycle_roles: vec![],
                rationale: "x".into(),
                evidence_grade: EvidenceGrade::Strong,
                evidence_fields: vec![],
            }],
        };
        let yaml = serde_yaml::to_string(&f).unwrap();
        let parsed: SubsystemsOverridesFile = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(parsed, f);
    }
```

- [ ] **Step 2: Run tests to verify they fail**

```sh
cd ~/Development/atlas-contracts
cargo test -p atlas-index subsystems_overrides 2>&1 | tail -20
```

Expected: compile errors — types don't exist yet.

- [ ] **Step 3: Implement schema types**

Add to `~/Development/atlas-contracts/crates/atlas-index/src/schema.rs` near the existing `OVERRIDES_SCHEMA_VERSION` constant (top of file):

```rust
pub const SUBSYSTEMS_OVERRIDES_SCHEMA_VERSION: u32 = 1;
```

Add new types after `OverridesFile` (around line 183):

```rust
/// Hand-authored subsystem boundary. Lives in `subsystems.overrides.yaml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubsystemOverride {
    pub id: String,
    /// Mixed glob and id forms. A `members` entry containing `/` or `*`
    /// is matched against component path segments; otherwise it is
    /// matched against component id directly. See the design spec for
    /// the resolution algorithm.
    #[serde(default)]
    pub members: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(default)]
    pub lifecycle_roles: Vec<LifecycleScope>,
    pub rationale: String,
    pub evidence_grade: EvidenceGrade,
    #[serde(default)]
    pub evidence_fields: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubsystemsOverridesFile {
    pub schema_version: u32,
    #[serde(default)]
    pub subsystems: Vec<SubsystemOverride>,
}

impl Default for SubsystemsOverridesFile {
    fn default() -> Self {
        SubsystemsOverridesFile {
            schema_version: SUBSYSTEMS_OVERRIDES_SCHEMA_VERSION,
            subsystems: Vec::new(),
        }
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

```sh
cd ~/Development/atlas-contracts
cargo test -p atlas-index subsystems_overrides 2>&1 | tail -10
```

Expected: 3 tests pass.

- [ ] **Step 5: Re-export from lib.rs**

Modify `~/Development/atlas-contracts/crates/atlas-index/src/lib.rs` — find the existing `pub use schema::{...}` line and add the new names:

```rust
pub use schema::{
    // ... existing exports ...
    SubsystemOverride, SubsystemsOverridesFile, SUBSYSTEMS_OVERRIDES_SCHEMA_VERSION,
};
```

- [ ] **Step 6: Verify lib re-exports compile**

```sh
cd ~/Development/atlas-contracts && cargo check -p atlas-index 2>&1 | tail -5
```

Expected: clean.

- [ ] **Step 7: Do not commit yet** — Tasks 2 and 3 also touch atlas-contracts; bundle them into one commit at end of Phase 1.

---

### Task 2: Subsystem output schema

**Files:**
- Modify: `~/Development/atlas-contracts/crates/atlas-index/src/schema.rs`
- Modify: `~/Development/atlas-contracts/crates/atlas-index/src/lib.rs`

- [ ] **Step 1: Write failing tests for output types**

Append to `mod tests`:

```rust
    #[test]
    fn subsystems_file_default_has_current_schema_version() {
        let f = SubsystemsFile::default();
        assert_eq!(f.schema_version, SUBSYSTEMS_SCHEMA_VERSION);
        assert!(f.subsystems.is_empty());
        assert!(f.generated_at.is_empty());
    }

    #[test]
    fn subsystem_entry_round_trips_through_yaml() {
        let e = SubsystemEntry {
            id: "auth".into(),
            role: Some("identity".into()),
            lifecycle_roles: vec![LifecycleScope::Runtime],
            rationale: "x".into(),
            evidence_grade: EvidenceGrade::Strong,
            evidence_fields: vec![],
            members: vec!["auth-service".into(), "identity-lib".into()],
            member_evidence: vec![
                MemberEvidence {
                    id: "auth-service".into(),
                    matched_via: "services/auth/*".into(),
                },
                MemberEvidence {
                    id: "identity-lib".into(),
                    matched_via: "libs/identity".into(),
                },
            ],
            notes: vec![],
        };
        let yaml = serde_yaml::to_string(&e).unwrap();
        let parsed: SubsystemEntry = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(parsed, e);
    }

    #[test]
    fn member_evidence_round_trips_through_yaml() {
        let m = MemberEvidence {
            id: "x-component".into(),
            matched_via: "id".into(),
        };
        let yaml = serde_yaml::to_string(&m).unwrap();
        let parsed: MemberEvidence = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(parsed, m);
    }
```

- [ ] **Step 2: Run tests to verify they fail**

```sh
cd ~/Development/atlas-contracts && cargo test -p atlas-index subsystem_entry 2>&1 | tail -10
```

Expected: compile errors.

- [ ] **Step 3: Implement output types**

Add the constant near the existing schema version constants:

```rust
pub const SUBSYSTEMS_SCHEMA_VERSION: u32 = 1;
```

Add the structs after `SubsystemsOverridesFile`:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemberEvidence {
    pub id: String,
    /// Glob string when the member resolved via a glob, the literal
    /// `"id"` when the member entry was an id form, or
    /// `"<glob> (no matches)"` when a glob produced zero matches.
    pub matched_via: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubsystemEntry {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(default)]
    pub lifecycle_roles: Vec<LifecycleScope>,
    pub rationale: String,
    pub evidence_grade: EvidenceGrade,
    #[serde(default)]
    pub evidence_fields: Vec<String>,
    /// Resolved component ids, sorted and deduped.
    #[serde(default)]
    pub members: Vec<String>,
    /// Audit trail: how each resolved member was matched.
    #[serde(default)]
    pub member_evidence: Vec<MemberEvidence>,
    /// Soft warnings about this subsystem (e.g. `"all members unresolved"`).
    #[serde(default)]
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubsystemsFile {
    pub schema_version: u32,
    pub generated_at: String,
    #[serde(default)]
    pub subsystems: Vec<SubsystemEntry>,
}

impl Default for SubsystemsFile {
    fn default() -> Self {
        SubsystemsFile {
            schema_version: SUBSYSTEMS_SCHEMA_VERSION,
            generated_at: String::new(),
            subsystems: Vec::new(),
        }
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

```sh
cd ~/Development/atlas-contracts && cargo test -p atlas-index subsystem 2>&1 | tail -10
```

Expected: all subsystem tests pass.

- [ ] **Step 5: Re-export from lib.rs**

Add to the `pub use schema::{...}` block:

```rust
    MemberEvidence, SubsystemEntry, SubsystemsFile, SUBSYSTEMS_SCHEMA_VERSION,
```

- [ ] **Step 6: Verify clean compile**

```sh
cd ~/Development/atlas-contracts && cargo check -p atlas-index 2>&1 | tail -5
```

Expected: clean.

---

### Task 3: yaml_io load/save for both subsystem files

**Files:**
- Modify: `~/Development/atlas-contracts/crates/atlas-index/src/yaml_io.rs`
- Modify: `~/Development/atlas-contracts/crates/atlas-index/src/lib.rs`

- [ ] **Step 1: Write failing tests for round-trip and schema-mismatch behaviour**

Append to `mod tests` in `yaml_io.rs`:

```rust
    fn sample_subsystems_overrides_file() -> SubsystemsOverridesFile {
        SubsystemsOverridesFile {
            schema_version: SUBSYSTEMS_OVERRIDES_SCHEMA_VERSION,
            subsystems: vec![SubsystemOverride {
                id: "auth".into(),
                members: vec!["services/auth/*".into(), "identity-core".into()],
                role: Some("identity".into()),
                lifecycle_roles: vec![],
                rationale: "x".into(),
                evidence_grade: EvidenceGrade::Strong,
                evidence_fields: vec![],
            }],
        }
    }

    fn sample_subsystems_file() -> SubsystemsFile {
        SubsystemsFile {
            schema_version: SUBSYSTEMS_SCHEMA_VERSION,
            generated_at: "2026-05-01T00:00:00Z".into(),
            subsystems: vec![],
        }
    }

    #[test]
    fn subsystems_overrides_save_then_load_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("subsystems.overrides.yaml");
        let original = sample_subsystems_overrides_file();
        save_subsystems_overrides_atomic(&path, &original).unwrap();
        let loaded = load_subsystems_overrides(&path).unwrap();
        assert_eq!(loaded, original);
    }

    #[test]
    fn subsystems_save_then_load_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("subsystems.yaml");
        let original = sample_subsystems_file();
        save_subsystems_atomic(&path, &original).unwrap();
        let loaded = load_subsystems(&path).unwrap();
        assert_eq!(loaded, original);
    }

    #[test]
    fn load_or_default_subsystems_overrides_returns_default_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("subsystems.overrides.yaml");
        let loaded = load_or_default_subsystems_overrides(&path).unwrap();
        assert_eq!(loaded, SubsystemsOverridesFile::default());
    }

    #[test]
    fn load_or_default_subsystems_returns_default_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("subsystems.yaml");
        let loaded = load_or_default_subsystems(&path).unwrap();
        assert_eq!(loaded, SubsystemsFile::default());
    }

    #[test]
    fn subsystems_overrides_load_rejects_wrong_schema_version_user_authored() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("subsystems.overrides.yaml");
        std::fs::write(
            &path,
            "schema_version: 999\nsubsystems: []\n",
        )
        .unwrap();
        let err = load_subsystems_overrides(&path).unwrap_err().to_string();
        assert!(
            err.contains("user-authored") && err.contains("Do NOT delete"),
            "expected user-authored migration hint, got: {err}"
        );
    }

    #[test]
    fn subsystems_load_rejects_wrong_schema_version_with_regenerate_hint() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("subsystems.yaml");
        std::fs::write(
            &path,
            "schema_version: 999\ngenerated_at: ''\nsubsystems: []\n",
        )
        .unwrap();
        let err = load_subsystems(&path).unwrap_err().to_string();
        assert!(
            err.contains("delete the file") && err.contains("atlas index"),
            "expected regenerate hint, got: {err}"
        );
    }
```

- [ ] **Step 2: Run tests to verify they fail**

```sh
cd ~/Development/atlas-contracts && cargo test -p atlas-index subsystems_ 2>&1 | tail -20
```

Expected: compile errors — functions don't exist.

- [ ] **Step 3: Implement load/save functions**

Add to `~/Development/atlas-contracts/crates/atlas-index/src/yaml_io.rs` after `save_externals_atomic`:

```rust
pub fn load_subsystems_overrides(path: &Path) -> Result<SubsystemsOverridesFile> {
    let content = read_to_string(path)?;
    let file: SubsystemsOverridesFile = serde_yaml::from_str(&content).with_context(|| {
        format!(
            "failed to parse {} as subsystems.overrides.yaml",
            path.display()
        )
    })?;
    require_schema_version(
        file.schema_version,
        SUBSYSTEMS_OVERRIDES_SCHEMA_VERSION,
        path,
        FileFlavour::UserAuthored,
    )?;
    Ok(file)
}

pub fn load_or_default_subsystems_overrides(path: &Path) -> Result<SubsystemsOverridesFile> {
    if !path.exists() {
        return Ok(SubsystemsOverridesFile::default());
    }
    load_subsystems_overrides(path)
}

pub fn save_subsystems_overrides_atomic(
    path: &Path,
    file: &SubsystemsOverridesFile,
) -> Result<()> {
    let yaml =
        serde_yaml::to_string(file).context("failed to serialise subsystems.overrides.yaml")?;
    write_atomic(path, yaml.as_bytes())
}

pub fn load_subsystems(path: &Path) -> Result<SubsystemsFile> {
    let content = read_to_string(path)?;
    let file: SubsystemsFile = serde_yaml::from_str(&content)
        .with_context(|| format!("failed to parse {} as subsystems.yaml", path.display()))?;
    require_schema_version(
        file.schema_version,
        SUBSYSTEMS_SCHEMA_VERSION,
        path,
        FileFlavour::Generated {
            tool_step: "atlas index",
        },
    )?;
    Ok(file)
}

pub fn load_or_default_subsystems(path: &Path) -> Result<SubsystemsFile> {
    if !path.exists() {
        return Ok(SubsystemsFile::default());
    }
    load_subsystems(path)
}

pub fn save_subsystems_atomic(path: &Path, file: &SubsystemsFile) -> Result<()> {
    let yaml = serde_yaml::to_string(file).context("failed to serialise subsystems.yaml")?;
    write_atomic(path, yaml.as_bytes())
}
```

The top of the file imports `SubsystemsOverridesFile`, `SubsystemsFile`, and the two schema-version constants from `super::schema` — verify the existing `use super::schema::{...}` includes them, and add if missing.

- [ ] **Step 4: Run tests to verify they pass**

```sh
cd ~/Development/atlas-contracts && cargo test -p atlas-index subsystems_ 2>&1 | tail -10
```

Expected: 6 new tests pass.

- [ ] **Step 5: Re-export from lib.rs**

Add to the existing `pub use yaml_io::{...}` block in `lib.rs`:

```rust
    load_or_default_subsystems, load_or_default_subsystems_overrides, load_subsystems,
    load_subsystems_overrides, save_subsystems_atomic, save_subsystems_overrides_atomic,
```

- [ ] **Step 6: Run full atlas-index test suite**

```sh
cd ~/Development/atlas-contracts && cargo test -p atlas-index 2>&1 | tail -10
```

Expected: all green.

---

### Task 4: `validate_participant_namespace` in component-ontology

**Files:**
- Modify: `~/Development/atlas-contracts/crates/component-ontology/src/schema.rs`

- [ ] **Step 1: Write failing tests**

Append to the `#[cfg(test)] mod tests` in `schema.rs`:

```rust
    #[test]
    fn validate_participant_namespace_passes_when_disjoint() {
        let components = ["auth-service", "storage"].iter().copied().collect();
        let subsystems = ["auth", "storage-system"].iter().copied().collect();
        let result = validate_participant_namespace(&components, &subsystems);
        assert!(result.is_ok(), "expected no collisions, got {result:?}");
    }

    #[test]
    fn validate_participant_namespace_reports_collisions() {
        let components: std::collections::BTreeSet<&str> =
            ["auth", "storage"].iter().copied().collect();
        let subsystems: std::collections::BTreeSet<&str> =
            ["auth", "metrics"].iter().copied().collect();
        let err = validate_participant_namespace(&components, &subsystems).unwrap_err();
        assert_eq!(err, vec!["auth"]);
    }

    #[test]
    fn validate_participant_namespace_handles_multiple_collisions_sorted() {
        let components: std::collections::BTreeSet<&str> =
            ["alpha", "beta", "delta"].iter().copied().collect();
        let subsystems: std::collections::BTreeSet<&str> =
            ["delta", "alpha"].iter().copied().collect();
        let err = validate_participant_namespace(&components, &subsystems).unwrap_err();
        assert_eq!(err, vec!["alpha", "delta"]);
    }
```

- [ ] **Step 2: Run tests to verify they fail**

```sh
cd ~/Development/atlas-contracts && cargo test -p component-ontology validate_participant_namespace 2>&1 | tail -10
```

Expected: compile errors.

- [ ] **Step 3: Implement function**

Add as a free function in `~/Development/atlas-contracts/crates/component-ontology/src/schema.rs` (top-level, near other public helpers, before the `#[cfg(test)]`):

```rust
/// Verify that no id appears in both component and subsystem namespaces.
/// Edge participants (`Edge::participants: Vec<String>`) are opaque
/// strings; collision-free namespaces guarantee unambiguous resolution.
///
/// Returns the sorted set of colliding ids, if any.
pub fn validate_participant_namespace(
    components: &std::collections::BTreeSet<&str>,
    subsystems: &std::collections::BTreeSet<&str>,
) -> Result<(), Vec<String>> {
    let mut collisions: Vec<String> = components
        .intersection(subsystems)
        .map(|s| (*s).to_string())
        .collect();
    if collisions.is_empty() {
        Ok(())
    } else {
        collisions.sort();
        Err(collisions)
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

```sh
cd ~/Development/atlas-contracts && cargo test -p component-ontology validate_participant_namespace 2>&1 | tail -10
```

Expected: 3 tests pass.

- [ ] **Step 5: Re-export from lib.rs**

Modify `~/Development/atlas-contracts/crates/component-ontology/src/lib.rs` and add `validate_participant_namespace` to the existing `pub use schema::{...}` line.

- [ ] **Step 6: Full atlas-contracts workspace tests pass**

```sh
cd ~/Development/atlas-contracts && cargo test --workspace 2>&1 | tail -15
```

Expected: all green.

- [ ] **Step 7: Commit Phase 1 in atlas-contracts**

```sh
cd ~/Development/atlas-contracts
git add -A
git status
git commit -m "$(cat <<'EOF'
Add subsystem seeding schemas

Introduces SubsystemsOverridesFile (input) and SubsystemsFile (output)
in atlas-index, plus validate_participant_namespace in component-ontology
for cross-namespace id-collision detection.

Schema versions both start at 1. Output is generated; input is
user-authored (parallel to components.overrides.yaml).
EOF
)"
git push origin main
```

Expected: commit lands cleanly.

---

## Phase 2 — Atlas engine integration

### Task 5: Add globset dependency and Salsa input field

**Files:**
- Modify: `~/Development/Atlas/crates/atlas-engine/Cargo.toml`
- Modify: `~/Development/Atlas/crates/atlas-engine/src/db.rs`

- [ ] **Step 1: Write failing setter test**

Append to the `#[cfg(test)] mod tests` block in `db.rs`. The test verifies skip-if-equal via Arc identity: when the new value equals the current value, the setter must not allocate a new `Arc`, so the `Arc<SubsystemsOverridesFile>` returned by `Workspace::subsystems_overrides` keeps its original allocation.

```rust
    #[test]
    fn set_subsystems_overrides_skip_if_equal_preserves_arc_identity() {
        // Build any AtlasDatabase the existing tests use; copy the
        // construction from the closest existing test in this file.
        let backend: Arc<dyn LlmBackend> = make_test_backend();
        let mut db = AtlasDatabase::new(
            backend,
            std::path::PathBuf::from("/tmp/x"),
            LlmFingerprint::default(),
        );
        let initial = atlas_index::SubsystemsOverridesFile::default();
        db.set_subsystems_overrides(initial.clone());
        let ws = db.workspace();
        let arc1 = ws
            .subsystems_overrides(&db as &dyn salsa::Database)
            .clone();
        db.set_subsystems_overrides(initial);
        let arc2 = ws
            .subsystems_overrides(&db as &dyn salsa::Database)
            .clone();
        assert!(
            Arc::ptr_eq(&arc1, &arc2),
            "skip-if-equal must keep the same Arc allocation"
        );
    }
```

If a `make_test_backend` helper does not exist, replace that line with whatever construction the surrounding `#[cfg(test)]` already uses (e.g. an in-line `Arc::new(SomeTestBackend::new())`).

- [ ] **Step 2: Run test to verify it fails**

```sh
cd ~/Development/Atlas && cargo test -p atlas-engine set_subsystems_overrides 2>&1 | tail -10
```

Expected: compile errors — `subsystems_overrides` field doesn't exist.

- [ ] **Step 3: Add globset to atlas-engine Cargo.toml**

Modify `crates/atlas-engine/Cargo.toml` `[dependencies]` section, add:

```toml
globset = "0.4"
```

- [ ] **Step 4: Add Salsa input field on `Workspace`**

In `crates/atlas-engine/src/db.rs`, find the `Workspace` input struct (around line 56) and add the new field next to `components_overrides`:

```rust
    #[returns(ref)]
    pub subsystems_overrides: Arc<SubsystemsOverridesFile>,
```

Add the import to the top of `db.rs`:

```rust
use atlas_index::SubsystemsOverridesFile;
```

(Or extend the existing `atlas_index::{...}` import line if one exists.)

- [ ] **Step 5: Add the setter with skip-if-equal guard**

After `set_components_overrides`, add:

```rust
    pub fn set_subsystems_overrides(&mut self, value: SubsystemsOverridesFile) {
        let ws = self.workspace();
        if **ws.subsystems_overrides(self as &dyn salsa::Database) == value {
            return;
        }
        ws.set_subsystems_overrides(self).to(Arc::new(value));
    }
```

- [ ] **Step 6: Initialise the new input in `AtlasDatabase::new`**

Find the `Workspace::new` call inside `AtlasDatabase::new` (the constructor that builds the Workspace input). Add the new field with its default:

```rust
        Arc::new(SubsystemsOverridesFile::default()),
```

— positionally placed alongside `components_overrides`. Inspect the surrounding code to insert it in the same order as the field declaration.

- [ ] **Step 7: Run setter test and full atlas-engine tests**

```sh
cd ~/Development/Atlas && cargo test -p atlas-engine set_subsystems_overrides 2>&1 | tail -5
cargo test -p atlas-engine 2>&1 | tail -10
```

Expected: new test passes; existing tests still pass.

---

### Task 6: L9 subsystem projection — resolution algorithm

**Files:**
- Create: `~/Development/Atlas/crates/atlas-engine/src/l9_subsystems.rs`
- Modify: `~/Development/Atlas/crates/atlas-engine/src/lib.rs`

- [ ] **Step 1: Write failing tests for resolution**

Create the file with `#[cfg(test)] mod tests` at the bottom; add these tests first:

```rust
//! L9 subsystem projection — resolves hand-authored subsystem overrides
//! against the live component tree and emits a `SubsystemsFile`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::Arc;

use atlas_index::{
    ComponentEntry, MemberEvidence, SubsystemEntry, SubsystemOverride, SubsystemsFile,
    SUBSYSTEMS_SCHEMA_VERSION,
};
use globset::{Glob, GlobMatcher};

use crate::db::AtlasDatabase;
use crate::l4_tree::all_components;

/// Produce `subsystems.yaml` from the workspace input + live components.
/// `generated_at` is left empty; the CLI stamps the wall clock at write
/// time. Salsa-side stable output preserves byte-identity on no-op
/// re-runs.
pub fn subsystems_yaml_snapshot(db: &AtlasDatabase) -> Arc<SubsystemsFile> {
    let ws = db.workspace();
    let overrides = ws.subsystems_overrides(db as &dyn salsa::Database).clone();
    let components = all_components(db);
    let resolved = resolve_subsystems(&overrides.subsystems, &components);
    Arc::new(SubsystemsFile {
        schema_version: SUBSYSTEMS_SCHEMA_VERSION,
        generated_at: String::new(),
        subsystems: resolved,
    })
}

/// Pure resolution helper, factored out so it can be tested without a
/// full `AtlasDatabase`. Inputs: the override list + every non-deleted
/// component. Output: one `SubsystemEntry` per override, with members
/// resolved to component ids.
pub(crate) fn resolve_subsystems(
    overrides: &[SubsystemOverride],
    components: &[ComponentEntry],
) -> Vec<SubsystemEntry> {
    let live: Vec<&ComponentEntry> = components.iter().filter(|c| !c.deleted).collect();
    let by_id: BTreeMap<&str, &ComponentEntry> =
        live.iter().map(|c| (c.id.as_str(), *c)).collect();
    overrides
        .iter()
        .map(|sub| resolve_one_subsystem(sub, &live, &by_id))
        .collect()
}

fn resolve_one_subsystem(
    sub: &SubsystemOverride,
    live: &[&ComponentEntry],
    by_id: &BTreeMap<&str, &ComponentEntry>,
) -> SubsystemEntry {
    let mut resolved_ids: BTreeSet<String> = BTreeSet::new();
    let mut evidence: Vec<MemberEvidence> = Vec::new();

    for member in &sub.members {
        if is_glob_form(member) {
            let matcher = match Glob::new(member) {
                Ok(g) => g.compile_matcher(),
                Err(_) => {
                    evidence.push(MemberEvidence {
                        id: String::new(),
                        matched_via: format!("{member} (invalid glob)"),
                    });
                    continue;
                }
            };
            let matches = match_glob(&matcher, live);
            if matches.is_empty() {
                evidence.push(MemberEvidence {
                    id: String::new(),
                    matched_via: format!("{member} (no matches)"),
                });
            } else {
                for c in matches {
                    if resolved_ids.insert(c.id.clone()) {
                        evidence.push(MemberEvidence {
                            id: c.id.clone(),
                            matched_via: member.clone(),
                        });
                    }
                }
            }
        } else if let Some(c) = by_id.get(member.as_str()) {
            if resolved_ids.insert(c.id.clone()) {
                evidence.push(MemberEvidence {
                    id: c.id.clone(),
                    matched_via: "id".into(),
                });
            }
        } else {
            // Unknown id — caller surfaces this as a hard error in the
            // post-L4 validation pass. Record it in evidence so the
            // projection is self-describing even if validation is
            // skipped.
            evidence.push(MemberEvidence {
                id: member.clone(),
                matched_via: "id (no such component)".into(),
            });
        }
    }

    let members: Vec<String> = resolved_ids.into_iter().collect();
    let mut notes: Vec<String> = Vec::new();
    if members.is_empty() {
        notes.push("all members unresolved".into());
    }

    SubsystemEntry {
        id: sub.id.clone(),
        role: sub.role.clone(),
        lifecycle_roles: sub.lifecycle_roles.clone(),
        rationale: sub.rationale.clone(),
        evidence_grade: sub.evidence_grade,
        evidence_fields: sub.evidence_fields.clone(),
        members,
        member_evidence: evidence,
        notes,
    }
}

fn is_glob_form(member: &str) -> bool {
    member.contains('/') || member.contains('*')
}

fn match_glob<'a>(matcher: &GlobMatcher, live: &'a [&'a ComponentEntry]) -> Vec<&'a ComponentEntry> {
    live.iter()
        .copied()
        .filter(|c| {
            c.path_segments
                .iter()
                .any(|seg| matcher.is_match(Path::new(&seg.path)))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use atlas_index::PathSegment;
    use component_ontology::EvidenceGrade;
    use std::path::PathBuf;

    fn comp(id: &str, path: &str) -> ComponentEntry {
        ComponentEntry {
            id: id.into(),
            parent: None,
            kind: "rust-library".into(),
            lifecycle_roles: vec![],
            language: None,
            build_system: None,
            role: None,
            path_segments: vec![PathSegment {
                path: PathBuf::from(path),
                content_sha: "0".repeat(64),
            }],
            manifests: vec![],
            doc_anchors: vec![],
            evidence_grade: EvidenceGrade::Strong,
            evidence_fields: vec![],
            rationale: "x".into(),
            deleted: false,
        }
    }

    fn override_with_members(id: &str, members: Vec<String>) -> SubsystemOverride {
        SubsystemOverride {
            id: id.into(),
            members,
            role: None,
            lifecycle_roles: vec![],
            rationale: "x".into(),
            evidence_grade: EvidenceGrade::Strong,
            evidence_fields: vec![],
        }
    }

    #[test]
    fn empty_input_yields_empty_output() {
        let out = resolve_subsystems(&[], &[]);
        assert!(out.is_empty());
    }

    #[test]
    fn glob_resolves_against_path_segments() {
        let comps = vec![
            comp("auth-service", "services/auth"),
            comp("auth-tools", "services/auth/tools"),
            comp("storage", "services/storage"),
        ];
        let subs = vec![override_with_members(
            "auth",
            vec!["services/auth/*".into()],
        )];
        let out = resolve_subsystems(&subs, &comps);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].members, vec!["auth-tools"]);
        assert_eq!(out[0].member_evidence.len(), 1);
        assert_eq!(out[0].member_evidence[0].matched_via, "services/auth/*");
    }

    #[test]
    fn id_form_resolves_directly() {
        let comps = vec![comp("identity-core", "libs/identity")];
        let subs = vec![override_with_members("auth", vec!["identity-core".into()])];
        let out = resolve_subsystems(&subs, &comps);
        assert_eq!(out[0].members, vec!["identity-core"]);
        assert_eq!(out[0].member_evidence[0].matched_via, "id");
    }

    #[test]
    fn glob_with_zero_matches_emits_no_matches_evidence() {
        let comps = vec![comp("storage", "services/storage")];
        let subs = vec![override_with_members(
            "auth",
            vec!["services/auth/*".into()],
        )];
        let out = resolve_subsystems(&subs, &comps);
        assert!(out[0].members.is_empty());
        assert_eq!(out[0].notes, vec!["all members unresolved".to_string()]);
        assert_eq!(
            out[0].member_evidence[0].matched_via,
            "services/auth/* (no matches)"
        );
    }

    #[test]
    fn unknown_id_form_emits_no_such_component_evidence() {
        let subs = vec![override_with_members("auth", vec!["nonexistent".into()])];
        let out = resolve_subsystems(&subs, &[]);
        assert!(out[0].members.is_empty());
        assert_eq!(out[0].notes, vec!["all members unresolved".to_string()]);
        assert_eq!(
            out[0].member_evidence[0].matched_via,
            "id (no such component)"
        );
    }

    #[test]
    fn duplicate_glob_matches_dedupe_in_members_but_keep_evidence_first_form() {
        let comps = vec![comp("auth-service", "services/auth")];
        let subs = vec![override_with_members(
            "auth",
            vec!["services/auth".into(), "auth-service".into()],
        )];
        let out = resolve_subsystems(&subs, &comps);
        assert_eq!(out[0].members, vec!["auth-service"]);
        // First form ("services/auth") wins; second is a no-op dedupe.
        assert_eq!(out[0].member_evidence.len(), 1);
        assert_eq!(out[0].member_evidence[0].matched_via, "services/auth");
    }

    #[test]
    fn deleted_components_are_skipped() {
        let mut comps = vec![comp("auth-service", "services/auth")];
        comps[0].deleted = true;
        let subs = vec![override_with_members("auth", vec!["auth-service".into()])];
        let out = resolve_subsystems(&subs, &comps);
        assert!(out[0].members.is_empty());
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

```sh
cd ~/Development/Atlas && cargo test -p atlas-engine l9_subsystems 2>&1 | tail -15
```

Expected: compile errors — module not registered.

- [ ] **Step 3: Register the module**

In `crates/atlas-engine/src/lib.rs`, add next to the existing `mod l9_projections;`:

```rust
mod l9_subsystems;
```

And add to the public re-exports:

```rust
pub use l9_subsystems::subsystems_yaml_snapshot;
```

- [ ] **Step 4: Run tests to verify they pass**

```sh
cd ~/Development/Atlas && cargo test -p atlas-engine l9_subsystems 2>&1 | tail -10
```

Expected: 6 unit tests pass.

---

### Task 7: Cross-namespace collision check helper

**Files:**
- Modify: `~/Development/Atlas/crates/atlas-engine/src/l9_subsystems.rs`

- [ ] **Step 1: Write failing tests**

Append to `#[cfg(test)] mod tests` in `l9_subsystems.rs`:

```rust
    #[test]
    fn collision_check_passes_when_disjoint() {
        let comps = vec![comp("auth-service", "services/auth")];
        let subs = vec![override_with_members("auth", vec![])];
        let result = check_subsystem_namespace(&subs, &comps);
        assert!(result.is_ok());
    }

    #[test]
    fn collision_check_reports_id_clash() {
        let comps = vec![comp("auth", "services/auth")];
        let subs = vec![override_with_members("auth", vec![])];
        let err = check_subsystem_namespace(&subs, &comps).unwrap_err();
        assert_eq!(err, vec!["auth"]);
    }

    #[test]
    fn collision_check_reports_unknown_id_form_member() {
        let comps = vec![comp("auth-service", "services/auth")];
        let subs = vec![override_with_members(
            "auth",
            vec!["nonexistent".into()],
        )];
        let err = check_subsystem_id_members(&subs, &comps).unwrap_err();
        assert_eq!(err, vec!["auth/nonexistent".to_string()]);
    }

    #[test]
    fn collision_check_id_member_present_passes() {
        let comps = vec![comp("identity-core", "libs/identity")];
        let subs = vec![override_with_members(
            "auth",
            vec!["identity-core".into()],
        )];
        assert!(check_subsystem_id_members(&subs, &comps).is_ok());
    }
```

- [ ] **Step 2: Run tests to verify they fail**

```sh
cd ~/Development/Atlas && cargo test -p atlas-engine l9_subsystems::tests::collision 2>&1 | tail -10
```

Expected: compile errors.

- [ ] **Step 3: Implement helpers**

Add to `l9_subsystems.rs` (top-level):

```rust
/// Returns the sorted set of subsystem ids that collide with component ids.
/// Hard error in the post-L4 validation stage.
pub fn check_subsystem_namespace(
    overrides: &[SubsystemOverride],
    components: &[ComponentEntry],
) -> Result<(), Vec<String>> {
    let component_ids: BTreeSet<&str> = components
        .iter()
        .filter(|c| !c.deleted)
        .map(|c| c.id.as_str())
        .collect();
    let subsystem_ids: BTreeSet<&str> = overrides.iter().map(|s| s.id.as_str()).collect();
    let mut collisions: Vec<String> = component_ids
        .intersection(&subsystem_ids)
        .map(|s| (*s).to_string())
        .collect();
    if collisions.is_empty() {
        Ok(())
    } else {
        collisions.sort();
        Err(collisions)
    }
}

/// Returns the sorted `<subsystem-id>/<member-id>` pairs whose id-form
/// member does not resolve to any component. Hard error in the post-L4
/// validation stage.
pub fn check_subsystem_id_members(
    overrides: &[SubsystemOverride],
    components: &[ComponentEntry],
) -> Result<(), Vec<String>> {
    let component_ids: BTreeSet<&str> = components
        .iter()
        .filter(|c| !c.deleted)
        .map(|c| c.id.as_str())
        .collect();
    let mut bad: Vec<String> = Vec::new();
    for sub in overrides {
        for member in &sub.members {
            if !is_glob_form(member) && !component_ids.contains(member.as_str()) {
                bad.push(format!("{}/{}", sub.id, member));
            }
        }
    }
    if bad.is_empty() {
        Ok(())
    } else {
        bad.sort();
        Err(bad)
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

```sh
cd ~/Development/Atlas && cargo test -p atlas-engine l9_subsystems 2>&1 | tail -10
```

Expected: 4 new tests pass; previous 6 still pass.

- [ ] **Step 5: Re-export the helpers**

In `crates/atlas-engine/src/lib.rs`, extend the `pub use l9_subsystems::{...}` line:

```rust
pub use l9_subsystems::{
    check_subsystem_id_members, check_subsystem_namespace, subsystems_yaml_snapshot,
};
```

- [ ] **Step 6: Verify clean engine compile**

```sh
cd ~/Development/Atlas && cargo check -p atlas-engine 2>&1 | tail -5
```

Expected: clean.

---

## Phase 3 — Atlas CLI integration

### Task 8: Extend `validate-overrides` to subsystems

**Files:**
- Modify: `~/Development/Atlas/crates/atlas-cli/src/validate.rs`
- Modify: `~/Development/Atlas/crates/atlas-cli/src/main.rs`

- [ ] **Step 1: Write failing tests**

Append to `crates/atlas-cli/src/validate.rs` `#[cfg(test)] mod tests`:

```rust
    use atlas_index::{SubsystemOverride, SubsystemsOverridesFile};

    #[test]
    fn validate_subsystems_flags_duplicate_ids() {
        let overrides = OverridesFile::default();
        let subs = SubsystemsOverridesFile {
            schema_version: 1,
            subsystems: vec![
                SubsystemOverride {
                    id: "auth".into(),
                    members: vec!["x".into()],
                    role: None,
                    lifecycle_roles: vec![],
                    rationale: "x".into(),
                    evidence_grade: component_ontology::EvidenceGrade::Strong,
                    evidence_fields: vec![],
                },
                SubsystemOverride {
                    id: "auth".into(),
                    members: vec!["y".into()],
                    role: None,
                    lifecycle_roles: vec![],
                    rationale: "x".into(),
                    evidence_grade: component_ontology::EvidenceGrade::Strong,
                    evidence_fields: vec![],
                },
            ],
        };
        let report = validate_overrides_with_subsystems(&overrides, &subs);
        assert!(
            report
                .errors()
                .any(|i| i.message.contains("duplicate subsystem id 'auth'")),
            "expected duplicate-id error, got: {:?}",
            report.issues
        );
    }

    #[test]
    fn validate_subsystems_flags_empty_members() {
        let overrides = OverridesFile::default();
        let subs = SubsystemsOverridesFile {
            schema_version: 1,
            subsystems: vec![SubsystemOverride {
                id: "auth".into(),
                members: vec![],
                role: None,
                lifecycle_roles: vec![],
                rationale: "x".into(),
                evidence_grade: component_ontology::EvidenceGrade::Strong,
                evidence_fields: vec![],
            }],
        };
        let report = validate_overrides_with_subsystems(&overrides, &subs);
        assert!(
            report
                .errors()
                .any(|i| i.message.contains("subsystem 'auth' has empty members")),
            "expected empty-members error, got: {:?}",
            report.issues
        );
    }

    #[test]
    fn validate_subsystems_passes_well_formed_input() {
        let overrides = OverridesFile::default();
        let subs = SubsystemsOverridesFile {
            schema_version: 1,
            subsystems: vec![SubsystemOverride {
                id: "auth".into(),
                members: vec!["services/auth/*".into()],
                role: None,
                lifecycle_roles: vec![],
                rationale: "x".into(),
                evidence_grade: component_ontology::EvidenceGrade::Strong,
                evidence_fields: vec![],
            }],
        };
        let report = validate_overrides_with_subsystems(&overrides, &subs);
        assert!(!report.has_errors(), "expected no errors, got: {:?}", report.issues);
    }
```

- [ ] **Step 2: Run tests to verify they fail**

```sh
cd ~/Development/Atlas && cargo test -p atlas-cli validate_subsystems 2>&1 | tail -10
```

Expected: compile errors — `validate_overrides_with_subsystems` doesn't exist.

- [ ] **Step 3: Implement extended validator**

Add to the top of `crates/atlas-cli/src/validate.rs`:

```rust
use atlas_index::SubsystemsOverridesFile;
```

Add the new function below the existing `validate_overrides`:

```rust
/// Like [`validate_overrides`] but also checks `SubsystemsOverridesFile`
/// for shape-level errors (duplicate ids, empty members). Cross-namespace
/// collision and id-resolution checks happen post-L4 in the engine.
pub fn validate_overrides_with_subsystems(
    overrides: &OverridesFile,
    subsystems: &SubsystemsOverridesFile,
) -> ValidationReport {
    let mut report = validate_overrides(overrides);
    let mut seen: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
    for sub in &subsystems.subsystems {
        if !seen.insert(sub.id.as_str()) {
            report.issues.push(ValidationIssue {
                severity: Severity::Error,
                pin_key: Some(sub.id.clone()),
                field: None,
                message: format!("duplicate subsystem id '{}'", sub.id),
                suggestion: None,
            });
        }
        if sub.members.is_empty() {
            report.issues.push(ValidationIssue {
                severity: Severity::Error,
                pin_key: Some(sub.id.clone()),
                field: Some("members".into()),
                message: format!(
                    "subsystem '{}' has empty members; remove the entry or add at least one glob/id",
                    sub.id
                ),
                suggestion: None,
            });
        }
    }
    report
}
```

- [ ] **Step 4: Run tests to verify they pass**

```sh
cd ~/Development/Atlas && cargo test -p atlas-cli validate_subsystems 2>&1 | tail -10
```

Expected: 3 new tests pass.

- [ ] **Step 5: Wire `validate-overrides` CLI subcommand**

In `crates/atlas-cli/src/main.rs`, find the `validate-overrides` subcommand handler. Locate the `load_or_default_overrides(...)` call and add:

```rust
let subsystems_path = output_dir.join("subsystems.overrides.yaml");
let subsystems = atlas_index::load_or_default_subsystems_overrides(&subsystems_path)?;
let report = crate::validate::validate_overrides_with_subsystems(&overrides, &subsystems);
```

(Replace the existing `validate_overrides(&overrides)` call with the new variant.) Adjust the `print_report` call to pass `subsystems_path` if the existing implementation prints a single path; otherwise keep the existing behaviour (the report itself carries enough context).

- [ ] **Step 6: Run cli tests**

```sh
cd ~/Development/Atlas && cargo test -p atlas-cli 2>&1 | tail -10
```

Expected: all green.

---

### Task 9: Wire pipeline.rs — load + set + save + post-L4 check

**Files:**
- Modify: `~/Development/Atlas/crates/atlas-cli/src/pipeline.rs`

- [ ] **Step 1: Write failing pipeline integration test**

Append to `crates/atlas-cli/tests/pipeline_integration.rs` (read the file first to match existing test setup):

```rust
#[test]
fn pipeline_emits_subsystems_yaml_when_overrides_present() {
    use atlas_index::{
        load_or_default_subsystems, save_subsystems_overrides_atomic, SubsystemOverride,
        SubsystemsOverridesFile, SUBSYSTEMS_OVERRIDES_SCHEMA_VERSION,
    };

    let (config, _tempdir) = setup_pipeline_fixture(); // existing helper
    let subs_path = config.output_dir.join("subsystems.overrides.yaml");
    let subs = SubsystemsOverridesFile {
        schema_version: SUBSYSTEMS_OVERRIDES_SCHEMA_VERSION,
        subsystems: vec![SubsystemOverride {
            id: "fixture-subsystem".into(),
            members: vec!["*".into()],
            role: None,
            lifecycle_roles: vec![],
            rationale: "test".into(),
            evidence_grade: component_ontology::EvidenceGrade::Strong,
            evidence_fields: vec![],
        }],
    };
    std::fs::create_dir_all(&config.output_dir).unwrap();
    save_subsystems_overrides_atomic(&subs_path, &subs).unwrap();

    run_index(config.clone()).unwrap();

    let out_path = config.output_dir.join("subsystems.yaml");
    assert!(out_path.exists(), "expected subsystems.yaml at {}", out_path.display());
    let loaded = load_or_default_subsystems(&out_path).unwrap();
    assert_eq!(loaded.subsystems.len(), 1);
    assert_eq!(loaded.subsystems[0].id, "fixture-subsystem");
}
```

(Use the existing `setup_pipeline_fixture` or equivalent; if the fixture name differs in the file, match what's there.)

- [ ] **Step 2: Run test to verify it fails**

```sh
cd ~/Development/Atlas && cargo test -p atlas-cli pipeline_emits_subsystems 2>&1 | tail -15
```

Expected: file not produced.

- [ ] **Step 3: Wire pipeline**

In `crates/atlas-cli/src/pipeline.rs`:

After the existing `let overrides_path = config.output_dir.join("components.overrides.yaml");`, add:

```rust
    let subsystems_overrides_path = config.output_dir.join("subsystems.overrides.yaml");
    let subsystems_path = config.output_dir.join("subsystems.yaml");
```

After the `load_or_default_overrides(...)` line, add:

```rust
    let subsystems_overrides = atlas_index::load_or_default_subsystems_overrides(
        &subsystems_overrides_path,
    )
    .map_err(IndexError::Other)?;
```

Replace the `validate_overrides(&overrides)` call with:

```rust
    let validation =
        crate::validate::validate_overrides_with_subsystems(&overrides, &subsystems_overrides);
```

After `db.set_components_overrides(overrides);`, add:

```rust
    db.set_subsystems_overrides(subsystems_overrides.clone());
```

After the L9 projections section (after `let related_file = (*related_components_yaml_snapshot(&db)).clone();`), add:

```rust
    let live_components_for_check: Vec<atlas_index::ComponentEntry> =
        atlas_engine::all_components(&db)
            .iter()
            .filter(|c| !c.deleted)
            .cloned()
            .collect();
    if let Err(collisions) =
        atlas_engine::check_subsystem_namespace(&subsystems_overrides.subsystems, &live_components_for_check)
    {
        return Err(IndexError::Other(anyhow::anyhow!(
            "subsystem id(s) {:?} collide with component ids; rename the subsystem(s)",
            collisions
        )));
    }
    if let Err(bad) =
        atlas_engine::check_subsystem_id_members(&subsystems_overrides.subsystems, &live_components_for_check)
    {
        return Err(IndexError::Other(anyhow::anyhow!(
            "id-form member(s) {:?} do not resolve to any component (use a glob if the path is forward-looking)",
            bad
        )));
    }

    let mut subsystems_file =
        (*atlas_engine::subsystems_yaml_snapshot(&db)).clone();
```

Then in the `if !config.dry_run` save block, add (after `save_related_components_atomic(...)`):

```rust
        // Stable generated_at: reuse prior timestamp if subsystems.yaml
        // is otherwise unchanged, so byte-identity holds on no-op re-runs.
        let prior_subsystems = atlas_index::load_or_default_subsystems(&subsystems_path)
            .unwrap_or_default();
        subsystems_file.generated_at = stable_generated_at_subsystems(
            &subsystems_path,
            &prior_subsystems,
            &subsystems_file,
            SystemTime::now(),
        );
        atlas_index::save_subsystems_atomic(&subsystems_path, &subsystems_file)
            .map_err(IndexError::Other)?;
```

Add the helper at the bottom of the file (mirror the existing `stable_generated_at` for `components.yaml`):

```rust
fn stable_generated_at_subsystems(
    path: &Path,
    prior: &atlas_index::SubsystemsFile,
    new: &atlas_index::SubsystemsFile,
    now: SystemTime,
) -> String {
    if !path.exists() {
        return format_iso8601(now);
    }
    let mut new_blank = new.clone();
    new_blank.generated_at = String::new();
    let mut prior_blank = prior.clone();
    prior_blank.generated_at = String::new();
    if new_blank == prior_blank {
        prior.generated_at.clone()
    } else {
        format_iso8601(now)
    }
}
```

(Use the same `format_iso8601` helper that `stable_generated_at` already uses; if it's local-only, duplicate or hoist it.)

- [ ] **Step 4: Run integration test to verify it passes**

```sh
cd ~/Development/Atlas && cargo test -p atlas-cli pipeline_emits_subsystems 2>&1 | tail -10
```

Expected: pass.

- [ ] **Step 5: Run full atlas-cli test suite**

```sh
cd ~/Development/Atlas && cargo test -p atlas-cli 2>&1 | tail -10
```

Expected: all green.

---

## Phase 4 — End-to-end fixtures

### Task 10: Rename-stability fixture

**Files:**
- Create: `~/Development/Atlas/crates/atlas-cli/tests/subsystems_integration.rs`

- [ ] **Step 1: Write the rename-stability test**

```rust
//! End-to-end fixtures for subsystem seeding: rename-stability and
//! cross-namespace collision halt.

mod common; // existing harness, if present; otherwise inline a fixture builder

use atlas_index::{
    load_or_default_subsystems, save_subsystems_overrides_atomic, SubsystemOverride,
    SubsystemsOverridesFile, SUBSYSTEMS_OVERRIDES_SCHEMA_VERSION,
};
use component_ontology::EvidenceGrade;

#[test]
fn rename_stable_glob_re_matches_after_directory_rename() {
    // First run: hand-author one glob ("services/auth/*") and one id
    // ("identity-core"). Both should resolve.
    let fixture = build_two_dir_fixture();
    let config = config_for(&fixture);

    let subs_path = config.output_dir.join("subsystems.overrides.yaml");
    std::fs::create_dir_all(&config.output_dir).unwrap();
    save_subsystems_overrides_atomic(
        &subs_path,
        &SubsystemsOverridesFile {
            schema_version: SUBSYSTEMS_OVERRIDES_SCHEMA_VERSION,
            subsystems: vec![SubsystemOverride {
                id: "auth".into(),
                members: vec!["services/auth/*".into(), "identity-core".into()],
                role: None,
                lifecycle_roles: vec![],
                rationale: "test".into(),
                evidence_grade: EvidenceGrade::Strong,
                evidence_fields: vec![],
            }],
        },
    )
    .unwrap();

    run_index(config.clone()).unwrap();
    let out = load_or_default_subsystems(&config.output_dir.join("subsystems.yaml")).unwrap();
    let auth = &out.subsystems[0];
    assert!(auth.members.contains(&"identity-core".to_string()));
    let glob_match_count = auth
        .member_evidence
        .iter()
        .filter(|m| m.matched_via == "services/auth/*")
        .count();
    assert!(glob_match_count >= 1, "glob should match at least one component");

    // Second run: rename the auth dir; expect glob to still match
    // (since the new path is also under services/auth/*) and id form to
    // match by id (rename is for path, not id).
    rename_dir(
        fixture.root.join("services/auth/handlers"),
        fixture.root.join("services/auth/api"),
    );

    run_index(config.clone()).unwrap();
    let out2 = load_or_default_subsystems(&config.output_dir.join("subsystems.yaml")).unwrap();
    let auth2 = &out2.subsystems[0];
    assert!(auth2.members.contains(&"identity-core".to_string()));
    assert!(auth2
        .member_evidence
        .iter()
        .any(|m| m.matched_via == "services/auth/*"));
}
```

The test depends on a `build_two_dir_fixture` helper that creates `services/auth/handlers/Cargo.toml` and `libs/identity/Cargo.toml`, plus `config_for` that turns a fixture into an `IndexConfig`. If `tests/common/` exists, extend it; otherwise inline the helpers in this file.

- [ ] **Step 2: Run test to verify it passes**

```sh
cd ~/Development/Atlas && cargo test -p atlas-cli rename_stable_glob 2>&1 | tail -15
```

Expected: pass (the implementation should already cover this case; the test is the spec of the behaviour).

---

### Task 11: Cross-namespace collision halt fixture

**Files:**
- Modify: `~/Development/Atlas/crates/atlas-cli/tests/subsystems_integration.rs`

- [ ] **Step 1: Write failing test**

Append:

```rust
#[test]
fn pipeline_halts_on_subsystem_id_collision_with_component() {
    // Build a fixture whose root manifest produces a top-level
    // component slug "auth", then hand-author a subsystem also
    // named "auth". Pipeline must halt with a collision error
    // before saving subsystems.yaml.
    let fixture = build_single_dir_fixture("auth"); // helper ⇒ component slug "auth"
    let config = config_for(&fixture);

    std::fs::create_dir_all(&config.output_dir).unwrap();
    save_subsystems_overrides_atomic(
        &config.output_dir.join("subsystems.overrides.yaml"),
        &SubsystemsOverridesFile {
            schema_version: SUBSYSTEMS_OVERRIDES_SCHEMA_VERSION,
            subsystems: vec![SubsystemOverride {
                id: "auth".into(),
                members: vec!["*".into()],
                role: None,
                lifecycle_roles: vec![],
                rationale: "test".into(),
                evidence_grade: EvidenceGrade::Strong,
                evidence_fields: vec![],
            }],
        },
    )
    .unwrap();

    let err = run_index(config.clone()).unwrap_err().to_string();
    assert!(
        err.contains("collide with component ids"),
        "expected collision error, got: {err}"
    );
    assert!(
        !config.output_dir.join("subsystems.yaml").exists(),
        "subsystems.yaml must not be saved when collision halts the pipeline"
    );
}
```

- [ ] **Step 2: Run test to verify it passes**

```sh
cd ~/Development/Atlas && cargo test -p atlas-cli pipeline_halts_on_subsystem 2>&1 | tail -10
```

Expected: pass (implementation already in place).

---

### Task 12: Byte-identity re-run fixture

**Files:**
- Modify: `~/Development/Atlas/crates/atlas-cli/tests/subsystems_integration.rs`

- [ ] **Step 1: Write failing test**

Append:

```rust
#[test]
fn subsystems_yaml_is_byte_identical_on_no_op_re_run() {
    let fixture = build_two_dir_fixture();
    let config = config_for(&fixture);

    std::fs::create_dir_all(&config.output_dir).unwrap();
    save_subsystems_overrides_atomic(
        &config.output_dir.join("subsystems.overrides.yaml"),
        &SubsystemsOverridesFile {
            schema_version: SUBSYSTEMS_OVERRIDES_SCHEMA_VERSION,
            subsystems: vec![SubsystemOverride {
                id: "auth".into(),
                members: vec!["services/auth/*".into()],
                role: None,
                lifecycle_roles: vec![],
                rationale: "test".into(),
                evidence_grade: EvidenceGrade::Strong,
                evidence_fields: vec![],
            }],
        },
    )
    .unwrap();

    run_index(config.clone()).unwrap();
    let first = std::fs::read(config.output_dir.join("subsystems.yaml")).unwrap();
    run_index(config.clone()).unwrap();
    let second = std::fs::read(config.output_dir.join("subsystems.yaml")).unwrap();
    assert_eq!(first, second, "subsystems.yaml must be byte-identical on no-op re-run");
}
```

- [ ] **Step 2: Run test to verify it passes**

```sh
cd ~/Development/Atlas && cargo test -p atlas-cli subsystems_yaml_is_byte 2>&1 | tail -10
```

Expected: pass.

---

### Task 13: README "Pre-seeding subsystems" subsection

**Files:**
- Modify: `~/Development/Atlas/README.md`

- [ ] **Step 1: Add the subsection**

In the existing **Usage** section of `README.md`, insert after the "Common flags" table:

````markdown
### Pre-seeding subsystems

A *subsystem* is a named group of components with hand-drawn boundaries.
Atlas reads them from `subsystems.overrides.yaml` (alongside `.atlas/`)
and emits the resolved boundaries into `.atlas/subsystems.yaml`.

```yaml
# subsystems.overrides.yaml
schema_version: 1
subsystems:
  - id: auth
    members:
      - services/auth/*           # glob: contains '/' or '*'
      - libs/identity             # glob
      - identity-core             # id: no '/' and no '*'
    role: identity-and-authorisation
    rationale: "owns all session/token surfaces"
    evidence_grade: strong
```

A `members` entry containing `/` or `*` is treated as a path glob and
matched against component path segments; otherwise it is treated as a
component id and looked up directly. Globs that match zero components
produce a warning; id forms that don't resolve are a hard error.

Subsystem ids share a namespace with component ids — `atlas index`
will halt before saving if a subsystem id collides with a component id.

Validate without running the pipeline:

```sh
atlas validate-overrides
```
````

- [ ] **Step 2: Sanity check the README still renders**

```sh
cd ~/Development/Atlas && grep -nE "^### Pre-seeding subsystems" README.md
```

Expected: matches the new subsection.

---

## Final Verification

- [ ] **Step 1: Full workspace tests pass**

```sh
cd ~/Development/atlas-contracts && cargo test --workspace 2>&1 | tail -10
cd ~/Development/Atlas && cargo test --workspace 2>&1 | tail -15
```

Expected: all green in both workspaces.

- [ ] **Step 2: Lints clean**

```sh
cd ~/Development/Atlas && cargo clippy --workspace --all-targets -- -D warnings 2>&1 | tail -20
cargo fmt --all -- --check 2>&1 | tail -5
```

Expected: clean. Per the user's standing instruction (memory `feedback_fix_all_lints`), fix any warnings or fmt drift surfaced anywhere in the workspace, not just in code touched by this plan.

- [ ] **Step 3: Atlas leaves uncommitted Atlas-side changes for the analyse-work phase to commit.**

Per the run-plan work phase instructions, do not commit Atlas changes here. atlas-contracts changes are committed inside Phase 1 because that repo runs outside the Atlas plan harness.

---

## Self-Review (against spec)

- ✅ **Spec coverage:** Every spec section maps to tasks — schemas (1, 2, 4), yaml_io (3), engine input (5), projection (6), validation (7, 8), pipeline (9), tests (10, 11, 12), docs (13).
- ✅ **No placeholders.** Every step has concrete code or a concrete command.
- ✅ **Type consistency.** `SubsystemOverride`, `SubsystemEntry`, `SubsystemsOverridesFile`, `SubsystemsFile`, `MemberEvidence` used consistently. `subsystems_yaml_snapshot`, `check_subsystem_namespace`, `check_subsystem_id_members`, `validate_overrides_with_subsystems` likewise.
- ⚠ **Two-stage validation distinction explicit:** pre-LLM stage uses `validate_overrides_with_subsystems` (shape, dup ids, empty input). Post-L4 stage uses `check_subsystem_namespace` + `check_subsystem_id_members` (id existence, namespace collision). Both must run.
- ⚠ **`validate_participant_namespace` (Task 4) is added but not yet called.** This is intentional: the function gives downstream consumers (and a future task wiring it into `related-components.yaml` validation) the helper they need. Wiring it into edge validation is out of v1 scope per the spec.
