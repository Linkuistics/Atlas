//! Concurrency caps for the agent runtime (plan §4 Task 4.6).
//!
//! The runtime acquires two permits per agent call: one keyed by
//! transport flavour (HTTP vs. subprocess), one keyed by stage. The
//! transport cap prevents fan-out from overwhelming a backend (HTTP
//! providers have per-account rate limits; subprocess backends are
//! `tokio::process` children competing for the system process table).
//! The per-stage cap prevents within-stage runaway concurrency — a
//! 200-component workspace would otherwise fork 200 surface agents
//! simultaneously.
//!
//! Both caps return [`tokio::sync::OwnedSemaphorePermit`] so a single
//! `call_agent` future can hold both permits across the inner
//! `backend.call_async(...)` await without borrowing `&self` past the
//! await point.

use std::sync::Arc;

use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::transport::TransportFlavour;

use super::audit::Stage;

/// Default HTTP cap. Eight concurrent HTTP backend calls is the
/// conservative starting point per plan §4 Task 4.6; tunable.
pub const DEFAULT_HTTP_CAP: usize = 8;

/// Default subprocess cap. Two concurrent subprocess backend calls
/// matches the typical `claude_code` + `codex` pairing (one client each).
pub const DEFAULT_SUBPROCESS_CAP: usize = 2;

/// Default per-stage cap. Eight concurrent agents per stage matches
/// the HTTP cap so a single-stage workload can saturate the transport
/// cap without contention on its own stage permit.
pub const DEFAULT_PER_STAGE_CAP: usize = 8;

/// Per-stage semaphore set. One `Semaphore` per `Stage` variant so the
/// stages are independent — a saturated Classify stage does not block
/// Surface agents that are ready to start.
pub struct PerStageSemaphores {
    dispatch_subsystem: Arc<Semaphore>,
    dispatch_component: Arc<Semaphore>,
    classify: Arc<Semaphore>,
    surface: Arc<Semaphore>,
    reduce: Arc<Semaphore>,
    project: Arc<Semaphore>,
}

impl PerStageSemaphores {
    /// Construct with the default cap on every stage.
    pub fn defaults() -> Self {
        Self::with_cap(DEFAULT_PER_STAGE_CAP)
    }

    /// Construct with one shared cap across every stage. Useful in
    /// tests that want to set the cap to 1 to force serial ordering.
    pub fn with_cap(cap: usize) -> Self {
        Self {
            dispatch_subsystem: Arc::new(Semaphore::new(cap)),
            dispatch_component: Arc::new(Semaphore::new(cap)),
            classify: Arc::new(Semaphore::new(cap)),
            surface: Arc::new(Semaphore::new(cap)),
            reduce: Arc::new(Semaphore::new(cap)),
            project: Arc::new(Semaphore::new(cap)),
        }
    }

    fn semaphore_for(&self, stage: Stage) -> Arc<Semaphore> {
        match stage {
            Stage::DispatchSubsystem => self.dispatch_subsystem.clone(),
            Stage::DispatchComponent => self.dispatch_component.clone(),
            Stage::Classify => self.classify.clone(),
            Stage::Surface => self.surface.clone(),
            Stage::Reduce => self.reduce.clone(),
            Stage::Project => self.project.clone(),
        }
    }
}

/// Runtime-wide concurrency caps. Held by `AgentRuntime` and consulted
/// at the top of `call_agent` to throttle fan-out.
pub struct Semaphores {
    http: Arc<Semaphore>,
    subprocess: Arc<Semaphore>,
    per_stage: PerStageSemaphores,
}

impl Semaphores {
    /// Production defaults: HTTP=8, subprocess=2, per-stage=8.
    pub fn defaults() -> Self {
        Self {
            http: Arc::new(Semaphore::new(DEFAULT_HTTP_CAP)),
            subprocess: Arc::new(Semaphore::new(DEFAULT_SUBPROCESS_CAP)),
            per_stage: PerStageSemaphores::defaults(),
        }
    }

    /// Construct from caller-supplied caps. Useful in tests.
    pub fn with_caps(http: usize, subprocess: usize, per_stage: usize) -> Self {
        Self {
            http: Arc::new(Semaphore::new(http)),
            subprocess: Arc::new(Semaphore::new(subprocess)),
            per_stage: PerStageSemaphores::with_cap(per_stage),
        }
    }

    /// Acquire one owned permit on the transport-flavour cap. The
    /// returned permit holds an `Arc` clone of the semaphore so it
    /// outlives a borrow of `&self`.
    ///
    /// # Panics
    ///
    /// Never; the semaphore lives for the lifetime of `self`. The
    /// `.expect("not closed")` is structurally unreachable.
    pub async fn acquire_transport(&self, transport: TransportFlavour) -> OwnedSemaphorePermit {
        let sem = match transport {
            TransportFlavour::HttpAnthropic | TransportFlavour::HttpOpenai => self.http.clone(),
            TransportFlavour::ClaudeCode | TransportFlavour::Codex => self.subprocess.clone(),
        };
        sem.acquire_owned().await.expect("semaphore not closed")
    }

    /// Acquire one owned permit on the stage cap.
    ///
    /// # Panics
    ///
    /// Same as [`acquire_transport`].
    pub async fn acquire_stage(&self, stage: Stage) -> OwnedSemaphorePermit {
        self.per_stage
            .semaphore_for(stage)
            .acquire_owned()
            .await
            .expect("semaphore not closed")
    }
}

impl Default for Semaphores {
    fn default() -> Self {
        Self::defaults()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn acquire_transport_http_throttles_at_cap() {
        let sems = Semaphores::with_caps(1, 1, 8);
        let _held = sems
            .acquire_transport(TransportFlavour::HttpAnthropic)
            .await;
        // Second acquire would block — assert it would by polling.
        let next = sems.acquire_transport(TransportFlavour::HttpAnthropic);
        tokio::pin!(next);
        // Two cooperative yields should still leave it pending.
        for _ in 0..2 {
            tokio::task::yield_now().await;
        }
        if futures_poll(&mut next).await {
            panic!("second permit should be pending while the first is held");
        }
    }

    #[tokio::test]
    async fn acquire_stage_independent_of_transport() {
        let sems = Semaphores::with_caps(1, 1, 1);
        let _http = sems.acquire_transport(TransportFlavour::HttpOpenai).await;
        // Stage permit is independent of transport, so this acquire
        // must succeed immediately.
        let _stage = sems.acquire_stage(Stage::Classify).await;
    }

    #[tokio::test]
    async fn stage_caps_are_per_stage_not_global() {
        let sems = Semaphores::with_caps(8, 8, 1);
        let _classify = sems.acquire_stage(Stage::Classify).await;
        // Different stage; must succeed.
        let _surface = sems.acquire_stage(Stage::Surface).await;
    }

    /// Poll a future once via cooperative yield, returning `true` if
    /// it completed and `false` if it is still pending.
    async fn futures_poll<F>(fut: &mut std::pin::Pin<&mut F>) -> bool
    where
        F: std::future::Future,
    {
        use std::future::poll_fn;
        let mut completed = false;
        poll_fn(|cx| {
            match fut.as_mut().poll(cx) {
                std::task::Poll::Ready(_) => completed = true,
                std::task::Poll::Pending => {}
            }
            std::task::Poll::Ready(())
        })
        .await;
        completed
    }
}
