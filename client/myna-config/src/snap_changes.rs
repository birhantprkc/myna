//! The snapd changes still running, as `GET /v2/changes?select=in-progress`
//! lists them, read as the user.
//!
//! snapd makes a connection visible before the change that made it has
//! finished: an install auto-connects `myna:backend` early, then fetches the
//! model and only at the end applies the mount to Myna's namespace. Setting up
//! waits for such a change before acting on what it sees, and an apply shows
//! what its privileged plan is waiting on; a model download above all.

use serde::Deserialize;

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
    pub fn summary(&self) -> &str {
        &self.summary
    }

    /// Whether it changes one of `snaps`. snapd writes names in summaries
    /// quoted (`Install "myna-parakeet" snap`), as plugs and slots
    /// (`Connect myna:backend to myna-parakeet:provider`) or as components
    /// (`myna-parakeet+model`), so any run of snap-name characters counts.
    /// Its tasks name the snaps they affect too; snapd's `for=` filter does
    /// not match a `snapctl-install` change, so this is decided here.
    pub fn concerns<S: AsRef<str>>(&self, snaps: &[S]) -> bool {
        names(&self.summary, snaps)
            || self.tasks.iter().any(|task| {
                task.data
                    .affected_snaps
                    .iter()
                    .any(|name| snaps.iter().any(|snap| snap.as_ref() == name))
            })
    }

    /// Its download while one runs, else its summary.
    pub fn progress(&self) -> ApplyProgress {
        let download = self.tasks.iter().find(|task| {
            task.status == "Doing"
                && task.kind.starts_with("download-")
                && task.progress.total > 0
                && !task.progress.label.is_empty()
        });
        match download {
            Some(task) => {
                // A component's label is `snap+component`, a snap's its name.
                let label = task.progress.label.as_str();
                let name = label
                    .split_once('+')
                    .map_or(label, |(_, component)| component);
                ApplyProgress::Download {
                    name: name.to_owned(),
                    done: task.progress.done.min(task.progress.total),
                    total: task.progress.total,
                }
            }
            None => ApplyProgress::Change {
                summary: self.summary.clone(),
            },
        }
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
    changes
        .iter()
        .find(|change| change.concerns(&[snap]))
        .map(ChangeInProgress::progress)
}
