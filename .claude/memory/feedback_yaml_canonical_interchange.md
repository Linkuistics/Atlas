---
name: YAML is the canonical interchange format for Atlas
description: User directive 2026-05-13 — use YAML for all Atlas interchange (LLM I/O envelopes, internal artifact formats, projection files). JSON is reserved for places we don't control (LLM tool-use APIs, JSONL event streams).
type: feedback
---

YAML is the canonical interchange format for Atlas. This includes:
- LLM final-output envelopes (the fenced block at the end of an agent's reasoning that we deserialize into typed structs)
- Atlas-internal artifact files (projections, audit verdicts, intermediate caches)
- User-authored config + override files (already YAML — preserved)
- Future Atlas-emitted exchange artifacts (anything we write to disk for human or downstream-LLM consumption)

**Why:** User directive 2026-05-13 during production-prompt sprint brainstorm, following discussion of LLM output reliability. Free-text-heavy outputs with mixed quoting, path strings, code references, and multi-line content are JSON's well-known failure-mode territory: under-escaping, trailing commas, brace-depth disorientation, and multi-line-string confusion. YAML's block scalars (`|` and `>`) make multi-line strings natural; no quoting overhead for most strings; nested structure is *visually* preserved (LLMs handle indentation well when shown a worked example, vs. invisible brace-matching). YAML failure modes (Norway problem, implicit-typing of `1.10` → `1.1`, special-char starters like `*`/`&`/`!`) are mitigable via per-field deserialization adapters; JSON's quoting failures are not.

**How to apply:**

- **LLM output envelopes.** Prompt the LLM to emit ```yaml fenced blocks; `serde_yaml::from_str` deserializes into the target struct. Schema-in-prompt advertisement uses YAML-shaped examples (more LLM-readable than JSON-schema text).
- **Atlas-internal artifacts.** New files default to YAML extension. Existing JSON artifacts (e.g., PR-7's `agent-runtime-projection.json`) should be migrated to `.yaml` when touched; treat the migration as part of the work that touches them.
- **`serde_yaml` is already a workspace dependency.** No new dependencies needed for the format choice.
- **Norway-problem mitigation discipline.** For fields whose values could be misparsed (string fields that might look like booleans / numbers / nulls — `component_id`, `language`, `kind`, version strings), use `#[serde(deserialize_with = "deserialize_string_strict")]` adapters or document a "values in this field MUST be quoted" prompt convention. Add a test asserting that `component_id: NO` (Norway) deserializes as the string "NO", not the bool false.
- **Exceptions where JSON is still appropriate** (these are not "interchange" in the canonical sense):
  - **LLM tool-use APIs.** Anthropic Messages and OpenAI chat-completions tool-use APIs are JSON-native — tool schemas, tool calls, tool results pass through JSON because that's the wire format. We don't override the API; we override only the LLM's *final* answer envelope.
  - **JSONL event streams.** `--log-events events.jsonl` is a streamed append-only event log; JSON-Lines is the standard for this pattern, and each line being independently parseable matters for streaming consumers. YAML lacks an equally well-supported "lines" variant. Event logs are streams, not interchange documents.
  - **Inter-process wire protocols (gRPC, MCP).** Driven by upstream protocol; we adopt what the protocol uses.
- **Wider applicability beyond Atlas.** Any future LLM-driven CLI we build: default to YAML for the parts we control, fall through to JSON only where the wire format mandates it.
