use myna_config::snap_changes::parse_in_progress;

/// `snap changes --abs-time` on the machine the race was seen on, seconds
/// after the model's install change auto-connected `myna:backend`.
const INSTALLING: &str = include_str!("fixtures/snap-changes-installing.txt");

#[test]
fn only_changes_not_ready_are_in_progress() {
    let pending = parse_in_progress(INSTALLING);
    assert_eq!(pending.len(), 1);
    assert_eq!(
        pending[0].summary(),
        "Install \"myna-parakeet\" snap from \"edge\" channel"
    );
    assert!(parse_in_progress("").is_empty());
    assert!(parse_in_progress("no changes found\n").is_empty());
}

#[test]
fn a_change_touches_the_snaps_its_summary_names() {
    let change = |summary: &str| {
        parse_in_progress(&format!("1 Doing 2026-09-28T09:37:28+01:00 - {summary}"))
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
        assert!(change(summary).touches(&ours), "{summary}");
    }
    for summary in [
        "Auto-refresh snap \"firefox\"",
        "Install \"myna-whisper\" snap",
        "Install \"mynah\" snap",
    ] {
        assert!(!change(summary).touches(&ours), "{summary}");
    }
}
