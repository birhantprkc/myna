//! The rules the wizard is built on: what opens it, how each component is
//! installed, and what it must not claim.

use myna_config::diagnostics::InstalledSnap;
use myna_config::onboarding::{
    assess, can_advance, completes, needs_onboarding, row_action, ComponentId, ExtensionState,
    Machine, RowAction, Step, MYNA_SNAP,
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

/// The wizard installs from its rows; nothing is left for a terminal.
#[test]
fn every_missing_snap_has_an_install_button() {
    for component in assess(Machine {
        user_daemons: true,
        ..Machine::default()
    }) {
        if matches!(component.id, ComponentId::Myna | ComponentId::Model) {
            assert_eq!(row_action(&component), RowAction::Install);
        }
    }
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
