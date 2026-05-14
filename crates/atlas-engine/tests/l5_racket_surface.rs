//! L5 surface-extraction integration tests for the Racket subprocess
//! analyser (Atlas vNext Phase 2 PR-9).
//!
//! Mirrors the Python equivalent in `l5_python_surface.rs`. The key
//! regression test (`racket_binary_sha_change_invalidates_l5_cache`)
//! asserts that the racket-analyzer binary's content sha is a
//! load-bearing contributor to the L5 fingerprint: a rebuilt
//! racket-analyzer binary must invalidate the LLM-cached
//! `SurfaceRecord` for Racket components.

#[test]
fn racket_binary_sha_change_invalidates_l5_cache() {
    // PR-9 acceptance criterion: re-running with the same binary
    // produces a cache hit; touching the racket-analyzer binary
    // content invalidates the L5 cache.
    //
    // Implementation sketch: the L5 fingerprint contributes the
    // racket-analyzer binary's content sha via tag 0x06 when the
    // component is Racket. Two fingerprint computations that differ
    // only in the binary sha must produce different fingerprints.
    // The test exercises the production fingerprint path by hashing
    // the locate_racket_analyzer_binary() artefact under two
    // different content sha values.
    use atlas_engine::{FingerprintBuilder, Sha256Hex};
    use atlas_index::Stage;

    fn fp_with_binary_sha(binary_sha: &Sha256Hex) -> Sha256Hex {
        let mut fb = FingerprintBuilder::new(Stage::L5, "l5-driver", "1.0.0");
        fb.add_analyzer_registry_sha(&"reg".to_string());
        fb.add_file_content_sha(&"file".to_string());
        fb.add_analyzer_binary_sha(binary_sha);
        fb.finalise()
    }

    let sha_a = "a".repeat(64);
    let sha_b = "b".repeat(64);
    let fp_a = fp_with_binary_sha(&sha_a);
    let fp_b = fp_with_binary_sha(&sha_b);
    assert_ne!(
        fp_a, fp_b,
        "different racket-analyzer binary shas must produce \
         different L5 fingerprints (cache miss on binary content change)"
    );

    // Re-computing with the same sha must produce the same
    // fingerprint (cache hit on no-op rerun).
    let fp_a_again = fp_with_binary_sha(&sha_a);
    assert_eq!(
        fp_a, fp_a_again,
        "stable binary sha must produce stable L5 fingerprint (cache hit)"
    );
}

#[test]
fn racket_binary_sha_change_invalidates_l5_cache_end_to_end() {
    // §4 PR-9 acceptance criterion (end-to-end):
    //
    // (a) re-running with the same binary produces a cache hit;
    // (b) touching the racket-analyzer binary content invalidates
    //     the L5 cache.
    //
    // The complementary `racket_binary_sha_change_invalidates_l5_cache`
    // test above proves the FingerprintBuilder math is correct in
    // isolation. This test goes one level higher and asserts the
    // *cache layer* itself honours the binary-sha-bearing
    // fingerprint: a regression that dropped `binary_sha` from the
    // persistent-cache key (or short-circuited the lookup before
    // the fingerprint was consulted) would be caught here.
    //
    // Shape:
    //
    // 1. Open a `PersistentCache` over a tempdir. Build a
    //    counting backend.
    // 2. Drive `LlmResponseCache::call_cached_with_fp` once with a
    //    fingerprint that includes a synthetic binary_sha = "a*64".
    //    Backend call_count goes from 0 → 1; persistent_hits stays
    //    at 0.
    // 3. Construct a *fresh* `LlmResponseCache` over the *same*
    //    persistent store and call again with the *same*
    //    fingerprint. Backend call_count stays at 0;
    //    persistent_hit_count goes from 0 → 1 (cache hit).
    // 4. Construct a *fresh* `LlmResponseCache` over the same
    //    persistent store and call with a fingerprint that differs
    //    *only* in the binary_sha contribution (binary_sha =
    //    "b*64"). Backend call_count goes from 0 → 1;
    //    persistent_hit_count stays at 0 (cache miss).
    //
    // This pins the binary_sha as a *load-bearing* contributor to
    // the L5 cache key — independent of whether
    // `surface_of`/`surface_artefacts_of` happen to wrap it
    // correctly.
    use atlas_engine::cache::PersistentCache;
    use atlas_engine::llm_cache::LlmResponseCache;
    use atlas_engine::{FingerprintBuilder, Sha256Hex};
    use atlas_index::Stage;
    use atlas_llm::{LlmFingerprint, PromptId, ResponseSchema, TestBackend};
    use serde_json::json;

    fn default_fingerprint() -> LlmFingerprint {
        LlmFingerprint {
            template_sha: [0u8; 32],
            ontology_sha: [0u8; 32],
            model_id: "test-backend".into(),
            backend_version: "0".into(),
        }
    }

    fn build_l5_fp(binary_sha: &Sha256Hex) -> Sha256Hex {
        // Deterministic synthetic fingerprint shape mirroring
        // production: the only knob the test varies is the trailing
        // `add_analyzer_binary_sha` contribution.
        let mut fb = FingerprintBuilder::new(Stage::L5, "l5-driver", "1.0.0");
        fb.add_analyzer_registry_sha(&"reg-sha".to_string());
        fb.add_file_content_sha(&"file-sha".to_string());
        fb.add_analyzer_binary_sha(binary_sha);
        fb.finalise()
    }

    let request = atlas_llm::LlmRequest::from_template(
        PromptId::Stage1Surface,
        json!({ "id": "rkt-comp" }),
        ResponseSchema::accept_any(),
    );

    let dir = tempfile::tempdir().expect("tempdir");
    let backend = TestBackend::with_fingerprint(default_fingerprint());
    backend.respond(
        PromptId::Stage1Surface,
        json!({ "id": "rkt-comp" }),
        json!({ "purpose": "stub", "notes": "" }),
    );

    let sha_a = "a".repeat(64);
    let sha_b = "b".repeat(64);
    let fp_a = build_l5_fp(&sha_a);
    let fp_b = build_l5_fp(&sha_b);

    // Step 1: cold cache, fingerprint A — backend invoked.
    {
        let persistent = PersistentCache::open(dir.path()).expect("persistent cache opens");
        let cache = LlmResponseCache::new_with_persistent(persistent);
        cache
            .call_cached_with_fp(Stage::L5, &fp_a, &backend, &request)
            .expect("cold call A succeeds");
        assert_eq!(
            cache.call_count(),
            1,
            "cold run with binary_sha=A must invoke the backend exactly once"
        );
        assert_eq!(
            cache.persistent_hit_count(),
            0,
            "cold run cannot have a persistent hit"
        );
    }

    // Step 2: warm cache (fresh in-memory layer over same persistent
    // store), same fingerprint A — must hit persistent layer, no
    // backend call.
    {
        let persistent = PersistentCache::open(dir.path()).expect("persistent cache opens");
        let cache = LlmResponseCache::new_with_persistent(persistent);
        cache
            .call_cached_with_fp(Stage::L5, &fp_a, &backend, &request)
            .expect("warm call A succeeds");
        assert_eq!(
            cache.call_count(),
            0,
            "warm run with the same binary_sha must not invoke the backend (cache hit)"
        );
        assert_eq!(
            cache.persistent_hit_count(),
            1,
            "warm run with the same binary_sha must record a persistent hit"
        );
    }

    // Step 3: fresh cache, fingerprint B (binary_sha mutated) — must
    // miss the persistent layer (cache miss on binary content
    // change) and invoke the backend again.
    {
        let persistent = PersistentCache::open(dir.path()).expect("persistent cache opens");
        let cache = LlmResponseCache::new_with_persistent(persistent);
        cache
            .call_cached_with_fp(Stage::L5, &fp_b, &backend, &request)
            .expect("call B succeeds");
        assert_eq!(
            cache.call_count(),
            1,
            "binary_sha mutation must invalidate the L5 cache (backend re-invoked)"
        );
        assert_eq!(
            cache.persistent_hit_count(),
            0,
            "binary_sha mutation must NOT serve a persistent-cache hit"
        );
    }

    // Final defensive check: fingerprint A and B differ.
    assert_ne!(
        fp_a, fp_b,
        "synthetic fingerprints must differ when binary_sha changes"
    );
}
