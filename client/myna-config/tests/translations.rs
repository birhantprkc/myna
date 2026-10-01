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

/// Every msgid in the template, continuation lines joined.
fn template_msgids() -> Vec<String> {
    let template = include_str!("../po/myna-config.pot");
    let mut msgids = Vec::new();
    let mut current: Option<String> = None;
    for line in template.lines() {
        let quoted = |rest: &str| rest.trim().trim_matches('"').to_owned();
        if let Some(rest) = line
            .strip_prefix("msgid_plural ")
            .or_else(|| line.strip_prefix("msgid "))
        {
            msgids.extend(current.take());
            current = Some(quoted(rest));
        } else if line.starts_with('"') {
            if let Some(text) = current.as_mut() {
                text.push_str(&quoted(line));
            }
        } else {
            msgids.extend(current.take());
        }
    }
    msgids.extend(current);
    msgids.retain(|msgid| !msgid.is_empty());
    msgids
}

// Users choose among speech models; "backend" is the code's word for them.
#[test]
fn user_visible_strings_say_model_and_use_no_em_dash() {
    let msgids = template_msgids();
    assert!(msgids.len() > 100, "parsed only {} msgids", msgids.len());
    let manual = include_str!("../data/myna-config.1");
    for text in msgids.iter().map(String::as_str).chain([manual]) {
        assert!(!text.to_lowercase().contains("backend"), "{text}");
        assert!(!text.contains('\u{2014}'), "{text}");
    }
}

/// The Model tab's rows are built from `presentation.rs`, and an apply
/// reports in `backend_apply.rs`: both reach the template.
#[test]
fn the_model_tab_settings_and_apply_reports_are_translatable() {
    use myna_config::domain::ConfigValue;
    use myna_config::presentation::{metadata_for, Validation};

    let msgids = template_msgids();
    let mut expected = Vec::new();
    for key in [
        "model",
        "engine",
        "streaming",
        "sleep-idle-seconds",
        "verbose",
        "ws.unix-socket",
        "stream-arm-seconds",
        "stream-silence-cut-seconds",
        "stream-force-cut-seconds",
        "stream-partial-cadence-seconds",
        "stream-partial-tail-seconds",
        "compute-type",
        "att-context-size",
    ] {
        let metadata = metadata_for(key, &ConfigValue::Null);
        expected.push(metadata.title().to_owned());
        expected.push(metadata.explanation().to_owned());
    }
    let unknown = metadata_for("future-setting", &ConfigValue::Text(String::new()));
    expected.push(unknown.explanation().to_owned());
    for validation in [
        Validation::Boolean,
        Validation::Number,
        Validation::Text,
        Validation::PositiveNumber,
        Validation::NonNegativeNumber,
        Validation::NonNegativeInteger,
        Validation::ReadOnly,
    ] {
        expected.push(validation.validate(&ConfigValue::Null).unwrap_err());
    }
    expected.extend(
        [
            "value is invalid",
            "value must be one of {choices}",
            "value must be a finite number",
            "selector value must be text",
            "the model's settings command could not be found",
            "the model's restart command could not be built",
            "The model did not restart.",
            "The model's restart could not be confirmed.",
        ]
        .map(str::to_owned),
    );
    for text in expected {
        assert!(msgids.contains(&text), "{text:?} is not in the template");
    }
}
