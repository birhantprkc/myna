use myna_config::snap_changes::{
    apply_progress, parse_change, parse_changes, pending_install, ApplyProgress,
};

#[test]
fn a_change_concerns_the_snaps_its_summary_names() {
    let change = |summary: &str| {
        parse_changes(serde_json::json!([{ "summary": summary }]))
            .unwrap()
            .pop()
            .unwrap()
    };
    let ours = ["myna", "myna-parakeet"];
    for summary in [
        "Install \"myna-parakeet\" snap from \"edge\" channel",
        "Auto-refresh snaps \"firefox\", \"myna\"",
        "Connect myna:backend to myna-parakeet:provider",
        "Install component \"myna-parakeet+model-parakeet-int8\"",
    ] {
        assert!(change(summary).concerns(&ours), "{summary}");
    }
    for summary in [
        "Auto-refresh snap \"firefox\"",
        "Install \"myna-whisper\" snap",
        "Install \"mynah\" snap",
    ] {
        assert!(!change(summary).concerns(&ours), "{summary}");
    }
}

/// `GET /v2/changes?select=in-progress` on Noble while `use-model small`
/// fetched its component. `for=myna-whisper` returns nothing for this change:
/// snapd cannot name the snap of a `snapctl-install` change.
const DOWNLOADING: &str = include_str!("fixtures/snapd-changes-component-download.json");

fn downloading() -> serde_json::Value {
    serde_json::from_str::<serde_json::Value>(DOWNLOADING).unwrap()["result"].take()
}

#[test]
fn a_component_download_reports_its_bytes() {
    let changes = parse_changes(downloading()).unwrap();

    assert_eq!(
        apply_progress(&changes, "myna-whisper"),
        Some(ApplyProgress::Download {
            name: "model-small".to_owned(),
            done: 13_718_564,
            total: 483_966_976,
        })
    );
}

#[test]
fn another_snaps_change_is_no_progress() {
    let changes = parse_changes(downloading()).unwrap();

    assert_eq!(apply_progress(&changes, "myna-parakeet"), None);
    assert_eq!(apply_progress(&[], "myna-whisper"), None);
}

#[test]
fn a_change_with_no_download_running_reports_its_summary() {
    let mut result = downloading();
    result[0]["tasks"][0]["status"] = "Done".into();
    let changes = parse_changes(result).unwrap();

    assert_eq!(
        apply_progress(&changes, "myna-whisper"),
        Some(ApplyProgress::Change {
            summary: "Installing components [model-small] for snap myna-whisper".to_owned(),
        })
    );
}

#[test]
fn a_change_is_ours_when_its_summary_names_the_snap() {
    let changes = parse_changes(serde_json::json!([
        {"summary": "Refresh \"myna-whisper\" snap", "tasks": [
            {"kind": "download-snap", "status": "Doing",
             "progress": {"label": "myna-whisper", "done": 5, "total": 10}}
        ]}
    ]))
    .unwrap();

    assert_eq!(
        apply_progress(&changes, "myna-whisper"),
        Some(ApplyProgress::Download {
            name: "myna-whisper".to_owned(),
            done: 5,
            total: 10,
        })
    );
}

#[test]
fn a_download_with_no_label_reports_its_summary() {
    let mut result = downloading();
    result[0]["tasks"][0]["progress"]["label"] = "".into();
    let changes = parse_changes(result).unwrap();

    assert_eq!(
        apply_progress(&changes, "myna-whisper"),
        Some(ApplyProgress::Change {
            summary: "Installing components [model-small] for snap myna-whisper".to_owned(),
        })
    );
}

#[test]
fn an_unreadable_change_list_is_an_error() {
    assert!(parse_changes(serde_json::json!({"not": "a list"})).is_err());
}

/// `GET /v2/changes/{id}` for `myna-parakeet` on Noble, as R0 logged it: the
/// snap is fetched, then the install hook's engine choice fetches the model
/// component in the same change.
fn parakeet_install(snap: (u64, u64), model: Option<(u64, u64)>) -> serde_json::Value {
    let snap_status = if model.is_some() || snap.0 == snap.1 {
        "Done"
    } else {
        "Doing"
    };
    let task = |kind: &str, status: &str, label: &str, (done, total): (u64, u64)| {
        serde_json::json!({"kind": kind, "status": status,
            "progress": {"label": label, "done": done, "total": total}})
    };
    let mut tasks = vec![
        task("download-snap", snap_status, "myna-parakeet", snap),
        task("mount-snap", "Done", "", (1, 1)),
        task("run-hook", "Done", "", (1, 1)),
    ];
    if let Some(model) = model {
        tasks.push(task(
            "download-component",
            "Doing",
            "myna-parakeet+model-parakeet-int8",
            model,
        ));
    }
    serde_json::json!({"id": "324", "kind": "install-snap", "ready": false, "status": "Doing",
        "summary": "Install \"myna-parakeet\" snap from \"latest/edge\" channel",
        "tasks": tasks})
}

const PARAKEET_INT8: u64 = 775_593_984;

#[test]
fn an_install_counts_every_download_of_its_change() {
    let change = parse_change(parakeet_install(
        (46_194_688, 46_194_688),
        Some((364_699_648, 729_399_296)),
    ))
    .unwrap();

    assert_eq!(change.id(), "324");
    assert!(!change.ready());
    assert_eq!(change.download_percent(PARAKEET_INT8), Some(52));
}

/// The model's size is known only once its task starts, so the expected
/// total keeps the snap's own download from reading as the whole install.
#[test]
fn an_install_is_measured_against_what_it_is_expected_to_fetch() {
    let change = parse_change(parakeet_install((23_097_344, 46_194_688), None)).unwrap();
    assert_eq!(change.download_percent(PARAKEET_INT8), Some(2));

    // A bigger download than expected is measured against itself.
    let change = parse_change(parakeet_install((10, 20), None)).unwrap();
    assert_eq!(change.download_percent(4), Some(50));
}

/// Mounting, hooks and services follow the downloads for 15 s or more; a
/// percentage then would read as done or stuck.
#[test]
fn an_install_between_downloads_has_no_percentage() {
    let change = parse_change(parakeet_install((46_194_688, 46_194_688), None)).unwrap();
    assert_eq!(change.download_percent(PARAKEET_INT8), None);
}

/// A download served from snapd's cache, or not started, shows 1/1 with no
/// label: no bytes to count.
#[test]
fn an_install_with_no_bytes_to_count_has_no_percentage() {
    let change = parse_change(parakeet_install((1, 1), Some((0, 1)))).unwrap();
    assert_eq!(change.download_percent(PARAKEET_INT8), None);
}

#[test]
fn a_finished_change_carries_its_error() {
    let mut result = parakeet_install((1, 1), None);
    result["ready"] = true.into();
    result["status"] = "Error".into();
    result["err"] = "cannot perform the following tasks:\n- Run install hook".into();
    let change = parse_change(result).unwrap();

    assert!(change.ready());
    assert_eq!(
        change.err(),
        Some("cannot perform the following tasks:\n- Run install hook")
    );
}

/// An install started in another window, or by a wizard since closed, is
/// followed rather than started again.
#[test]
fn a_running_install_of_the_snap_is_found() {
    let changes = parse_changes(serde_json::json!([
        parakeet_install((1, 1), None),
        {"id": "7", "kind": "install-snap", "ready": false,
         "summary": "Install \"myna\" snap from \"latest/edge\" channel"},
    ]))
    .unwrap();

    assert_eq!(pending_install(&changes, "myna").map(|c| c.id()), Some("7"));
    assert_eq!(
        pending_install(&changes, "myna-parakeet").map(|c| c.id()),
        Some("324")
    );
}

/// `select=in-progress` also lists a held auto-refresh of a snap since
/// removed: ready, status Hold, never finishing (stonking, change 2100).
#[test]
fn only_an_unfinished_install_of_that_snap_counts() {
    let changes = parse_changes(serde_json::json!([
        {"id": "2100", "kind": "auto-refresh", "ready": true, "status": "Hold",
         "summary": "Auto-refresh snap \"myna\""},
        {"id": "3", "kind": "install-snap", "ready": true, "status": "Done",
         "summary": "Install \"myna\" snap"},
        {"id": "4", "kind": "connect-snap", "ready": false,
         "summary": "Connect myna:backend to myna-parakeet:provider"},
        {"id": "5", "kind": "install-snap", "ready": false,
         "summary": "Install \"myna-parakeet\" snap"},
    ]))
    .unwrap();

    assert_eq!(pending_install(&changes, "myna").map(|c| c.id()), None);
}

/// A download task queued but not sized yet has no bytes to show.
#[test]
fn a_download_of_unknown_size_reports_its_summary() {
    let mut result = downloading();
    result[0]["tasks"][0]["progress"]["total"] = 0.into();
    result[0]["tasks"][0]["progress"]["done"] = 0.into();
    let changes = parse_changes(result).unwrap();

    assert_eq!(
        apply_progress(&changes, "myna-whisper"),
        Some(ApplyProgress::Change {
            summary: "Installing components [model-small] for snap myna-whisper".to_owned(),
        })
    );
}

/// Change 2100 on stonking: an auto-refresh held while Myna ran, whose snap
/// was then removed. snapd still lists it under `select=in-progress`.
#[test]
fn a_held_change_snapd_has_finished_is_not_in_progress() {
    let changes = parse_changes(serde_json::json!([{
        "id": "2100",
        "kind": "auto-refresh",
        "summary": "Auto-refresh snap \"myna\"",
        "status": "Hold",
        "ready": true,
        "tasks": [],
    }]))
    .unwrap();

    assert!(changes.is_empty(), "{changes:?}");
}
