use std::process::Command as Process;

use myna_config::{parse_args, Command, APP_ID, GETTEXT_DOMAIN};

#[test]
fn application_identity_is_stable() {
    assert_eq!(APP_ID, "com.canonical.Myna.Config");
    assert_eq!(GETTEXT_DOMAIN, "myna-config");
}

#[test]
fn no_arguments_launches_the_application() {
    assert_eq!(parse_args(Vec::<String>::new()).unwrap(), Command::Launch);
}

#[test]
fn informational_arguments_are_recognized() {
    assert_eq!(
        parse_args(["--help".to_string()]).unwrap(),
        Command::PrintHelp
    );
    assert_eq!(parse_args(["-h".to_string()]).unwrap(), Command::PrintHelp);
    assert_eq!(
        parse_args(["--version".to_string()]).unwrap(),
        Command::PrintVersion
    );
}

#[test]
fn unknown_arguments_are_rejected() {
    let error = parse_args(["--unknown".to_string()]).unwrap_err();
    assert!(error.contains("--unknown"));
    assert!(error.contains("Usage:"));
}

/// App Center and GNOME Software describe the application from this file.
#[test]
fn appstream_metadata_names_the_application_and_validates() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/data/com.canonical.Myna.Config.metainfo.xml"
    );
    let metainfo = std::fs::read_to_string(path).expect("metainfo");
    assert!(metainfo.contains(&format!("<id>{APP_ID}</id>")));
    assert!(metainfo.contains(&format!(
        "<launchable type=\"desktop-id\">{APP_ID}.desktop</launchable>"
    )));
    assert!(metainfo.contains(&format!(
        "<translation type=\"gettext\">{GETTEXT_DOMAIN}</translation>"
    )));

    let output = Process::new("appstreamcli")
        .args(["validate", "--no-net", path])
        .output()
        .expect("run appstreamcli from the appstream package");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
}
