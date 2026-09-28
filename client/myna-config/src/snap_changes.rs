//! The snapd changes still running, as `snap changes --abs-time` lists them.
//!
//! snapd makes a connection visible before the change that made it has
//! finished: an install auto-connects `myna:backend` early, then fetches the
//! model and only at the end applies the mount to Myna's namespace. Setting up
//! waits for such a change before acting on what it sees.

/// One change whose Ready column is `-`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnapChange {
    summary: String,
}

impl SnapChange {
    pub fn summary(&self) -> &str {
        &self.summary
    }

    /// Whether the summary names one of `snaps`. snapd writes names quoted
    /// (`Install "myna-parakeet" snap`), as plugs and slots
    /// (`Connect myna:backend to myna-parakeet:provider`) or as components
    /// (`myna-parakeet+model`), so any run of snap-name characters counts.
    pub fn touches<S: AsRef<str>>(&self, snaps: &[S]) -> bool {
        self.summary
            .split(|c: char| !(c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'))
            .any(|word| snaps.iter().any(|snap| snap.as_ref() == word))
    }
}

/// The changes not yet ready. Anything that is not a change row, including
/// the header and snapd's "no changes found", is skipped.
pub fn parse_in_progress(output: &str) -> Vec<SnapChange> {
    output
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let id = fields.next()?;
            let _status = fields.next()?;
            let _spawn = fields.next()?;
            let ready = fields.next()?;
            let summary = fields.collect::<Vec<_>>().join(" ");
            (id.bytes().all(|byte| byte.is_ascii_digit()) && ready == "-")
                .then_some(SnapChange { summary })
        })
        .collect()
}
