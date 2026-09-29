//! The snapd changes still running, as `snap changes --abs-time` lists them.
//!
//! snapd makes a connection visible before the change that made it has
//! finished: an install auto-connects `myna:backend` early, then fetches the
//! model and only at the end applies the mount to Myna's namespace. Setting up
//! waits for such a change before acting on what it sees.
//!
//! An apply reads the same changes from snapd's REST API instead, to show
//! what the privileged plan is waiting on, a model download above all.

use serde::Deserialize;

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
        names(&self.summary, snaps)
    }
}

fn names<S: AsRef<str>>(summary: &str, snaps: &[S]) -> bool {
    summary
        .split(|c: char| !(c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'))
        .any(|word| snaps.iter().any(|snap| snap.as_ref() == word))
}

/// A change as `GET /v2/changes?select=in-progress` lists it.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct ChangeInProgress {
    summary: String,
    #[serde(default)]
    tasks: Vec<ChangeTask>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
struct ChangeTask {
    kind: String,
    status: String,
    #[serde(default)]
    progress: TaskProgress,
    #[serde(default)]
    data: TaskData,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
struct TaskProgress {
    #[serde(default)]
    label: String,
    #[serde(default)]
    done: u64,
    #[serde(default)]
    total: u64,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
struct TaskData {
    #[serde(default, rename = "affected-snaps")]
    affected_snaps: Vec<String>,
}

impl ChangeInProgress {
    /// Its tasks name the snaps they affect; snapd's `for=` filter does not
    /// match a `snapctl-install` change, so this is decided here.
    fn concerns(&self, snap: &str) -> bool {
        names(&self.summary, &[snap])
            || self
                .tasks
                .iter()
                .any(|task| task.data.affected_snaps.iter().any(|name| name == snap))
    }
}

/// The `result` of a changes listing.
pub fn parse_changes(result: serde_json::Value) -> Result<Vec<ChangeInProgress>, String> {
    serde_json::from_value(result).map_err(|error| error.to_string())
}

/// What an apply on one backend is waiting on in snapd.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ApplyProgress {
    /// Fetching `name`, a component such as `model-small` or a snap.
    Download { name: String, done: u64, total: u64 },
    /// Any other step, as snapd summarises the change.
    Change { summary: String },
}

/// The first change on `snap` still running: its download while one runs,
/// else its summary.
pub fn apply_progress(changes: &[ChangeInProgress], snap: &str) -> Option<ApplyProgress> {
    let change = changes.iter().find(|change| change.concerns(snap))?;
    let download = change.tasks.iter().find(|task| {
        task.status == "Doing" && task.kind.starts_with("download-") && task.progress.total > 0
    });
    Some(match download {
        Some(task) => {
            let label = task.progress.label.as_str();
            let name = label
                .strip_prefix(snap)
                .and_then(|rest| rest.strip_prefix('+'))
                .unwrap_or(label);
            ApplyProgress::Download {
                name: if name.is_empty() { snap } else { name }.to_owned(),
                done: task.progress.done.min(task.progress.total),
                total: task.progress.total,
            }
        }
        None => ApplyProgress::Change {
            summary: change.summary.clone(),
        },
    })
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
