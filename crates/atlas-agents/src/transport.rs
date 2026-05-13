//! Transport flavour: enumeration of the four backend transports the
//! LLM-spine runtime knows how to drive.
//!
//! The flavour distinguishes wire protocol, not provider — the two
//! Anthropic transports (`ClaudeCode` subprocess vs. `HttpAnthropic`)
//! share a provider but have different reliability / cost profiles, and
//! the persistent transcript cache (`atlas-engine::llm_cache::call_agent_cached`,
//! recast §6.1) hashes the flavour into the cache key (via [`TransportFlavour::as_str`])
//! so switching transports between runs invalidates the cache cleanly.
//!
//! Layering note: `atlas-engine` does not depend on `atlas-agents`
//! (cycle avoidance — `atlas-agents` depends on `atlas-engine`), so the
//! transcript-cache fingerprint stores the *string* form (`as_str()`).
//! The enum stays in `atlas-agents` where the agent-runtime types live.
//!
//! See `docs/superpowers/specs/2026-05-11-atlas-llm-spine-recast-design.md`
//! and `docs/superpowers/brainstorms/2026-05-12-atlas-vnext-phase7-brainstorm.md`.

use serde::{Deserialize, Serialize};

pub use atlas_llm::Provider;

/// Wire flavour for a backend transport. Hashes into the transcript-cache
/// fingerprint so that the same logical agent run over two different
/// transports produces two cache entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TransportFlavour {
    /// Anthropic via the `claude` subprocess.
    ClaudeCode,
    /// OpenAI via the `codex` subprocess.
    Codex,
    /// Anthropic via the Messages HTTP API.
    HttpAnthropic,
    /// OpenAI via the Responses HTTP API.
    HttpOpenai,
}

impl TransportFlavour {
    /// Stable wire string used as a cache-key contributor. Lowercase
    /// snake_case so the serialised cache-key form is filesystem-safe.
    /// Load-bearing: a rename here would invalidate every transcript-
    /// cache entry on disk.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ClaudeCode => "claude_code",
            Self::Codex => "codex",
            Self::HttpAnthropic => "http_anthropic",
            Self::HttpOpenai => "http_openai",
        }
    }

    /// Underlying LLM provider. Two flavours per provider — subprocess
    /// vs. HTTP — share `provider()` but not `as_str()`.
    pub fn provider(self) -> Provider {
        match self {
            Self::ClaudeCode | Self::HttpAnthropic => Provider::Anthropic,
            Self::Codex | Self::HttpOpenai => Provider::OpenAi,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn as_str_is_stable_snake_case() {
        // The cache-key contributor is load-bearing — any rename here
        // invalidates every transcript-cache entry on disk. Lock it
        // down so an accidental enum rename does not silently invalidate
        // the cache.
        assert_eq!(TransportFlavour::ClaudeCode.as_str(), "claude_code");
        assert_eq!(TransportFlavour::Codex.as_str(), "codex");
        assert_eq!(TransportFlavour::HttpAnthropic.as_str(), "http_anthropic");
        assert_eq!(TransportFlavour::HttpOpenai.as_str(), "http_openai");
    }

    #[test]
    fn provider_groups_transports_by_vendor() {
        assert_eq!(TransportFlavour::ClaudeCode.provider(), Provider::Anthropic);
        assert_eq!(
            TransportFlavour::HttpAnthropic.provider(),
            Provider::Anthropic
        );
        assert_eq!(TransportFlavour::Codex.provider(), Provider::OpenAi);
        assert_eq!(TransportFlavour::HttpOpenai.provider(), Provider::OpenAi);
    }
}
