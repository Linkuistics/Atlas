---
name: No iterator stubs for known singletons
description: When refactoring removes plurality (Vec/iterator → singular), simplify the API shape end-to-end — don't leave `impl Iterator<Item=&T>` or `&[T]` accessors over collections that are now guaranteed length-1.
type: feedback
originSessionId: 59c1d765-ecdc-4f31-971e-3aa9bb256513
---
User stated 2026-05-10 during Phase 5 brainstorm: "Don't keep iterators for known singletons."

**Why:** Iterator/slice stubs over length-1 collections preserve the cognitive tax of the deleted plurality without any functional benefit. Every reader still has to pattern-match on iteration, every caller still does `.next().unwrap()` or `[0]`. The point of removing plurality is to *simplify the reader experience*; half-measures undo that gain.

**How to apply:**

- When converting `Vec<T>` → singular `T` (or `Option<T>`), update return types and accessors to match: `fn roots(&self) -> &[PathBuf]` becomes `fn root(&self) -> &Path`, not `fn roots(&self) -> std::iter::Once<&Path>`.
- For Phase 5 specifically: `Workspace.root: PathBuf` returns `&Path`; callers that previously did `for r in workspace.roots()` get rewritten to use the singular form, not refactored to iterate over a one-element collection.
- Applies to refactors generally — any "we used to support N, now it's always 1" simplification should drive the API shape, not just the data shape.
