//! Per-model, per-hardware RTF tier assessment for the streaming gate (T039/T040).
//!
//! A [`TierAssessment`] is a measured RTF for one model on one machine,
//! recorded in `results/streaming-tiers.json` by the lab (myna-bench run) and
//! shipped as a static data file. The gate (FR-002): streaming is viable only
//! when an RTF recorded on this hardware is below the threshold (~1.0); no
//! measurement → batch (safe default, FR-010).

use serde::Deserialize;

/// Default RTF threshold: the model must process audio faster than it arrives.
pub const DEFAULT_RTF_THRESHOLD: f64 = 1.0;

/// One measured model×hardware data point (data-model.md, feature 007). The
/// file's other fields (`model`, `strategy`, `measured_at`) are not read: the
/// gate is per hardware.
#[derive(Debug, Deserialize)]
pub struct TierAssessment {
    pub hardware: String,
    pub rtf: f64,
}

/// The tier table: assessments loaded from the shipped baseline file.
#[derive(Debug, Default, Deserialize)]
pub struct TierTable {
    #[serde(default)]
    pub assessments: Vec<TierAssessment>,
}

impl TierTable {
    /// Parse from the JSON baseline file shape.
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }
}

/// FR-002: the streaming gate. An RTF below the threshold → streaming; at or
/// above, or unmeasured → batch (safe default). The model axis is left open:
/// which model the server serves is not knowable before a session opens, so
/// take the most permissive outcome over every model measured on this
/// hardware. Safe because the server gates
/// itself as well - a batch-only backend simply never emits `Unstable`.
pub fn streaming_viable_here(table: &TierTable, hardware: &str, threshold: f64) -> bool {
    table
        .assessments
        .iter()
        .any(|a| a.hardware == hardware && a.rtf < threshold)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table() -> TierTable {
        TierTable {
            assessments: vec![
                TierAssessment {
                    hardware: "gpu-rtx".into(),
                    rtf: 0.3,
                },
                TierAssessment {
                    hardware: "cpu-i5".into(),
                    rtf: 1.4,
                },
            ],
        }
    }

    /// T037/T044: the threshold is strict, and no measurement is batch.
    #[test]
    fn rtf_at_threshold_or_unmeasured_forces_batch() {
        // RTF == 1.0 means inference keeps pace exactly — no headroom for the
        // committed frontier to stay ahead. Batch.
        let t = TierTable {
            assessments: vec![TierAssessment {
                hardware: "h".into(),
                rtf: 1.0,
            }],
        };
        assert!(!streaming_viable_here(&t, "h", DEFAULT_RTF_THRESHOLD));
        assert!(!streaming_viable_here(
            &TierTable::default(),
            "gpu-rtx",
            DEFAULT_RTF_THRESHOLD
        ));
    }

    #[test]
    fn any_measured_model_on_this_hardware_opens_the_gate() {
        // gpu-rtx measured under the threshold, cpu-i5 over it.
        assert!(streaming_viable_here(
            &table(),
            "gpu-rtx",
            DEFAULT_RTF_THRESHOLD
        ));
        assert!(!streaming_viable_here(
            &table(),
            "cpu-i5",
            DEFAULT_RTF_THRESHOLD
        ));
    }

    #[test]
    fn unmeasured_hardware_stays_batch() {
        assert!(!streaming_viable_here(
            &table(),
            "unmeasured",
            DEFAULT_RTF_THRESHOLD
        ));
    }
}
