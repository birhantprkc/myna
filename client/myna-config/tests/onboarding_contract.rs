//! The rules the wizard is built on: what opens it, how each component is
//! installed, and what it must not claim.

use myna_config::diagnostics::InstalledSnap;
use myna_config::onboarding::{
    assess, can_advance, install_commands, needs_onboarding, ComponentId, Machine, Step, MYNA_SNAP,
    RECOMMENDED_BACKEND_SNAP,
};

fn snap(name: &str) -> InstalledSnap {
    InstalledSnap {
        name: name.to_owned(),
        version: "1".to_owned(),
    }
}

#[test]
fn a_machine_with_no_myna_opens_the_wizard() {
    let components = assess(Machine::new(&[], 1));
    assert!(needs_onboarding(&components));
    assert!(components
        .iter()
        .any(|component| component.id == ComponentId::Myna && !component.satisfied));
}

#[test]
fn a_ready_machine_never_opens_the_wizard() {
    let machine = Machine::new(&[snap(MYNA_SNAP)], 1);
    assert!(!needs_onboarding(&assess(machine)));
}

/// snapd refuses to install a snap declaring a user daemon on a stock machine,
/// so App Center's install would fail, and the command sets the flag first.
#[test]
fn myna_is_installed_from_a_terminal_with_user_daemons_enabled() {
    let command = install_commands();
    let flag = command
        .find("experimental.user-daemons=true")
        .expect("the command enables user daemons");
    assert!(flag < command.find("snap install").unwrap());
}

/// Dictation works without the shell extension, and onboarding no longer
/// mentions it: only the two snaps are assessed.
#[test]
fn the_wizard_assesses_only_the_two_snaps() {
    let ids: Vec<ComponentId> = assess(Machine::default())
        .iter()
        .map(|component| component.id)
        .collect();
    assert_eq!(ids, [ComponentId::Myna, ComponentId::Model]);
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
