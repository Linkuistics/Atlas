//! Atomic-write kill-during-write fixture suite (Phase 3 plan §4
//! PR-12, design §6.3).
//!
//! Stress-tests the atomic-write semantics for the two stateful files
//! Phase 3 introduces:
//!
//! - the drift baseline snapshot
//!   (`.atlas/cache/contract-shas-snapshot.yaml`, PR-8) and
//! - the per-component modularity history file
//!   (`<component>/.atlas/cache/modularity.yaml`, PR-10).
//!
//! For each, two named fixtures cover the symmetric branches of the
//! atomic-write contract — kill-before-rename leaves the destination
//! fully-old; kill-after-rename leaves it fully-new — plus a
//! 10-iteration random-kill stress test that walks alternating
//! before/after kill points via a seeded RNG so any regression
//! reproduces deterministically.
//!
//! The fixtures exercise the *real* serialise + atomic_write pipeline
//! that the CLI handlers (`crates/atlas-cli/src/reports.rs`) execute
//! on disk. To keep the tests pure, they do NOT call into the CLI
//! handler (which renders summaries and walks an
//! [`atlas_engine::AtlasDatabase`] populated from a tempdir tree);
//! they construct fixture state values directly, push them through
//! `serde_yaml::to_string`, and write them via
//! [`atlas_engine::atomic_write`] — exactly the call sequence the CLI
//! handler performs.
//!
//! ## Hook usage
//!
//! Both panic-injection hooks live in
//! [`atlas_engine::atomic_write::test_hooks_pub`] (gated behind the
//! `atomic_write_panic_after_temp` cargo feature). PR-8 introduced
//! the before-rename hook; PR-12 adds the symmetric after-rename
//! hook. Both are one-shot, thread-local, and auto-disarm when they
//! fire — see the cross-crate canonical example in
//! `crates/atlas-cli/tests/atlas_drift.rs`'s
//! `atlas_drift_kill_during_snapshot_write_leaves_file_intact`.

use std::path::Path;

use atlas_engine::atomic_write;
use atlas_engine::atomic_write::test_hooks_pub::{
    arm_panic_after_rename, arm_panic_before_rename, disarm_panic_after_rename,
    disarm_panic_before_rename,
};
use chrono::{DateTime, TimeZone, Utc};
use component_ontology::ComponentId;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use tempfile::TempDir;

use atlas_reports::drift::drift_pure;
use atlas_reports::modularity::{
    ComponentModularity, ModularityHistoryEntry, ModularityHistoryMetrics, ModularityMetrics,
};
use atlas_reports::snapshot::{ContractShaEntry, ContractShaSnapshot};

// ---------------------------------------------------------------------
// Helpers — fixture builders.
// ---------------------------------------------------------------------

fn ts(year: i32, month: u32, day: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(year, month, day, 0, 0, 0).unwrap()
}

fn cid(s: &str) -> ComponentId {
    ComponentId::parse(s).unwrap()
}

/// Build a baseline drift snapshot with two contracts at known shas.
/// The fixture's "S1" state for the drift kill-during-write tests.
fn baseline_drift_snapshot() -> ContractShaSnapshot {
    ContractShaSnapshot {
        schema_version: 1,
        captured_at: ts(2026, 5, 1),
        contract_shas: vec![
            ContractShaEntry {
                id: "atlas-contracts/index-schema/v1".to_string(),
                content_sha: "sha256:baseline-A".to_string(),
            },
            ContractShaEntry {
                id: "atlas-contracts/eval-schema/v1".to_string(),
                content_sha: "sha256:baseline-B".to_string(),
            },
        ],
    }
}

/// Compute the next-state drift snapshot via the canonical
/// `drift_pure` entry point. Returns `(report, new_snapshot)` so the
/// test can write whichever files the CLI handler would write.
///
/// The fixture mutates contract `index-schema/v1` from
/// `sha256:baseline-A` to `sha256:next-A` so the pure diff produces
/// a non-empty changes vector — exactly the path the CLI handler
/// takes when a real run detects drift.
fn compute_next_drift_snapshot(
    prev: &ContractShaSnapshot,
) -> (atlas_reports::drift::DriftReport, ContractShaSnapshot) {
    use atlas_reports::drift::CurrentContract;

    let current_contracts = vec![
        CurrentContract {
            id: "atlas-contracts/index-schema/v1".to_string(),
            content_sha: "sha256:next-A".to_string(),
        },
        CurrentContract {
            id: "atlas-contracts/eval-schema/v1".to_string(),
            content_sha: "sha256:baseline-B".to_string(),
        },
    ];
    let now = ts(2026, 5, 8);
    drift_pure(&current_contracts, &[], Some(prev), now, now)
}

/// Build a per-component modularity payload with a 5-entry history
/// (newest first), simulating a component that has run modularity
/// five times with five distinct surface fingerprints.
fn baseline_component_modularity() -> ComponentModularity {
    let history = (1..=5)
        .rev()
        .map(|i| ModularityHistoryEntry {
            generated_at: ts(2026, 4, i),
            surface_fingerprint: format!("sha256:fp-{i}"),
            metrics: ModularityHistoryMetrics {
                afferent_coupling: 1,
                efferent_coupling: 0,
                instability: 0.0,
                cohesion: 1.0,
                surface_complexity: 1,
            },
        })
        .collect::<Vec<_>>();
    ComponentModularity {
        schema_version: 1,
        component_id: cid("ravel-lite/api"),
        generated_at: ts(2026, 4, 5),
        metrics: ModularityMetrics {
            afferent_coupling: 1,
            efferent_coupling: 0,
            instability: 0.0,
            cohesion: 1.0,
            surface_stability: 1.0,
            surface_complexity: 1,
        },
        history,
    }
}

/// Hard cap on per-component history entries (Phase 3 design §4.3).
/// Mirrors the private constant in `atlas_reports::modularity`; we
/// re-declare it here because the integration test cannot reach the
/// crate-private `rotate_history` helper. The cap is fixed by the
/// design spec and is not configurable, so re-declaring is safe — a
/// drift in the library value would also be a spec violation.
const HISTORY_CAP: usize = 5;

/// Apply Phase 3 design §4.3 history rotation locally — prepend
/// `new_entry` to `prior` (newest-first), drop the oldest if the
/// total exceeds five, and short-circuit when the head's
/// fingerprint matches `new_entry`'s. Mirrors
/// `atlas_reports::modularity::rotate_history` (which is
/// `pub(crate)` and thus unreachable from an integration test).
fn rotate_history_local(
    prior: Vec<ModularityHistoryEntry>,
    new_entry: ModularityHistoryEntry,
) -> Vec<ModularityHistoryEntry> {
    if let Some(head) = prior.first() {
        if head.surface_fingerprint == new_entry.surface_fingerprint {
            return prior;
        }
    }
    let mut out = Vec::with_capacity((prior.len() + 1).min(HISTORY_CAP));
    out.push(new_entry);
    for entry in prior.into_iter().take(HISTORY_CAP - 1) {
        out.push(entry);
    }
    out
}

/// Apply one rotation step on a baseline modularity payload: a fresh
/// surface fingerprint produces a new entry, the oldest is dropped.
/// Mirrors the rotation step the CLI handler performs by way of
/// `atlas_reports::modularity()` + `rotate_history`.
fn rotated_component_modularity(prior: &ComponentModularity) -> ComponentModularity {
    let new_entry = ModularityHistoryEntry {
        generated_at: ts(2026, 5, 8),
        surface_fingerprint: "sha256:fp-6".to_string(),
        metrics: ModularityHistoryMetrics {
            afferent_coupling: 2,
            efferent_coupling: 0,
            instability: 0.0,
            cohesion: 1.0,
            surface_complexity: 1,
        },
    };
    let history = rotate_history_local(prior.history.clone(), new_entry);
    ComponentModularity {
        schema_version: 1,
        component_id: prior.component_id.clone(),
        generated_at: ts(2026, 5, 8),
        metrics: ModularityMetrics {
            afferent_coupling: 2,
            efferent_coupling: 0,
            instability: 0.0,
            cohesion: 1.0,
            surface_stability: 0.0, // a fresh fingerprint breaks stability
            surface_complexity: 1,
        },
        history,
    }
}

/// Wrapper around the CLI handler's snapshot write — serialise via
/// `serde_yaml`, write via [`atomic_write`]. Mirrors the call
/// sequence in `crates/atlas-cli/src/reports.rs::write_drift_outputs`.
fn write_snapshot_via_atomic(path: &Path, snap: &ContractShaSnapshot) {
    let yaml = serde_yaml::to_string(snap).expect("snapshot serialisation must not fail");
    atomic_write(path, yaml.as_bytes()).expect("atomic_write must not fail without an armed hook");
}

/// Same shape, for a per-component modularity payload.
fn write_modularity_via_atomic(path: &Path, payload: &ComponentModularity) {
    let yaml = serde_yaml::to_string(payload).expect("modularity serialisation must not fail");
    atomic_write(path, yaml.as_bytes()).expect("atomic_write must not fail without an armed hook");
}

// ---------------------------------------------------------------------
// AC #1 + #2: drift snapshot kill-during-write / kill-after-rename.
// ---------------------------------------------------------------------

/// AC: pre-populate snapshot with state S1; arm the panic-before-
/// rename hook; invoke the wrapper that calls `atomic_write` with
/// state S2 (catch the panic via `catch_unwind`); re-read the
/// snapshot; assert content equals S1 byte-for-byte (the rename
/// never landed).
#[test]
fn drift_snapshot_kill_during_write_leaves_file_intact() {
    let tmp = TempDir::new().unwrap();
    let snap_path = tmp.path().join(".atlas/cache/contract-shas-snapshot.yaml");

    // Establish S1 on disk by writing the baseline snapshot via the
    // exact same helper the kill path will exercise.
    let s1 = baseline_drift_snapshot();
    write_snapshot_via_atomic(&snap_path, &s1);
    let baseline_bytes = std::fs::read(&snap_path).expect("S1 must be written");

    // Compute S2 via the canonical pure-function diff path.
    let (_report, s2) = compute_next_drift_snapshot(&s1);
    assert_ne!(
        serde_yaml::to_string(&s1).unwrap(),
        serde_yaml::to_string(&s2).unwrap(),
        "fixture sanity: S2 must differ from S1 so a successful write would change the file"
    );

    // Arm the one-shot before-rename hook and attempt to write S2.
    let snap_path_for_closure = snap_path.clone();
    let s2_for_closure = s2.clone();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        arm_panic_before_rename();
        write_snapshot_via_atomic(&snap_path_for_closure, &s2_for_closure);
    }));
    // Defensive disarm in case `catch_unwind` unwound before the
    // hook's one-shot disarm fired.
    disarm_panic_before_rename();
    assert!(
        result.is_err(),
        "the armed before-rename hook must fire and propagate the panic"
    );

    // The destination must still equal S1 byte-for-byte: the rename
    // never landed, so the file is fully-old.
    let post_bytes = std::fs::read(&snap_path).expect("snapshot must still be readable");
    assert_eq!(
        post_bytes, baseline_bytes,
        "kill-before-rename must leave the snapshot fully-old (half-written would corrupt baseline)"
    );

    // And it must still parse as a valid `ContractShaSnapshot`.
    let parsed: ContractShaSnapshot = serde_yaml::from_slice(&post_bytes)
        .expect("post-kill snapshot must parse as ContractShaSnapshot");
    assert_eq!(parsed, s1);
}

/// AC: same fixture; arm the panic-AFTER-rename hook; the rename
/// completes before the panic fires; re-read; assert content equals
/// the new S2.
#[test]
fn drift_snapshot_kill_after_rename_succeeds() {
    let tmp = TempDir::new().unwrap();
    let snap_path = tmp.path().join(".atlas/cache/contract-shas-snapshot.yaml");

    let s1 = baseline_drift_snapshot();
    write_snapshot_via_atomic(&snap_path, &s1);

    let (_report, s2) = compute_next_drift_snapshot(&s1);
    let s2_bytes = serde_yaml::to_string(&s2).unwrap().into_bytes();

    let snap_path_for_closure = snap_path.clone();
    let s2_for_closure = s2.clone();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        arm_panic_after_rename();
        write_snapshot_via_atomic(&snap_path_for_closure, &s2_for_closure);
    }));
    disarm_panic_after_rename();
    assert!(
        result.is_err(),
        "the armed after-rename hook must fire and propagate the panic"
    );

    let post_bytes = std::fs::read(&snap_path).expect("snapshot must still be readable");
    assert_eq!(
        post_bytes, s2_bytes,
        "kill-after-rename must leave the snapshot fully-new (the rename landed before the panic)"
    );
    let parsed: ContractShaSnapshot = serde_yaml::from_slice(&post_bytes)
        .expect("post-kill snapshot must parse as ContractShaSnapshot");
    assert_eq!(parsed, s2);
}

// ---------------------------------------------------------------------
// AC #3 + #4: modularity history kill-during-write / kill-after-rename.
// ---------------------------------------------------------------------

/// AC: per-component file with 5 prior entries; arm before-rename
/// panic; attempt to write the rotated 6th entry; re-read; assert
/// all 5 prior entries are intact.
#[test]
fn modularity_history_kill_during_write_preserves_prior_5_entries() {
    let tmp = TempDir::new().unwrap();
    // The CLI handler writes per-component modularity to
    // `<component>/.atlas/cache/modularity.yaml`; mirror that path
    // shape under the tempdir.
    let mod_path = tmp
        .path()
        .join("ravel-lite/api/.atlas/cache/modularity.yaml");

    // Populate the 5-entry baseline.
    let prior = baseline_component_modularity();
    assert_eq!(prior.history.len(), 5, "fixture sanity: 5 prior entries");
    write_modularity_via_atomic(&mod_path, &prior);
    let baseline_bytes = std::fs::read(&mod_path).expect("prior modularity must be written");

    // Compute the rotation: rotated history has 5 entries (newest
    // first; oldest dropped) — the kill must leave the prior payload
    // (also 5 entries) untouched.
    let rotated = rotated_component_modularity(&prior);
    assert_eq!(rotated.history.len(), 5, "rotation cap is 5");
    assert_ne!(
        rotated.history[0].surface_fingerprint, prior.history[0].surface_fingerprint,
        "fixture sanity: rotation prepends a new entry"
    );

    let mod_path_for_closure = mod_path.clone();
    let rotated_for_closure = rotated.clone();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        arm_panic_before_rename();
        write_modularity_via_atomic(&mod_path_for_closure, &rotated_for_closure);
    }));
    disarm_panic_before_rename();
    assert!(
        result.is_err(),
        "the armed before-rename hook must fire and propagate the panic"
    );

    // The destination must equal the prior bytes — the kill prevented
    // the rename, so the rotated payload never landed and all 5 prior
    // entries are intact.
    let post_bytes = std::fs::read(&mod_path).expect("modularity must still be readable");
    assert_eq!(
        post_bytes, baseline_bytes,
        "kill-before-rename must leave the modularity payload fully-old (all 5 prior entries intact)"
    );
    let parsed: ComponentModularity = serde_yaml::from_slice(&post_bytes)
        .expect("post-kill modularity must parse as ComponentModularity");
    assert_eq!(parsed.history.len(), 5);
    assert_eq!(parsed, prior);
    // Spot-check every fingerprint to make the AC's "all 5 prior
    // entries intact" assertion explicit.
    for (got, want) in parsed.history.iter().zip(prior.history.iter()) {
        assert_eq!(got.surface_fingerprint, want.surface_fingerprint);
    }
}

/// AC: same fixture, but arm the after-rename panic; re-read; assert
/// the rotation persisted (oldest dropped, new entry first).
#[test]
fn modularity_history_kill_after_rename_persists_rotation() {
    let tmp = TempDir::new().unwrap();
    let mod_path = tmp
        .path()
        .join("ravel-lite/api/.atlas/cache/modularity.yaml");

    let prior = baseline_component_modularity();
    write_modularity_via_atomic(&mod_path, &prior);

    let rotated = rotated_component_modularity(&prior);
    let rotated_bytes = serde_yaml::to_string(&rotated).unwrap().into_bytes();

    let mod_path_for_closure = mod_path.clone();
    let rotated_for_closure = rotated.clone();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        arm_panic_after_rename();
        write_modularity_via_atomic(&mod_path_for_closure, &rotated_for_closure);
    }));
    disarm_panic_after_rename();
    assert!(
        result.is_err(),
        "the armed after-rename hook must fire and propagate the panic"
    );

    let post_bytes = std::fs::read(&mod_path).expect("modularity must still be readable");
    assert_eq!(
        post_bytes, rotated_bytes,
        "kill-after-rename must persist the rotated payload (oldest dropped, new entry first)"
    );
    let parsed: ComponentModularity = serde_yaml::from_slice(&post_bytes)
        .expect("post-kill modularity must parse as ComponentModularity");
    assert_eq!(parsed, rotated);
    assert_eq!(parsed.history.len(), 5);
    assert_eq!(parsed.history[0].surface_fingerprint, "sha256:fp-6");
    // The oldest prior entry (`fp-1`) must be gone.
    assert!(
        parsed
            .history
            .iter()
            .all(|e| e.surface_fingerprint != "sha256:fp-1"),
        "rotation persisted: oldest entry (fp-1) must be dropped"
    );
}

// ---------------------------------------------------------------------
// AC #5: 10-iteration random-kill stress test (deterministic via
// seeded RNG). Alternates before/after kill points; on each
// iteration re-reads the file and asserts the bytes are *exactly*
// either the prior or the new state — never partial.
// ---------------------------------------------------------------------

/// Hard-coded RNG seed so any failure of this stress test is
/// reproducible: a regression repeats the exact same kill-point
/// sequence on the next run.
const STRESS_SEED: u64 = 12345;

#[test]
fn drift_and_modularity_atomic_writes_are_kill_safe_under_random_stress() {
    let mut rng = StdRng::seed_from_u64(STRESS_SEED);

    let tmp = TempDir::new().unwrap();
    let snap_path = tmp.path().join(".atlas/cache/contract-shas-snapshot.yaml");
    let mod_path = tmp
        .path()
        .join("ravel-lite/api/.atlas/cache/modularity.yaml");

    // Seed both files with their baseline (S1) state. We track the
    // last committed bytes for each file so the loop can assert
    // "exactly prior OR exactly new" after each kill.
    let mut last_drift_state = baseline_drift_snapshot();
    let mut last_drift_bytes = serde_yaml::to_string(&last_drift_state)
        .unwrap()
        .into_bytes();
    write_snapshot_via_atomic(&snap_path, &last_drift_state);
    assert_eq!(
        std::fs::read(&snap_path).unwrap(),
        last_drift_bytes,
        "stress fixture sanity: drift baseline persisted"
    );

    let mut last_mod_state = baseline_component_modularity();
    let mut last_mod_bytes = serde_yaml::to_string(&last_mod_state).unwrap().into_bytes();
    write_modularity_via_atomic(&mod_path, &last_mod_state);
    assert_eq!(
        std::fs::read(&mod_path).unwrap(),
        last_mod_bytes,
        "stress fixture sanity: modularity baseline persisted"
    );

    for iteration in 0..10 {
        // Decide kill point for this iteration. The seeded RNG
        // alternates between before-rename and after-rename so the
        // schedule is deterministic — a regression repeats the same
        // sequence on the next run.
        let kill_after = rng.gen::<bool>();
        // Decide which file is targeted this iteration (drift snapshot
        // vs modularity payload). Both stateful files share the same
        // atomic-write contract; alternating exercises both serialise
        // pipelines under both kill points.
        let target_drift = rng.gen::<bool>();

        if target_drift {
            // Build a fresh next-state for the drift snapshot. We
            // mutate one contract's sha each iteration so every
            // attempted write is genuinely different from the prior.
            let mut next = last_drift_state.clone();
            next.contract_shas[0].content_sha = format!("sha256:stress-iter-{iteration}");
            next.captured_at = ts(2026, 5, 8 + (iteration as u32 % 20));

            let next_bytes = serde_yaml::to_string(&next).unwrap().into_bytes();
            run_kill_iteration(&snap_path, &next_bytes, kill_after, &|path, bytes| {
                atomic_write(path, bytes).unwrap();
            });

            // The post-kill bytes must be exactly the prior state OR
            // exactly the new state — never a half-written prefix.
            let got = std::fs::read(&snap_path).unwrap();
            assert!(
                got == last_drift_bytes || got == next_bytes,
                "iteration {iteration}: drift snapshot must be exactly prior OR new, never partial \
                 (kill_after={kill_after}, len_got={}, len_prior={}, len_new={})",
                got.len(),
                last_drift_bytes.len(),
                next_bytes.len(),
            );
            // After-rename kills: rename landed before the panic, so
            // the new bytes are now the committed state.
            // Before-rename kills: rename never landed, so the prior
            // bytes are still the committed state.
            if kill_after {
                last_drift_state = next;
                last_drift_bytes = next_bytes;
            }
        } else {
            // Build a fresh next-state modularity payload by rotating
            // a unique fingerprint each iteration.
            let new_entry = ModularityHistoryEntry {
                generated_at: ts(2026, 5, 8),
                surface_fingerprint: format!("sha256:stress-fp-{iteration}"),
                metrics: ModularityHistoryMetrics {
                    afferent_coupling: 1,
                    efferent_coupling: 0,
                    instability: 0.0,
                    cohesion: 1.0,
                    surface_complexity: 1,
                },
            };
            let next_history = rotate_history_local(last_mod_state.history.clone(), new_entry);
            let next = ComponentModularity {
                schema_version: 1,
                component_id: last_mod_state.component_id.clone(),
                generated_at: ts(2026, 5, 8),
                metrics: last_mod_state.metrics.clone(),
                history: next_history,
            };
            let next_bytes = serde_yaml::to_string(&next).unwrap().into_bytes();
            run_kill_iteration(&mod_path, &next_bytes, kill_after, &|path, bytes| {
                atomic_write(path, bytes).unwrap();
            });

            let got = std::fs::read(&mod_path).unwrap();
            assert!(
                got == last_mod_bytes || got == next_bytes,
                "iteration {iteration}: modularity payload must be exactly prior OR new, never partial \
                 (kill_after={kill_after}, len_got={}, len_prior={}, len_new={})",
                got.len(),
                last_mod_bytes.len(),
                next_bytes.len(),
            );
            // Whichever branch fired, the parser must still succeed —
            // a half-written YAML would not. This is the strongest
            // form of the "never partial" invariant.
            let _: ComponentModularity = serde_yaml::from_slice(&got).unwrap_or_else(|err| {
                panic!("iteration {iteration}: modularity payload must be parseable; got {err}")
            });
            if kill_after {
                last_mod_state = next;
                last_mod_bytes = next_bytes;
            }
        }
    }
}

/// Run one kill iteration: arm either the before-rename or after-
/// rename hook, invoke `caller` (which performs an atomic_write to
/// `path` with `bytes`) inside `catch_unwind`, and defensively
/// disarm afterwards. The hook fires once per iteration and
/// auto-disarms on fire — the explicit disarm is belt-and-braces
/// in case `catch_unwind` unwinds before the auto-disarm runs.
fn run_kill_iteration<F>(path: &Path, bytes: &[u8], kill_after: bool, caller: &F)
where
    F: Fn(&Path, &[u8]),
{
    let path_owned = path.to_path_buf();
    let bytes_owned = bytes.to_vec();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if kill_after {
            arm_panic_after_rename();
        } else {
            arm_panic_before_rename();
        }
        caller(&path_owned, &bytes_owned);
    }));
    // Belt-and-braces disarms.
    disarm_panic_before_rename();
    disarm_panic_after_rename();
    assert!(
        result.is_err(),
        "stress iteration: armed hook must fire (kill_after={kill_after})"
    );
}
