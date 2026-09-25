// tests/simulator.rs — hermetic test for the --serve-dbus simulator's pure
// mapping (lab controls → com.canonical.Myna.Dictation wire properties).
// The drift checks round-trip through the REAL vumeter math to catch
// calibration drift between the slider and the rendered level.

use myna_hud::simulator::{default_status_message, envelope_to_levels, PUBLISH_HZ};
use myna_hud::states::wire;
use myna_hud::vumeter::levels_to_intensity;

// --- default_status_message: the publisher's content-free labels ---------

#[test]
fn default_status_messages_match_the_publisher_contract() {
    let cases = [
        (wire::IDLE, ""),
        (wire::LOADING, "Loading model…"),
        (wire::RECORDING, "Listening"),
        (wire::TRANSCRIBING, "Transcribing"),
        (wire::FINALIZING, "Finishing"),
        (wire::NOTICE, "No speech detected"),
        (wire::ERROR, "Error: Microphone unavailable"),
        ("quantizing", "Active"),
    ];
    for (state, message) in cases {
        assert_eq!(default_status_message(state), message, "{state}");
    }
}

// --- envelope_to_levels: invert the vumeter so the slider is 1:1 ----------

#[test]
fn envelope_round_trips_through_the_real_vumeter() {
    // The slider is the smoothed envelope; the wire carries raw RMS/peak
    // which the consumer pushes back through levels_to_intensity. This
    // deliberate transcription of the calibration is what makes the lab's
    // preview and the hosted HUD agree — and what catches drift if the
    // vumeter constants ever change without the simulator following.
    for level in [0.05, 0.2, 0.5, 0.8, 1.0] {
        let (rms, peak) = envelope_to_levels(level);
        let intensity = levels_to_intensity(rms, peak, 0.0);
        assert!(
            (intensity - level).abs() < 1e-9,
            "slider {level} → rms {rms:.4} → intensity {intensity:.4}"
        );
    }
}

#[test]
fn envelope_zero_is_zero_and_values_are_legal() {
    assert_eq!(envelope_to_levels(0.0), (0.0, 0.0));
    for level in [-0.5, 0.5, 1.7] {
        let (rms, peak) = envelope_to_levels(level);
        assert!(
            (0.0..=1.0).contains(&rms) && (0.0..=1.0).contains(&peak),
            "clamped inputs stay in range"
        );
    }
}

#[test]
fn publish_rate_matches_the_contract_cadence() {
    // ~15-20 Hz per C4 — 20 keeps the consumer seeing the update rate it
    // was tuned against rather than the lab's render-loop rate.
    assert!((15.0..=20.0).contains(&PUBLISH_HZ));
}
