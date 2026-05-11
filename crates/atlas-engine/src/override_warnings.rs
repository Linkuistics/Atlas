//! Phase 6 PR-4: closed-enumeration override warnings.
//!
//! Today (Phase 6), the closed enumeration carries exactly three
//! variants. Future LLM-spine work (Phase 7+) extends this enum; current
//! scope is closed at these three so `--strict-overrides` has a stable
//! contract.
//!
//! Architecture: the engine surfaces non-fatal override-related issues
//! through an [`OverrideWarningCollector`] handle plumbed through the
//! call chain (or installed on the [`crate::AtlasDatabase`] as a side
//! channel). Two production collectors:
//!
//! - [`PermissiveCollector`] — echoes each warning to `stderr` and never
//!   sets `has_errors()`. The historical (Phase 3 / pre-PR-4) behaviour.
//! - [`StrictCollector`] — echoes to `stderr` AND flips an internal
//!   `has_errors` flag on every emit; the CLI consults `has_errors()`
//!   after [`crate::run_index`] returns and exits non-zero if set.
//!
//! Tests that need to assert on the warning text install a
//! [`CapturingCollector`] (see the module's `testing` section) which
//! records every emit into an in-memory `Vec<String>` instead of
//! writing to process stderr.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

/// Closed enumeration of override warnings escalated to errors when
/// `--strict-overrides` is set. Adding a new variant is a breaking
/// change to the strict-mode contract; do so deliberately, with
/// matching test coverage in `crates/atlas-cli/tests/strict_overrides_contract.rs`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OverrideWarning {
    /// An `edges_suppress` directive in the merged overrides matched
    /// no analyser-discovered edge. Permissive: log + no-op. Strict:
    /// non-zero exit.
    EdgesSuppressNoMatch { directive: String, scope: String },
    /// An override entry under `edges_add` or `edges_suppress`
    /// references an unknown edge kind. The entry is dropped (the
    /// edge is never materialised, or never suppressed). Permissive:
    /// log + drop. Strict: non-zero exit.
    ///
    /// The `scope` field disambiguates which directive triggered the
    /// warning (it includes the literal `edges_add` or
    /// `edges_suppress` substring of the source list).
    EdgesOverrideUnknownKind { kind: String, scope: String },
    /// A central `subsystems.overrides.yaml` `members:` entry names an
    /// id-form member that does not resolve to any extant component.
    /// The entry is skipped (the subsystem ends up with the
    /// rest of its resolvable members). Permissive: log + skip.
    /// Strict: non-zero exit.
    SubsystemOverrideNonExistent { name: String, scope: String },
}

impl OverrideWarning {
    /// Render the warning as a human-readable line. The format is
    /// stable across the closed enumeration so existing
    /// stderr-substring tests survive the PR-4 refactor.
    pub fn human_message(&self) -> String {
        match self {
            OverrideWarning::EdgesSuppressNoMatch { directive, scope } => format!(
                "warning: edges_suppress directive `{directive}` in {scope} matched no edges \
                 — override has no effect (no match)"
            ),
            OverrideWarning::EdgesOverrideUnknownKind { kind, scope } => format!(
                "warning: override entry references unknown kind `{kind}` in {scope} \
                 — entry not applied"
            ),
            OverrideWarning::SubsystemOverrideNonExistent { name, scope } => format!(
                "warning: subsystems.overrides.yaml references component `{name}` in {scope} \
                 but no such component exists in the workspace — override entry does not exist \
                 (no extant component; skipped)"
            ),
        }
    }
}

/// Collector for non-fatal override warnings. Production code holds a
/// trait object; tests and the CLI substitute concrete impls.
///
/// `Send + Sync` because the L6 / L9 walks now run on a parallel pool
/// (Phase 3 PR-* parallel L5 pre-warm; the collector must outlive
/// parallel `surface_of` calls without locking).
pub trait OverrideWarningCollector: Send + Sync {
    /// Record a warning. Permissive impls write to stderr; strict
    /// impls write to stderr AND set the `has_errors` flag.
    fn emit(&self, warning: OverrideWarning);
    /// `true` iff at least one warning has been emitted under a strict
    /// policy. The CLI uses this to decide its exit code after
    /// `run_index` returns.
    fn has_errors(&self) -> bool;
}

/// Default collector for non-strict runs. Echoes every warning to
/// `stderr` and never sets `has_errors()`.
pub struct PermissiveCollector;

impl OverrideWarningCollector for PermissiveCollector {
    fn emit(&self, warning: OverrideWarning) {
        eprintln!("{}", warning.human_message());
    }
    fn has_errors(&self) -> bool {
        false
    }
}

/// Collector for `--strict-overrides` runs. Echoes every warning to
/// `stderr` AND sets `has_errors()` after the first emit.
pub struct StrictCollector {
    has_errors: AtomicBool,
}

impl StrictCollector {
    pub fn new() -> Self {
        Self {
            has_errors: AtomicBool::new(false),
        }
    }
}

impl Default for StrictCollector {
    fn default() -> Self {
        Self::new()
    }
}

impl OverrideWarningCollector for StrictCollector {
    fn emit(&self, warning: OverrideWarning) {
        eprintln!("{}", warning.human_message());
        self.has_errors.store(true, Ordering::SeqCst);
    }
    fn has_errors(&self) -> bool {
        self.has_errors.load(Ordering::SeqCst)
    }
}

/// Test-only collector that records every emitted warning into an
/// in-memory vector instead of writing to process `stderr`. Exposed so
/// integration tests can assert on the warning text without
/// stderr-capture acrobatics.
///
/// `error_on_emit` controls the strictness contract: when `true`, the
/// collector reports `has_errors()` after at least one emit (the
/// strict-mode equivalent); when `false`, `has_errors()` is always
/// `false` (the permissive-mode equivalent).
pub struct CapturingCollector {
    error_on_emit: bool,
    warnings: Mutex<Vec<OverrideWarning>>,
}

impl CapturingCollector {
    /// Build a permissive capturing collector (mirrors
    /// [`PermissiveCollector`] but records emits instead of writing to
    /// stderr).
    pub fn new_permissive() -> Self {
        Self {
            error_on_emit: false,
            warnings: Mutex::new(Vec::new()),
        }
    }

    /// Build a strict capturing collector (mirrors [`StrictCollector`]).
    pub fn new_strict() -> Self {
        Self {
            error_on_emit: true,
            warnings: Mutex::new(Vec::new()),
        }
    }

    /// Snapshot of every warning emitted so far, in order.
    pub fn warnings(&self) -> Vec<OverrideWarning> {
        self.warnings
            .lock()
            .expect("warnings mutex poisoned")
            .clone()
    }

    /// Concatenation of every warning's `human_message()`, one per
    /// line, suitable for substring assertions in integration tests.
    pub fn rendered(&self) -> String {
        let mut out = String::new();
        for w in self
            .warnings
            .lock()
            .expect("warnings mutex poisoned")
            .iter()
        {
            out.push_str(&w.human_message());
            out.push('\n');
        }
        out
    }
}

impl OverrideWarningCollector for CapturingCollector {
    fn emit(&self, warning: OverrideWarning) {
        self.warnings
            .lock()
            .expect("warnings mutex poisoned")
            .push(warning);
    }
    fn has_errors(&self) -> bool {
        self.error_on_emit
            && !self
                .warnings
                .lock()
                .expect("warnings mutex poisoned")
                .is_empty()
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
        c.emit(OverrideWarning::EdgesOverrideUnknownKind {
            kind: "bogus".into(),
            scope: "y".into(),
        });
        assert!(c.has_errors());
    }

    #[test]
    fn capturing_collector_records_emits_in_order() {
        let c = CapturingCollector::new_permissive();
        c.emit(OverrideWarning::EdgesSuppressNoMatch {
            directive: "a".into(),
            scope: "scope-a".into(),
        });
        c.emit(OverrideWarning::EdgesOverrideUnknownKind {
            kind: "nope".into(),
            scope: "scope-b".into(),
        });
        let got = c.warnings();
        assert_eq!(got.len(), 2);
        let rendered = c.rendered();
        assert!(rendered.contains("scope-a"));
        assert!(rendered.contains("nope"));
        // permissive shape: emits do not flip has_errors.
        assert!(!c.has_errors());
    }

    #[test]
    fn capturing_collector_strict_flips_has_errors() {
        let c = CapturingCollector::new_strict();
        assert!(!c.has_errors());
        c.emit(OverrideWarning::SubsystemOverrideNonExistent {
            name: "ghost".into(),
            scope: "subsystems.overrides.yaml subsystem `gamma`".into(),
        });
        assert!(c.has_errors());
    }

    #[test]
    fn human_message_subsystem_includes_does_not_exist_substring() {
        // PR-3's `subsystem_overlay.rs` test asserts the warning text
        // includes one of `does not exist`, `not found`, or `no extant`.
        // Keep both `does not exist` and `no extant` so the existing
        // PR-3 + new PR-4 tests pass under the same renderer.
        let w = OverrideWarning::SubsystemOverrideNonExistent {
            name: "ghost".into(),
            scope: "subsystem `gamma`".into(),
        };
        let msg = w.human_message();
        assert!(msg.contains("does not exist"));
        assert!(msg.contains("no extant"));
        assert!(msg.contains("ghost"));
    }
}
