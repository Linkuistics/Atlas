---
name: Cross-provider LLM audit beats same-model self-audit
description: For LLM-output validation, use a *different provider* than the producer (e.g. OpenAI auditing Anthropic output, or vice versa). User empirically found this a major improvement during Phase 7 Atlas brainstorm.
type: feedback
originSessionId: e4e7343b-07b1-45ba-b660-fdbccef4a564
---
For any "LLM produces output → LLM validates output" pattern (audit lanes, second-opinion review, structural verification of generated artefacts), the auditor should run on a **different provider** than the producer. Concretely: if production agents run Anthropic-flavoured backends (`claude_code`, `http_anthropic`), the auditor backend is OpenAI-flavoured (`http_openai`, `codex`), and vice versa. The choice of backend within a provider is flexible.

**Why:** User observed empirically during Atlas Phase 7 brainstorm (2026-05-11) that cross-provider audit yields a "major improvement" over same-model audit. The architectural reason: Anthropic and OpenAI models have asymmetric failure modes (caution profile, hallucination shape, instruction-following slack, confidence calibration). Same-model audit is tautological — the producer's blind spots become the auditor's blind spots; low-confidence rationalisations get accepted. Cross-provider audit breaks that symmetry: provider A's blind spot is provider B's salient observation.

**How to apply:**

- **Atlas Phase 7+ audit lane.** Lane B (the LLM-auditor lane in the recast spec §4.3) uses a different-provider auditor by construction. PR-0 declares the producer→auditor provider mapping as a config rule: Anthropic-flavoured producer → OpenAI-flavoured auditor, and vice versa.
- **Audit budget bucketing.** Token spend for audit calls lives in a separate *provider* bucket from production calls (it's not just "audit vs production" — it's "audit-provider-X vs production-provider-Y"). The polyglot smoke test's cold-token budget assertion is per-provider per-bucket, otherwise an auditor-budget regression can hide behind producer-budget headroom.
- **Degraded mode.** If only one provider is configured at runtime, audit falls back to same-model with an explicit warning surfaced on the event bus (`AuditDegraded { reason: "single-provider config" }`). PR-0 declares this; the warning is not a hard fail.
- **Transcript-cache key.** Audit-lane cache entries include `audit_provider` in the fingerprint to prevent cross-pollination when the audit provider changes between runs.
- **Wider applicability beyond Atlas.** Any future LLM-output-validation work (code review agents, schema-validation oracles, fact-check loops) should default to cross-provider when both providers are available.

The pattern generalises: *any* "model-grades-model" setup benefits from provider asymmetry. Same-model audit is the cheap-and-tautological choice; cross-provider is the higher-quality default whenever two providers are configured.
