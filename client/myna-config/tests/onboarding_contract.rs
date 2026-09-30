//! The rules the wizard is built on: what opens it, how each component is
//! installed, and what it must not claim.

use myna_config::diagnostics::InstalledSnap;
use myna_config::onboarding::{
    assess, can_advance, completes, install_commands, needs_onboarding, ComponentId,
    ExtensionState, Machine, Step, MYNA_SNAP, RECOMMENDED_BACKEND_SNAP,
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
        .any(|component| component.id == ComponentId::Myna && !component.satisfied()));
}

#[test]
fn a_ready_machine_never_opens_the_wizard() {
    let machine = Machine {
        user_daemons: true,
        ..Machine::new(&[snap(MYNA_SNAP)], 1)
    };
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

/// The flag, both snaps, and the extension, in the order the step lists
/// them.
#[test]
fn the_wizard_assesses_the_flag_both_snaps_and_the_extension() {
    let ids: Vec<ComponentId> = assess(Machine::default())
        .iter()
        .map(|component| component.id)
        .collect();
    assert_eq!(
        ids,
        [
            ComponentId::UserDaemons,
            ComponentId::Myna,
            ComponentId::Model,
            ComponentId::ShellExtension
        ]
    );
}

/// Dictation works without the extension, falling back to notifications, so
/// a machine without it neither opens the wizard nor holds Next; only
/// installing it too moves the step on by itself.
#[test]
fn the_extension_is_optional_but_completes_the_step() {
    let ready = |extension| {
        assess(Machine {
            user_daemons: true,
            extension,
            ..Machine::new(&[snap(MYNA_SNAP)], 1)
        })
    };
    let without = ready(ExtensionState::Unavailable);
    assert!(!needs_onboarding(&without));
    assert!(can_advance(Step::Components, &without));
    assert!(!completes(&assess(Machine::default()), &without));
    assert!(completes(
        &assess(Machine::default()),
        &ready(ExtensionState::Enabled)
    ));
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
