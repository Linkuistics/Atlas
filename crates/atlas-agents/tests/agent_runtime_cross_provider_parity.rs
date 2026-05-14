//! PR-5 — Within-LLM-spine cross-transport parity check (Wave 5).
//!
//! Decision row 1 framing (memory
//! `feedback_no_deterministic_engine_comparison`): the parity worth
//! gating is *within* the LLM-spine runtime — same workspace, same
//! prompts, two opposing (primary, auditor) provider orderings —
//! producing structurally-equivalent canonical artifacts. This is NOT
//! a deterministic-engine-vs-runtime parity harness.
//!
//! Two ignored runs:
//!
//! - Run A: `default_transport = HttpAnthropic` + `defaults.model =
//!   anthropic/claude-opus-4-7`. Producer = Anthropic; cross-provider
//!   auditor = OpenAI (routed by Lane B via the
//!   [`atlas_agents::runtime::ForProviderFn`] closure).
//! - Run B: the reverse — producer = OpenAI; auditor = Anthropic.
//!
//! Asserts the three equivalence rules from plan §4 Task 5 step 5.2:
//! component-id set equality, subsystem-id set equality, and edge
//! multiset equality keyed on `(from, to, kind)`. Strict equality is
//! the plan-time framing; tolerated provider-side refinements (if any
//! surface) get recorded in the PR-5 closeout note as deviation, not
//! relaxed via assertion weakening.
//!
//! `#[ignore]`-gated: the test issues real HTTP calls to both
//! Anthropic and OpenAI and only runs when both env vars are present
//! (`cargo test -p atlas-agents --test
//! agent_runtime_cross_provider_parity --release -- --ignored`). CI
//! does not exercise it; the PR-5 calibration measurement step does.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;

use atlas_agents::events::EventBus;
use atlas_agents::runtime::projection_to_canonical::{
    project_l9_to_canonical, CanonicalArtifactSet,
};
use atlas_agents::runtime::ForProviderFn;
use atlas_agents::transport::TransportFlavour;
use atlas_agents::{default_tool_catalog, AgentRuntime, Semaphores, Workspace};
use atlas_engine::llm_cache::LlmResponseCache;
use atlas_llm::{AtlasConfig, BackendRouter, LlmBackend, Provider};
use tempfile::{NamedTempFile, TempDir};

#[tokio::test]
#[ignore = "real HTTP calls to Anthropic and OpenAI; requires ANTHROPIC_API_KEY and OPENAI_API_KEY"]
async fn cross_provider_canonical_artifact_parity_holds() {
    // Early-return-with-message preserves the `--ignored` opt-in
    // semantic without panicking if a developer runs it without one of
    // the two keys exported (`#[ignore]` already gates default `cargo
    // test`, but the early-return prevents partial setup work).
    if std::env::var("ANTHROPIC_API_KEY").is_err() || std::env::var("OPENAI_API_KEY").is_err() {
        eprintln!(
            "skipping cross_provider_canonical_artifact_parity_holds: \
             ANTHROPIC_API_KEY and OPENAI_API_KEY both required"
        );
        return;
    }

    let workspace_dir = build_synthetic_workspace_with_three_subsystems();

    let anthropic_primary =
        run_workspace_via_agent_runtime(workspace_dir.path(), Producer::Anthropic).await;
    let openai_primary =
        run_workspace_via_agent_runtime(workspace_dir.path(), Producer::OpenAi).await;

    // Rule 1: component-id set equality.
    let comps_a: HashSet<&str> = component_ids(&anthropic_primary);
    let comps_o: HashSet<&str> = component_ids(&openai_primary);
    assert_eq!(
        comps_a, comps_o,
        "component-id sets must match across providers"
    );

    // Rule 2: subsystem-id set equality.
    let subs_a: HashSet<&str> = subsystem_ids(&anthropic_primary);
    let subs_o: HashSet<&str> = subsystem_ids(&openai_primary);
    assert_eq!(
        subs_a, subs_o,
        "subsystem-id sets must match across providers"
    );

    // Rule 3: edge multiset equality keyed on (from, to, kind).
    let edges_a = edge_multiset(&anthropic_primary);
    let edges_o = edge_multiset(&openai_primary);
    assert_eq!(
        edges_a, edges_o,
        "edge multiset (from, to, kind) must match across providers"
    );
}

#[derive(Debug, Clone, Copy)]
enum Producer {
    Anthropic,
    OpenAi,
}

impl Producer {
    fn default_transport(self) -> TransportFlavour {
        match self {
            Self::Anthropic => TransportFlavour::HttpAnthropic,
            Self::OpenAi => TransportFlavour::HttpOpenai,
        }
    }

    fn defaults_model(self) -> &'static str {
        // Decision row 7 — Opus 4.7 + GPT-5-Codex pairing locked at
        // PR-1; the calibration baseline uses this pair.
        match self {
            Self::Anthropic => "anthropic/claude-opus-4-7",
            Self::OpenAi => "openai/gpt-5-codex",
        }
    }
}

fn component_ids(set: &CanonicalArtifactSet) -> HashSet<&str> {
    set.components
        .components
        .iter()
        .map(|c| c.id.as_str())
        .collect()
}

fn subsystem_ids(set: &CanonicalArtifactSet) -> HashSet<&str> {
    set.subsystems
        .subsystems
        .iter()
        .map(|s| s.id.as_str())
        .collect()
}

fn edge_multiset(set: &CanonicalArtifactSet) -> HashMap<(String, String, String), u32> {
    let mut map: HashMap<(String, String, String), u32> = HashMap::new();
    for edge in &set.related.edges {
        *map.entry((edge.from.clone(), edge.to.clone(), edge.kind.clone()))
            .or_insert(0) += 1;
    }
    map
}

/// Three subsystems, six single-language Rust components — small
/// enough to keep classify-stage convergence deterministic across two
/// real-LLM runs, large enough to exercise dispatch + classify +
/// reduce + project on a non-trivial input. No
/// `subsystems.overrides.yaml` / `components.overrides.yaml`, so the
/// dispatch agent fires (which is what the parity check measures).
fn build_synthetic_workspace_with_three_subsystems() -> TempDir {
    let dir = TempDir::new().expect("tempdir");
    let root = dir.path();

    std::fs::write(
        root.join("Cargo.toml"),
        "[workspace]\nresolver = \"2\"\nmembers = [\
         \"crates/auth\",\"crates/auth-tokens\",\
         \"crates/store-db\",\"crates/store-cache\",\
         \"crates/api-gateway\",\"crates/api-rate-limit\"\
         ]\n",
    )
    .expect("workspace Cargo.toml");

    // Auth subsystem
    write_rust_crate(root, "crates/auth", "pub fn login() {}\n");
    write_rust_crate(root, "crates/auth-tokens", "pub fn issue_token() {}\n");
    // Store subsystem
    write_rust_crate(root, "crates/store-db", "pub fn connect() {}\n");
    write_rust_crate(root, "crates/store-cache", "pub fn get() {}\n");
    // Gateway subsystem
    write_rust_crate(root, "crates/api-gateway", "pub fn route() {}\n");
    write_rust_crate(root, "crates/api-rate-limit", "pub fn allow() {}\n");

    dir
}

fn write_rust_crate(root: &Path, rel: &str, lib_body: &str) {
    let dir = root.join(rel);
    std::fs::create_dir_all(dir.join("src")).expect("crate src dir");
    let name = rel.rsplit('/').next().expect("crate basename");
    std::fs::write(
        dir.join("Cargo.toml"),
        format!(
            "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[lib]\npath = \"src/lib.rs\"\n"
        ),
    )
    .expect("crate Cargo.toml");
    std::fs::write(dir.join("src").join("lib.rs"), lib_body).expect("crate lib.rs");
}

async fn run_workspace_via_agent_runtime(
    workspace_root: &Path,
    producer: Producer,
) -> CanonicalArtifactSet {
    let config_file = write_config_for_producer(producer);
    let atlas_config = AtlasConfig::load(config_file.path()).expect("AtlasConfig::load");
    let prompts_dir = TempDir::new().expect("prompts tempdir");
    let router = BackendRouter::new_for_agent_runtime(
        &atlas_config,
        prompts_dir.path(),
        workspace_root,
        [0u8; 32],
        [0u8; 32],
        None,
    )
    .expect("BackendRouter::new_for_agent_runtime");
    let router = Arc::new(router);

    let router_for_closure = Arc::clone(&router);
    let for_provider: Arc<ForProviderFn> =
        Arc::new(move |p: Provider| router_for_closure.backend_for_provider(p).cloned());

    let audit_dir = TempDir::new().expect("audit tempdir");
    let runtime = AgentRuntime {
        backend_router: Arc::clone(&router) as Arc<dyn LlmBackend>,
        tools: Arc::new(default_tool_catalog()),
        cache: Arc::new(LlmResponseCache::new()),
        event_bus: Arc::new(EventBus::with_default_capacity()),
        semaphores: Semaphores::defaults(),
        default_transport: producer.default_transport(),
        default_max_steps: 8,
        max_iterations: 5,
        for_provider: Some(for_provider),
        // HTTP-only run; subprocess transports are not exercised here.
        mcp_server: None,
        audit_dir: audit_dir.path().to_path_buf(),
    };

    let workspace = Workspace::new(workspace_root);
    let projection = runtime
        .run_workspace(&workspace)
        .await
        .expect("run_workspace");

    let canonical_dir = TempDir::new().expect("canonical tempdir");
    project_l9_to_canonical(&projection, canonical_dir.path()).expect("project_l9_to_canonical")
}

fn write_config_for_producer(producer: Producer) -> NamedTempFile {
    let model = producer.defaults_model();
    let yaml = format!(
        "providers:\n  anthropic:\n    api_key: \"${{ANTHROPIC_API_KEY}}\"\n  \
           openai:\n    api_key: \"${{OPENAI_API_KEY}}\"\n\
         defaults:\n  model: \"{model}\"\n  params:\n    max_tokens: 4096\n"
    );
    let file = NamedTempFile::new().expect("config tempfile");
    std::fs::write(file.path(), yaml.as_bytes()).expect("write config");
    file
}
