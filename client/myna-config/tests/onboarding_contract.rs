//! The rules the wizard is built on: what opens it, how each component is
//! installed, and what it must not claim.

use myna_config::diagnostics::InstalledSnap;
use myna_config::onboarding::{
    assess, can_advance, install_commands, needs_onboarding, outstanding, ComponentId, Machine,
    Step, MYNA_INSTALL_COMMAND, MYNA_SNAP, RECOMMENDED_BACKEND_SNAP,
};

fn snap(name: &str) -> InstalledSnap {
    InstalledSnap {
        name: name.to_owned(),
        version: "1".to_owned(),
    }
}

#[test]
fn a_machine_with_no_myna_opens_the_wizard() {
    let components = assess(Machine::new(&[], 0, true));
    assert!(needs_onboarding(&components));
    assert!(outstanding(&components)
        .iter()
        .any(|component| component.id == ComponentId::Myna));
}

#[test]
fn a_ready_machine_never_opens_the_wizard() {
    let machine = Machine::new(&[snap(MYNA_SNAP)], 1, true);
    assert!(!needs_onboarding(&assess(machine)));
}

/// snapd refuses to install a snap declaring a user daemon on a stock machine,
/// so App Center's install would fail, and the command sets the flag first.
#[test]
fn myna_is_installed_from_a_terminal_with_user_daemons_enabled() {
    let myna = assess(Machine::default())
        .into_iter()
        .find(|component| component.id == ComponentId::Myna)
        .expect("the wizard assesses Myna");
    assert!(myna.required);
    let command = MYNA_INSTALL_COMMAND;
    let flag = command
        .find("experimental.user-daemons=true")
        .expect("the command enables user daemons");
    assert!(flag < command.find("snap install").unwrap());
}

/// The extension is not published anywhere snapd can reach, and dictation
/// works without it, so it must not block the flow.
#[test]
fn the_shell_extension_is_explained_and_does_not_gate_the_flow() {
    let extension = assess(Machine::default())
        .into_iter()
        .find(|component| component.id == ComponentId::ShellExtension)
        .expect("the wizard assesses the shell extension");
    assert!(!extension.required);

    let only_extension_missing = assess(Machine::new(&[snap(MYNA_SNAP)], 1, false));
    assert!(can_advance(Step::Components, &only_extension_missing));
}

#[test]
fn the_component_step_is_the_only_gate() {
    let bare = assess(Machine::default());
    assert!(can_advance(Step::Welcome, &bare));
    assert!(!can_advance(Step::Components, &bare));
    assert!(can_advance(Step::Shortcut, &bare));
}

/// Both snaps are published to edge only; a command without the channel
/// fails with "no stable revision".
#[test]
fn every_store_install_command_asks_for_edge() {
    let installs: Vec<String> = install_commands()
        .lines()
        .filter(|line| line.contains("snap install"))
        .map(str::to_owned)
        .collect();
    assert_eq!(
        installs,
        [MYNA_SNAP, RECOMMENDED_BACKEND_SNAP]
            .map(|name| format!("sudo snap install --edge {name}"))
    );
}
