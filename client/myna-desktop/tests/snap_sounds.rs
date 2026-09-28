//! The daemon's snap environment reaches the sound themes the snap mounts:
//! the gnome extension's `sound-themes` plug lands them under
//! `$SNAP/data-dir/sounds`, and the theme search only looks where
//! `XDG_DATA_DIRS` points.

use std::path::{Path, PathBuf};

use myna_desktop::sound::theme::sound_dirs;

/// The checkout. `MYNA_REPO_ROOT` names it when `client/` runs as a copy of
/// its own, which is how cargo-mutants builds.
fn repo_root() -> PathBuf {
    std::env::var_os("MYNA_REPO_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."))
}

/// The block of the `myna` app (the daemon) in snapcraft.yaml.
fn daemon_app(yaml: &str) -> String {
    yaml.lines()
        .skip_while(|line| *line != "  myna:")
        .skip(1)
        .take_while(|line| line.is_empty() || line.starts_with("    "))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn the_daemon_searches_where_the_snap_mounts_the_sound_themes() {
    let yaml = std::fs::read_to_string(repo_root().join("myna-snap/snap/snapcraft.yaml")).unwrap();
    assert!(
        yaml.contains("extensions: [gnome]"),
        "the gnome extension is what declares the sound-themes plug"
    );
    let app = daemon_app(&yaml);
    assert!(
        app.lines()
            .any(|line| line.trim_start().starts_with("- sound-themes")),
        "the daemon plugs sound-themes:\n{app}"
    );
    let data_dirs = app
        .lines()
        .find_map(|line| line.trim_start().strip_prefix("XDG_DATA_DIRS:"))
        .expect("the daemon sets XDG_DATA_DIRS")
        .trim()
        .trim_matches('"')
        .replace("$XDG_DATA_DIRS", "")
        .replace("$SNAP", "/snap/myna/x1");

    assert!(
        sound_dirs(None, None, Some(&data_dirs))
            .contains(&PathBuf::from("/snap/myna/x1/data-dir/sounds")),
        "XDG_DATA_DIRS={data_dirs} misses the sound-themes mount"
    );
}
