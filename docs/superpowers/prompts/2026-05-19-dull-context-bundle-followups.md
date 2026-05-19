# dull-context bundle follow-ups — kickoff prompt

Use this prompt to open the **dull-context follow-up session**. The work is in `/Users/antony/Development/dull-context` (NOT Atlas — this prompt lives in Atlas's `prompts/` directory by convention, but every action below targets the dull-context repo). The `bundle --component <id>` feature shipped at `61f4f3b` and surfaced three discrete follow-ups, two of which are filed as bullets in the plan's `## Follow-up notes from implementation` section. The third is a cross-task design observation from the final code review.

---

## Invocation

Hand the body below to a fresh session. The session works through three follow-ups sequentially. Each is small enough that a full brainstorm-plan-execute cycle is overkill — but the design forks in Items 2 and 3 each warrant one `AskUserQuestion` round to lock the approach before code. Item 1 is mechanical and needs no question.

## Body

Working repo: `/Users/antony/Development/dull-context`. Branch: `main` (currently at `61f4f3b`, clean tree, no remote configured — **DO NOT push**, the user adds the remote later). Stack: TypeScript ESM, Node 20+, Commander, Zod, vitest, pnpm, `tsconfig.exactOptionalPropertyTypes: true`. The plan with follow-up notes is `docs/superpowers/plans/2026-05-19-bundle-component.md`; the spec is `docs/superpowers/specs/2026-05-19-bundle-component-design.md`.

### Reading order

1. `docs/superpowers/plans/2026-05-19-bundle-component.md` — skim the **File Structure** + **Per-task quality gates** sections, then read the `## Follow-up notes from implementation` section at the very bottom in full. Bullets 1 + 2 are Items 1 + 2 of this session.
2. `git log --oneline c5c30ed..HEAD` to see the nine commits the `bundle --component` work landed (`c24076d` → `61f4f3b`). The two follow-up note commits (`84981ac` build, `61f4f3b` summarize) are the most recent.
3. `src/bundle/component.ts` + `src/bundle/context.ts` — these are the two files Item 3 (frontmatter rename) touches.
4. `src/summarize/components.ts` + `src/summarize/prompts.ts` — Item 2 (summarize duplicate-heading) touches one or both of these depending on the chosen fix.
5. `package.json` — Item 1 (build template-copy) edits the `"build"` script line.

### Locked decisions (inherited; NOT re-litigated)

- **Commits land on local `main` directly** — no feature branch, no worktree. No remote → no push. Match existing commit style (`feat:` / `fix:` / `refactor:` / `docs:`, short imperative subject, no Claude attribution).
- **Quality gates before every commit:** `pnpm test && pnpm check && pnpm lint && pnpm build`, all clean. No `--no-verify`, no skipped checks. Never `git commit --amend` — always a new commit for fixes.
- **TDD where applicable:** Items 2 and 3 touch logic, so write the failing test first, watch it fail, implement, watch it pass. Item 1 is build-config only — a verification step suffices (rm -rf dist; pnpm build; ls dist/...).
- **No scope creep beyond the three items.** Don't touch the `bundle --component` code itself unless one of the items requires it. Don't refactor adjacent modules. If you discover another real bug, file it as a *new* follow-up note in the same plan section rather than fixing inline.
- **Item ordering:** Item 1 → Item 2 → Item 3. Item 1 first because it's trivial and unblocks any future fresh-build verification cycles the other two might need.

### Item 1 — Build script: copy `.eta` templates

**Background:** Plan note bullet 1 (`docs/superpowers/plans/2026-05-19-bundle-component.md` § Follow-up notes from implementation). `pnpm build` runs `tsc` only, which doesn't copy `.eta` template files from `src/render/templates/` to `dist/src/render/templates/`. Commands that load templates (`render-vault`, possibly others) fail after a clean build until templates are manually copied.

**Fix:** Extend `"build"` in `package.json` so it copies templates as part of the build. Simplest viable form: `"build": "tsc && cp -r src/render/templates dist/src/render/templates"`. If the project already uses a build helper like `tsc-alias` or `copyfiles`, prefer that — but adding a new dev-dep is unlikely worth it for a one-line copy step.

**Verification:**
1. `rm -rf dist`
2. `pnpm build`
3. `ls dist/src/render/templates/` — both `.eta` files present (the project has `atlas.md.eta` and `component.md.eta` per the prior session's discovery)
4. `pnpm test && pnpm check && pnpm lint` — clean

**Commit message** (exact): `fix: copy render templates into dist during build`

**After the commit lands**, remove (or strike-through) bullet 1 from the plan's `## Follow-up notes from implementation` section in a second small commit: `docs: mark build template-copy follow-up resolved`. Keep the section heading; don't delete bullets 2 + 3.

### Item 2 — Summarize: duplicate `## Purpose` heading

**Background:** Plan note bullet 2 (same file/section). The claude summary provider sometimes returns text starting with `## Purpose\n\n`. `replaceMarkdownSection` then writes that text *under* the existing `## Purpose` heading, producing two consecutive `## Purpose` headings in the vault note. `markdownSectionContent` correctly extracts the empty content between the two headings, so `bundle --component <id>` produces a structurally valid bundle with an empty Purpose section. Caught during live verification of `frontend.app.web` against `tests/fixtures/mini-repo`.

**Design fork (ask the user):** Two viable fixes, both noted in the plan:
- **(a) Prompt-side:** Harden the claude summary prompt in `src/summarize/prompts.ts` so the model is forbidden from emitting the heading itself. Cheaper, depends on prompt discipline holding.
- **(b) Helper-side:** Make `replaceMarkdownSection` (in `src/markdown/sections.ts`) strip a leading `## <same-heading>\n` from its `content` argument before insertion. More defensive, slightly couples `replaceMarkdownSection`'s contract to the heading-name argument.

Use one `AskUserQuestion` round with these two options + a third "(c) Both — prompt fix as primary, defensive strip as belt-and-suspenders". The third is arguably the safest — prompts drift over time, defensive code is forever. Surface that reasoning in the question's option descriptions and let the user pick.

**TDD:** Write a failing test FIRST that captures the bug.
- If pursuing (a) only: the test is harder to write deterministically because the bug is in LLM output — likely skip a unit test and rely on a manual prompt-revision verification.
- If pursuing (b) or (c): add a test to `tests/markdown-sections.test.ts` that calls `replaceMarkdownSection(vaultNote, "Purpose", "## Purpose\n\nThe actual content.\n")` and asserts the result has exactly one `## Purpose` line. Watch it fail, then implement the strip, then watch it pass.

**Verification (live):** In a sandbox under `$CLAUDE_JOB_DIR` (the previous session's sandbox lives at `${CLAUDE_JOB_DIR:-/tmp}/bundle-live-verify`; reuse it if present), run init + scan + render-vault + `summarize --provider claude` against `tests/fixtures/mini-repo`. Then `grep -c '^## Purpose$' vault/dull/components/*.md` — every count must be 1. The user has authorized claude API spend for verification. If running afresh, the sandbox path may have been cleaned; redo init + scan + render-vault if so.

**Commit message** (template — adjust prefix based on chosen fix):
- (a) only: `fix: forbid claude summary from emitting heading itself`
- (b) only: `fix: strip duplicate heading in replaceMarkdownSection`
- (c) both: two commits, one per fix, in the order applied.

**After Item 2 lands**, remove bullet 2 from the plan in a `docs:` commit (or fold into the resolved-follow-ups commit at the end if Item 3 is also done in this session).

### Item 3 — `source_model:` frontmatter convention rename

**Background:** The final cross-task code review of the `bundle --component` work flagged a semantic-accuracy issue in the markdown frontmatter convention used by both the existing `changed.md` renderer (`src/bundle/context.ts`) and the new `bundle/components/<id>.md` renderer (`src/bundle/component.ts`). Both emit `source_model: cache/dull/bundles/.../<X>.yaml` where the value points at the markdown's own YAML sibling (the structured form of the same bundle). The name implies "where the source model came from", but the value is "the YAML twin of this markdown". The YAML half of the component bundle correctly uses `sources.componentModel: cache/dull/model/components/<id>.yaml` for the actual upstream source. This observation is NOT in the plan's Follow-up notes section yet — file it as bullet 3 of that section in the same commit that resolves it.

**Design fork (ask the user):**
- **(A) Rename to match the value.** `source_model:` → `yaml_sibling:` (or `bundle_yaml:`). Cleanest. Affects `src/bundle/context.ts`, `src/bundle/component.ts`, `tests/bundle.test.ts`, `tests/bundle-component.test.ts`, and the spec doc's YAML schema + markdown template snippets. Breaking for any downstream consumer reading the markdown, which is currently just the user's own LLM workflow.
- **(B) Change the value to match the name.** `source_model:` keeps the name but for component bundles points at `cache/dull/model/components/<id>.yaml`. For `changed.md` there's no obvious upstream source model (the changed bundle just lists changed files), so (B) is awkward for the `changed` renderer.

Recommend (A) — it's coherent across both bundle kinds. Ask the user with options A + B + a brief rationale.

**TDD:** Find and modify the affected test assertions FIRST (e.g., `tests/bundle.test.ts` asserts on `source_model:` in the markdown; `tests/bundle-component.test.ts` does too). Change the expectations to the new convention, watch the tests fail, then update `context.ts` and `component.ts`, then watch them pass.

**Spec doc update:** Modify `docs/superpowers/specs/2026-05-19-bundle-component-design.md` — both the YAML schema example and the markdown template under the design's "YAML schema" + "Markdown template" sections — to use the new field name. Commit as a separate `docs:` commit.

**Commit messages** (template based on chosen option):
- (A): `refactor: rename source_model frontmatter field to <new-name>`
- Followed by: `docs: update bundle spec for renamed frontmatter field`

### Resolved-follow-ups cleanup commit

After Items 1 + 2 + 3 (or whichever subset this session completes) have landed, make one final commit that prunes the resolved bullets from the plan's `## Follow-up notes from implementation` section. If all three are resolved, the section may end up empty — in that case, delete the section header too. If only some are resolved, leave the others. Commit: `docs: prune resolved follow-ups from bundle-component plan`.

### Drop on completion

When the final commit of this session lands (the resolved-follow-ups cleanup, or the last item's commit if cleanup is rolled in), `git rm /Users/antony/Development/Atlas/docs/superpowers/prompts/2026-05-19-dull-context-bundle-followups.md` in that same commit. This is the Atlas `prompts/` directory drop-on-completion convention. The `git rm` happens in the Atlas repo, NOT dull-context — that's a separate commit in a separate repo. Either skip the drop and let the user handle it, OR navigate to Atlas and commit the deletion there with message: `docs: drop dull-context bundle-followups prompt (completed)`.

### Operating discipline

- **One `AskUserQuestion` round per design fork** (Items 2 and 3 each have one). Don't run a full `superpowers:brainstorming` cycle — the spec-level decisions for both items are already made; only the fix-path question remains.
- **TDD for Items 2 and 3.** Item 1 is build-config and skips TDD.
- **Each item commits independently.** Don't bundle Item 1 + Item 2 into one commit. Per the project's "one commit per logical change" pattern, each item lands as its own commit (or two — fix + spec doc — if the change spans code + spec).
- **Live verification for Item 2 only.** Items 1 and 3 are verified by unit/integration tests. Item 2's fix needs an end-to-end run against the claude provider to confirm the duplicate heading no longer appears in vault notes.
- **No new memory files.** This session implements three locked fixes; if a surprising decisional pattern emerges (e.g., the user picks an unexpected option in one of the forks), THAT might warrant a feedback memory in the Atlas memory system (`/Users/antony/Development/Atlas/.claude/memory/`). Otherwise no memory writes.

### Acceptance gate

Before declaring this session complete:

- Each of Items 1 + 2 + 3 has either (a) been resolved with a commit landing on dull-context's `main`, or (b) been explicitly deferred by the user. Don't silently skip.
- Plan's `## Follow-up notes from implementation` section accurately reflects current state — resolved bullets removed, deferred bullets retained, any newly-discovered follow-ups appended.
- `pnpm test && pnpm check && pnpm lint && pnpm build` all clean after every commit.
- For Item 2: `grep -c '^## Purpose$' vault/dull/components/*.md` returns 1 for every file after a fresh summarize run.
- For Item 3: the spec doc's example YAML and markdown template are consistent with the implementation.
- This prompt file is either dropped via `git rm` in Atlas, or its remaining handling has been raised with the user.

### Begin at Item 1

Verify dull-context state matches the expected baseline:

```bash
cd /Users/antony/Development/dull-context
git status                  # clean
git log -1 --format=%H      # 61f4f3b
```

If the SHA has drifted (the user or another session moved main forward), STOP and reconcile before proceeding. Otherwise begin Item 1: read `package.json`'s `"build"` line, then make the one-line edit.

---

## Why this prompt exists in `docs/superpowers/prompts/`

The Atlas `prompts/` directory holds in-flight kickoff prompts that bootstrap follow-up sessions across projects (not just Atlas — this prompt targets dull-context). They are dropped via `git rm` in the same commit that completes the work they kick off. This is the dull-context bundle follow-ups kickoff; when the final item or cleanup commit lands, this file gets dropped from Atlas. If only some items are resolved, the session author should rewrite this prompt with the unresolved items remaining, replacing the existing file under the same name (the slug stays stable, only the body shrinks).
