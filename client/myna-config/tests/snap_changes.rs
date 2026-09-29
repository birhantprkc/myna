use myna_config::snap_changes::{apply_progress, parse_changes, ApplyProgress};

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
