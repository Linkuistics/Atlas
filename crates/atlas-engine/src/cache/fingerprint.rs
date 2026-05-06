//! Stage fingerprint builder for the persistent cache.
//!
//! Per design §8.1, every cacheable computation declares its
//! fingerprint inputs explicitly. The cache key is
//!
//! ```text
//! sha256(stage_id || analyzer_id || analyzer_version || sorted(fingerprint_inputs))
//! ```
//!
//! where `fingerprint_inputs` is the unordered multiset of contributions
//! supplied by the stage's `add_*` calls. Order independence is achieved
//! by collecting each contribution as `(tag, bytes)` into a `BTreeSet`
//! and hashing the sorted entries on `finalise`. The leading tag byte
//! distinguishes contributions that share a wire form but carry
//! different semantics (e.g. a file content sha vs a prompt sha).
//!
//! ## Tag table
//!
//! | Tag | Method                            | Bytes contributed                     |
//! |-----|-----------------------------------|---------------------------------------|
//! | `0x01` | `add_file_content_sha`         | UTF-8 bytes of the sha256 hex string  |
//! | `0x02` | `add_prompt_sha`               | UTF-8 bytes of the sha256 hex string  |
//! | `0x03` | `add_llm_fingerprint`          | length-framed concatenation of `template_sha` ‖ `ontology_sha` ‖ `model_id` ‖ `backend_version` |
//! | `0x04` | `add_participant_surface_sha`  | UTF-8 bytes of the sha256 hex string  |
//!
//! Each entry in the `BTreeSet` is `(tag, bytes)`; sort order is by
//! tag first, then lex order on bytes. New tags must be appended (never
//! reused) — a tag value is part of the on-disk fingerprint and a
//! collision with prior data would silently change a stage's hash.
//!
//! Framing: when `finalise` hashes each `(tag, bytes)` entry, it writes
//! the tag byte, the byte length as a little-endian `u64`, then the
//! payload. This pattern is also used inside `add_llm_fingerprint` to
//! frame the four sub-fields so two backends with permuted but equal
//! field bytes produce different fingerprints.

use std::collections::BTreeSet;

use atlas_index::Stage;
use atlas_llm::LlmFingerprint;
use sha2::{Digest, Sha256};

use super::Sha256Hex;

const TAG_FILE_CONTENT_SHA: u8 = 0x01;
const TAG_PROMPT_SHA: u8 = 0x02;
const TAG_LLM_FINGERPRINT: u8 = 0x03;
const TAG_PARTICIPANT_SURFACE_SHA: u8 = 0x04;

/// Builder for a stage cache fingerprint.
///
/// Construct with [`FingerprintBuilder::new`] (which seeds the hasher
/// with the stage / analyzer id / analyzer version frame), then call
/// any of the `add_*` methods in any order. [`FingerprintBuilder::finalise`]
/// returns the lowercase 64-character sha256 hex string used as the
/// cache key.
///
/// Order independence: contributions are stored in a `BTreeSet`,
/// keyed on `(tag, bytes)`. Two builders that received the same set
/// of `add_*` calls in different orders produce the same fingerprint.
///
/// Framing: each contribution is hashed as `tag (1 byte) || len (8
/// bytes LE) || bytes`. The tag distinguishes contributions whose
/// payloads might otherwise collide.
#[derive(Debug, Clone)]
pub struct FingerprintBuilder {
    /// Stage / analyzer-id / analyzer-version preamble. Hashed first
    /// in `finalise` so the same set of contributions across two
    /// stages yields different fingerprints.
    preamble: Vec<u8>,
    /// Tagged contribution entries. `BTreeSet` enforces both
    /// order independence and de-duplication of identical
    /// `(tag, bytes)` calls (idempotent within a builder).
    entries: BTreeSet<(u8, Vec<u8>)>,
}

impl FingerprintBuilder {
    /// Start a new builder for the given stage and analyzer identity.
    /// `analyzer_id` and `analyzer_version` are framed into the
    /// preamble; both are part of the cache key per design §8.1.
    pub fn new(stage: Stage, analyzer_id: &str, analyzer_version: &str) -> Self {
        let mut preamble = Vec::with_capacity(64);
        // Stage tag: a single byte derived from the variant. Keeping
        // this stable matters — a future stage variant must extend the
        // match without re-numbering existing variants. Using the same
        // byte order as the `Stage` declaration gives `L1=1`, `L2=2`,
        // …, `L9=9`.
        let stage_byte: u8 = match stage {
            Stage::L1 => 1,
            Stage::L2 => 2,
            Stage::L3 => 3,
            Stage::L4 => 4,
            Stage::L5 => 5,
            Stage::L6 => 6,
            Stage::L7 => 7,
            Stage::L8 => 8,
            Stage::L9 => 9,
        };
        push_framed(&mut preamble, b"stage", &[stage_byte]);
        push_framed(&mut preamble, b"analyzer_id", analyzer_id.as_bytes());
        push_framed(
            &mut preamble,
            b"analyzer_version",
            analyzer_version.as_bytes(),
        );

        FingerprintBuilder {
            preamble,
            entries: BTreeSet::new(),
        }
    }

    /// Contribute a file content sha. Used by L1 / L2 / L5 fingerprints.
    pub fn add_file_content_sha(&mut self, sha: &Sha256Hex) {
        self.entries
            .insert((TAG_FILE_CONTENT_SHA, sha.as_bytes().to_vec()));
    }

    /// Contribute a prompt sha. Used by every LLM-bearing stage (L3 /
    /// L5 / L6 / L8) per §8.1.
    pub fn add_prompt_sha(&mut self, sha: &Sha256Hex) {
        self.entries
            .insert((TAG_PROMPT_SHA, sha.as_bytes().to_vec()));
    }

    /// Contribute the full LLM fingerprint (template sha, ontology
    /// sha, model id, backend version). Each sub-field is internally
    /// framed so a permuted but byte-equal field set produces a
    /// different fingerprint.
    pub fn add_llm_fingerprint(&mut self, fp: &LlmFingerprint) {
        let mut payload =
            Vec::with_capacity(32 + 32 + fp.model_id.len() + fp.backend_version.len());
        push_framed(&mut payload, b"template_sha", &fp.template_sha);
        push_framed(&mut payload, b"ontology_sha", &fp.ontology_sha);
        push_framed(&mut payload, b"model_id", fp.model_id.as_bytes());
        push_framed(
            &mut payload,
            b"backend_version",
            fp.backend_version.as_bytes(),
        );
        self.entries.insert((TAG_LLM_FINGERPRINT, payload));
    }

    /// Contribute a participant component's surface sha. L6-only per
    /// §8.1: when a participant's surface changes, every L6 cache
    /// entry that named it as a participant misses on next access.
    pub fn add_participant_surface_sha(&mut self, sha: &Sha256Hex) {
        self.entries
            .insert((TAG_PARTICIPANT_SURFACE_SHA, sha.as_bytes().to_vec()));
    }

    /// Finalise the builder and return the lowercase 64-character
    /// sha256 hex string of the framed contributions.
    pub fn finalise(self) -> Sha256Hex {
        let mut hasher = Sha256::new();
        hasher.update(&self.preamble);
        for (tag, bytes) in &self.entries {
            hasher.update([*tag]);
            hasher.update((bytes.len() as u64).to_le_bytes());
            hasher.update(bytes);
        }
        let digest: [u8; 32] = hasher.finalize().into();
        let mut hex = String::with_capacity(64);
        for b in digest {
            use std::fmt::Write;
            // 64-char lowercase hex; matches the engine-wide
            // sha256_hex helper in `l9_projections.rs`.
            write!(&mut hex, "{b:02x}").expect("writing to String never fails");
        }
        hex
    }
}

/// Append a framed `name || value` pair to `buf`. Used both inside the
/// preamble (to attach a human-readable label to each preamble
/// component) and inside `add_llm_fingerprint` (to keep the four LLM
/// sub-fields separable).
fn push_framed(buf: &mut Vec<u8>, name: &[u8], value: &[u8]) {
    buf.extend_from_slice(&(name.len() as u64).to_le_bytes());
    buf.extend_from_slice(name);
    buf.extend_from_slice(&(value.len() as u64).to_le_bytes());
    buf.extend_from_slice(value);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fp() -> LlmFingerprint {
        LlmFingerprint {
            template_sha: [9u8; 32],
            ontology_sha: [3u8; 32],
            model_id: "claude-sonnet-4-7".into(),
            backend_version: "v0.1".into(),
        }
    }

    #[test]
    fn finalise_is_64_char_hex() {
        let fp = FingerprintBuilder::new(Stage::L3, "cargo-toml-classifier", "1.0.0").finalise();
        assert_eq!(fp.len(), 64);
        assert!(fp
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
    }

    #[test]
    fn distinct_stages_produce_distinct_fingerprints() {
        let a = FingerprintBuilder::new(Stage::L3, "id", "v").finalise();
        let b = FingerprintBuilder::new(Stage::L5, "id", "v").finalise();
        assert_ne!(a, b);
    }

    #[test]
    fn distinct_analyzer_ids_and_versions_change_fingerprint() {
        let base = FingerprintBuilder::new(Stage::L3, "id-a", "1.0").finalise();
        let id = FingerprintBuilder::new(Stage::L3, "id-b", "1.0").finalise();
        let ver = FingerprintBuilder::new(Stage::L3, "id-a", "1.1").finalise();
        assert_ne!(base, id);
        assert_ne!(base, ver);
        assert_ne!(id, ver);
    }

    #[test]
    fn add_calls_are_order_independent() {
        let mut a = FingerprintBuilder::new(Stage::L5, "an", "v");
        a.add_file_content_sha(&"aaaa".to_string());
        a.add_file_content_sha(&"bbbb".to_string());
        a.add_prompt_sha(&"cccc".to_string());

        let mut b = FingerprintBuilder::new(Stage::L5, "an", "v");
        b.add_prompt_sha(&"cccc".to_string());
        b.add_file_content_sha(&"bbbb".to_string());
        b.add_file_content_sha(&"aaaa".to_string());

        assert_eq!(a.finalise(), b.finalise());
    }

    #[test]
    fn duplicate_add_calls_are_idempotent() {
        // Same `(tag, bytes)` is de-duplicated by the BTreeSet, so a
        // caller that inserts the same sha twice produces the same
        // fingerprint as a caller that inserts it once.
        let mut once = FingerprintBuilder::new(Stage::L1, "an", "v");
        once.add_file_content_sha(&"abcd".to_string());

        let mut twice = FingerprintBuilder::new(Stage::L1, "an", "v");
        twice.add_file_content_sha(&"abcd".to_string());
        twice.add_file_content_sha(&"abcd".to_string());

        assert_eq!(once.finalise(), twice.finalise());
    }

    #[test]
    fn tag_disambiguates_same_bytes_across_methods() {
        // An L6 builder with `add_file_content_sha("abc")` must differ
        // from one with `add_participant_surface_sha("abc")`. Without
        // the tag byte the framed payload would otherwise be equal.
        let mut a = FingerprintBuilder::new(Stage::L6, "an", "v");
        a.add_file_content_sha(&"abc".to_string());
        let mut b = FingerprintBuilder::new(Stage::L6, "an", "v");
        b.add_participant_surface_sha(&"abc".to_string());
        assert_ne!(a.finalise(), b.finalise());
    }

    #[test]
    fn add_file_content_sha_changes_fingerprint() {
        let base = FingerprintBuilder::new(Stage::L1, "an", "v").finalise();
        let mut with_one = FingerprintBuilder::new(Stage::L1, "an", "v");
        with_one.add_file_content_sha(&"deadbeef".to_string());
        assert_ne!(base, with_one.finalise());
    }

    #[test]
    fn add_prompt_sha_changes_fingerprint() {
        let base = FingerprintBuilder::new(Stage::L3, "an", "v").finalise();
        let mut with_one = FingerprintBuilder::new(Stage::L3, "an", "v");
        with_one.add_prompt_sha(&"deadbeef".to_string());
        assert_ne!(base, with_one.finalise());
    }

    #[test]
    fn add_llm_fingerprint_changes_fingerprint() {
        let base = FingerprintBuilder::new(Stage::L3, "an", "v").finalise();
        let mut with_fp = FingerprintBuilder::new(Stage::L3, "an", "v");
        with_fp.add_llm_fingerprint(&fp());
        assert_ne!(base, with_fp.finalise());
    }

    #[test]
    fn distinct_llm_fingerprints_change_fingerprint() {
        let mut a = FingerprintBuilder::new(Stage::L3, "an", "v");
        a.add_llm_fingerprint(&fp());
        let mut other = fp();
        other.model_id = "claude-haiku-4-5".into();
        let mut b = FingerprintBuilder::new(Stage::L3, "an", "v");
        b.add_llm_fingerprint(&other);
        assert_ne!(a.finalise(), b.finalise());
    }

    #[test]
    fn add_participant_surface_sha_changes_fingerprint() {
        let base = FingerprintBuilder::new(Stage::L6, "an", "v").finalise();
        let mut with_one = FingerprintBuilder::new(Stage::L6, "an", "v");
        with_one.add_participant_surface_sha(&"abc".to_string());
        assert_ne!(base, with_one.finalise());
    }

    #[test]
    fn equal_inputs_produce_equal_fingerprints() {
        let mut a = FingerprintBuilder::new(Stage::L6, "edges-llm", "1.0");
        a.add_llm_fingerprint(&fp());
        a.add_prompt_sha(&"prompt".to_string());
        a.add_participant_surface_sha(&"part-a".to_string());
        a.add_participant_surface_sha(&"part-b".to_string());
        let af = a.finalise();

        let mut b = FingerprintBuilder::new(Stage::L6, "edges-llm", "1.0");
        b.add_llm_fingerprint(&fp());
        b.add_prompt_sha(&"prompt".to_string());
        b.add_participant_surface_sha(&"part-a".to_string());
        b.add_participant_surface_sha(&"part-b".to_string());
        let bf = b.finalise();

        assert_eq!(af, bf);
    }

    // ---------------- proptest: any input change perturbs the hash --
    //
    // The property: for any well-formed sequence of `add_*` calls,
    //   - permuting the call order does not change the fingerprint;
    //   - mutating any single contribution changes the fingerprint.
    //
    // The proptest harness shrinks input lists to small examples so the
    // failure mode (if any) is debuggable.

    use proptest::prelude::*;

    /// One contribution to a `FingerprintBuilder`. Modelled as a
    /// tagged enum so proptest can shrink across both the chosen
    /// `add_*` method and the bytes it carries.
    #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
    enum Contribution {
        FileContent(String),
        Prompt(String),
        Llm(String, String, [u8; 32], [u8; 32]),
        ParticipantSurface(String),
    }

    fn arb_hex_sha() -> impl Strategy<Value = String> {
        // 1..=8 hex chars is enough to distinguish entries; keeps
        // proptest counterexamples small.
        proptest::collection::vec(any::<u8>().prop_map(|b| b % 16), 1..=8)
            .prop_map(|nibbles| nibbles.iter().map(|n| format!("{n:x}")).collect())
    }

    fn arb_contribution() -> impl Strategy<Value = Contribution> {
        prop_oneof![
            arb_hex_sha().prop_map(Contribution::FileContent),
            arb_hex_sha().prop_map(Contribution::Prompt),
            (
                "[a-z0-9-]{1,8}",
                "[a-z0-9.]{1,8}",
                any::<[u8; 32]>(),
                any::<[u8; 32]>(),
            )
                .prop_map(|(model, backend, t_sha, o_sha)| Contribution::Llm(
                    model, backend, t_sha, o_sha
                )),
            arb_hex_sha().prop_map(Contribution::ParticipantSurface),
        ]
    }

    fn apply(builder: &mut FingerprintBuilder, c: &Contribution) {
        match c {
            Contribution::FileContent(s) => builder.add_file_content_sha(s),
            Contribution::Prompt(s) => builder.add_prompt_sha(s),
            Contribution::Llm(model, backend, t_sha, o_sha) => {
                builder.add_llm_fingerprint(&LlmFingerprint {
                    template_sha: *t_sha,
                    ontology_sha: *o_sha,
                    model_id: model.clone(),
                    backend_version: backend.clone(),
                });
            }
            Contribution::ParticipantSurface(s) => builder.add_participant_surface_sha(s),
        }
    }

    fn build(stage: Stage, an: &str, ver: &str, contribs: &[Contribution]) -> Sha256Hex {
        let mut b = FingerprintBuilder::new(stage, an, ver);
        for c in contribs {
            apply(&mut b, c);
        }
        b.finalise()
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]

        #[test]
        fn permutation_preserves_fingerprint(
            mut contribs in proptest::collection::vec(arb_contribution(), 0..6),
        ) {
            // Dedupe so the fingerprint is order-independent across
            // permutations; equal `(tag, bytes)` entries collapse in
            // the BTreeSet so a permutation that retains duplicates
            // still hashes the same.
            let original = build(Stage::L3, "an", "v1", &contribs);
            // A reverse permutation suffices; fingerprint equality
            // for any permutation follows because BTreeSet sorts.
            contribs.reverse();
            let permuted = build(Stage::L3, "an", "v1", &contribs);
            prop_assert_eq!(original, permuted);
        }

        #[test]
        fn appending_a_distinct_contribution_changes_fingerprint(
            base in proptest::collection::vec(arb_contribution(), 0..6),
            extra in arb_contribution(),
        ) {
            // Skip if `extra` is already present; the BTreeSet
            // dedupes, so adding a duplicate is a no-op by design.
            let original_set: std::collections::BTreeSet<_> = base.iter().cloned().collect();
            prop_assume!(!original_set.contains(&extra));

            let original = build(Stage::L4, "an", "v", &base);
            let mut extended = base.clone();
            extended.push(extra);
            let extended_fp = build(Stage::L4, "an", "v", &extended);
            prop_assert_ne!(original, extended_fp);
        }

        #[test]
        fn changing_analyzer_id_changes_fingerprint(
            contribs in proptest::collection::vec(arb_contribution(), 0..6),
        ) {
            let a = build(Stage::L3, "an-a", "v", &contribs);
            let b = build(Stage::L3, "an-b", "v", &contribs);
            prop_assert_ne!(a, b);
        }

        #[test]
        fn changing_analyzer_version_changes_fingerprint(
            contribs in proptest::collection::vec(arb_contribution(), 0..6),
        ) {
            let a = build(Stage::L3, "an", "v-1", &contribs);
            let b = build(Stage::L3, "an", "v-2", &contribs);
            prop_assert_ne!(a, b);
        }
    }
}
