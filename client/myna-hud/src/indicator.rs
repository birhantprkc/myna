//! indicator — the state the simple level views (bar, vumeter)
//! share: the latest level push, the dictation state and the eased level.
//!
//! Each view owns one [`Indicator`] and asks it for a [`Frame`] when it
//! draws; the drawing itself stays in the view.

use std::cell::Cell;
use std::time::Instant;

use crate::hud_logic::IndicatorState;
use crate::states::{DictationState, Severity};
use crate::vumeter;

/// A level push and when it arrived — the VU decays by *arrival age* (R16a).
#[derive(Clone, Copy)]
struct LevelSample {
    rms: f64,
    peak: f64,
    at: Instant,
}

/// One frame's worth of indicator state.
pub struct Frame {
    /// What to draw, per [`crate::hud_logic::indicator_state`].
    pub state: IndicatorState,
    /// How long the current dictation state has lasted, ms.
    pub state_ms: f64,
}

#[derive(Default)]
pub struct Indicator {
    level: Cell<Option<LevelSample>>,
    key: Cell<Option<DictationState>>,
    severity: Cell<Option<Severity>>,
    state_since: Cell<Option<Instant>>,
    /// The desktop's reduce-animation preference — slows the pulse.
    reduced_motion: Cell<bool>,
    /// The last smoothed level and when it was computed (eased toward the
    /// pushed sample by [`crate::hud_logic::smooth_level`]).
    smoothed_level: Cell<f64>,
    last_frame: Cell<Option<Instant>>,
}

impl Indicator {
    /// Record a level push. Never deduplicated — the arrival time is what
    /// keeps a steady voice from decaying (R16a).
    pub fn push_level(&self, rms: f64, peak: f64) {
        self.level.set(Some(LevelSample {
            rms,
            peak,
            at: Instant::now(),
        }));
    }

    /// Snap the next frame straight to the pushed level instead of easing
    /// toward it (`smooth_level` does that on a zero dt).
    pub fn restart_easing(&self) {
        self.last_frame.set(None);
    }

    /// Record the dictation state, restarting the state clock when it
    /// changed.
    pub fn set_state(&self, key: DictationState, severity: Option<Severity>) {
        if self.key.get() == Some(key) && self.severity.get() == severity {
            return;
        }
        self.key.set(Some(key));
        self.severity.set(severity);
        self.state_since.set(Some(Instant::now()));
    }

    /// Record the reduce-animation preference; `false` when unchanged.
    pub fn set_reduced_motion(&self, reduced: bool) -> bool {
        self.reduced_motion.replace(reduced) != reduced
    }

    /// The state to draw now. Advances the level easing.
    pub fn frame(&self) -> Frame {
        let (key, severity, state_ms) = match self.key.get() {
            Some(key) => {
                let since = self.state_since.get().unwrap_or_else(Instant::now);
                (
                    key,
                    self.severity.get(),
                    since.elapsed().as_secs_f64() * 1000.0,
                )
            }
            None => (DictationState::Idle, None, 0.0),
        };
        let reduced_motion = self.reduced_motion.get();
        let intensity = self.smoothed_intensity(reduced_motion);
        Frame {
            state: crate::hud_logic::indicator_state(key, severity, intensity, reduced_motion),
            state_ms,
        }
    }

    fn smoothed_intensity(&self, reduced_motion: bool) -> f64 {
        let now = Instant::now();
        let dt_ms = match self.last_frame.get() {
            Some(prev) => now.duration_since(prev).as_secs_f64() * 1000.0,
            None => 0.0,
        };
        let raw = match self.level.get() {
            Some(LevelSample { rms, peak, at }) => {
                vumeter::levels_to_intensity(rms, peak, at.elapsed().as_secs_f64() * 1000.0)
            }
            None => 0.0,
        };
        let smoothed =
            crate::hud_logic::smooth_level(self.smoothed_level.get(), raw, dt_ms, reduced_motion);
        self.smoothed_level.set(smoothed);
        self.last_frame.set(Some(now));
        smoothed
    }
}
