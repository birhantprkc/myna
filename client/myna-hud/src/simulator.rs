//! simulator — the `--serve-dbus` mode's PURE mapping (lab controls →
//! `com.canonical.Myna.Dictation` wire properties; the zbus publisher that
//! consumes it is T132).
//!
//! The simulator makes the lab a stand-in for `myna-desktop --dbus`: it owns
//! the bus name the renderer watches, so a live session shows the real HUD
//! driven by the lab's controls instead of by speech — no microphone, no
//! model, no backend.
//!
//! Nothing here decides how the HUD *looks*: this module maps the lab's
//! controls onto the four wire properties of
//! `specs/004-gnome-shell-indicator/contracts/dbus-interface.md`, and that
//! mapping is the whole content of this file.

use crate::states::wire;
use crate::vumeter::{DB_CEILING, DB_FLOOR};

/// The publish cadence: ~15-20 Hz per the contract's C4, not the lab's
/// render-loop rate, so the consumer sees the update rate it was tuned
/// against.
pub const PUBLISH_HZ: f64 = 20.0;

/// Content-free default labels for the simulator publisher. These match
/// `myna-desktop`'s `StatusMessage` defaults; the lab may override them live
/// to exercise arbitrary publisher text.
pub fn default_status_message(state: &str) -> &'static str {
    match state {
        wire::IDLE => "",
        wire::LOADING => "Loading model…",
        wire::RECORDING => "Listening",
        wire::TRANSCRIBING => "Transcribing",
        wire::FINALIZING => "Finishing",
        wire::NOTICE => "No speech detected",
        wire::ERROR => "Error: Microphone unavailable",
        _ => "Active",
    }
}

/// The vumeter takes `max(rms, peak * 0.55)`, so any peak below
/// `rms / 0.55` leaves RMS in charge. 1.8 keeps a plausible ~5 dB crest
/// above RMS while staying under that limit, so the slider still maps
/// exactly onto the HUD intensity instead of the peak term quietly taking
/// over at the top of the range.
const PEAK_OVER_RMS: f64 = 1.8;

/// Invert the vumeter's `boost_level` so the slider drives the HUD 1:1.
///
/// The lab's slider is the *intensity* the indicators draw, but the wire
/// carries raw RMS and peak, which the consumer pushes back through
/// [`crate::vumeter::levels_to_intensity`]. Publishing the slider value
/// directly would put the lab's preview and the hosted HUD at visibly
/// different levels for the same setting; inverting the
/// calibration here is what makes the two agree (and what catches drift if
/// the vumeter constants ever change without the simulator following).
///
pub fn envelope_to_levels(envelope: f64) -> (f64, f64) {
    let level = envelope.clamp(0.0, 1.0);
    if level <= 0.0 {
        return (0.0, 0.0);
    }
    let db = DB_FLOOR + level * (DB_CEILING - DB_FLOOR);
    let rms = 10f64.powf(db / 20.0).min(1.0);
    (rms, (rms * PEAK_OVER_RMS).min(1.0))
}
