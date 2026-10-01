//! Whether this session's model is resident yet: what splits the indicator's
//! `Recording` into loading and listening.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use myna_orchestrator::{OrchestratorEvent, TextSink};

/// Session-scoped readiness tracking for the `loading`/`recording` split
/// (R4/P2): the controller maps both `Loading` and `Ready` orchestrator events
/// to `IndicatorState::Recording`, so this keeps whether `Ready` has been seen
/// this session. A cheaply clonable handle — the [`ReadinessTee`] writes it
/// from the session's event stream while indicators read it from the
/// controller's calls; the event is always observed *before* the controller
/// routes it to `set_state`, so the flag is fresh.
#[derive(Debug, Clone, Default)]
pub struct Readiness {
    ready_seen: Arc<AtomicBool>,
}

impl Readiness {
    /// A fresh, cold session (no `Ready` seen).
    pub fn new() -> Self {
        Self::default()
    }

    /// `Loading` seen — the session is in the cold-load window.
    pub fn note_loading(&self) {
        self.ready_seen.store(false, Ordering::SeqCst);
    }

    /// `Ready` seen — subsequent `Recording` publishes `recording`.
    pub fn note_ready(&self) {
        self.ready_seen.store(true, Ordering::SeqCst);
    }

    /// New session: back to cold until the next `Ready`.
    pub fn reset(&self) {
        self.ready_seen.store(false, Ordering::SeqCst);
    }

    /// Whether `Ready` has been seen this session.
    pub fn ready_seen(&self) -> bool {
        self.ready_seen.load(Ordering::SeqCst)
    }
}

/// A [`TextSink`] wrapper that tracks `Loading`/`Ready` into a [`Readiness`],
/// forwarding every event to the real sink unchanged. Wired per session by the
/// daemon's session factory; invisible to the controller.
pub struct ReadinessTee<S: TextSink> {
    inner: S,
    readiness: Readiness,
}

impl<S: TextSink> ReadinessTee<S> {
    /// Wrap `inner`, updating `readiness` as liveness events flow past.
    pub fn new(inner: S, readiness: Readiness) -> Self {
        Self { inner, readiness }
    }
}

#[async_trait]
impl<S: TextSink> TextSink for ReadinessTee<S> {
    async fn emit(&mut self, event: OrchestratorEvent) {
        match event {
            OrchestratorEvent::Loading => self.readiness.note_loading(),
            OrchestratorEvent::Ready => self.readiness.note_ready(),
            _ => {}
        }
        self.inner.emit(event).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// R4: the tracker resets each session — a warm session (`Ready` seen)
    /// followed by a fresh cold session reports `loading` again until the new
    /// `Ready`.
    #[test]
    fn readiness_tracks_loading_then_ready_per_session() {
        let readiness = Readiness::new();
        assert!(!readiness.ready_seen());

        readiness.note_loading();
        assert!(!readiness.ready_seen());
        readiness.note_ready();
        assert!(readiness.ready_seen());

        readiness.reset();
        assert!(!readiness.ready_seen(), "new session starts cold again");
    }

    /// The tee observes `Loading`/`Ready` and forwards everything unchanged.
    #[tokio::test]
    async fn readiness_tee_tracks_liveness_and_forwards() {
        use myna_orchestrator::CollectingSink;

        let readiness = Readiness::new();
        let mut tee = ReadinessTee::new(CollectingSink::default(), readiness.clone());

        tee.emit(OrchestratorEvent::Loading).await;
        assert!(!readiness.ready_seen());
        tee.emit(OrchestratorEvent::Ready).await;
        assert!(readiness.ready_seen());
        tee.emit(OrchestratorEvent::Transcribing).await;
        assert!(readiness.ready_seen(), "unrelated events don't touch it");

        assert_eq!(tee.inner.events.len(), 3, "every event forwarded");
    }
}
