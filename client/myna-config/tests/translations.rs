//! `build/install-translations.sh`, which the deb build runs to ship the
//! catalogs and translate the desktop entry and AppStream metainfo.

use std::path::{Path, PathBuf};
use std::process::Command;

const ID: &str = "com.canonical.Myna.Config";

fn scratch(tag: &str) -> PathBuf {
    let directory = std::env::temp_dir().join(format!("myna-config-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).expect("scratch directory");
    directory
}

fn install(po: &Path, destination: &Path) {
    let output = Command::new("sh")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/build/install-translations.sh"
        ))
        .arg(po)
        .arg(destination)
        .output()
        .expect("run install-translations.sh (needs msgfmt from gettext)");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn read(destination: &Path, path: &str) -> String {
    std::fs::read_to_string(destination.join(path)).expect(path)
}

#[test]
fn a_catalog_translates_the_desktop_entry_and_metainfo() {
    let po = scratch("po");
    let destination = scratch("translated");
    std::fs::write(po.join("LINGUAS"), "# comment\nfr\n").expect("LINGUAS");
    std::fs::write(
        po.join("fr.po"),
        "msgid \"\"\nmsgstr \"Content-Type: text/plain; charset=UTF-8\\n\"\n\n\
         msgid \"Myna Settings\"\nmsgstr \"Paramètres de Myna\"\n\n\
         msgid \"Set up and configure local dictation\"\n\
         msgstr \"Installer et configurer la dictée locale\"\n",
    )
    .expect("fr.po");

    install(&po, &destination);

    assert!(destination
        .join("usr/share/locale/fr/LC_MESSAGES/myna-config.mo")
        .is_file());
    let desktop = read(
        &destination,
        &format!("usr/share/applications/{ID}.desktop"),
    );
    assert!(desktop.contains("Name[fr]=Paramètres de Myna"), "{desktop}");
    let metainfo = read(
        &destination,
        &format!("usr/share/metainfo/{ID}.metainfo.xml"),
    );
    assert!(
        metainfo.contains(r#"<name xml:lang="fr">Paramètres de Myna</name>"#),
        "{metainfo}"
    );
    assert!(
        metainfo.contains(
            r#"<summary xml:lang="fr">Installer et configurer la dictée locale</summary>"#
        ),
        "{metainfo}"
    );
    std::fs::remove_dir_all(&po).ok();
    std::fs::remove_dir_all(&destination).ok();
}

#[test]
fn the_shipped_catalogs_install_valid_files() {
    let po = Path::new(env!("CARGO_MANIFEST_DIR")).join("po");
    let destination = scratch("shipped");

    install(&po, &destination);

    let linguas = std::fs::read_to_string(po.join("LINGUAS")).expect("LINGUAS");
    for lang in linguas
        .lines()
        .map(|line| line.split('#').next().unwrap_or_default().trim())
        .filter(|lang| !lang.is_empty())
    {
        assert!(
            destination
                .join(format!(
                    "usr/share/locale/{lang}/LC_MESSAGES/myna-config.mo"
                ))
                .is_file(),
            "{lang} has no compiled catalog"
        );
    }
    let metainfo = destination.join(format!("usr/share/metainfo/{ID}.metainfo.xml"));
    let output = Command::new("appstreamcli")
        .args(["validate", "--no-net"])
        .arg(&metainfo)
        .output()
        .expect("run appstreamcli from the appstream package");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let merged = read(
        &destination,
        &format!("usr/share/metainfo/{ID}.metainfo.xml"),
    );
    assert!(merged.contains(">Canonical</name>"), "{merged}");
    std::fs::remove_dir_all(&destination).ok();
}
