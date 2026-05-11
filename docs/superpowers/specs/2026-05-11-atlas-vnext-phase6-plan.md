# Atlas vNext Phase 6 — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. The Phase 6 status file at `docs/superpowers/plans/2026-05-11-phase6-status.md` carries the per-PR checkbox state across sessions.

**Goal:** Ship the four pre-pivot Phase 6 candidate items (manifest-recognition extension; contract rename-match owner-follows; per-component `subsystem` field overlay; `--strict-overrides` flag with closed-enumeration warning surface and dual-mode contract test) as the final **deterministic-spine release** before the LLM-spine recast begins in Phase 7. Land the canonical §10 + §4.3 + §7.1 + §8.1 retext from the recast spec in PR-5 closeout.

**Architecture:** Phase 6 is an *editorial-tier user-authoring release*. Six PRs (PR-0 plan + PR-1 manifest extension + PR-2 rename-match owner-follows + PR-3 subsystem overlay + PR-4 `--strict-overrides` + PR-5 acceptance/closeout/retext). Net: ~+300 LOC production code (split across surface-rewriting + warning-collector machinery), ~+400 LOC test code, ~+100 lines design prose (the §10 retext). The Phase 3 polyglot smoke test (`crates/atlas-cli/tests/phase3_polyglot_fixture.rs`) is the cumulative regression guard — every Phase 6 PR re-runs it before flipping its status checkbox. Cold polyglot LLM-call count must remain at the Phase 2 PR-14 baseline (~26 calls); warm + reports = 0. **PR-1 LLM-call risk: see Step 1.1 caveat.**

**Tech Stack:** Rust workspace (Atlas + `component-ontology` + `atlas-index` schema crates, in-tree as of Phase 5); Salsa engine; serde for YAML I/O; clap for CLI args; existing test harnesses (`#[cfg(test)] mod tests` for unit, `crates/*-cli/tests/*.rs` for integration). No new crates introduced.

---

## 0. Reading order

Before this plan, read:

1. `docs/superpowers/specs/2026-05-11-atlas-llm-spine-recast-design.md` end-to-end. **This is the canonical source of the architectural inversion**; Phase 6 is the final deterministic-spine release that ships *before* the recast. PR-5 lands the §10 / §4.3 / §7.1 / §8.1 retext verbatim from §13 of that spec. Where this plan and the recast spec disagree on the retext content, the recast spec wins.
2. `docs/superpowers/specs/2026-05-10-atlas-vnext-phase5-plan.md` §0 (reading order) and §1 (deliverable, restated) for the prior-phase plan structure this plan follows; and `docs/superpowers/plans/2026-05-10-phase5-status.md` for the status-file shape PR-0 reproduces.
3. `docs/superpowers/specs/2026-05-06-atlas-system-model-design.md` §4.3 (the principle being retextually inverted in PR-5), §10.6 (the Phase 6 entry whose detailed enumeration this plan replaces), §10.7 onward (the phasing PR-5 retexts), §11 (decisions and open questions; §11.2.4 contract rename-match is the seam PR-2 fills), and the glossary.
4. `docs/superpowers/specs/2026-05-06-override-scoping-scattered-atlas.md` for the override-scoping warning surface PR-4 escalates.
5. Memory entries that constrain Phase 6:
   - `feedback_atlas_llm_spine_intent` — strategic preference for LLM-as-spine; Phase 6 ships *before* that inversion begins, and items 3 + 4 strengthen the user-authoring override discipline the recast will depend on.
   - `project_phase6_paused_for_llm_spine` — captures the four candidate items reached in the pre-pivot brainstorm with their pre-decided shape (this plan operationalises that shape; do not re-derive scope).
   - `feedback_no_iterator_stubs_for_singletons` — Phase 6 introduces no new collections that warrant iterator stubs.
   - `feedback_no_tail_pipe_for_long_tests` — never `tail`-pipe `cargo test` invocations of the polyglot smoke test.
   - `feedback_release_workspace_build_for_polyglot` — before any release polyglot test, `cargo build --release --workspace` first.
   - `feedback_atlas_memory_in_repo` — memory updates in PR-5 land in `.claude/memory/` in-repo paths.
   - `feedback_worktree_base_verification` — verify each worktree's base sha matches current main before parallel subagent dispatch.

This plan does *not* re-derive scope; it sequences and grounds what the pre-pivot brainstorm decided and what the LLM-spine recast spec §12 confirmed survives. The PR boundaries, acceptance criteria, and §10 retext content are anchored in those documents.

---

## 1. Phase 6 deliverable, restated

End of Phase 6, the Atlas codebase shall exhibit the following properties without changing any user-observable behaviour beyond the four newly-enabled features:

- **Makefile and shell-script files recognised as manifests.** ~~`is_manifest_file()` returns `true` for `Makefile`, `makefile`, `GNUmakefile`, any `*.mk` file, and any `*.sh` file. No paired classifier ships in this phase (deferred to Phase 9c per recast spec §11.3); recognised candidates fall through L3 to the existing `LlmClassify` fallback.~~ **DEFERRED 2026-05-11 to Phase 9c.** At PR-1 pre-flight (Step 1.1), the polyglot fixture was found to contain `build_glue/Makefile` and `scripts/deploy.sh` (inherited from the Phase 2 PR-14 fixture, surfaced today via `.atlas/components.overrides.yaml` `additions:` entries — not via manifest auto-discovery). Landing recognition-only would (a) raise the cumulative regression guard's cold LLM-call count from ~26 to ~28 via LlmClassify fallback, breaking the strict equality assertion, and (b) collide auto-discovered components with the existing `additions:` entries at the same paths. Recognition + paired classifier land together in Phase 9c per recast spec §11.3.
- **Contract rename-match owner-follows applied.** When the component rename-match (`crates/atlas-index/src/rename_match.rs`) maps `prior_id A → new_id B`, contracts owned by `A` follow to `B`: contract IDs whose owner-prefix is `A` are rewritten to use `B` as the owner-prefix; `DefinesContract` and `ConsumesContract` edges in `related-components.yaml` have their participants rewritten to the new contract IDs. The α implementation (id-embeds-owner) is chosen; β (content-sha-stable) deferred to Phase 10 (fuzzy contract matching). Independent fuzzy contract matching (a contract whose owner did *not* rename but whose content moved or split) remains out of scope per `.claude/memory/project_phase6_paused_for_llm_spine`.
- **`subsystem:` per-component override applied as overlay.** The parsed-but-ignored `subsystem: Option<String>` field on `ComponentFieldOverrides` (`crates/atlas-index/src/schema.rs:516-549`) is wired through L9 subsystem resolution as an overlay on `subsystems.overrides.yaml`. **Precedence: per-component override wins over central yaml** (closer-to-source wins; aligns with §4.1 plain-text-canonical and the co-located authoring discipline). A new warning class `SubsystemOverrideNonExistent` fires when `subsystems.overrides.yaml` lists a member-id that resolves to no extant component (the new class enumerated in PR-4's strict-mode list).
- **`--strict-overrides` CLI flag.** A new boolean flag on `atlas index` (added to `IndexArgs` in `crates/atlas-cli/src/main.rs`) escalates a *closed* enumeration of override warnings to errors with non-zero exit. The closed list contains exactly three variants:
  - `EdgesSuppressNoMatch { directive, scope }` (today's `eprintln!` at `l6_edges.rs:305-308`)
  - `EdgesAddUnknownKind { kind, scope }` (today's `eprintln!` at `l6_edges.rs:244-248`)
  - `SubsystemOverrideNonExistent { name, scope }` (new in PR-3)
  Pre-existing hard `TreeAssemblyError::PerComponentScopeViolation` errors are *unaffected* by the flag (they were already non-recoverable); only soft warnings are escalated.
- **Dual-mode override contract test.** A new integration test at `crates/atlas-cli/tests/strict_overrides_contract.rs` exercises every variant of the closed enumeration in both modes:
  - **Permissive mode (no `--strict-overrides`):** warning text appears on stderr; exit code = 0.
  - **Strict mode (`--strict-overrides`):** warning text appears on stderr; exit code ≠ 0.
  This subsumes the deferred Phase 3 PR-10 stderr-capture test (`edges_suppress` no-match warning was the original target; strict-mode contract test now covers all three variants).
- **Canonical §10 + §4.3 + §7.1 + §8.1 retext landed.** `docs/superpowers/specs/2026-05-06-atlas-system-model-design.md` updated per recast spec §13: §10 acquires new rows for Phases 7–11 (LLM-spine runtime, Cargo retirement, language retirement waves, LLM-driven analyses, server mode); §4.3 is replaced with the inversion paragraph; §7.1 Analyser interface marked retired with a forward-pointer to recast spec §5.1; §7.3 cost classes marked retired; §8.1 fingerprint table notes the `iteration_number + prior_model_sha` extension (forward-pointer; lands in Phase 7).
- **Audit greps clean.** `git grep -E 'TODO.*phase6|XXX.*phase6|FIXME.*phase6' crates/ docs/` returns zero hits at PR-5 close. `git grep -nE 'parsed but ignored|parsed-but-ignored' crates/` returns zero hits (PR-3 fixes this).
- **Cumulative regression guard green.** `cargo test -p atlas-cli --test phase3_polyglot_fixture --release --no-fail-fast` is green at every PR boundary (PR-1 through PR-5), with cold-call count at the Phase 2 PR-14 baseline (~26 calls) and warm-call count at 0. **Exception**: PR-1's manifest extension may increase the cold count *only if* the polyglot fixture contains `.mk` or `.sh` files; the fixture does not (verified pre-PR-1, see Step 1.1), so cold count remains baseline.

---

## 2. File structure (what each PR touches)

This map locks the decomposition. Each task below produces self-contained changes that pass the cumulative regression guard on landing.

### Files **created** in Phase 6

| Path | PR | Purpose |
|---|---|---|
| `docs/superpowers/specs/2026-05-11-atlas-vnext-phase6-plan.md` | PR-0 | This plan. |
| `docs/superpowers/plans/2026-05-11-phase6-status.md` | PR-0 | Per-PR checkbox status file. |
| `docs/superpowers/prompts/2026-05-11-vnext-continue.md` | PR-0 | Cross-session resume prompt (Phase-6-shaped). |
| `crates/atlas-engine/src/override_warnings.rs` | PR-4 | New module defining the `OverrideWarning` closed enum + `OverrideWarningCollector` trait. |
| `crates/atlas-cli/tests/contract_rename_owner_follows.rs` | PR-2 | Integration test for contract rename-match owner-follows. |
| `crates/atlas-cli/tests/subsystem_overlay.rs` | PR-3 | Integration test for per-component `subsystem` field overlay (extends `subsystems_integration.rs` patterns). |
| `crates/atlas-cli/tests/strict_overrides_contract.rs` | PR-4 | Dual-mode (permissive + strict) contract test for the closed override-warning enumeration. |

### Files **modified** in Phase 6

| Path | PR | Change |
|---|---|---|
| `crates/atlas-engine/src/manifest_patterns.rs:8-37` | PR-1 | Add `"Makefile"`, `"makefile"`, `"GNUmakefile"` to `EXACT_MANIFEST_BASENAMES`. |
| `crates/atlas-engine/src/manifest_patterns.rs:55-85` | PR-1 | Add suffix recognition for `.mk` and `.sh` extensions in `is_manifest_file()`. |
| `crates/atlas-engine/src/manifest_patterns.rs:147-225` | PR-1 | Add three new tests: `recognises_makefile_variants_by_basename`, `recognises_mk_files_by_suffix`, `recognises_shell_scripts_by_suffix`. |
| `crates/atlas-index/src/rename_match.rs` | PR-2 | Add `rename_owned_contracts()` helper (or equivalent) that takes the rename-match map and rewrites contract IDs in component output. |
| `crates/atlas-index/src/surfaces.rs:254-277` | PR-2 | If `Contract { id, … }` needs explicit owner tracking beyond the id-prefix convention, add helper methods (read-only inspection of owner). |
| `crates/atlas-engine/src/l5_surface.rs` | PR-2 | Apply contract-id rewrites at the post-rename-match seam (where component IDs are stabilised after rename-match; contracts owned by renamed components rewrite their id-prefix in the same pass). |
| `crates/atlas-engine/src/l6_edges.rs` | PR-2 | Update related-components edge participants: any edge with `participants[i]` referencing an old contract id rewrites to the new contract id. |
| `crates/atlas-engine/src/l4_tree.rs:272-325` | PR-3 | Remove the `let _ = fo.subsystem.as_ref();` no-op (line ~324); apply per-component `subsystem` override to component's resolved subsystem membership via the new overlay mechanism. |
| `crates/atlas-engine/src/l9_subsystems.rs:22-200` | PR-3 | Extend `resolve_subsystems()` to accept per-component overrides as overlay input; merge per-component overrides over central subsystems.overrides.yaml entries (per-component wins). Add `SubsystemOverrideNonExistent` warning emission when central yaml references unknown member. |
| `crates/atlas-cli/src/main.rs:66-132` | PR-4 | Add `#[arg(long)] strict_overrides: bool` to `IndexArgs` (clap derive style, matching existing flag pattern). |
| `crates/atlas-cli/src/pipeline.rs` | PR-4 | Propagate `strict_overrides` from `IndexArgs` into `IndexConfig`; pass through to `run_index()`. |
| `crates/atlas-engine/src/lib.rs` | PR-4 | Add `pub mod override_warnings;` and `pub use override_warnings::{OverrideWarning, OverrideWarningCollector};`. Add `strict_overrides: bool` to `IndexConfig` struct. |
| `crates/atlas-engine/src/l6_edges.rs:244-248, 305-308` | PR-4 | Replace the two `eprintln!()` warning sites with `collector.emit(OverrideWarning::EdgesAddUnknownKind { ... })` and `collector.emit(OverrideWarning::EdgesSuppressNoMatch { ... })`. The collector implementations (permissive: writes to stderr; strict: writes to stderr *and* sets a sticky `has_errors` flag) live in `override_warnings.rs`. |
| `crates/atlas-engine/src/l9_subsystems.rs` | PR-4 | Replace the `SubsystemOverrideNonExistent` warning emission (added in PR-3) with `collector.emit(OverrideWarning::SubsystemOverrideNonExistent { ... })`. |
| `crates/atlas-cli/tests/phase3_overrides_edges.rs:353-392` | PR-4 | Update the `edges_suppress_no_match_leaves_set_unchanged` test to assert *both* stderr text (deferred Phase 3 PR-10 work) *and* exit code = 0 in permissive mode. The dual-mode test in `strict_overrides_contract.rs` covers the strict-mode counterpart. |
| `docs/superpowers/specs/2026-05-06-atlas-system-model-design.md` | PR-5 | Apply the §13 retext from recast spec: §4.3 replacement, §7.1 retirement note, §7.3 retirement note, §8.1 extension note, §10.6 (Phase 6 SHIPPED), §10.7–§10.11 new rows for Phases 7–11. |
| `docs/superpowers/plans/2026-05-11-phase6-status.md` | PR-1..PR-5 | Flip per-PR checkboxes; PR-5 lands closeout note. |
| `.claude/memory/project_phase4_plus_roadmap.md` | PR-5 | Mark Phase 6 SHIPPED; advance Phase 7 (LLM-spine runtime) to next-up. |
| `.claude/memory/project_phase6_paused_for_llm_spine.md` | PR-5 | Mark as superseded by `project_phase6_shipped.md` (or transition to "Phase 6 SHIPPED 2026-05-NN" note). |
| `.claude/memory/MEMORY.md` | PR-5 | Update index entries for Phase 6 closeout. |

### Files **deleted** in Phase 6

| Path | PR | LOC |
|---|---|---|
| (none) | — | — |

Phase 6 is **purely additive** (new tests, new module, new CLI flag, new overlay path). No production-code deletions. The closest thing to a "deletion" is PR-3 removing the `let _ = fo.subsystem.as_ref();` no-op line (1 LOC, replaced by the wire-through code).

---

## 3. Dependency graph

```
PR-0 (plan + status + continuation prompt)
  │
  ├──► PR-1 (manifest extension) ─────────────────────────────┐
  │                                                            │
  ├──► PR-2 (contract rename-match owner-follows) ─────────────┤
  │                                                            │
  └──► PR-3 (subsystem field overlay)                           │
          │                                                    │
          ▼                                                    │
       PR-4 (--strict-overrides + closed enum + dual-mode test) │
          │                                                    │
          ▼                                                    │
       PR-5 (acceptance + closeout + §10/§4.3/§7/§8 retext) ◄───┘
```

**Parallel-safe waves:**

- **Wave 0:** PR-0 (this commit).
- **Wave 1 (after PR-0):** **PR-1, PR-2, and PR-3 are parallel-safe — disjoint code surfaces.**
  - PR-1 edits only `crates/atlas-engine/src/manifest_patterns.rs`.
  - PR-2 edits `crates/atlas-index/src/rename_match.rs`, `crates/atlas-engine/src/l5_surface.rs`, `crates/atlas-engine/src/l6_edges.rs` (the *participant rewrite* logic, not the `eprintln!` sites PR-4 touches).
  - PR-3 edits `crates/atlas-engine/src/l4_tree.rs` and `crates/atlas-engine/src/l9_subsystems.rs`.
  - The three may be dispatched concurrently in separate worktrees. Verify each worktree's base commit matches current main before the subagent proceeds (memory `feedback_worktree_base_verification`).
- **Wave 2 (after Wave 1):** PR-4 alone. PR-4 depends on PR-3's new `SubsystemOverrideNonExistent` warning class for the closed-enumeration list; it also touches `l6_edges.rs:244-248,305-308` (a different region than PR-2's edits but on the same file, so PR-2 must land first to avoid trivial merge conflicts).
- **Wave 3 (after PR-4):** PR-5. Final closeout + canonical retext.

The Phase 3 polyglot smoke test (`crates/atlas-cli/tests/phase3_polyglot_fixture.rs`) is the cumulative regression guard for every Phase 6 PR. Each PR's checkbox-flip step includes a final `cargo test -p atlas-cli --test phase3_polyglot_fixture --release --no-fail-fast` invocation; the strict LLM-call-budget assertions (cold = Phase 2 PR-14 baseline ~26 calls; warm + reports = 0) catch any drift.

---

## 4. Tasks

### Task 0: PR-0 — Plan + status + continuation prompt *(docs only)*

**Files:**
- Create: `docs/superpowers/specs/2026-05-11-atlas-vnext-phase6-plan.md` (this file)
- Create: `docs/superpowers/plans/2026-05-11-phase6-status.md`
- Create: `docs/superpowers/prompts/2026-05-11-vnext-continue.md`

- [ ] **Step 0.1: Verify clean working tree**

```bash
git status
git log --oneline -5
```

Expected: clean working tree (the recast spec commit `409dcc5` is the most recent on main; any in-flight uncommitted memory file updates may be present and are deliberately left for PR-5).

- [ ] **Step 0.2: This plan file already exists from this writing-plans session — confirm**

The plan you are reading lives at `docs/superpowers/specs/2026-05-11-atlas-vnext-phase6-plan.md`. PR-0 includes it as one of three deliverables. The remaining two — status file and continuation prompt — are written next.

- [ ] **Step 0.3: Create the Phase 6 status file**

Write `docs/superpowers/plans/2026-05-11-phase6-status.md`:

```markdown
# Atlas vNext Phase 6 — Status

Companion to `docs/superpowers/specs/2026-05-11-atlas-vnext-phase6-plan.md`. This file tracks per-PR completion state across sessions. The continuation prompt at `docs/superpowers/prompts/2026-05-11-vnext-continue.md` (Phase-6-shaped) reads this file (via the `*phase6-plan*` wildcard match) to find the next PR to dispatch.

**Last updated:** 2026-05-11 (PR-0 landed: plan + status + continuation prompt).

## PR status

Mark `[~]` when a subagent is dispatched and not yet merged; mark `[x]` when the PR is reviewed and committed. Append a one-line note (date + commit sha + anything load-bearing the next session needs to know).

- [x] PR-0 — Plan + status + continuation prompt (docs only)
- [ ] PR-1 — `is_manifest_file` Makefile/shell extension (small)
- [ ] PR-2 — Contract rename-match owner-follows (medium)
- [ ] PR-3 — `subsystem` field overlay (medium)
- [ ] PR-4 — `--strict-overrides` + closed enum + dual-mode contract test (medium)
- [ ] PR-5 — Acceptance + closeout + canonical §10/§4.3/§7/§8 retext (docs + verification)

When every box is `[x]`, Phase 6 is complete and the continuation prompt should report success and route to brainstorm/plan for Phase 7 (LLM-spine runtime per canonical §10.7, recast spec §11.1).

## Dependency graph (informational; canonical in plan §3)

\```
PR-0 ──► PR-1 ─┐
       │       │
       ├──► PR-2 ─┤
       │       │
       └──► PR-3 ──► PR-4 ──► PR-5
                            ▲
                            │
        (PR-1, PR-2 join here too)
\```

**Parallel-safe waves:**

- **Wave 0:** PR-0 (this commit).
- **Wave 1 (after PR-0):** PR-1, PR-2, PR-3 dispatched in parallel — disjoint code surfaces. Use `superpowers:dispatching-parallel-agents`; verify each worktree's base commit matches current main before subagent proceeds.
- **Wave 2 (after Wave 1):** PR-4 alone. Depends on PR-3's `SubsystemOverrideNonExistent` warning class.
- **Wave 3 (after PR-4):** PR-5 — acceptance + closeout + canonical retext.

The Phase 3 polyglot smoke test (`crates/atlas-cli/tests/phase3_polyglot_fixture.rs`) is the cumulative regression guard. Each PR re-runs it; cold = ~26 calls; warm + reports = 0.

## Per-PR notes

Sessions append session-relevant context here as PRs land.

### PR-0
2026-05-11 — Landed: the Phase 6 plan, this status file, and the continuation prompt. Commit: `<sha>`. Plan: `docs/superpowers/specs/2026-05-11-atlas-vnext-phase6-plan.md`. Continuation prompt: `docs/superpowers/prompts/2026-05-11-vnext-continue.md`. Recast spec context: `docs/superpowers/specs/2026-05-11-atlas-llm-spine-recast-design.md` on main as `409dcc5`.
```

- [ ] **Step 0.4: Create the Phase 6 continuation prompt**

Write `docs/superpowers/prompts/2026-05-11-vnext-continue.md` by templated copy-edit of `docs/superpowers/prompts/2026-05-10-vnext-continue.md`, substituting Phase 5 → Phase 6 references. Key substitutions: the orientation Step 1 reads the Phase 6 design (the recast spec `2026-05-11-atlas-llm-spine-recast-design.md` is the design anchor for Phase 6's PR-5 retext content), the Phase 6 plan (this file), and the Phase 6 status file. Step 2 dispatches parallel subagents for Wave 1 (PR-1 + PR-2 + PR-3); subsequent waves are sequential. The non-negotiables section retains the cumulative regression guard requirements. The Phase 4-shaped continuation prompt remains for forensic context. **Do not auto-write a Phase 7 plan upon Phase 6 completion** — Phase 7 requires its own brainstorm via `superpowers:brainstorming` (the recast spec captures architectural intent but per-PR scope still needs design).

The full continuation prompt structure follows Phase 5's template (`docs/superpowers/prompts/2026-05-10-vnext-continue.md`), with the following replacements:

- "Phase 5" → "Phase 6" throughout.
- "2026-05-10-atlas-vnext-phase5-design.md" → "2026-05-11-atlas-llm-spine-recast-design.md" (the recast spec serves as the design anchor for Phase 6's PR-5 retext).
- "2026-05-10-atlas-vnext-phase5-plan.md" → "2026-05-11-atlas-vnext-phase6-plan.md".
- "2026-05-10-phase5-status.md" → "2026-05-11-phase6-status.md".
- Phase 4 mentions as "complete" remain; add Phase 5 to the "complete" list.
- The "Step 3 — Special PR-handling notes" section rewrites to enumerate Phase 6's specific PR-1..PR-5 caveats (manifest-extension LLM-call risk, contract id α implementation choice, subsystem overlay precedence rule, strict-overrides closed enumeration, retext anchored in recast spec §13).
- The "Step 4 — Phase 6 complete; consider Phase 7" reframes the next-phase question: "Phase 6 is complete. Phase 7 (LLM-spine runtime per canonical §10.7 / recast spec §11.1) is the next phase. Want me to brainstorm Phase 7 scope?"

- [ ] **Step 0.5: Pre-flight sanity check (the build is clean)**

```bash
cargo build --workspace
```

Expected: clean build. PR-0 changes no code; this is a pre-flight to confirm the worktree is in a known-good state before Wave 1 dispatches.

- [ ] **Step 0.6: Commit PR-0**

```bash
git add docs/superpowers/specs/2026-05-11-atlas-vnext-phase6-plan.md \
        docs/superpowers/plans/2026-05-11-phase6-status.md \
        docs/superpowers/prompts/2026-05-11-vnext-continue.md
git commit -m "$(cat <<'EOF'
phase6: PR-0 plan + status + continuation prompt

No code changes. Lays the per-PR scaffolding for the four
pre-pivot Phase 6 candidate items (manifest extension; contract
rename-match owner-follows; subsystem field overlay;
--strict-overrides + closed enum + dual-mode contract test) plus
PR-5 closeout (canonical §10/§4.3/§7/§8 retext from recast spec
§13). Phase 6 is the final deterministic-spine release before the
LLM-spine recast begins in Phase 7. Companion to
2026-05-11-atlas-llm-spine-recast-design.md (on main as 409dcc5).

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

Expected: one commit on the Phase 6 worktree branch.

---

### Task 1: PR-1 — `is_manifest_file` Makefile/shell extension *(structural, small)*

**Files:**
- Modify: `crates/atlas-engine/src/manifest_patterns.rs:8-37` (EXACT_MANIFEST_BASENAMES)
- Modify: `crates/atlas-engine/src/manifest_patterns.rs:55-85` (is_manifest_file suffix rules)
- Modify: `crates/atlas-engine/src/manifest_patterns.rs:147-225` (tests module — add three new tests)

**Pre-flight constraint:** The polyglot fixture (`crates/atlas-cli/tests/fixtures/phase3_polyglot/`) **must not** contain `.mk` or `.sh` files at PR-1's start. If it does, the cumulative regression guard's cold LLM-call count will jump from ~26 to ~26+N (where N = number of newly-recognised candidates) because the new manifest candidates fall through L3 to `LlmClassify`. Verify in Step 1.1 below.

**Risk flag (load-bearing for PR-1 acceptance):** Adding `.sh` to recognition without a paired classifier means *any future workspace* containing shell scripts will produce LlmClassify fallback calls for each `.sh` file. This is intentional per the brainstorm-approved "B1 purity" choice (recognition-only; no classifier this PR). The classifier proper folds into Phase 9c per recast spec §11.3. Document this in the PR description; the user accepts the trade-off.

- [ ] **Step 1.1: Verify the polyglot fixture has no `.mk` or `.sh` files**

```bash
find crates/atlas-cli/tests/fixtures/phase3_polyglot -name '*.mk' -o -name '*.sh' -o -name 'Makefile' -o -name 'makefile' -o -name 'GNUmakefile'
```

Expected: empty output. If any files surface, **STOP** — the cumulative regression guard will fail and the LLM-call-budget invariant breaks. Surface the discovery to the user before proceeding.

- [ ] **Step 1.2: Read current `manifest_patterns.rs` to anchor the edits**

```bash
sed -n '1,90p' crates/atlas-engine/src/manifest_patterns.rs
```

Expected: view of `EXACT_MANIFEST_BASENAMES` (lines 8-37) and `is_manifest_file()` (lines 55-85). Note the existing suffix-rule pattern (e.g. `.nix` at lines 64-66; `.csproj`/`.sln` at lines 70-72; `.sld` at lines 74-76) for mirroring.

- [ ] **Step 1.3: Add Makefile basenames to `EXACT_MANIFEST_BASENAMES`**

In `crates/atlas-engine/src/manifest_patterns.rs:8-37`, append three entries to the array. The current shape is:

```rust
const EXACT_MANIFEST_BASENAMES: &[&str] = &[
    "Cargo.toml", "package.json", "tsconfig.json", "pubspec.yaml", "info.rkt",
    "go.mod", "setup.py", "Gemfile", "pom.xml", "build.gradle", "CMakeLists.txt",
    "Dockerfile", "flake.nix", "shard.yml", "mix.exs", "composer.json", "deno.json",
];
```

Edit to:

```rust
const EXACT_MANIFEST_BASENAMES: &[&str] = &[
    "Cargo.toml", "package.json", "tsconfig.json", "pubspec.yaml", "info.rkt",
    "go.mod", "setup.py", "Gemfile", "pom.xml", "build.gradle", "CMakeLists.txt",
    "Dockerfile", "flake.nix", "shard.yml", "mix.exs", "composer.json", "deno.json",
    "Makefile", "makefile", "GNUmakefile",
];
```

- [ ] **Step 1.4: Add `.mk` suffix recognition to `is_manifest_file()`**

In `crates/atlas-engine/src/manifest_patterns.rs:55-85`, after the existing suffix-rule blocks (`.nix`, `.csproj`/`.sln`, `.sld`) and before the Dockerfile / Docker-Compose delegation, add:

```rust
if name.ends_with(".mk") {
    return true;
}
```

Mirror the exact pattern of the `.nix` block at lines 64-66.

- [ ] **Step 1.5: Add `.sh` suffix recognition to `is_manifest_file()`**

Immediately after Step 1.4's edit, add:

```rust
if name.ends_with(".sh") {
    return true;
}
```

- [ ] **Step 1.6: Add three new tests to the `tests` module**

In `crates/atlas-engine/src/manifest_patterns.rs:147-225` (the `#[cfg(test)] mod tests` block), append three test functions. Mirror the shape of existing tests (e.g. `recognises_nix_files_by_suffix` for suffix tests; `recognises_canonical_compose_files` for basename tests):

```rust
#[test]
fn recognises_makefile_variants_by_basename() {
    assert!(is_manifest_file(Path::new("project/Makefile")));
    assert!(is_manifest_file(Path::new("project/makefile")));
    assert!(is_manifest_file(Path::new("project/GNUmakefile")));
}

#[test]
fn recognises_mk_files_by_suffix() {
    assert!(is_manifest_file(Path::new("project/rules.mk")));
    assert!(is_manifest_file(Path::new("project/build/extras.mk")));
}

#[test]
fn recognises_shell_scripts_by_suffix() {
    assert!(is_manifest_file(Path::new("project/build.sh")));
    assert!(is_manifest_file(Path::new("project/scripts/release.sh")));
}
```

Use the imports already in scope at the top of the test module (`use super::*;` and `use std::path::Path;`).

- [ ] **Step 1.7: Run the unit tests for `manifest_patterns`**

```bash
cargo test -p atlas-engine --lib manifest_patterns
```

Expected: all tests pass, including the three new ones.

- [ ] **Step 1.8: Run the full workspace test suite**

```bash
cargo test --workspace --release --no-fail-fast
```

Expected: clean. Do NOT pipe through `tail` (memory `feedback_no_tail_pipe_for_long_tests`).

- [ ] **Step 1.9: Run the cumulative regression guard**

```bash
cargo build --release --workspace
cargo test -p atlas-cli --test phase3_polyglot_fixture --release --no-fail-fast
```

Expected: `phase3_polyglot_fixture` passes. Cold LLM-call count unchanged from Phase 2 PR-14 baseline (~26 calls); warm + reports = 0. **If the cold count jumps**: the polyglot fixture has a `.mk` or `.sh` file that Step 1.1 missed — investigate before flipping the PR-1 checkbox.

- [ ] **Step 1.10: Lints + fmt clean**

```bash
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

Expected: clean.

- [ ] **Step 1.11: Commit PR-1**

```bash
git add crates/atlas-engine/src/manifest_patterns.rs
git commit -m "$(cat <<'EOF'
phase6: PR-1 is_manifest_file Makefile/shell extension

Extends manifest recognition to include Makefile / makefile /
GNUmakefile (exact basenames) and *.mk / *.sh (suffix rules).
Recognition-only: no paired classifier ships in this phase
(deferred to Phase 9c per LLM-spine recast spec §11.3). Polyglot
fixture verified contains no .mk or .sh files; cumulative
regression guard cold count unchanged at Phase 2 PR-14 baseline.

Per `.claude/memory/project_phase6_paused_for_llm_spine` item #1
("smallest item").

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

Expected: one commit.

- [ ] **Step 1.12: Flip PR-1 checkbox in status file**

Edit `docs/superpowers/plans/2026-05-11-phase6-status.md`:
- Mark `- [x] PR-1 — is_manifest_file Makefile/shell extension (small)`.
- Append under `### PR-1` in the per-PR notes section: `2026-05-NN — Commit: <sha> on main. All cargo gates clean; polyglot smoke test cold count = <N> (Phase 2 PR-14 baseline).`

Commit the status flip as a separate commit:

```bash
git add docs/superpowers/plans/2026-05-11-phase6-status.md
git commit -m "phase6: PR-1 status checkbox + note

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

Expected: one commit.

---

### Task 2: PR-2 — Contract rename-match owner-follows *(structural, medium)*

**Files:**
- Modify: `crates/atlas-index/src/rename_match.rs` (add `rename_owned_contracts()` helper or extend existing rename map application)
- Modify: `crates/atlas-engine/src/l5_surface.rs` (post-rename contract id rewrite seam)
- Modify: `crates/atlas-engine/src/l6_edges.rs` (related-components edge participant rewrite for rewritten contract ids)
- Modify: `crates/atlas-index/src/surfaces.rs:254-277` (only if helper methods needed for contract owner inspection)
- Create: `crates/atlas-cli/tests/contract_rename_owner_follows.rs` (integration test)

**Pre-flight constraint:** PR-2 commits to the **α implementation (id-embeds-owner)**. Contract ids today use owner-prefix format (`<component-id>/<contract-name>`, e.g. `atlas-contracts/components-yaml-schema`). When `prior_id A → new_id B`, the rewrite rule is: contracts whose id starts with `A/` get their prefix rewritten to `B/`. β (content-sha-stable, where contract ids derive from content sha and are owner-invariant) is **deferred to Phase 10** (fuzzy contract matching per recast spec §11.4) and explicitly not chosen here.

Independent fuzzy contract matching (where a contract's owner did *not* rename but the contract's *content* moved or split) is also **out of scope** for PR-2 per `.claude/memory/project_phase6_paused_for_llm_spine`. Only owner-follows propagation is implemented.

- [ ] **Step 2.1: Read current rename-match implementation**

```bash
sed -n '1,140p' crates/atlas-index/src/rename_match.rs
```

Expected: view of the greedy bipartite matching algorithm (path-segment content-SHA overlap, ≥0.70 threshold). Note the return type — likely `Vec<(prior_idx, new_idx)>` or similar — and where this map is consumed downstream.

- [ ] **Step 2.2: Find the call sites that apply rename-match results**

```bash
grep -rn "rename_match" crates/atlas-engine/src/ crates/atlas-index/src/
```

Expected: identifies the post-rename-match seam (likely in L3 or L4 — `l3_classify.rs` or `l4_tree.rs`) where the rename map is applied to stabilise component IDs across runs. This is the seam contract owner-follows extends.

- [ ] **Step 2.3: Find all places contract ids are emitted**

```bash
grep -rn "contracts_defined\|DefinesContract\|ConsumesContract\|fn.*contract.*id" crates/atlas-engine/src/ crates/atlas-index/src/
```

Expected: identifies (a) where surfaces.yaml's `contracts_defined` is written, (b) where related-components.yaml's `DefinesContract`/`ConsumesContract` edges are emitted. PR-2 needs to apply the rename map at both sites.

- [ ] **Step 2.4: Write the failing integration test (TDD)**

Create `crates/atlas-cli/tests/contract_rename_owner_follows.rs` with the following content (modelled on the existing `crates/atlas-cli/tests/contract_edge_in_workspace.rs` salvaged in Phase 5 PR-4):

```rust
//! Phase 6 PR-2: contract rename-match owner-follows.
//!
//! When a component is renamed (path moves; rename-match maps
//! `prior_id A → new_id B`), contracts owned by `A` follow to `B`:
//! their id-prefix rewrites from `A/...` to `B/...`, and edges in
//! related-components.yaml have their participants updated.

use std::fs;
use tempfile::TempDir;

mod common;
use common::{run_atlas_index, read_components_yaml, read_related_components_yaml, read_surfaces_yaml};

#[test]
fn contract_owner_follows_component_rename() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    // Run 1: workspace has `original-name` containing contract `c1`.
    fs::create_dir_all(root.join("original-name/src")).unwrap();
    fs::write(root.join("original-name/Cargo.toml"), r#"
[package]
name = "original-name"
version = "0.1.0"
"#).unwrap();
    fs::write(root.join("original-name/src/lib.rs"), r#"
pub struct C1 { pub x: i32 }
"#).unwrap();

    run_atlas_index(root).unwrap();

    let surfaces_pre = read_surfaces_yaml(&root.join("original-name/.atlas/cache/surfaces.yaml"));
    let contract_id_pre = surfaces_pre.contracts_defined.first().expect("contract defined").id.clone();
    assert!(contract_id_pre.starts_with("original-name/"), "expected owner-prefix; got {}", contract_id_pre);

    // Rename the component directory.
    fs::rename(root.join("original-name"), root.join("renamed-component")).unwrap();
    // Cargo.toml's `name` field stays the same — rename-match's signal is
    // path-segment content overlap, not the package name.

    run_atlas_index(root).unwrap();

    // Assertion 1: surfaces.yaml under renamed-component contains the same contract
    // but with the new owner-prefix.
    let surfaces_post = read_surfaces_yaml(&root.join("renamed-component/.atlas/cache/surfaces.yaml"));
    let contract_id_post = surfaces_post.contracts_defined.first().expect("contract defined").id.clone();
    assert!(contract_id_post.starts_with("renamed-component/"),
            "expected new owner-prefix; got {}", contract_id_post);

    // Assertion 2: related-components.yaml's DefinesContract edge has the new participant id.
    let related = read_related_components_yaml(&root.join(".atlas/cache/related-components.yaml"));
    let defines = related.find_edges_by_kind("defines-contract");
    let participants: Vec<&str> = defines.iter().flat_map(|e| e.participants.iter().map(|s| s.as_str())).collect();
    assert!(participants.contains(&contract_id_post.as_str()),
            "expected defines-contract edge to reference new contract id; got participants={:?}", participants);
    assert!(!participants.contains(&contract_id_pre.as_str()),
            "expected no edge to reference stale pre-rename contract id");
}
```

Then add the test helpers (`common::run_atlas_index`, `read_components_yaml`, `read_related_components_yaml`, `read_surfaces_yaml`) by mirroring the patterns in `crates/atlas-cli/tests/contract_edge_in_workspace.rs`. If a `common` module doesn't already exist, create `crates/atlas-cli/tests/common/mod.rs` (or extend the existing one).

- [ ] **Step 2.5: Run the test to verify it fails for the right reason**

```bash
cargo test -p atlas-cli --test contract_rename_owner_follows --release --no-fail-fast
```

Expected: **FAIL**. The failure should be on Assertion 1 (`contract_id_post.starts_with("renamed-component/")`) — the contract id retains the pre-rename owner-prefix because no owner-follows logic exists yet. If the test fails for a different reason (e.g. panic in fixture setup, missing helper, etc.), fix the test plumbing first.

- [ ] **Step 2.6: Implement contract id rewrite at the post-rename-match seam**

Identify the function in `crates/atlas-engine/src/l5_surface.rs` (or wherever the post-rename-match seam landed in Step 2.2) that runs *after* rename_match returns the prior→new id map but *before* surfaces.yaml is written. Inject a new helper call:

```rust
// After component-id stabilisation via rename-match, propagate id rewrites
// to contracts owned by renamed components. Per LLM-spine recast plan §1 PR-2,
// uses α (id-embeds-owner) implementation: contracts whose id starts with
// `<prior_id>/` get their prefix rewritten to `<new_id>/`.
for (prior_id, new_id) in &rename_map {
    if prior_id == new_id { continue; }
    rewrite_contract_owner_prefix(&mut surfaces, prior_id, new_id);
}
```

Define `rewrite_contract_owner_prefix()` either in `crates/atlas-index/src/rename_match.rs` (as a public helper alongside the existing rename-match code) or inline at the call site if the function is small (~20 LOC). Recommended location: `rename_match.rs` to keep all rename logic co-located.

```rust
/// Phase 6 PR-2: apply the owner-follows rule to all contracts that begin
/// with `prior_id/...`. Rewrites the prefix to `new_id/...` in-place on every
/// `Contract.id` and on any `definition_binding` reference that embeds it.
/// α implementation (id-embeds-owner); β content-sha-stable is deferred to
/// Phase 10.
pub fn rewrite_contract_owner_prefix(
    surfaces: &mut SurfacesFile,
    prior_id: &ComponentId,
    new_id: &ComponentId,
) {
    let old_prefix = format!("{}/", prior_id);
    let new_prefix = format!("{}/", new_id);
    for contract in &mut surfaces.contracts_defined {
        if let Some(suffix) = contract.id.strip_prefix(&old_prefix) {
            contract.id = format!("{}{}", new_prefix, suffix);
        }
    }
}
```

Adjust the exact type names (`SurfacesFile`, `Contract`, `ComponentId`) to match the actual code; the explore report flagged `surfaces.rs:254-277` as the canonical type definition site.

- [ ] **Step 2.7: Propagate the rewrites into related-components.yaml edge participants**

In `crates/atlas-engine/src/l6_edges.rs`, find the function that emits `DefinesContract` and `ConsumesContract` edges. After edges are computed but before they are written, apply the same rewrite to edge participants:

```rust
// Apply Phase 6 PR-2 contract owner-follows to edge participants. Any edge
// participant referencing a contract id with a prior owner-prefix gets rewritten.
for edge in &mut edges {
    for participant in &mut edge.participants {
        for (prior_id, new_id) in rename_map {
            if prior_id == new_id { continue; }
            let old_prefix = format!("{}/", prior_id);
            let new_prefix = format!("{}/", new_id);
            if let Some(suffix) = participant.strip_prefix(&old_prefix) {
                *participant = format!("{}{}", new_prefix, suffix);
            }
        }
    }
}
```

If `rename_map` is not in scope at this site, plumb it through via `AnalysisContext` or an equivalent context type.

- [ ] **Step 2.8: Re-run the integration test to verify it passes**

```bash
cargo test -p atlas-cli --test contract_rename_owner_follows --release --no-fail-fast
```

Expected: **PASS**. Both assertions hold.

- [ ] **Step 2.9: Add unit tests for `rewrite_contract_owner_prefix`**

In `crates/atlas-index/src/rename_match.rs`'s test module, add:

```rust
#[test]
fn rewrite_contract_owner_prefix_rewrites_matching_prefix() {
    let mut surfaces = SurfacesFile {
        contracts_defined: vec![
            Contract { id: "old-name/c1".to_string(), /* ... */ },
            Contract { id: "old-name/c2".to_string(), /* ... */ },
            Contract { id: "unrelated/c3".to_string(), /* ... */ },
        ],
        /* ... */
    };
    rewrite_contract_owner_prefix(&mut surfaces, &"old-name".into(), &"new-name".into());
    assert_eq!(surfaces.contracts_defined[0].id, "new-name/c1");
    assert_eq!(surfaces.contracts_defined[1].id, "new-name/c2");
    assert_eq!(surfaces.contracts_defined[2].id, "unrelated/c3");
}

#[test]
fn rewrite_contract_owner_prefix_no_op_when_prior_equals_new() {
    let mut surfaces = SurfacesFile {
        contracts_defined: vec![Contract { id: "same/c1".to_string(), /* ... */ }],
        /* ... */
    };
    rewrite_contract_owner_prefix(&mut surfaces, &"same".into(), &"same".into());
    assert_eq!(surfaces.contracts_defined[0].id, "same/c1");
}
```

Adjust `Contract` field initialisation to match the actual type (use `..Default::default()` if a default exists, or list every field explicitly).

- [ ] **Step 2.10: Run all unit tests for `atlas-index` and `atlas-engine`**

```bash
cargo test -p atlas-index --lib
cargo test -p atlas-engine --lib
```

Expected: all pass, including new unit tests.

- [ ] **Step 2.11: Full workspace test suite**

```bash
cargo test --workspace --release --no-fail-fast
```

Expected: clean.

- [ ] **Step 2.12: Cumulative regression guard**

```bash
cargo build --release --workspace
cargo test -p atlas-cli --test phase3_polyglot_fixture --release --no-fail-fast
```

Expected: clean. Cold = ~26; warm + reports = 0. **Owner-follows is purely deterministic — it adds no LLM call sites.** If cold count drifts, investigate.

- [ ] **Step 2.13: Lints + fmt clean**

```bash
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

Expected: clean.

- [ ] **Step 2.14: Commit PR-2**

```bash
git add crates/atlas-index/src/rename_match.rs \
        crates/atlas-engine/src/l5_surface.rs \
        crates/atlas-engine/src/l6_edges.rs \
        crates/atlas-cli/tests/contract_rename_owner_follows.rs \
        crates/atlas-cli/tests/common/mod.rs
# (Include any other modified files surfaced during implementation.)
git commit -m "$(cat <<'EOF'
phase6: PR-2 contract rename-match owner-follows

When component rename-match maps `prior_id A → new_id B`,
contracts owned by A follow to B: surfaces.yaml contract ids
rewrite from `A/...` to `B/...`, and related-components.yaml
edge participants are updated to reference the new contract ids.

α implementation (id-embeds-owner; consistent with today's
owner-prefix id format). β (content-sha-stable) deferred to
Phase 10 fuzzy contract matching per LLM-spine recast spec §11.4.
Independent fuzzy contract matching also deferred to Phase 10.

Per `.claude/memory/project_phase6_paused_for_llm_spine` item #2.
Closes the §11.2.4 canonical-design open question.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

Expected: one commit.

- [ ] **Step 2.15: Flip PR-2 checkbox in status file**

Mark `- [x] PR-2 — Contract rename-match owner-follows (medium)` in `docs/superpowers/plans/2026-05-11-phase6-status.md`. Append per-PR notes with commit sha and verification gates. Commit the status flip as a separate commit (`phase6: PR-2 status checkbox + note`).

---

### Task 3: PR-3 — `subsystem` field overlay *(structural, medium)*

**Files:**
- Modify: `crates/atlas-engine/src/l4_tree.rs:272-325` (remove the `let _ = fo.subsystem.as_ref();` no-op; wire the field through)
- Modify: `crates/atlas-engine/src/l9_subsystems.rs:22-200` (extend `resolve_subsystems()` to accept per-component overrides as overlay; merge with central yaml; emit new warning)
- Create: `crates/atlas-cli/tests/subsystem_overlay.rs` (integration test for the overlay precedence + warning)

**Pre-flight constraint:** PR-3 commits to **per-component override wins over `subsystems.overrides.yaml`** as the precedence rule. The reasoning: §4.1 plain-text-canonical favours closer-to-source authoring; the per-component `<path>/.atlas/components.overrides.yaml` lives next to the component code while the central file sits at workspace root. If the user wants central-wins behaviour, they edit the central file directly (it explicitly takes precedence over LLM-discovery in Phase 7+; per-component vs central is a different axis). Document the precedence rule prominently in the commit message and the new test names.

**Warning class:** The new `SubsystemOverrideNonExistent` warning fires when `subsystems.overrides.yaml` lists a `members:` entry whose id resolves to no extant component. Per-component overrides cannot trigger this warning by construction (the override is co-located with the component, so the component must exist for the override file to be found).

- [ ] **Step 3.1: Read current state — confirm the no-op exists**

```bash
sed -n '300,330p' crates/atlas-engine/src/l4_tree.rs
```

Expected: line ~324 contains `let _ = fo.subsystem.as_ref();` with a comment about "no destination field on ComponentEntry yet."

- [ ] **Step 3.2: Read `l9_subsystems.rs` to understand resolution flow**

```bash
sed -n '1,200p' crates/atlas-engine/src/l9_subsystems.rs
```

Expected: view of `subsystems_yaml_snapshot()`, `resolve_subsystems()`, `resolve_one_subsystem()`, and related helpers. Note: today's resolution reads only `subsystems.yaml` + `subsystems.overrides.yaml`; per-component override surface doesn't exist.

- [ ] **Step 3.3: Write the failing integration test (TDD)**

Create `crates/atlas-cli/tests/subsystem_overlay.rs`:

```rust
//! Phase 6 PR-3: per-component `subsystem` field overlay.
//!
//! Per-component `<path>/.atlas/components.overrides.yaml` with a
//! `field_overrides.subsystem: ...` entry assigns the component to
//! the named subsystem, taking precedence over `subsystems.overrides.yaml`
//! at workspace root. This test exercises three cases:
//!
//! 1. Per-component override applied when central yaml is silent.
//! 2. Per-component override wins over central yaml entry (closer-to-source).
//! 3. Central yaml referencing a non-existent component emits the new
//!    `SubsystemOverrideNonExistent` warning.

use std::fs;
use tempfile::TempDir;

mod common;
use common::{run_atlas_index, read_subsystems_yaml};

#[test]
fn per_component_subsystem_override_applies_when_central_silent() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    fs::create_dir_all(root.join("crate-a/.atlas")).unwrap();
    fs::write(root.join("crate-a/Cargo.toml"), r#"[package]
name = "crate-a"
version = "0.1.0""#).unwrap();
    fs::write(root.join("crate-a/.atlas/components.overrides.yaml"), r#"
field_overrides:
  subsystem: alpha
"#).unwrap();
    // No central subsystems.overrides.yaml.

    run_atlas_index(root).unwrap();

    let subsystems = read_subsystems_yaml(&root.join(".atlas/cache/subsystems.yaml"));
    let alpha = subsystems.find_subsystem("alpha").expect("alpha subsystem");
    assert!(alpha.members.iter().any(|m| m == "crate-a"),
            "expected crate-a in subsystem alpha; got members={:?}", alpha.members);
}

#[test]
fn per_component_subsystem_override_wins_over_central_yaml() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    fs::create_dir_all(root.join("crate-b/.atlas")).unwrap();
    fs::create_dir_all(root.join(".atlas")).unwrap();
    fs::write(root.join("crate-b/Cargo.toml"), r#"[package]
name = "crate-b"
version = "0.1.0""#).unwrap();
    // Per-component says alpha.
    fs::write(root.join("crate-b/.atlas/components.overrides.yaml"), r#"
field_overrides:
  subsystem: alpha
"#).unwrap();
    // Central says beta — per-component should win.
    fs::write(root.join(".atlas/subsystems.overrides.yaml"), r#"
subsystems:
  - id: beta
    members:
      - crate-b
"#).unwrap();

    run_atlas_index(root).unwrap();

    let subsystems = read_subsystems_yaml(&root.join(".atlas/cache/subsystems.yaml"));
    let alpha = subsystems.find_subsystem("alpha").expect("alpha subsystem");
    assert!(alpha.members.iter().any(|m| m == "crate-b"),
            "expected crate-b in subsystem alpha (per-component wins); got alpha members={:?}", alpha.members);
    let beta = subsystems.find_subsystem("beta");
    if let Some(beta) = beta {
        assert!(!beta.members.iter().any(|m| m == "crate-b"),
                "expected crate-b NOT in subsystem beta; got beta members={:?}", beta.members);
    }
}

#[test]
fn central_yaml_referencing_nonexistent_component_emits_warning() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    fs::create_dir_all(root.join("crate-c")).unwrap();
    fs::create_dir_all(root.join(".atlas")).unwrap();
    fs::write(root.join("crate-c/Cargo.toml"), r#"[package]
name = "crate-c"
version = "0.1.0""#).unwrap();
    fs::write(root.join(".atlas/subsystems.overrides.yaml"), r#"
subsystems:
  - id: gamma
    members:
      - nonexistent-component
"#).unwrap();

    let output = run_atlas_index(root).unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("nonexistent-component"),
            "expected warning mentioning the missing component; got stderr: {}", stderr);
    assert!(stderr.contains("does not exist") || stderr.contains("not found") || stderr.contains("no extant"),
            "expected warning to indicate non-existence; got stderr: {}", stderr);
    assert_eq!(output.status.code(), Some(0),
               "permissive mode: warning should not fail the run (exit = 0); got {:?}", output.status.code());
}
```

The `run_atlas_index` helper should return a struct with `stderr`, `stdout`, and `status` fields (mirror existing patterns in `crates/atlas-cli/tests/phase3_overrides_edges.rs`).

- [ ] **Step 3.4: Run the new test — verify it fails**

```bash
cargo test -p atlas-cli --test subsystem_overlay --release --no-fail-fast
```

Expected: all three tests **FAIL**. Test 1 fails because the `subsystem` field is currently ignored. Test 2 fails for the same reason. Test 3 fails because the new warning class doesn't exist.

- [ ] **Step 3.5: Remove the no-op in `l4_tree.rs`**

In `crates/atlas-engine/src/l4_tree.rs:272-325`, replace the `let _ = fo.subsystem.as_ref();` no-op (line ~324) with attaching the override to the `ComponentEntry` via a new field or via a side-channel collector passed to L9. The simplest implementation: pass a `&mut BTreeMap<ComponentId, String>` (component → subsystem override) through the call chain, populated here:

```rust
if let Some(subsystem_name) = fo.subsystem.as_ref() {
    per_component_subsystem_overrides.insert(component_id.clone(), subsystem_name.clone());
}
```

This map is then passed to `l9_subsystems::resolve_subsystems()` as a new parameter.

- [ ] **Step 3.6: Extend `resolve_subsystems()` to accept the overlay**

In `crates/atlas-engine/src/l9_subsystems.rs`, extend the function signature:

```rust
pub fn resolve_subsystems(
    components: &[ComponentEntry],
    central_overrides: &SubsystemsOverridesFile,
    per_component_overrides: &BTreeMap<ComponentId, String>,
    warnings: &mut dyn Write,
) -> Vec<Subsystem> {
    let mut resolved: Vec<Subsystem> = Vec::new();

    // 1. Apply central overrides first (lower precedence).
    for entry in &central_overrides.subsystems {
        let members = resolve_one_subsystem(entry, components, warnings);
        if !members.is_empty() {
            resolved.push(Subsystem {
                id: entry.id.clone(),
                members,
            });
        }
    }

    // 2. Apply per-component overrides on top — per-component wins over
    // central per LLM-spine recast plan §1 PR-3 precedence rule.
    for (component_id, subsystem_name) in per_component_overrides {
        // Remove component_id from any other subsystem it was placed in by central.
        for subsystem in &mut resolved {
            subsystem.members.retain(|m| m != component_id);
        }
        // Add to the named subsystem (creating if needed).
        if let Some(subsystem) = resolved.iter_mut().find(|s| s.id == *subsystem_name) {
            if !subsystem.members.contains(component_id) {
                subsystem.members.push(component_id.clone());
            }
        } else {
            resolved.push(Subsystem {
                id: subsystem_name.clone(),
                members: vec![component_id.clone()],
            });
        }
    }

    // 3. Clean up empty subsystems left after step 2's retain().
    resolved.retain(|s| !s.members.is_empty());

    resolved
}
```

- [ ] **Step 3.7: Emit `SubsystemOverrideNonExistent` warning**

Inside `resolve_one_subsystem()` (or its sub-helper), when a `members:` entry lists an id that resolves to no extant component, emit the warning:

```rust
let _ = writeln!(
    warnings,
    "warning: subsystems.overrides.yaml references component `{}` in subsystem `{}` but no such component exists in the workspace — override entry skipped",
    member_id,
    subsystem_id
);
```

This warning's text contains the substring `does not exist` to match the test assertion in Step 3.3. Adjust the literal text to satisfy `stderr.contains("does not exist") || stderr.contains("not found") || stderr.contains("no extant")` from Test 3.

- [ ] **Step 3.8: Update all `resolve_subsystems` call sites**

```bash
grep -rn "resolve_subsystems" crates/atlas-engine/src/
```

Expected: at least one caller (likely `l9_subsystems::subsystems_yaml_snapshot()` and the relevant L9 Salsa query). Update each caller to pass the per-component overrides map (which it must obtain from L4's output).

- [ ] **Step 3.9: Re-run the integration tests — verify they pass**

```bash
cargo test -p atlas-cli --test subsystem_overlay --release --no-fail-fast
```

Expected: all three tests **PASS**.

- [ ] **Step 3.10: Run full unit-test suite for `atlas-engine`**

```bash
cargo test -p atlas-engine --lib
```

Expected: pass. The existing `unparseable_lifecycle_emits_warning_and_skips_override` test (in `l4_tree.rs:1582-1625`) still holds; the new overlay code does not regress lifecycle/language/kind override handling.

- [ ] **Step 3.11: Add unit test for `resolve_subsystems` overlay precedence**

In `crates/atlas-engine/src/l9_subsystems.rs`'s test module, add:

```rust
#[test]
fn per_component_subsystem_override_wins_over_central() {
    let components = vec![
        ComponentEntry { id: "comp-a".into(), /* ... */ },
    ];
    let central = SubsystemsOverridesFile {
        subsystems: vec![SubsystemEntry {
            id: "beta".into(),
            members: vec!["comp-a".into()],
        }],
    };
    let mut per_comp = BTreeMap::new();
    per_comp.insert("comp-a".into(), "alpha".to_string());

    let mut warnings: Vec<u8> = Vec::new();
    let resolved = resolve_subsystems(&components, &central, &per_comp, &mut warnings);

    let alpha = resolved.iter().find(|s| s.id == "alpha").expect("alpha");
    assert!(alpha.members.iter().any(|m| m == "comp-a"));
    let beta = resolved.iter().find(|s| s.id == "beta");
    assert!(beta.is_none() || !beta.unwrap().members.iter().any(|m| m == "comp-a"));
}

#[test]
fn central_referencing_nonexistent_component_emits_warning() {
    let components: Vec<ComponentEntry> = vec![];  // empty workspace
    let central = SubsystemsOverridesFile {
        subsystems: vec![SubsystemEntry {
            id: "gamma".into(),
            members: vec!["missing-comp".into()],
        }],
    };
    let per_comp = BTreeMap::new();

    let mut warnings: Vec<u8> = Vec::new();
    let _ = resolve_subsystems(&components, &central, &per_comp, &mut warnings);

    let warnings_str = String::from_utf8(warnings).unwrap();
    assert!(warnings_str.contains("missing-comp"));
    assert!(warnings_str.contains("does not exist"));
}
```

Adjust to match real type names and constructors.

- [ ] **Step 3.12: Full workspace test suite + lints + fmt**

```bash
cargo test --workspace --release --no-fail-fast
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

Expected: clean.

- [ ] **Step 3.13: Cumulative regression guard**

```bash
cargo build --release --workspace
cargo test -p atlas-cli --test phase3_polyglot_fixture --release --no-fail-fast
```

Expected: clean. Cold = ~26; warm + reports = 0. Subsystem overlay is purely deterministic; no LLM call sites added.

- [ ] **Step 3.14: Commit PR-3**

```bash
git add crates/atlas-engine/src/l4_tree.rs \
        crates/atlas-engine/src/l9_subsystems.rs \
        crates/atlas-cli/tests/subsystem_overlay.rs
# (Include any other modified files.)
git commit -m "$(cat <<'EOF'
phase6: PR-3 subsystem field overlay

Wires the parsed-but-ignored per-component `subsystem:` override
field through L9 subsystem resolution. Per-component overrides
(co-located in `<path>/.atlas/components.overrides.yaml`) win over
central `subsystems.overrides.yaml` at workspace root, per the
closer-to-source authoring discipline (LLM-spine recast spec §4.1).

Adds new warning class: `SubsystemOverrideNonExistent`, fired when
central yaml `members:` references a component that doesn't exist
in the workspace. Per-component overrides cannot trigger this
warning by construction (co-location implies existence).

Per `.claude/memory/project_phase6_paused_for_llm_spine` item #3.
Closes the Phase 3 PR-9 deferral noted in canonical §10.6.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

Expected: one commit.

- [ ] **Step 3.15: Flip PR-3 checkbox in status file** (separate commit; same pattern as PR-1/PR-2).

---

### Task 4: PR-4 — `--strict-overrides` + closed enumeration + dual-mode contract test *(structural, medium)*

**Files:**
- Create: `crates/atlas-engine/src/override_warnings.rs` (new module: `OverrideWarning` enum + `OverrideWarningCollector` trait)
- Modify: `crates/atlas-engine/src/lib.rs` (export the new module; add `strict_overrides: bool` to `IndexConfig`)
- Modify: `crates/atlas-cli/src/main.rs:66-132` (add `--strict-overrides` flag to `IndexArgs`)
- Modify: `crates/atlas-cli/src/pipeline.rs` (propagate flag from `IndexArgs` to `IndexConfig` and `run_index()`)
- Modify: `crates/atlas-engine/src/l6_edges.rs:244-248,305-308` (replace `eprintln!()` with `collector.emit(OverrideWarning::...)`)
- Modify: `crates/atlas-engine/src/l9_subsystems.rs` (replace the PR-3 warning emission with `collector.emit(OverrideWarning::SubsystemOverrideNonExistent { ... })`)
- Modify: `crates/atlas-cli/tests/phase3_overrides_edges.rs:353-392` (extend the `edges_suppress_no_match_leaves_set_unchanged` test to assert stderr text + exit code 0 in permissive mode)
- Create: `crates/atlas-cli/tests/strict_overrides_contract.rs` (dual-mode contract test covering all three closed-enum variants)

**Pre-flight constraint:** PR-4 depends on PR-3's `SubsystemOverrideNonExistent` warning emission existing. Confirm PR-3 is `[x]` in the status file before dispatching PR-4. The closed enum's variants are **fixed at three**: `EdgesSuppressNoMatch`, `EdgesAddUnknownKind`, `SubsystemOverrideNonExistent`. Future warning classes (Phase 7+ LLM-spine work) extend this enum; current scope is closed at three.

- [ ] **Step 4.1: Write the failing dual-mode contract test (TDD)**

Create `crates/atlas-cli/tests/strict_overrides_contract.rs`:

```rust
//! Phase 6 PR-4: dual-mode contract test for `--strict-overrides`.
//!
//! Exercises every variant of the closed `OverrideWarning` enumeration
//! in both modes:
//!   - Permissive (no `--strict-overrides`): warning text on stderr; exit 0.
//!   - Strict (`--strict-overrides`): warning text on stderr; exit non-zero.
//!
//! This subsumes the deferred Phase 3 PR-10 stderr-capture test for
//! `edges_suppress no-match` (now one of three variants).

use std::fs;
use std::process::Output;
use tempfile::TempDir;

mod common;
use common::{run_atlas_index, run_atlas_index_with_args};

fn fixture_with_edges_suppress_no_match(tmp: &TempDir) {
    let root = tmp.path();
    // ... build fixture that triggers EdgesSuppressNoMatch warning ...
    fs::create_dir_all(root.join("crate-a")).unwrap();
    fs::write(root.join("crate-a/Cargo.toml"), r#"[package]
name = "crate-a"
version = "0.1.0""#).unwrap();
    fs::create_dir_all(root.join(".atlas")).unwrap();
    fs::write(root.join(".atlas/related-components.overrides.yaml"), r#"
edges_suppress:
  - kind: depends-on
    from: crate-a
    to: nonexistent-crate
"#).unwrap();
}

fn fixture_with_edges_add_unknown_kind(tmp: &TempDir) {
    let root = tmp.path();
    fs::create_dir_all(root.join("crate-a")).unwrap();
    fs::write(root.join("crate-a/Cargo.toml"), r#"[package]
name = "crate-a"
version = "0.1.0""#).unwrap();
    fs::create_dir_all(root.join(".atlas")).unwrap();
    fs::write(root.join(".atlas/related-components.overrides.yaml"), r#"
edges_add:
  - kind: bogus-not-a-real-kind
    from: crate-a
    to: crate-a
"#).unwrap();
}

fn fixture_with_subsystem_override_nonexistent(tmp: &TempDir) {
    let root = tmp.path();
    fs::create_dir_all(root.join("crate-a")).unwrap();
    fs::write(root.join("crate-a/Cargo.toml"), r#"[package]
name = "crate-a"
version = "0.1.0""#).unwrap();
    fs::create_dir_all(root.join(".atlas")).unwrap();
    fs::write(root.join(".atlas/subsystems.overrides.yaml"), r#"
subsystems:
  - id: gamma
    members:
      - nonexistent-component
"#).unwrap();
}

#[test]
fn edges_suppress_no_match_permissive_emits_warning_exits_zero() {
    let tmp = TempDir::new().unwrap();
    fixture_with_edges_suppress_no_match(&tmp);
    let output: Output = run_atlas_index(tmp.path()).unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("edges_suppress") || stderr.contains("no match"));
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn edges_suppress_no_match_strict_emits_warning_exits_nonzero() {
    let tmp = TempDir::new().unwrap();
    fixture_with_edges_suppress_no_match(&tmp);
    let output: Output = run_atlas_index_with_args(tmp.path(), &["--strict-overrides"]).unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("edges_suppress") || stderr.contains("no match"));
    assert_ne!(output.status.code(), Some(0));
}

#[test]
fn edges_add_unknown_kind_permissive_emits_warning_exits_zero() {
    let tmp = TempDir::new().unwrap();
    fixture_with_edges_add_unknown_kind(&tmp);
    let output = run_atlas_index(tmp.path()).unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("bogus-not-a-real-kind") || stderr.contains("unknown kind"));
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn edges_add_unknown_kind_strict_emits_warning_exits_nonzero() {
    let tmp = TempDir::new().unwrap();
    fixture_with_edges_add_unknown_kind(&tmp);
    let output = run_atlas_index_with_args(tmp.path(), &["--strict-overrides"]).unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("bogus-not-a-real-kind") || stderr.contains("unknown kind"));
    assert_ne!(output.status.code(), Some(0));
}

#[test]
fn subsystem_override_nonexistent_permissive_emits_warning_exits_zero() {
    let tmp = TempDir::new().unwrap();
    fixture_with_subsystem_override_nonexistent(&tmp);
    let output = run_atlas_index(tmp.path()).unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("nonexistent-component"));
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn subsystem_override_nonexistent_strict_emits_warning_exits_nonzero() {
    let tmp = TempDir::new().unwrap();
    fixture_with_subsystem_override_nonexistent(&tmp);
    let output = run_atlas_index_with_args(tmp.path(), &["--strict-overrides"]).unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("nonexistent-component"));
    assert_ne!(output.status.code(), Some(0));
}
```

Extend `common::mod.rs` with `run_atlas_index_with_args` if it doesn't already exist (the existing `run_atlas_index` mirror gets a variant that takes additional CLI args).

- [ ] **Step 4.2: Run the test — verify it fails**

```bash
cargo test -p atlas-cli --test strict_overrides_contract --release --no-fail-fast
```

Expected: most tests **FAIL** (the `--strict-overrides` flag doesn't exist yet; warnings don't escalate; the second test in each pair will report exit 0). The permissive-mode tests *may* pass if the existing `eprintln!()` text matches the substring asserts — that's fine.

- [ ] **Step 4.3: Create the `override_warnings` module**

Create `crates/atlas-engine/src/override_warnings.rs`:

```rust
//! Phase 6 PR-4: closed-enumeration override warnings.
//!
//! Today (Phase 6), the closed enumeration carries exactly three
//! variants. Future LLM-spine work (Phase 7+) extends this enum; current
//! scope is closed at these three so `--strict-overrides` has a stable
//! contract.

use std::io::Write;

/// Closed enumeration of override warnings escalated to errors when
/// `--strict-overrides` is set. Adding a new variant is a breaking
/// change to the strict-mode contract; do so deliberately, with
/// matching test coverage in `crates/atlas-cli/tests/strict_overrides_contract.rs`.
#[derive(Debug, Clone)]
pub enum OverrideWarning {
    /// `related-components.overrides.yaml` `edges_suppress` directive
    /// did not match any actual edge.
    EdgesSuppressNoMatch {
        directive: String,
        scope: String,
    },
    /// `related-components.overrides.yaml` `edges_add` entry references
    /// a kind not in the registered ontology.
    EdgesAddUnknownKind {
        kind: String,
        scope: String,
    },
    /// `subsystems.overrides.yaml` lists a `members:` entry referencing
    /// a component that does not exist in the workspace.
    SubsystemOverrideNonExistent {
        name: String,
        scope: String,
    },
}

impl OverrideWarning {
    pub fn human_message(&self) -> String {
        match self {
            OverrideWarning::EdgesSuppressNoMatch { directive, scope } => format!(
                "warning: edges_suppress directive `{}` in {} matched no edges — override has no effect",
                directive, scope
            ),
            OverrideWarning::EdgesAddUnknownKind { kind, scope } => format!(
                "warning: edges_add entry references unknown edge kind `{}` in {} — entry not applied",
                kind, scope
            ),
            OverrideWarning::SubsystemOverrideNonExistent { name, scope } => format!(
                "warning: subsystems.overrides.yaml references component `{}` in {} but no such component exists in the workspace — override entry does not exist (skipped)",
                name, scope
            ),
        }
    }
}

/// Collects override warnings. `permissive` writes to stderr and continues;
/// `strict` writes to stderr *and* sets a sticky `has_errors` flag that the
/// CLI checks before deciding the exit code.
pub trait OverrideWarningCollector: Send + Sync {
    fn emit(&self, warning: OverrideWarning);
    fn has_errors(&self) -> bool;
}

/// Default permissive collector — writes to stderr; never sets errors.
pub struct PermissiveCollector;

impl OverrideWarningCollector for PermissiveCollector {
    fn emit(&self, warning: OverrideWarning) {
        eprintln!("{}", warning.human_message());
    }
    fn has_errors(&self) -> bool { false }
}

/// Strict collector — writes to stderr; sets `has_errors` on first emit.
pub struct StrictCollector {
    has_errors: std::sync::atomic::AtomicBool,
}

impl StrictCollector {
    pub fn new() -> Self {
        Self { has_errors: std::sync::atomic::AtomicBool::new(false) }
    }
}

impl OverrideWarningCollector for StrictCollector {
    fn emit(&self, warning: OverrideWarning) {
        eprintln!("{}", warning.human_message());
        self.has_errors.store(true, std::sync::atomic::Ordering::SeqCst);
    }
    fn has_errors(&self) -> bool {
        self.has_errors.load(std::sync::atomic::Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permissive_collector_never_has_errors() {
        let c = PermissiveCollector;
        c.emit(OverrideWarning::EdgesSuppressNoMatch {
            directive: "x".into(),
            scope: "y".into(),
        });
        assert!(!c.has_errors());
    }

    #[test]
    fn strict_collector_sets_errors_on_emit() {
        let c = StrictCollector::new();
        assert!(!c.has_errors());
        c.emit(OverrideWarning::EdgesAddUnknownKind {
            kind: "bogus".into(),
            scope: "y".into(),
        });
        assert!(c.has_errors());
    }
}
```

The `_ = Write;` import is kept in case future warning variants want to render into a buffer (e.g. for the test assertions); for now the collectors write directly to stderr.

- [ ] **Step 4.4: Wire the module into `atlas-engine`**

In `crates/atlas-engine/src/lib.rs`, add:

```rust
pub mod override_warnings;
pub use override_warnings::{OverrideWarning, OverrideWarningCollector, PermissiveCollector, StrictCollector};
```

Also extend the existing `IndexConfig` struct (or equivalent) with:

```rust
pub strict_overrides: bool,
```

The default is `false` (permissive).

- [ ] **Step 4.5: Add the CLI flag**

In `crates/atlas-cli/src/main.rs:66-132`, add to `IndexArgs`:

```rust
/// Escalate override warnings (edges_suppress no-match, edges_add
/// unknown-kind, subsystems.overrides.yaml non-existent member) to
/// errors. Sets a non-zero exit code if any warning fires. The closed
/// list of warnings is defined in
/// `crates/atlas-engine/src/override_warnings.rs::OverrideWarning`.
#[arg(long)]
strict_overrides: bool,
```

Mirror the flag declaration style of existing booleans (e.g. `--no-overrides`, `--dry-run`).

- [ ] **Step 4.6: Propagate the flag through `pipeline.rs`**

In `crates/atlas-cli/src/pipeline.rs`, when building `IndexConfig` from `IndexArgs`, set:

```rust
strict_overrides: args.strict_overrides,
```

When invoking the engine, also instantiate the collector:

```rust
let collector: Box<dyn atlas_engine::OverrideWarningCollector> = if config.strict_overrides {
    Box::new(atlas_engine::StrictCollector::new())
} else {
    Box::new(atlas_engine::PermissiveCollector)
};
```

Pass `collector` through to `run_index()`. After the engine returns, check `collector.has_errors()` and exit non-zero if true:

```rust
let result = run_index(/* … */, collector.as_ref()).await?;
if collector.has_errors() {
    eprintln!("error: --strict-overrides set; override warnings escalated to errors. See above.");
    std::process::exit(1);
}
```

- [ ] **Step 4.7: Replace `eprintln!()` warning sites in `l6_edges.rs`**

At `crates/atlas-engine/src/l6_edges.rs:244-248` (the `edges_add` unknown-kind warning), replace:

```rust
eprintln!("...");  // old text
```

with:

```rust
collector.emit(OverrideWarning::EdgesAddUnknownKind {
    kind: kind.to_string(),
    scope: scope_descriptor.to_string(),
});
```

At `crates/atlas-engine/src/l6_edges.rs:305-308` (the `edges_suppress` no-match warning), apply the analogous replacement with `OverrideWarning::EdgesSuppressNoMatch`.

Plumb `collector: &dyn OverrideWarningCollector` through the call chain (function signatures up to the L6 entry point may need updating).

- [ ] **Step 4.8: Replace the PR-3 warning emission in `l9_subsystems.rs`**

In `crates/atlas-engine/src/l9_subsystems.rs`, replace the `writeln!(warnings, "warning: subsystems.overrides.yaml references component ...")` (added in PR-3) with:

```rust
collector.emit(OverrideWarning::SubsystemOverrideNonExistent {
    name: member_id.to_string(),
    scope: format!("subsystems.overrides.yaml subsystem `{}`", subsystem_id),
});
```

Plumb `collector` through `resolve_subsystems()` signature (the previous `&mut dyn Write` parameter is replaced by `&dyn OverrideWarningCollector`).

- [ ] **Step 4.9: Update the existing test in `phase3_overrides_edges.rs`**

In `crates/atlas-cli/tests/phase3_overrides_edges.rs:353-392`, replace the existing `edges_suppress_no_match_leaves_set_unchanged` test's "no stderr assertion" comment with concrete assertions:

```rust
let output = run_atlas_index(tmp.path()).unwrap();
let stderr = String::from_utf8_lossy(&output.stderr);
assert!(stderr.contains("edges_suppress"));
assert!(stderr.contains("matched no edges") || stderr.contains("no match"));
assert_eq!(output.status.code(), Some(0));  // permissive: exit 0
// ... existing assertion that the edge set is unchanged ...
```

This subsumes the deferred Phase 3 PR-10 work explicitly.

- [ ] **Step 4.10: Run the dual-mode contract test — verify it passes**

```bash
cargo test -p atlas-cli --test strict_overrides_contract --release --no-fail-fast
```

Expected: all 6 tests **PASS**.

- [ ] **Step 4.11: Run the updated `phase3_overrides_edges.rs` test**

```bash
cargo test -p atlas-cli --test phase3_overrides_edges --release --no-fail-fast
```

Expected: all tests pass, including the now-asserting `edges_suppress_no_match_leaves_set_unchanged`.

- [ ] **Step 4.12: Run unit tests for `override_warnings.rs`**

```bash
cargo test -p atlas-engine --lib override_warnings
```

Expected: both unit tests pass.

- [ ] **Step 4.13: Full workspace test + lints + fmt + polyglot smoke**

```bash
cargo test --workspace --release --no-fail-fast
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo build --release --workspace
cargo test -p atlas-cli --test phase3_polyglot_fixture --release --no-fail-fast
```

Expected: all clean. Cold = ~26; warm + reports = 0.

- [ ] **Step 4.14: Commit PR-4**

```bash
git add crates/atlas-engine/src/override_warnings.rs \
        crates/atlas-engine/src/lib.rs \
        crates/atlas-engine/src/l6_edges.rs \
        crates/atlas-engine/src/l9_subsystems.rs \
        crates/atlas-cli/src/main.rs \
        crates/atlas-cli/src/pipeline.rs \
        crates/atlas-cli/tests/phase3_overrides_edges.rs \
        crates/atlas-cli/tests/strict_overrides_contract.rs
git commit -m "$(cat <<'EOF'
phase6: PR-4 --strict-overrides + closed enum + dual-mode test

Adds `--strict-overrides` CLI flag (clap derive on IndexArgs) that
escalates a closed enumeration of override warnings to errors with
non-zero exit code. The closed list contains exactly three variants:

  - EdgesSuppressNoMatch (was eprintln! in l6_edges.rs:305-308)
  - EdgesAddUnknownKind  (was eprintln! in l6_edges.rs:244-248)
  - SubsystemOverrideNonExistent (new in PR-3)

Pre-existing TreeAssemblyError::PerComponentScopeViolation hard
errors are unaffected; the flag escalates only soft warnings.

New dual-mode contract test
(crates/atlas-cli/tests/strict_overrides_contract.rs) exercises
every variant in both modes (permissive: exit 0; strict: exit
non-zero). Subsumes the deferred Phase 3 PR-10 stderr-capture test.

Per `.claude/memory/project_phase6_paused_for_llm_spine` item #4
+ item #5 (Phase 3 PR-10 deferral folded in here).

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

Expected: one commit.

- [ ] **Step 4.15: Flip PR-4 checkbox in status file** (separate commit; same pattern).

---

### Task 5: PR-5 — Acceptance + closeout + canonical §10/§4.3/§7/§8 retext *(docs + verification)*

**Files:**
- Modify: `docs/superpowers/specs/2026-05-06-atlas-system-model-design.md` (§4.3 replacement, §7.1+§7.3 retirement, §8.1 extension note, §10.6 SHIPPED, §10.7–§10.11 new rows)
- Modify: `docs/superpowers/plans/2026-05-11-phase6-status.md` (closeout note + Upgrade notes if any)
- Modify: `.claude/memory/project_phase4_plus_roadmap.md` (mark Phase 6 SHIPPED; advance Phase 7 to next-up)
- Modify: `.claude/memory/project_phase6_paused_for_llm_spine.md` (note: superseded; Phase 6 SHIPPED)
- Modify: `.claude/memory/MEMORY.md` (update entries)

**Pre-flight constraint:** Every Phase 6 PR (PR-1 through PR-4) must be `[x]` in the status file before PR-5 begins. PR-5 is the final acceptance gate plus the canonical-design retext that formalises the LLM-spine recast in the architectural-canon document.

- [ ] **Step 5.1: Verify all prior PRs landed**

```bash
grep -E "^\- \[x\] PR-" docs/superpowers/plans/2026-05-11-phase6-status.md
```

Expected: PR-0 through PR-4 all show `[x]`. If any are `[ ]` or `[~]`, **STOP** — surface the gap before proceeding.

- [ ] **Step 5.2: Final pre-retext verification suite**

```bash
cargo build --workspace
cargo test --workspace --release --no-fail-fast
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo build --release --workspace
cargo test -p atlas-cli --test phase3_polyglot_fixture --release --no-fail-fast
```

Expected: all clean. Cold = ~26; warm + reports = 0.

- [ ] **Step 5.3: Retext canonical §4.3**

In `docs/superpowers/specs/2026-05-06-atlas-system-model-design.md`, locate §4.3 ("Determinism over fuzziness"; today's text describes deterministic-cheap first, LLM as fallback). Replace the entire subsection body with the §3.1 retext from the recast spec:

```markdown
### 4.3 LLM is the spine; deterministic code is the scaffolding

Atlas's analytical work is performed by an LLM agent runtime over a tree of per-stage tasks. Deterministic Rust code is reserved for tasks that are *genuinely* deterministic — parsing structured manifests, walking filesystem trees, computing content shas, validating schemas, replaying cached transcripts, and supporting the agent runtime itself. Each deterministic component must justify *why it is deterministic*; "easier to code than to prompt" is not sufficient justification.

The Phase 6 → Phase 7 boundary is the inversion moment in the codebase. Phase 6 ships as the final deterministic-spine release; Phase 7 ships the LLM-spine runtime; subsequent phases retire language-specific deterministic analysers in waves. See `docs/superpowers/specs/2026-05-11-atlas-llm-spine-recast-design.md` for the architectural detail behind this inversion.
```

- [ ] **Step 5.4: Mark §7.1 retired**

Locate §7.1 ("Analyser interface (Rust trait, in-process)"). Add a leading note:

```markdown
### 7.1 Analyser interface (Rust trait, in-process)

> **RETIRED Phase 7.** The `Analyzer` trait is superseded by the `Tool` trait defined in `docs/superpowers/specs/2026-05-11-atlas-llm-spine-recast-design.md` §5.1. The text below is retained as historical context for v1 / Phase-1-through-Phase-6 deterministic-spine behaviour; new analytical work uses the agent runtime described in the recast spec.

[existing §7.1 text retained verbatim below this note]
```

- [ ] **Step 5.5: Mark §7.3 retired**

Locate §7.3 ("Cost classes and dispatch"). Add an analogous leading note:

```markdown
### 7.3 Cost classes and dispatch

> **RETIRED Phase 7.** Cost-class dispatch (`deterministic-cheap < deterministic-expensive < llm-cheap < llm-expensive`) is replaced by LLM-agent dispatch per recast spec §4.2. The text below is retained as historical context for the deterministic-spine era.

[existing §7.3 text retained verbatim below this note]
```

- [ ] **Step 5.6: Extend §8.1 fingerprint table**

Locate §8.1 ("The fingerprint discipline"; contains the table mapping stage to fingerprint inputs). Append a forward-pointer note after the table:

```markdown
**Phase 7 extension.** When the LLM-spine agent runtime lands (see `docs/superpowers/specs/2026-05-11-atlas-llm-spine-recast-design.md` §6.1), the fingerprint discriminator for L3 / L5 / L6 / L8 stages extends with `iteration_number` (for fixed-point iteration) and `prior_model_sha` (so each iteration of the agent tree caches separately). The existing inputs in the table above remain canonical; the iteration extension is additive.
```

- [ ] **Step 5.7: Retext §10 phasing table**

Locate §10 ("Phasing and migration"). Update §10.6 (Phase 6) to mark it SHIPPED, with the commit SHA placeholder for backfill:

```markdown
### 10.6 Phase 6 — User-facing schema cleanups

**SHIPPED 2026-05-NN.** Final deterministic-spine release before the LLM-spine recast begins in Phase 7. Five PRs (PR-0 plan + PR-1 manifest extension + PR-2 contract rename-match owner-follows + PR-3 subsystem field overlay + PR-4 --strict-overrides + closed enum + dual-mode contract test + PR-5 acceptance + closeout + this retext). Closes the §11.2.4 contract-rename-match canonical-design open question (α id-embeds-owner implementation; β content-sha-stable deferred to Phase 10). Companion design + plan: `docs/superpowers/specs/2026-05-11-atlas-vnext-phase6-plan.md`. Final commit: `<PR-5-COMMIT-SHA>`.
```

Then **add** new subsections §10.7 through §10.11 *after* the existing §10.6. Use the table content from recast spec §13.1:

```markdown
### 10.7 Phase 7 — LLM-spine runtime

LLM-spine runtime: agent runtime, toolbox, transcript cache, event bus, TUI, fixed-point iteration loop, audit lane. No language retirements; existing deterministic classifiers wrap as `Tool` implementations the agent invokes. Calibrates the cache primitive against known-good reference behaviour and ships the live TUI progress UX. See `docs/superpowers/specs/2026-05-11-atlas-llm-spine-recast-design.md` §11.1.

### 10.8 Phase 8 — Cargo retirement

First language LLM-driven: retires `cargo_classifier.rs` and Cargo-specific surface analysis in favour of LLM agents driving the toolbox. Calibrates the cold-token budget for the polyglot smoke test (locks in empirical per-language numbers; warm = 0 invariant unchanged). See recast spec §11.2.

### 10.9 Phase 9 — Remaining language retirements (waves)

Retires the remaining 9 hand-coded language classifiers in three waves: 9a (TS/JS + Python), 9b (C# + Dart), 9c (Elixir + Racket + LispKit + Compose + Dockerfile + the deferred Make/shell classifier from Phase 6's pre-pivot brainstorm). Each wave is its own phase with its own PRs, budget assertions, and reference-output comparisons. Mature-language surface analyser code (Rust + TS/JS) collapses to text-scoping `Tool` implementations; weak-tooling languages get no text-scoping helpers — agents read whole files. See recast spec §11.3.

### 10.10 Phase 10 — LLM-driven analyses

Pattern detection (recurring component / edge shapes; anti-patterns) as a new L8 agent stage; fuzzy contract matching (deferred from Phase 6 pre-pivot brainstorm) extends contract rename-match with semantic similarity beyond owner-follows / content-sha-stability; qualitative LLM-driven augmentation to existing Phase 3 modularity reports; LLM confidence threshold calibration. **Moved earlier** than today's §10.9 placement, since the agent runtime makes these analyses natural once it exists. See recast spec §11.4.

### 10.11 Phase 11 — Server mode

Long-running service with reactive recomputation, query API, file watcher, Salsa input updates, gRPC / HTTP+GraphQL, subscriptions, lifecycle, GC. Also ships the **web-app subscriber** to the agent runtime's event bus (the server already runs the bus across process boundaries; the web app subscribes via WebSocket / SSE). See recast spec §11.5.
```

If the existing canonical design has §10.7, §10.8, etc. with different content (per-language refinements, subprocess convergence, LLM-driven analyses, server mode), **replace** them with the new content above. The §10.11 "Migration from v1" subsection (which is OBSOLETE today) is unchanged.

- [ ] **Step 5.8: Sanity check the canonical-design retext compiles as Markdown**

```bash
# Visually inspect:
sed -n '/^### 4.3 /,/^## /p' docs/superpowers/specs/2026-05-06-atlas-system-model-design.md
sed -n '/^### 7.1 /,/^### 7.2 /p' docs/superpowers/specs/2026-05-06-atlas-system-model-design.md
sed -n '/^### 7.3 /,/^## /p' docs/superpowers/specs/2026-05-06-atlas-system-model-design.md
sed -n '/^### 8.1 /,/^### 8.2 /p' docs/superpowers/specs/2026-05-06-atlas-system-model-design.md
sed -n '/^### 10.6 /,/^## /p' docs/superpowers/specs/2026-05-06-atlas-system-model-design.md
```

Expected: each retext section reads cleanly. Cross-references to the recast spec are correct. Section numbering is monotonic.

- [ ] **Step 5.9: Update memory — mark Phase 6 SHIPPED**

Read `.claude/memory/project_phase4_plus_roadmap.md`. Find the Phase 6 entry; mark SHIPPED with the date. Advance the "next-up" pointer to Phase 7 (LLM-spine runtime per canonical §10.7 / recast spec §11.1).

Read `.claude/memory/project_phase6_paused_for_llm_spine.md`. Add a note at the top: "**SUPERSEDED 2026-05-NN.** The four candidate items shipped as Phase 6 per `docs/superpowers/specs/2026-05-11-atlas-vnext-phase6-plan.md`. The LLM-spine recast begins in Phase 7 per `docs/superpowers/specs/2026-05-11-atlas-llm-spine-recast-design.md`. This memory retained for forensic context."

Update `.claude/memory/MEMORY.md` index entries to reflect the new state.

- [ ] **Step 5.10: Audit greps**

```bash
git grep -nE 'TODO.*phase6|XXX.*phase6|FIXME.*phase6' crates/ docs/
git grep -nE 'parsed but ignored|parsed-but-ignored' crates/
git grep -n 'eprintln!.*edges_suppress\|eprintln!.*edges_add' crates/atlas-engine/src/
```

Expected: all three greps return zero hits. (The third confirms PR-4's `eprintln!()` replacements are complete; any surviving hit is an unmigrated warning site.)

- [ ] **Step 5.11: Final cumulative regression guard**

```bash
cargo build --release --workspace
cargo test -p atlas-cli --test phase3_polyglot_fixture --release --no-fail-fast
```

Expected: clean. Cold = ~26; warm + reports = 0.

- [ ] **Step 5.12: Append closeout note to status file**

In `docs/superpowers/plans/2026-05-11-phase6-status.md`, append after the existing per-PR notes:

```markdown
---

## Phase 6 — complete

2026-05-NN. All six PRs merged to main. Cumulative LOC delta:
- PR-0: docs only (plan + status + continuation prompt).
- PR-1: +~30 LOC production (manifest extension + tests).
- PR-2: +~200 LOC production (contract owner-follows rewrite seam + integration test).
- PR-3: +~150 LOC production (subsystem overlay + warning + integration test).
- PR-4: +~250 LOC production (override_warnings.rs + CLI flag + dual-mode contract test).
- PR-5: docs only (canonical design retext) + memory updates.

Final commits (sha → title):
- PR-0: `<sha>` (plan + status + continuation prompt)
- PR-1: `<sha>` (manifest extension)
- PR-2: `<sha>` (contract owner-follows)
- PR-3: `<sha>` (subsystem overlay)
- PR-4: `<sha>` (--strict-overrides + closed enum + dual-mode test)
- PR-5: `<sha>` (closeout + canonical retext)

### Phase 6 → Phase 7 handoff

Phase 6 is the **final deterministic-spine release**. Phase 7 begins the LLM-spine recast per `docs/superpowers/specs/2026-05-11-atlas-llm-spine-recast-design.md`. The recast spec captures architectural intent but per-PR scope still needs design — run `superpowers:brainstorming` for Phase 7 before plan-writing.

The user-authoring override discipline strengthened in PR-3 + PR-4 (per-component overlay precedence, `--strict-overrides` flag, closed warning enumeration) is *load-bearing* for Phase 7: the LLM-decided dispatch decisions land as YAML artefacts under user-overridable overlays per recast spec §4.2.
```

- [ ] **Step 5.13: Commit PR-5 — canonical retext + closeout**

```bash
git add docs/superpowers/specs/2026-05-06-atlas-system-model-design.md \
        docs/superpowers/plans/2026-05-11-phase6-status.md \
        .claude/memory/project_phase4_plus_roadmap.md \
        .claude/memory/project_phase6_paused_for_llm_spine.md \
        .claude/memory/MEMORY.md
git commit -m "$(cat <<'EOF'
phase6: PR-5 acceptance + closeout + canonical retext

Final acceptance gate for Phase 6 (the final deterministic-spine
release before the LLM-spine recast). Retexts the canonical
system-model design per recast spec §13:

  - §4.3 inverted (LLM is the spine; deterministic is scaffolding)
  - §7.1 Analyser interface marked RETIRED Phase 7 (forward-pointer
    to recast spec §5.1 Tool trait)
  - §7.3 cost classes marked RETIRED Phase 7
  - §8.1 fingerprint table extended with forward-pointer to Phase 7
    iteration_number + prior_model_sha discriminators
  - §10.6 Phase 6 marked SHIPPED
  - §10.7–§10.11 new entries for Phases 7–11 (LLM-spine runtime,
    Cargo retirement, language retirement waves, LLM-driven
    analyses moved earlier, server mode)

Memory updates: Phase 6 SHIPPED in project_phase4_plus_roadmap;
project_phase6_paused_for_llm_spine marked SUPERSEDED.

Cumulative regression guard: cold = ~26 LLM calls (Phase 2 PR-14
baseline); warm + reports = 0. All cargo gates clean.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

Expected: one commit.

- [ ] **Step 5.14: Backfill PR-5's commit SHA into the canonical design**

```bash
PR5_SHA=$(git rev-parse HEAD)
# In docs/superpowers/specs/2026-05-06-atlas-system-model-design.md §10.6,
# replace `<PR-5-COMMIT-SHA>` with the actual SHA above.
```

Use `sed -i ''` (macOS) or `sed -i` (Linux) to do the substitution, then commit:

```bash
git add docs/superpowers/specs/2026-05-06-atlas-system-model-design.md
git commit -m "phase6: PR-5 backfill commit sha in canonical §10.6

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

Expected: one final commit completing Phase 6.

- [ ] **Step 5.15: Flip PR-5 checkbox in status file** (one final separate commit; same pattern). Mark Phase 6 complete.

---

## 5. Acceptance summary

| PR | Acceptance gate |
|----|----------------|
| PR-0 | Plan + status + continuation prompt files exist; `cargo build --workspace` clean. |
| ~~PR-1~~ | **DEFERRED to Phase 9c.** Documented in §1 above; no acceptance gate in Phase 6. |
| PR-2 | New integration test + 2 unit tests pass; polyglot smoke cold count unchanged; existing tests not regressed. |
| PR-3 | New integration test (3 cases) + 2 unit tests pass; polyglot smoke clean; existing override tests not regressed. |
| PR-4 | New dual-mode contract test (6 cases) + 2 unit tests for collectors pass; existing `phase3_overrides_edges` test updated with stderr+exit assertions and passing; polyglot smoke clean. |
| PR-5 | PR-2, PR-3, PR-4 checkboxes `[x]` (PR-1 deferred); canonical-design retext applied per recast spec §13; audit greps clean; memory updates landed; polyglot smoke cold = ~26 / warm + reports = 0. PR-5 §10.6 narrative records PR-1's deferral to Phase 9c. |

End-of-phase acceptance: PR-0, PR-2, PR-3, PR-4, PR-5 all `[x]`; PR-1 deferred to Phase 9c with note in §10.6; cumulative regression guard cold count unchanged; canonical design retext in place; Phase 7 surfaced as next-up in memory.

---

## 6. Out-of-scope reminders

Explicitly **not** in Phase 6:

- Independent fuzzy contract matching (a contract's owner did not rename but the contract content moved or split). Deferred to Phase 10 per recast spec §11.4.
- **Make / shell manifest recognition** (was PR-1 of Phase 6). **Deferred to Phase 9c** 2026-05-11 after polyglot-fixture pre-flight found `build_glue/Makefile` + `scripts/deploy.sh` already surfaced via `additions:`; recognition-only would have broken the cumulative regression guard. Recognition + paired classifier ship together in Phase 9c per recast spec §11.3.
- Cache compression. Deferred to its own cache-architecture phase post-Phase-11.
- Worktree commit-sha annotations. Dropped during pre-pivot brainstorm (Phase 5's multi-root collapse removed the motivating use case).
- Any LLM-spine runtime work. Begins in Phase 7.

---

## 7. References

- `docs/superpowers/specs/2026-05-11-atlas-llm-spine-recast-design.md` — the design anchor for Phase 6's PR-5 retext content.
- `docs/superpowers/specs/2026-05-10-atlas-vnext-phase5-plan.md` — prior-phase plan structure this plan follows.
- `docs/superpowers/plans/2026-05-10-phase5-status.md` — status-file structure PR-0 reproduces.
- `docs/superpowers/prompts/2026-05-10-vnext-continue.md` — continuation-prompt template Phase 6's version copy-edits.
- `docs/superpowers/specs/2026-05-06-atlas-system-model-design.md` — canonical system-model design (the document PR-5 retexts).
- `docs/superpowers/specs/2026-05-08-atlas-vnext-phase3-design.md` — Phase 3 design (whose PR-10 deferral PR-4 closes).
- `.claude/memory/project_phase6_paused_for_llm_spine.md` — captures the four pre-pivot candidate items operationalised here.
- `.claude/memory/feedback_atlas_llm_spine_intent.md` — strategic preference; Phase 6 ships before this inversion begins.
- `crates/atlas-engine/src/manifest_patterns.rs` — PR-1 surface.
- `crates/atlas-index/src/rename_match.rs` — PR-2 surface.
- `crates/atlas-engine/src/l4_tree.rs`, `crates/atlas-engine/src/l9_subsystems.rs` — PR-3 surface.
- `crates/atlas-cli/src/main.rs`, `crates/atlas-cli/src/pipeline.rs`, `crates/atlas-engine/src/l6_edges.rs` — PR-4 surface.
