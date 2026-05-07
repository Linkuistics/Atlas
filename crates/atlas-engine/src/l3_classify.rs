//! L3 classification — decide whether a candidate directory is a
//! component and, if so, what kind. Resolution order (§4.1):
//!
//! 1. **Pin short-circuit.** An entry in `overrides.pins` keyed by the
//!    candidate's would-be id wins over everything; no LLM is
//!    consulted.
//! 2. **Deterministic rule.** The tabulated rules in
//!    [`crate::heuristics`] inspect the rationale bundle and any
//!    pre-loaded manifest contents; if one matches, it becomes the
//!    classification.
//! 3. **LLM fallback.** Ambiguous candidates are handed to
//!    [`atlas_llm::PromptId::Classify`] with the rationale bundle and
//!    manifest snippets as inputs; the response is parsed into
//!    [`Classification`].
//!
//! The query is keyed by `(workspace, candidate_dir)` rather than by a
//! full `Candidate`. Salsa memoises the underlying L1 signal queries,
//! so rebuilding the bundle inside L3 is free and keeps the query-key
//! type to primitives. This avoids forcing `salsa::Update` onto
//! nested signal types.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use atlas_analyzers::{
    cargo_classifier::CargoClassificationOutput,
    dispatcher::DispatchOutcome,
    dockerfile_classifier::DockerfileClassificationOutput,
    llm_classify::{LlmClassifyOutput, LlmHook, LlmHookError},
    python_classifier::PythonClassificationOutput,
    ts_js_classifier::TsJsClassificationOutput,
    AnalysisContext, Target, TargetFile,
};
use atlas_index::{CostClass, OverridesFile, PinValue, Stage};
use atlas_llm::{LlmRequest, PromptId, ResponseSchema};
use component_ontology::{EvidenceGrade, LifecycleScope};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::db::{AtlasDatabase, Workspace};
use crate::defaults::{
    parse_embedded, render_kinds_for_prompt, render_lifecycle_scopes_for_prompt,
};
use crate::heuristics::{classify_deterministic, ManifestContents};
use crate::l1_queries::{doc_headings, file_content, git_boundaries, manifests_in, shebangs};
use crate::roots::best_root_for;
use crate::types::{Classification, ComponentKind, RationaleBundle};

/// Maximum bytes of each manifest passed to the LLM. Generous enough
/// for any real Cargo.toml / package.json, small enough to keep
/// prompts cheap.
const MANIFEST_SNIPPET_LIMIT: usize = 16 * 1024;

/// Keys produced by [`build_llm_inputs`]. Single source of truth for the
/// bidirectional template/builder coverage check in
/// [`crate::prompt_token_coverage`]: validated against `classify.md` at
/// compile time, and against the runtime builder output by a unit test.
pub(crate) const BUILD_INPUTS_KEYS: &[&str] = &[
    "DIR_RELATIVE",
    "RATIONALE_BUNDLE",
    "MANIFEST_CONTENTS",
    "COMPONENT_KINDS",
    "LIFECYCLE_SCOPES",
];

/// Keys [`build_llm_inputs`] supplies for cache-key fingerprinting only,
/// never referenced by the rendered prompt. Empty for L3.
pub(crate) const CACHE_ONLY_KEYS: &[&str] = &[];

/// Classify the candidate whose directory is `candidate_dir`. Returns
/// `Arc<Classification>` so callers can cache the result cheaply.
///
/// Resolution order under PR-5's analyser-registry seam:
///
/// 1. **Pin short-circuit.** A user pin trumps every analyser.
/// 2. **Deterministic registry pass.** The
///    [`atlas_analyzers::AnalyzerRegistry`] dispatches every
///    `DeterministicCheap` / `DeterministicExpensive` analyser at
///    L3. Cargo and Dockerfile classification live here.
/// 3. **Legacy non-Cargo deterministic rules.** npm, pyproject, and
///    bare-git rules live in [`crate::heuristics::classify_deterministic`].
///    They run only after the registry's deterministic pass declines.
/// 4. **LLM-classify analyser.** The registry's LLM-cost-class pass
///    runs against an [`AnalysisContext`] carrying a hook bound to
///    [`AtlasDatabase::call_llm_cached`].
/// 5. **Weak-unknown fallback.** A last-resort classification when
///    even the LLM cannot produce a verdict (e.g. Setup error).
///
/// The L3+ stage cache fingerprint (PR-10's responsibility for
/// wiring) includes the analyser registry's
/// [`add_analyzer_registry_sha`](crate::FingerprintBuilder::add_analyzer_registry_sha)
/// contribution, so a registry shape change invalidates every
/// downstream entry automatically.
pub fn is_component(
    db: &AtlasDatabase,
    workspace: Workspace,
    candidate_dir: PathBuf,
) -> Arc<Classification> {
    // 1. Pin short-circuit runs before any expensive read of
    //    manifest bytes so a user's hand-authored decision always
    //    wins without LLM expense.
    let dyn_db: &dyn salsa::Database = db;
    let overrides = workspace.components_overrides(dyn_db).clone();
    // Multi-root: pin lookup tries every root in turn, since a
    // candidate dir under root[i] is relativised against root[i] for
    // the relative-path key form. The "best" root is the longest
    // prefix of the candidate, which `pinned_classification` selects
    // internally via the supplied roots slice.
    let roots = workspace.roots(dyn_db).clone();
    if let Some(classification) = pinned_classification(&overrides, &candidate_dir, &roots) {
        return Arc::new(classification);
    }

    // Build the rationale bundle by scoping L1 queries to the
    // candidate directory.
    let bundle = build_bundle(db, workspace, &candidate_dir);

    // Pre-load manifest contents for the deterministic rules + LLM
    // inputs.  Content reads go through `file_content` (untracked
    // helper), which pulls through the tracked `File::bytes` edge.
    let snippets = load_manifest_snippets(db, &bundle.manifests);

    let owning_root = best_root_for(&roots, &candidate_dir)
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| roots[0].clone());

    // 2. Build the analyser Target from the candidate's pre-loaded
    //    manifests and dispatch the registry's deterministic pass
    //    (cost class `Deterministic*`). Cargo + Dockerfile live here.
    let target = build_analyzer_target(&candidate_dir, &snippets, db);
    let registry = db.analyzer_registry();

    // L3 stage fingerprint (PR-10) for the LLM-classify branch. Per
    // design §8.1 the L3 cache key contributes:
    //
    // - `analyzer_registry_sha` — registry-shape change invalidates;
    // - `llm_fingerprint` — model / backend / template / ontology;
    // - `prompt_sha` — the *rendered* classify prompt sha, computed
    //   over the JSON-canonicalised inputs the hook will pass to the
    //   backend. This is what discriminates between candidate
    //   directories that share manifest content but differ in
    //   rationale (doc headings, shebangs, git-root flag);
    // - `file_content_sha` — every manifest the analyser consumed,
    //   so a manifest content change invalidates only the entries
    //   that read that manifest.
    //
    // The deterministic registry pass below does not consult this
    // fingerprint; it short-circuits on Cargo / Dockerfile rules.
    // The persistent cache only helps the LLM-classify branch.
    let llm_inputs_for_fingerprint =
        build_llm_inputs(&owning_root, &candidate_dir, &bundle, &snippets);
    let rendered_prompt_sha = sha256_hex_bytes(
        serde_json::to_string(&llm_inputs_for_fingerprint)
            .unwrap_or_default()
            .as_bytes(),
    );
    // The L3 stage cache fingerprint only gates the LLM-classify
    // branch (the deterministic registry pass short-circuits before
    // it). The (analyser_id, analyser_version) pair seeds the
    // fingerprint with the LLM analyser's identity so a future LLM
    // analyser bump invalidates entries cleanly. PR-4 deletes the
    // legacy `l3-driver` placeholder; the analyser-registry-sha
    // contribution below remains the canonical drift signal across
    // the whole L3 dispatch.
    let l3_fingerprint = {
        let llm_fp = workspace.llm_fingerprint(dyn_db);
        let mut fb = crate::FingerprintBuilder::new(
            Stage::L3,
            atlas_analyzers::llm_classify::ANALYZER_ID,
            atlas_analyzers::llm_classify::ANALYZER_VERSION,
        );
        fb.add_analyzer_registry_sha(&registry.registry_sha());
        fb.add_llm_fingerprint(llm_fp.as_ref());
        fb.add_prompt_sha(&rendered_prompt_sha);
        for tf in &target.manifests {
            fb.add_file_content_sha(&tf.content_sha);
        }
        fb.finalise()
    };

    let det_ctx = AnalysisContext::deterministic_only();
    let (det_outcome, det_id, det_version) =
        registry.dispatch_with_filter(&det_ctx, &target, Stage::L3, |c| {
            matches!(
                c,
                CostClass::DeterministicCheap | CostClass::DeterministicExpensive
            )
        });
    if let Some(classification) = adapt_dispatch_outcome(det_outcome, det_id, det_version) {
        return Arc::new(classification);
    }

    // 3. Legacy non-Cargo deterministic rules (npm, pyproject,
    //    bare-git). PR-5 keeps these in `crate::heuristics`; future
    //    PRs may migrate them to the registry too.
    let manifest_contents = ManifestContents {
        cargo_toml: snippet_text(&snippets, "Cargo.toml"),
        package_json: snippet_text(&snippets, "package.json"),
        pyproject_toml: snippet_text(&snippets, "pyproject.toml"),
    };
    let candidate = crate::types::Candidate {
        dir: candidate_dir.clone(),
        rationale_bundle: bundle.clone(),
    };
    if let Some(classification) = classify_deterministic(&candidate, &manifest_contents) {
        return Arc::new(classification);
    }

    // 4. LLM-classify analyser via the registry. The hook routes
    //    through `db.call_llm_cached_with_fp(...)` so per-candidate
    //    LLM verdicts are memoised through both the in-memory and
    //    persistent layers. The `Arc<dyn LlmHook>` here holds a
    //    `!Sync` `AtlasDatabase` (Salsa's `ZalsaLocal` is `!Sync`),
    //    which clippy correctly flags. The `LlmHook` trait drops
    //    `Sync` from its bounds (see its docstring) and the hook is
    //    only ever invoked from the same thread that constructed
    //    it; the `Arc` shape is dictated by the analyser
    //    crate's `AnalysisContext` API.
    #[allow(clippy::arc_with_non_send_sync)]
    let hook: Arc<dyn LlmHook> = Arc::new(EngineLlmHook {
        db: db.clone(),
        workspace_root: owning_root.clone(),
        candidate_dir: candidate_dir.clone(),
        bundle: bundle.clone(),
        snippets: snippets.clone(),
        stage_fingerprint: l3_fingerprint.clone(),
    });
    let llm_ctx = AnalysisContext::with_llm(hook);
    let (llm_outcome, llm_id, llm_version) =
        registry.dispatch_with_filter(&llm_ctx, &target, Stage::L3, |c| {
            matches!(c, CostClass::LlmCheap | CostClass::LlmExpensive)
        });
    if let Some(classification) = adapt_dispatch_outcome(llm_outcome, llm_id, llm_version) {
        return Arc::new(classification);
    }

    // 5. Weak-unknown fallback.
    Arc::new(unknown_classification(
        "no analyser produced a verdict (deterministic and LLM passes both declined)".into(),
    ))
}

/// Build a [`Target`] for the registry from the candidate's
/// pre-loaded manifests. Manifests live on `Target.manifests` keyed
/// by basename; `top_level_files` exposes the entry names directly
/// under `candidate_dir` so analysers can do quick presence checks.
fn build_analyzer_target(
    candidate_dir: &Path,
    snippets: &BTreeMap<PathBuf, String>,
    db: &AtlasDatabase,
) -> Target {
    let mut manifests = Vec::with_capacity(snippets.len());
    for (path, text) in snippets {
        if path.parent() != Some(candidate_dir) {
            // Only consider manifests that live directly under the
            // candidate dir — sub-component manifests should not feed
            // into the parent's classification.
            continue;
        }
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();
        let content_sha = sha256_hex_bytes(text.as_bytes());
        manifests.push(TargetFile {
            name,
            relpath: path
                .strip_prefix(candidate_dir)
                .unwrap_or(path)
                .to_path_buf(),
            bytes: text.as_bytes().to_vec(),
            content_sha,
        });
    }

    // For Dockerfile detection we also accept the file even when the
    // engine did not classify it as a "manifest" in
    // `manifest_patterns`. Probe the live filesystem via `file_content`
    // for a `Dockerfile` next to the candidate dir.
    if !manifests.iter().any(|m| m.name == "Dockerfile") {
        let dockerfile_path = candidate_dir.join("Dockerfile");
        if let Some(bytes) = file_content(db, &dockerfile_path) {
            let bytes_vec: Vec<u8> = bytes.as_ref().clone();
            let content_sha = sha256_hex_bytes(&bytes_vec);
            manifests.push(TargetFile {
                name: "Dockerfile".into(),
                relpath: PathBuf::from("Dockerfile"),
                bytes: bytes_vec,
                content_sha,
            });
        }
    }

    let mut top_level_files: Vec<String> = manifests.iter().map(|m| m.name.clone()).collect();
    // Best-effort: enumerate directory entries so analysers can probe
    // for `src/`, `Dockerfile`, etc. Failures are silently swallowed —
    // the engine's L1 walk owns canonical truth; this listing is only
    // an analyser hint.
    if let Ok(entries) = std::fs::read_dir(candidate_dir) {
        for e in entries.flatten() {
            if let Some(name) = e.file_name().to_str() {
                if !top_level_files.iter().any(|n| n == name) {
                    top_level_files.push(name.to_string());
                }
            }
        }
    }

    Target {
        dir: candidate_dir.to_path_buf(),
        languages: BTreeSet::new(),
        manifests,
        top_level_files,
    }
}

fn sha256_hex_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest: [u8; 32] = hasher.finalize().into();
    let mut hex = String::with_capacity(64);
    use std::fmt::Write;
    for b in digest {
        write!(&mut hex, "{b:02x}").expect("writing to String never fails");
    }
    hex
}

/// Translate a `(DispatchOutcome, analyser_id, analyser_version)`
/// triple from the registry into a `Classification`. Returns `None`
/// for `AllDeclined` so the caller can fall through to the next pass.
/// The `analyser_id` / `analyser_version` strings are stamped onto
/// the resulting [`Classification`] so the per-component projection
/// downstream can record which analyser produced the verdict.
fn adapt_dispatch_outcome(
    outcome: DispatchOutcome,
    analyser_id: &str,
    analyser_version: &str,
) -> Option<Classification> {
    match outcome {
        DispatchOutcome::AllDeclined { errors } => {
            for err in errors {
                eprintln!("warning: L3 analyser error: {err}");
            }
            None
        }
        DispatchOutcome::Confident { output, .. } => Some(classification_from_output(
            output,
            analyser_id,
            analyser_version,
        )),
        DispatchOutcome::Graded { output, .. } => Some(classification_from_output(
            output,
            analyser_id,
            analyser_version,
        )),
    }
}

/// Downcast a [`Box<dyn StageOutput>`] to one of the three concrete
/// output types this PR knows about, then translate to
/// [`Classification`]. Returns the weak-unknown classification on a
/// downcast miss (which would indicate an analyser registered with
/// an unexpected output type — a Phase 2 plugin). The `analyser_id`
/// / `analyser_version` strings flow into every produced
/// classification so the per-component YAML records who classified
/// the component.
fn classification_from_output(
    output: Box<dyn atlas_analyzers::StageOutput>,
    analyser_id: &str,
    analyser_version: &str,
) -> Classification {
    if let Some(cargo) = (*output)
        .as_any()
        .downcast_ref::<CargoClassificationOutput>()
    {
        return cargo_to_classification(cargo, analyser_id, analyser_version);
    }
    if let Some(docker) = (*output)
        .as_any()
        .downcast_ref::<DockerfileClassificationOutput>()
    {
        return docker_to_classification(docker, analyser_id, analyser_version);
    }
    if let Some(ts_js) = (*output)
        .as_any()
        .downcast_ref::<TsJsClassificationOutput>()
    {
        return ts_js_to_classification(ts_js, analyser_id, analyser_version);
    }
    if let Some(py) = (*output)
        .as_any()
        .downcast_ref::<PythonClassificationOutput>()
    {
        return python_to_classification(py, analyser_id, analyser_version);
    }
    if let Some(llm) = (*output).as_any().downcast_ref::<LlmClassifyOutput>() {
        return parse_llm_response(llm.response.clone(), analyser_id, analyser_version)
            .unwrap_or_else(|reason| {
                unknown_classification(format!("LLM response parse failed: {reason}"))
            });
    }
    unknown_classification(
        "analyser produced an output type the L3 adapter does not recognise".into(),
    )
}

fn cargo_to_classification(
    out: &CargoClassificationOutput,
    analyser_id: &str,
    analyser_version: &str,
) -> Classification {
    let kind = ComponentKind::parse(&out.kind).unwrap_or(ComponentKind::NonComponent);
    let lifecycle_roles = out
        .lifecycle_roles
        .iter()
        .filter_map(|s| LifecycleScope::parse(s))
        .collect();
    let mut languages = BTreeSet::new();
    languages.insert("rust".to_string());
    Classification {
        kind,
        languages,
        build_system: out.build_system.clone(),
        lifecycle_roles,
        role: out.role.clone(),
        evidence_grade: EvidenceGrade::Strong,
        evidence_fields: out.evidence_fields.clone(),
        rationale: out.rationale.clone(),
        is_boundary: out.is_boundary,
        analyser_id: analyser_id.to_string(),
        analyser_version: analyser_version.to_string(),
    }
}

fn docker_to_classification(
    out: &DockerfileClassificationOutput,
    analyser_id: &str,
    analyser_version: &str,
) -> Classification {
    let kind = ComponentKind::parse(&out.kind).unwrap_or(ComponentKind::NonComponent);
    let lifecycle_roles = out
        .lifecycle_roles
        .iter()
        .filter_map(|s| LifecycleScope::parse(s))
        .collect();
    Classification {
        kind,
        languages: BTreeSet::new(),
        build_system: out.build_system.clone(),
        lifecycle_roles,
        role: out.role.clone(),
        evidence_grade: EvidenceGrade::Strong,
        evidence_fields: out.evidence_fields.clone(),
        rationale: out.rationale.clone(),
        is_boundary: out.is_boundary,
        analyser_id: analyser_id.to_string(),
        analyser_version: analyser_version.to_string(),
    }
}

fn ts_js_to_classification(
    out: &TsJsClassificationOutput,
    analyser_id: &str,
    analyser_version: &str,
) -> Classification {
    let kind = ComponentKind::parse(&out.kind).unwrap_or(ComponentKind::NonComponent);
    let lifecycle_roles = out
        .lifecycle_roles
        .iter()
        .filter_map(|s| LifecycleScope::parse(s))
        .collect();
    let mut languages = BTreeSet::new();
    languages.insert(out.language.clone());
    Classification {
        kind,
        languages,
        build_system: out.build_system.clone(),
        lifecycle_roles,
        role: out.role.clone(),
        evidence_grade: EvidenceGrade::Strong,
        evidence_fields: out.evidence_fields.clone(),
        rationale: out.rationale.clone(),
        is_boundary: out.is_boundary,
        analyser_id: analyser_id.to_string(),
        analyser_version: analyser_version.to_string(),
    }
}

/// Translate a [`PythonClassificationOutput`] into a [`Classification`].
/// PR-3 of Phase 2: the new `python-package` kind lands here, and
/// the analyser identity flows through unchanged via PR-4's plumbing.
fn python_to_classification(
    out: &PythonClassificationOutput,
    analyser_id: &str,
    analyser_version: &str,
) -> Classification {
    let kind = ComponentKind::parse(&out.kind).unwrap_or(ComponentKind::NonComponent);
    let lifecycle_roles = out
        .lifecycle_roles
        .iter()
        .filter_map(|s| LifecycleScope::parse(s))
        .collect();
    let mut languages = BTreeSet::new();
    languages.insert(out.language.clone());
    Classification {
        kind,
        languages,
        build_system: out.build_system.clone(),
        lifecycle_roles,
        role: out.role.clone(),
        evidence_grade: EvidenceGrade::Strong,
        evidence_fields: out.evidence_fields.clone(),
        rationale: out.rationale.clone(),
        is_boundary: out.is_boundary,
        analyser_id: analyser_id.to_string(),
        analyser_version: analyser_version.to_string(),
    }
}

/// Engine-side [`LlmHook`] implementation. The
/// [`atlas_analyzers::LlmClassifyAnalyzer`] holds the trait shape;
/// this struct provides the actual prompt build + cached call. Lives
/// inside `atlas-engine` because building the inputs requires the
/// engine's prompt-rendering helpers.
struct EngineLlmHook {
    db: AtlasDatabase,
    workspace_root: PathBuf,
    candidate_dir: PathBuf,
    bundle: RationaleBundle,
    snippets: BTreeMap<PathBuf, String>,
    /// L3 stage fingerprint (PR-10) — contributors are assembled in
    /// [`is_component`] and are independent of whether the call goes
    /// to the LLM. Threaded through the hook so the persistent cache
    /// short-circuit fires on the LLM-cost-class pass.
    stage_fingerprint: crate::cache::Sha256Hex,
}

impl LlmHook for EngineLlmHook {
    fn classify(
        &self,
        _inputs: &serde_json::Value,
    ) -> Result<Arc<serde_json::Value>, LlmHookError> {
        // Build the canonical inputs from the engine's rationale
        // bundle (the analyser passes a placeholder payload — the
        // real prompt structure lives here).
        let inputs = build_llm_inputs(
            &self.workspace_root,
            &self.candidate_dir,
            &self.bundle,
            &self.snippets,
        );
        let request = LlmRequest {
            prompt_template: PromptId::Classify,
            inputs,
            schema: ResponseSchema::accept_any(),
        };
        match self
            .db
            .call_llm_cached_with_fp(Stage::L3, &self.stage_fingerprint, &request)
        {
            Ok(value) => Ok(value),
            Err(err) => Err(LlmHookError::Call(err.to_string())),
        }
    }
}

fn build_bundle(db: &AtlasDatabase, workspace: Workspace, candidate_dir: &Path) -> RationaleBundle {
    let dyn_db: &dyn salsa::Database = db;
    let manifests_here: Vec<PathBuf> = manifests_in(dyn_db, workspace, candidate_dir.to_path_buf())
        .iter()
        .filter(|m| m.parent() == Some(candidate_dir))
        .cloned()
        .collect();

    let git_dirs_here = git_boundaries(dyn_db, workspace, candidate_dir.to_path_buf());
    let is_git_root = git_dirs_here.iter().any(|d| d == candidate_dir);

    let doc_headings_here = doc_headings(dyn_db, workspace, candidate_dir.to_path_buf())
        .as_ref()
        .clone();
    let shebangs_here = shebangs(dyn_db, workspace, candidate_dir.to_path_buf())
        .as_ref()
        .clone();

    RationaleBundle {
        manifests: manifests_here,
        is_git_root,
        doc_headings: doc_headings_here,
        shebangs: shebangs_here,
    }
}

fn load_manifest_snippets(db: &AtlasDatabase, paths: &[PathBuf]) -> BTreeMap<PathBuf, String> {
    let mut out = BTreeMap::new();
    for path in paths {
        let bytes = match file_content(db, path) {
            Some(bytes) => bytes,
            None => continue,
        };
        let limit = bytes.len().min(MANIFEST_SNIPPET_LIMIT);
        let Ok(text) = std::str::from_utf8(&bytes[..limit]) else {
            continue;
        };
        out.insert(path.clone(), text.to_string());
    }
    out
}

fn snippet_text<'a>(snippets: &'a BTreeMap<PathBuf, String>, basename: &str) -> Option<&'a str> {
    snippets
        .iter()
        .find(|(path, _)| path.file_name().and_then(|n| n.to_str()) == Some(basename))
        .map(|(_, text)| text.as_str())
}

/// Look up a pin for the candidate under every key form L3 recognises.
/// Returns the pin's `Classification` or `None`.
///
/// Keys tried, in order:
/// 1. The candidate's dir-path relative to the workspace root (e.g.
///    `crates/Atlas-Engine`).
/// 2. The candidate dir's leaf basename (e.g. `Atlas-Engine`).
/// 3. The slugified relative path (e.g. `crates/atlas-engine`) — the
///    id shape a user sees in `components.yaml`, which diverges from
///    the raw path when the directory contains uppercase letters,
///    spaces, or other non-kebab characters.
/// 4. The slugified basename (e.g. `atlas-engine`).
/// 5. The `id` of any `overrides.additions` entry whose first
///    `path_segment` points at this dir — so an authored addition +
///    pin pair both key off the addition's explicit id.
fn pinned_classification(
    overrides: &OverridesFile,
    candidate_dir: &Path,
    roots: &[PathBuf],
) -> Option<Classification> {
    // Multi-root: relativise the candidate against the longest root
    // that contains it. Pre-vNext callers (single-root) see the same
    // behaviour because `roots = vec![root]` returns that one root as
    // the longest prefix.
    let owning_root = best_root_for(roots, candidate_dir)
        .unwrap_or_else(|| roots.first().map(|p| p.as_path()).unwrap_or(candidate_dir));
    let rel = candidate_dir
        .strip_prefix(owning_root)
        .unwrap_or(candidate_dir);
    let rel_str = path_to_forward_slash(rel);
    let basename = candidate_dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_string();

    let mut keys_tried: Vec<String> = Vec::new();
    let push_unique = |keys: &mut Vec<String>, key: String| {
        if !key.is_empty() && !keys.contains(&key) {
            keys.push(key);
        }
    };

    push_unique(&mut keys_tried, rel_str.clone());
    push_unique(&mut keys_tried, basename.clone());
    push_unique(&mut keys_tried, crate::identifiers::slugify_path(rel));
    if let Some(slug) = crate::identifiers::slugify_segment(&basename) {
        push_unique(&mut keys_tried, slug);
    }
    for addition in &overrides.additions {
        if let Some(first_seg) = addition.path_segments.first() {
            let abs_path = if first_seg.path.is_absolute() {
                first_seg.path.clone()
            } else {
                owning_root.join(&first_seg.path)
            };
            if abs_path == candidate_dir {
                push_unique(&mut keys_tried, addition.id.as_str().to_string());
            }
        }
    }

    for key in &keys_tried {
        // Pins are keyed by ComponentId; only well-formed component-id
        // strings can match. Strings that fail to parse (e.g. raw paths
        // with slashes that don't validate) simply won't match — that is
        // the same behaviour as before where they wouldn't be present
        // as String keys either.
        if let Ok(cid) = component_ontology::ComponentId::parse(key) {
            if let Some(pins) = overrides.pins.get(&cid) {
                return Some(pins_to_classification(pins));
            }
        }
    }
    None
}

fn pins_to_classification(pins: &BTreeMap<String, PinValue>) -> Classification {
    let kind_str = pin_string(pins.get("kind"));
    let kind = kind_str
        .as_deref()
        .and_then(ComponentKind::parse)
        .unwrap_or(ComponentKind::NonComponent);
    // Pin name is `language` (singular) — a hand-authored pin still
    // names one language at a time. The engine widens it to a
    // `BTreeSet<String>` because the rest of the schema uses the set
    // form; an empty pin produces an empty set.
    let language = pin_string(pins.get("language"));
    let mut languages = BTreeSet::new();
    if let Some(l) = language {
        languages.insert(l);
    }
    let build_system = pin_string(pins.get("build_system"));
    let role = pin_string(pins.get("role"));

    Classification {
        kind,
        languages,
        build_system,
        lifecycle_roles: Vec::new(),
        role,
        evidence_grade: EvidenceGrade::Strong,
        evidence_fields: pins.keys().map(|k| format!("pin:{k}")).collect(),
        rationale: "human pin".to_string(),
        is_boundary: !matches!(pins.get("suppress"), Some(PinValue::Suppress { .. })),
        // Hand-authored pins bypass the analyser registry, so the
        // sentinel `"override"` records the provenance without
        // pretending an analyser was consulted.
        analyser_id: OVERRIDE_ANALYSER_ID.to_string(),
        analyser_version: OVERRIDE_ANALYSER_VERSION.to_string(),
    }
}

fn pin_string(pin: Option<&PinValue>) -> Option<String> {
    match pin {
        Some(PinValue::Value { value, .. }) => Some(value.clone()),
        _ => None,
    }
}

fn path_to_forward_slash(path: &Path) -> String {
    path.components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

// `classify_via_llm` previously called `db.call_llm_cached` directly
// from `is_component`. PR-5 replaces it with the
// `EngineLlmHook` plus the `LlmClassifyAnalyzer` registered on the
// analyser registry — see the top of this file.

fn build_llm_inputs(
    workspace_root: &Path,
    candidate_dir: &Path,
    bundle: &RationaleBundle,
    snippets: &BTreeMap<PathBuf, String>,
) -> Value {
    let rel = candidate_dir
        .strip_prefix(workspace_root)
        .unwrap_or(candidate_dir);
    let rel_str = path_to_forward_slash(rel);

    let manifests_rel: Vec<String> = bundle
        .manifests
        .iter()
        .map(|m| {
            let rel = m.strip_prefix(workspace_root).unwrap_or(m);
            path_to_forward_slash(rel)
        })
        .collect();

    let doc_headings_json: Vec<Value> = bundle
        .doc_headings
        .iter()
        .map(|h| {
            let rel = h.path.strip_prefix(workspace_root).unwrap_or(&h.path);
            json!({
                "path": path_to_forward_slash(rel),
                "level": h.level,
                "text": h.text,
            })
        })
        .collect();

    let shebangs_json: Vec<Value> = bundle
        .shebangs
        .iter()
        .map(|s| {
            let rel = s.path.strip_prefix(workspace_root).unwrap_or(&s.path);
            json!({
                "path": path_to_forward_slash(rel),
                "interpreter": s.interpreter,
            })
        })
        .collect();

    let manifest_contents_json: BTreeMap<String, String> = snippets
        .iter()
        .map(|(path, text)| {
            let rel = path.strip_prefix(workspace_root).unwrap_or(path);
            (path_to_forward_slash(rel), text.clone())
        })
        .collect();

    // `{{COMPONENT_KINDS}}` and `{{LIFECYCLE_SCOPES}}` are referenced in
    // classify.md; without them prompt::render fails with an unknown-token
    // error and every LLM-fallback classification degrades to `unknown`.
    // Mirrors the L5/L6 `{{CATALOG_COMPONENTS}}` / `{{ONTOLOGY_KINDS}}`
    // injection pattern.
    let kinds_yaml = parse_embedded().expect("embedded component-kinds YAML must parse");
    let kinds_block = render_kinds_for_prompt(&kinds_yaml);
    let lifecycle_block = render_lifecycle_scopes_for_prompt(&kinds_yaml);

    json!({
        "DIR_RELATIVE": rel_str,
        "RATIONALE_BUNDLE": {
            "manifests": manifests_rel,
            "is_git_root": bundle.is_git_root,
            "doc_headings": doc_headings_json,
            "shebangs": shebangs_json,
        },
        "MANIFEST_CONTENTS": manifest_contents_json,
        "COMPONENT_KINDS": kinds_block,
        "LIFECYCLE_SCOPES": lifecycle_block,
    })
}

/// Parameterless wrapper around [`build_llm_inputs`] for the unified
/// prompt/builder coverage matrix in [`crate::prompt_token_coverage`].
/// Stubs out the workspace, candidate dir, rationale bundle, and
/// snippets so the matrix can call all four builders uniformly.
#[cfg(test)]
pub(crate) fn build_llm_inputs_for_tests() -> Value {
    let bundle = RationaleBundle {
        manifests: Vec::new(),
        is_git_root: false,
        doc_headings: Vec::new(),
        shebangs: Vec::new(),
    };
    let snippets = BTreeMap::new();
    build_llm_inputs(
        Path::new("/ws"),
        Path::new("/ws/some-dir"),
        &bundle,
        &snippets,
    )
}

fn parse_llm_response(
    value: Value,
    analyser_id: &str,
    analyser_version: &str,
) -> Result<Classification, String> {
    // Accept a Classification shape plus a handful of minor
    // deviations the LLM may introduce (missing optional fields,
    // integer-typed levels, etc.).
    let object = value
        .as_object()
        .ok_or_else(|| format!("expected JSON object, got {}", value))?;

    let kind_str = object
        .get("kind")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "response missing string `kind`".to_string())?;
    let kind =
        ComponentKind::parse(kind_str).ok_or_else(|| format!("unknown kind `{kind_str}`"))?;

    // The classifier prompt template asks the LLM for a single
    // `language` string (the v1 prompt has not been re-trained for
    // the polyglot case). Lift that single value into a one-element
    // `languages` set; if the LLM returns no language at all, the
    // set is empty.
    let mut languages: BTreeSet<String> = BTreeSet::new();
    if let Some(l) = object.get("language").and_then(|v| v.as_str()) {
        languages.insert(l.to_string());
    }
    if let Some(arr) = object.get("languages").and_then(|v| v.as_array()) {
        for item in arr {
            if let Some(s) = item.as_str() {
                languages.insert(s.to_string());
            }
        }
    }
    let build_system = object
        .get("build_system")
        .and_then(|v| v.as_str())
        .map(String::from);
    let role = object
        .get("role")
        .and_then(|v| v.as_str())
        .map(String::from);
    let rationale = object
        .get("rationale")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let is_boundary = object
        .get("is_boundary")
        .and_then(|v| v.as_bool())
        .unwrap_or(!matches!(kind, ComponentKind::NonComponent));
    let evidence_grade = object
        .get("evidence_grade")
        .and_then(|v| v.as_str())
        .and_then(EvidenceGrade::parse)
        .unwrap_or(EvidenceGrade::Medium);
    let evidence_fields = object
        .get("evidence_fields")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let lifecycle_roles = object
        .get("lifecycle_roles")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().and_then(LifecycleScope::parse))
                .collect()
        })
        .unwrap_or_default();

    Ok(Classification {
        kind,
        languages,
        build_system,
        lifecycle_roles,
        role,
        evidence_grade,
        evidence_fields,
        rationale,
        is_boundary,
        analyser_id: analyser_id.to_string(),
        analyser_version: analyser_version.to_string(),
    })
}

/// Sentinel id used when a hand-authored override (a pin or an
/// `overrides.additions` entry) supplies the classification rather
/// than an analyser. Distinct from
/// [`atlas_analyzers::NONE_ANALYZER_ID`] so the per-component YAML
/// can distinguish "user-authored" from "no analyser ran".
pub const OVERRIDE_ANALYSER_ID: &str = "override";

/// Sentinel version paired with [`OVERRIDE_ANALYSER_ID`].
pub const OVERRIDE_ANALYSER_VERSION: &str = "0.0.0";

/// Weakest possible "unknown" verdict — non-component, weak evidence,
/// `is_boundary: false`. Used as the L3 fallback when the LLM call or
/// response parse fails, and re-used by L8 to synthesise a verdict
/// when a per-subdir map call panics.
pub(crate) fn unknown_classification(reason: String) -> Classification {
    Classification {
        kind: ComponentKind::NonComponent,
        languages: BTreeSet::new(),
        build_system: None,
        lifecycle_roles: Vec::new(),
        role: None,
        evidence_grade: EvidenceGrade::Weak,
        evidence_fields: Vec::new(),
        rationale: reason,
        is_boundary: false,
        // The unknown verdict is the "no analyser produced a
        // verdict" fall-through; the dispatcher's
        // [`atlas_analyzers::NONE_ANALYZER_ID`] sentinel is the
        // matching identity for that path.
        analyser_id: atlas_analyzers::NONE_ANALYZER_ID.to_string(),
        analyser_version: atlas_analyzers::NONE_ANALYZER_VERSION.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    use atlas_index::{AlwaysTrue, ComponentEntry, PathSegment, PinValue};
    use component_ontology::EvidenceGrade;

    fn overrides_with_pin(id: &str, field: &str, value: &str) -> OverridesFile {
        let mut pins = BTreeMap::new();
        let mut field_pins = BTreeMap::new();
        field_pins.insert(
            field.to_string(),
            PinValue::Value {
                value: value.to_string(),
                reason: None,
            },
        );
        pins.insert(
            component_ontology::ComponentId::parse(id).unwrap(),
            field_pins,
        );
        OverridesFile {
            pins,
            ..OverridesFile::default()
        }
    }

    #[test]
    fn pin_matches_relative_path_key() {
        let overrides = overrides_with_pin("crates/foo", "kind", "spec");
        let got = pinned_classification(
            &overrides,
            Path::new("/ws/crates/foo"),
            &[PathBuf::from("/ws")],
        )
        .expect("pin should match relative path key");
        assert_eq!(got.kind, ComponentKind::Spec);
        assert_eq!(got.rationale, "human pin");
    }

    #[test]
    fn pin_matches_basename_key() {
        // User-friendly fallback: pin by bare basename when the
        // relative-path form isn't used.
        let overrides = overrides_with_pin("foo", "kind", "spec");
        let got = pinned_classification(
            &overrides,
            Path::new("/ws/crates/foo"),
            &[PathBuf::from("/ws")],
        )
        .expect("pin should match basename key");
        assert_eq!(got.kind, ComponentKind::Spec);
    }

    #[test]
    fn pin_matches_slugified_relative_path_for_mixed_case_dir() {
        // A user reading `id: crates/atlas-engine` from components.yaml
        // and pinning that id should match a directory named
        // `crates/Atlas-Engine` — the slugified id form is what they see
        // in the generated YAML.
        let overrides = overrides_with_pin("crates/atlas-engine", "kind", "rust-library");
        let got = pinned_classification(
            &overrides,
            Path::new("/ws/crates/Atlas-Engine"),
            &[PathBuf::from("/ws")],
        )
        .expect("pin should match slugified relative path");
        assert_eq!(got.kind, ComponentKind::RustLibrary);
    }

    #[test]
    fn pin_matches_slugified_basename_for_mixed_case_dir() {
        let overrides = overrides_with_pin("atlas-engine", "kind", "rust-library");
        let got = pinned_classification(
            &overrides,
            Path::new("/ws/crates/Atlas-Engine"),
            &[PathBuf::from("/ws")],
        )
        .expect("pin should match slugified basename");
        assert_eq!(got.kind, ComponentKind::RustLibrary);
    }

    #[test]
    fn pin_matches_addition_id() {
        // Authored addition + pin pair: the addition declares the id,
        // the pin key references that id.
        let mut overrides = overrides_with_pin("my-spec", "kind", "spec");
        overrides.additions.push(ComponentEntry {
            id: component_ontology::ComponentId::parse("my-spec").unwrap(),
            parent: None,
            kind: "spec".into(),
            lifecycle_roles: Vec::new(),
            languages: BTreeSet::new(),
            build_system: None,
            role: None,
            path_segments: vec![PathSegment {
                path: PathBuf::from("specs/my-spec"),
                content_sha: "0".into(),
            }],
            manifests: Vec::new(),
            doc_anchors: Vec::new(),
            evidence_grade: EvidenceGrade::Strong,
            evidence_fields: Vec::new(),
            rationale: "spec".into(),
            deleted: false,
        });
        let got = pinned_classification(
            &overrides,
            Path::new("/ws/specs/my-spec"),
            &[PathBuf::from("/ws")],
        )
        .expect("pin should match via addition id");
        assert_eq!(got.kind, ComponentKind::Spec);
    }

    #[test]
    fn suppress_pin_sets_is_boundary_false() {
        let mut field_pins = BTreeMap::new();
        field_pins.insert(
            "suppress".to_string(),
            PinValue::Suppress {
                suppress: AlwaysTrue,
            },
        );
        let mut pins = BTreeMap::new();
        pins.insert(
            component_ontology::ComponentId::parse("foo").unwrap(),
            field_pins,
        );
        let overrides = OverridesFile {
            pins,
            ..OverridesFile::default()
        };
        let got = pinned_classification(&overrides, Path::new("/ws/foo"), &[PathBuf::from("/ws")])
            .expect("suppress pin should produce a classification");
        assert!(!got.is_boundary);
    }

    #[test]
    fn parse_llm_response_accepts_minimal_object() {
        let value = serde_json::json!({
            "kind": "rust-library",
            "evidence_grade": "medium",
            "rationale": "some reason",
            "is_boundary": true,
        });
        let got = parse_llm_response(value, "llm-classify-fallback", "1.0.0").unwrap();
        assert_eq!(got.kind, ComponentKind::RustLibrary);
        assert!(got.is_boundary);
        assert_eq!(got.analyser_id, "llm-classify-fallback");
        assert_eq!(got.analyser_version, "1.0.0");
    }

    #[test]
    fn parse_llm_response_rejects_unknown_kind() {
        let value = serde_json::json!({
            "kind": "nonsense",
            "rationale": "x",
        });
        assert!(parse_llm_response(value, "llm-classify-fallback", "1.0.0").is_err());
    }

    #[test]
    fn rendered_classify_prompt_contains_candidate_dir() {
        // Bidirectional template/builder coverage is enforced at compile
        // time by `prompt_token_coverage.rs`; this test only sanity-checks
        // that the rendered output actually contains candidate context.
        let template = include_str!("../../../defaults/prompts/classify.md");
        let bundle = RationaleBundle {
            manifests: Vec::new(),
            is_git_root: false,
            doc_headings: Vec::new(),
            shebangs: Vec::new(),
        };
        let snippets = BTreeMap::new();
        let inputs = build_llm_inputs(
            Path::new("/ws"),
            Path::new("/ws/some-dir"),
            &bundle,
            &snippets,
        );
        let object = inputs.as_object().expect("inputs must be a JSON object");

        let mut tokens = BTreeMap::new();
        for (key, value) in object {
            let rendered = match value {
                Value::String(s) => s.clone(),
                other => serde_json::to_string(other).unwrap_or_default(),
            };
            tokens.insert(key.clone(), rendered);
        }
        let rendered = atlas_llm::prompt::render(template, &tokens)
            .expect("classify.md must render with build_llm_inputs output");

        assert!(
            rendered.contains("some-dir"),
            "rendered classify prompt must contain candidate dir_relative; \
             got prompt without it (length={})",
            rendered.len()
        );
    }

    #[test]
    fn parse_llm_response_defaults_is_boundary_by_kind() {
        // NonComponent defaults to is_boundary=false even when the
        // field is absent; every other kind defaults to true.
        let lib_value = serde_json::json!({
            "kind": "rust-library",
            "rationale": "r",
        });
        assert!(
            parse_llm_response(lib_value, "llm-classify-fallback", "1.0.0")
                .unwrap()
                .is_boundary
        );
        let nc_value = serde_json::json!({
            "kind": "non-component",
            "rationale": "r",
        });
        assert!(
            !parse_llm_response(nc_value, "llm-classify-fallback", "1.0.0")
                .unwrap()
                .is_boundary
        );
    }
}
