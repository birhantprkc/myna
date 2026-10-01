//! Audible session cues: a sound when dictation starts listening, when it
//! stops, and when it fails, behind the `sounds` setting.
//!
//! The cues are derived from the indicator timeline rather than threaded
//! through the controller: [`Chiming`] wraps whatever [`Indicator`] the daemon
//! runs and hears every state it is shown, reading the same [`Readiness`] the
//! HUD does, so what the user hears can never disagree with what the HUD
//! shows. [`Chime`] is the port;
//! [`player::Player`] plays Myna's own sounds.

use async_trait::async_trait;

use crate::indicator::readiness::Readiness;
use crate::indicator::{Indicator, IndicatorState};
use crate::live::Live;

pub mod player;

/// The three moments a session is heard.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Cue {
    Start,
    Stop,
    Error,
}

/// Plays a cue. Must return at once: the controller awaits the indicator.
pub trait Chime: Send {
    fn play(&self, cue: Cue);
}

/// Where the session is, as far as the cues care.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Phase {
    #[default]
    Idle,
    /// Pressed, the model not yet ready: nothing is heard yet.
    Loading,
    Listening,
    /// Stopped listening, the transcript still to come.
    Ending,
    /// A critical error is showing.
    Failed,
}

/// The cue one indicator state earns, and the phase it leaves behind. `ready`
/// is whether the session's model has said `Ready`.
///
/// Start once per session, when listening begins: the first `Recording` with
/// the model ready, so a press that fails while loading is an Error alone.
/// Stop once, at whichever comes first of the end of listening (`Finalizing`)
/// and the end of the session; a session that never started listening has
/// nothing to stop. A recoverable notice ("No speech detected") is an
/// ordinary end. A critical error always sounds, even after Stop, but a
/// repeat of the one already showing does not.
fn step(phase: Phase, state: &IndicatorState, ready: bool) -> (Phase, Option<Cue>) {
    match state {
        IndicatorState::Recording | IndicatorState::Transcribing if phase == Phase::Listening => {
            (Phase::Listening, None)
        }
        IndicatorState::Recording if ready => (Phase::Listening, Some(Cue::Start)),
        IndicatorState::Recording => (Phase::Loading, None),
        IndicatorState::Transcribing => (phase, None),
        IndicatorState::Finalizing => match phase {
            Phase::Listening => (Phase::Ending, Some(Cue::Stop)),
            Phase::Loading => (Phase::Ending, None),
            other => (other, None),
        },
        IndicatorState::Error {
            recoverable: false, ..
        } => (
            Phase::Failed,
            (phase != Phase::Failed).then_some(Cue::Error),
        ),
        IndicatorState::Hidden | IndicatorState::Error { .. } => (
            Phase::Idle,
            (phase == Phase::Listening).then_some(Cue::Stop),
        ),
    }
}

/// An [`Indicator`] that also plays the session's cues while `enabled` holds.
pub struct Chiming<I> {
    inner: I,
    chime: Box<dyn Chime>,
    enabled: Live<bool>,
    readiness: Readiness,
    phase: Phase,
}

impl<I: Indicator> Chiming<I> {
    pub fn new(
        inner: I,
        chime: impl Chime + 'static,
        enabled: Live<bool>,
        readiness: Readiness,
    ) -> Self {
        Self {
            inner,
            chime: Box::new(chime),
            enabled,
            readiness,
            phase: Phase::Idle,
        }
    }
}

#[async_trait]
impl<I: Indicator> Indicator for Chiming<I> {
    async fn set_state(&mut self, state: IndicatorState) {
        let (phase, cue) = step(self.phase, &state, self.readiness.ready_seen());
        self.phase = phase;
        if let Some(cue) = cue.filter(|_| self.enabled.get()) {
            self.chime.play(cue);
        }
        self.inner.set_state(state).await;
    }

    async fn set_audio_drops(&mut self, not_active: u64) {
        self.inner.set_audio_drops(not_active).await;
    }

    async fn set_last_error(&mut self, headline: &str, detail: &str) {
        self.inner.set_last_error(headline, detail).await;
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::indicator::mock::MockIndicator;

    #[derive(Clone, Default)]
    struct Heard(Arc<Mutex<Vec<Cue>>>);

    impl Chime for Heard {
        fn play(&self, cue: Cue) {
            self.0.lock().unwrap().push(cue);
        }
    }

    /// The cues a session earns, its model resident throughout.
    fn heard(states: &[IndicatorState]) -> Vec<Cue> {
        heard_while(&states.iter().map(|s| (s.clone(), true)).collect::<Vec<_>>())
    }

    /// The cues for states each shown with whether `Ready` had been seen.
    fn heard_while(states: &[(IndicatorState, bool)]) -> Vec<Cue> {
        let mut phase = Phase::Idle;
        states
            .iter()
            .filter_map(|(state, ready)| {
                let (next, cue) = step(phase, state, *ready);
                phase = next;
                cue
            })
            .collect()
    }

    const LOADING: (IndicatorState, bool) = (Recording, false);
    const LISTENING: (IndicatorState, bool) = (Recording, true);

    use IndicatorState::{Finalizing, Hidden, Recording, Transcribing};

    fn critical() -> IndicatorState {
        IndicatorState::critical("backend gone")
    }

    fn notice() -> IndicatorState {
        IndicatorState::recoverable("No speech detected")
    }

    #[test]
    fn a_session_starts_and_stops_once() {
        assert_eq!(
            heard(&[
                Recording,
                Transcribing,
                Recording,
                Transcribing,
                Finalizing,
                Hidden
            ]),
            [Cue::Start, Cue::Stop]
        );
    }

    #[test]
    fn a_session_that_ends_without_finalizing_still_stops() {
        assert_eq!(heard(&[Recording, Hidden]), [Cue::Start, Cue::Stop]);
        assert_eq!(heard(&[Recording, notice()]), [Cue::Start, Cue::Stop]);
    }

    #[test]
    fn a_notice_after_the_stop_is_not_a_second_stop() {
        assert_eq!(
            heard(&[Recording, Finalizing, notice(), Hidden]),
            [Cue::Start, Cue::Stop]
        );
    }

    #[test]
    fn a_failure_sounds_even_after_the_stop_and_only_once() {
        assert_eq!(
            heard(&[Recording, Finalizing, critical(), critical(), Hidden]),
            [Cue::Start, Cue::Stop, Cue::Error]
        );
        assert_eq!(heard(&[Recording, critical()]), [Cue::Start, Cue::Error]);
    }

    #[test]
    fn a_press_refused_before_capture_is_an_error_alone() {
        assert_eq!(heard(&[critical()]), [Cue::Error]);
    }

    #[test]
    fn a_press_that_fails_before_the_model_is_ready_is_an_error_alone() {
        assert_eq!(heard_while(&[LOADING, (critical(), false)]), [Cue::Error]);
    }

    #[test]
    fn the_start_waits_for_the_model() {
        assert_eq!(
            heard_while(&[
                LOADING,
                LOADING,
                LISTENING,
                (Finalizing, true),
                (Hidden, true)
            ]),
            [Cue::Start, Cue::Stop]
        );
    }

    #[test]
    fn a_session_that_ends_while_loading_is_silent() {
        assert_eq!(
            heard_while(&[LOADING, (Finalizing, false), (Hidden, false)]),
            []
        );
        assert_eq!(heard_while(&[LOADING, (notice(), false)]), []);
        assert_eq!(
            heard_while(&[LOADING, (Hidden, false), LISTENING]),
            [Cue::Start]
        );
    }

    #[test]
    fn the_next_session_starts_again_after_any_ending() {
        assert_eq!(
            heard(&[Recording, Hidden, Recording, Finalizing, Recording]),
            [Cue::Start, Cue::Stop, Cue::Start, Cue::Stop, Cue::Start]
        );
        assert_eq!(heard(&[critical(), Recording]), [Cue::Error, Cue::Start]);
    }

    #[test]
    fn a_stray_state_while_idle_is_silent() {
        assert_eq!(heard(&[Hidden, Transcribing, Finalizing, notice()]), []);
    }

    #[derive(Default)]
    struct Drops(Arc<Mutex<Vec<u64>>>);

    #[async_trait]
    impl Indicator for Drops {
        async fn set_state(&mut self, _state: IndicatorState) {}

        async fn set_audio_drops(&mut self, not_active: u64) {
            self.0.lock().unwrap().push(not_active);
        }
    }

    #[tokio::test]
    async fn audio_drops_reach_the_wrapped_indicator() {
        let drops = Drops::default();
        let seen = drops.0.clone();
        let mut chiming = Chiming::new(drops, Heard::default(), Live::new(true), Readiness::new());
        chiming.set_audio_drops(7).await;
        assert_eq!(*seen.lock().unwrap(), [7]);
    }

    #[tokio::test]
    async fn the_indicator_sees_every_state_and_the_setting_gates_only_the_sound() {
        let indicator = MockIndicator::new();
        let shown = indicator.log();
        let chime = Heard::default();
        let enabled = Live::new(true);
        let readiness = Readiness::new();
        readiness.note_ready();
        let mut chiming = Chiming::new(indicator, chime.clone(), enabled.clone(), readiness);

        chiming.set_state(Recording).await;
        enabled.set(false);
        chiming.set_state(Finalizing).await;
        chiming.set_state(Hidden).await;
        enabled.set(true);
        chiming.set_state(Recording).await;

        assert_eq!(
            *shown.lock().unwrap(),
            [Recording, Finalizing, Hidden, Recording]
        );
        assert_eq!(*chime.0.lock().unwrap(), [Cue::Start, Cue::Start]);
    }

    #[tokio::test]
    async fn the_start_cue_reads_the_session_readiness() {
        let chime = Heard::default();
        let readiness = Readiness::new();
        let mut chiming = Chiming::new(
            MockIndicator::new(),
            chime.clone(),
            Live::new(true),
            readiness.clone(),
        );

        chiming.set_state(Recording).await;
        chiming.set_state(critical()).await;
        readiness.note_ready();
        chiming.set_state(Recording).await;

        assert_eq!(*chime.0.lock().unwrap(), [Cue::Error, Cue::Start]);
    }
}
