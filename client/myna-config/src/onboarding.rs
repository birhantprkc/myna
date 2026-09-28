//! What a machine is missing before dictation works, and the three-step flow
//! that fixes it.
//!
//! GTK-independent so it can be exercised headlessly. The rules here decide
//! *what* is missing and how the user installs it; the strings the user reads
//! live in `onboarding_ui`.

use crate::diagnostics::InstalledSnap;

/// The client snap.
pub const MYNA_SNAP: &str = "myna";

/// The recommended backend. Its install hook selects an engine, which installs
/// the matching model component, so a plain install is a working backend.
pub const RECOMMENDED_BACKEND_SNAP: &str = "myna-parakeet";

/// One thing onboarding checks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ComponentId {
    /// The dictation client itself.
    Myna,
    /// A speech-to-text backend with its model.
    Model,
}

/// Installs Myna from a terminal. The flag comes first: snapd refuses a snap
/// declaring a user daemon unless `experimental.user-daemons` is set or its
/// snap-id is on snapd's hardcoded allowlist.
pub const MYNA_INSTALL_COMMAND: &str =
    "sudo snap set system experimental.user-daemons=true\nsudo snap install --edge myna";

/// Installs the recommended backend from a terminal.
pub const MODEL_INSTALL_COMMAND: &str = "sudo snap install --edge myna-parakeet";

/// Everything the component step asks the user to paste, one command per
/// line. It always lists every command: rerunning one is harmless.
pub fn install_commands() -> String {
    format!("{MYNA_INSTALL_COMMAND}\n{MODEL_INSTALL_COMMAND}")
}

/// One assessed component.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Component {
    pub id: ComponentId,
    pub satisfied: bool,
}

/// The observations onboarding is assessed from. `backend_discovered` is
/// discovery's answer, not a name match on the snap inventory: which snaps are
/// backends is a property of the interfaces they publish.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Machine {
    pub myna_installed: bool,
    pub backend_discovered: bool,
}

impl Machine {
    pub fn new(installed_snaps: &[InstalledSnap], backends_discovered: usize) -> Self {
        Self {
            myna_installed: installed_snaps.iter().any(|snap| snap.name == MYNA_SNAP),
            backend_discovered: backends_discovered > 0,
        }
    }
}

/// Assess every component dictation needs.
pub fn assess(machine: Machine) -> Vec<Component> {
    vec![
        Component {
            id: ComponentId::Myna,
            satisfied: machine.myna_installed,
        },
        Component {
            id: ComponentId::Model,
            satisfied: machine.backend_discovered,
        },
    ]
}

/// Whether the wizard should open at all.
pub fn needs_onboarding(components: &[Component]) -> bool {
    components.iter().any(|component| !component.satisfied)
}

/// The wizard's steps, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Step {
    Welcome,
    Components,
    Shortcut,
}

impl Step {
    pub const fn first() -> Self {
        Self::Welcome
    }

    pub const fn next(self) -> Option<Self> {
        match self {
            Self::Welcome => Some(Self::Components),
            Self::Components => Some(Self::Shortcut),
            Self::Shortcut => None,
        }
    }
}

/// Whether the step's forward button is sensitive. Only the component step
/// gates: every component must be satisfied before the flow can
/// claim dictation is set up.
pub fn can_advance(step: Step, components: &[Component]) -> bool {
    match step {
        Step::Welcome | Step::Shortcut => true,
        Step::Components => !needs_onboarding(components),
    }
}

/// Whether the step re-reads the machine on its own. The component step does
/// while something is missing: the user installs in another window, which the
/// wizard may never lose focus to.
pub fn polls(step: Step, components: &[Component]) -> bool {
    step == Step::Components && needs_onboarding(components)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(name: &str) -> InstalledSnap {
        InstalledSnap {
            name: name.to_owned(),
            version: "1".to_owned(),
        }
    }

    #[test]
    fn a_bare_machine_needs_onboarding() {
        assert!(needs_onboarding(&assess(Machine::new(&[], 0))));
    }

    #[test]
    fn an_installed_backend_snap_is_not_a_discovered_backend() {
        // The snap being on disk says nothing about it publishing the socket
        // interface; discovery is the only source for that.
        let machine = Machine::new(&[snap("myna"), snap("myna-parakeet")], 0);
        assert!(!machine.backend_discovered);
        assert!(needs_onboarding(&assess(machine)));
    }

    #[test]
    fn the_component_step_gates_on_every_component() {
        let bare = assess(Machine::default());
        assert!(can_advance(Step::Welcome, &bare));
        assert!(!can_advance(Step::Components, &bare));

        let model_missing = assess(Machine::new(&[snap("myna")], 0));
        assert!(!can_advance(Step::Components, &model_missing));
        let ready = assess(Machine::new(&[snap("myna")], 1));
        assert!(can_advance(Step::Components, &ready));
    }

    #[test]
    fn only_the_component_step_polls_and_only_while_something_is_missing() {
        let bare = assess(Machine::default());
        let ready = assess(Machine::new(&[snap("myna")], 1));
        assert!(polls(Step::Components, &bare));
        assert!(!polls(Step::Components, &ready));
        assert!(!polls(Step::Welcome, &bare));
        assert!(!polls(Step::Shortcut, &bare));
    }

    #[test]
    fn the_steps_form_one_ordered_walk() {
        let mut step = Step::first();
        let mut walked = vec![step];
        while let Some(next) = step.next() {
            step = next;
            walked.push(step);
        }
        assert_eq!(
            walked,
            vec![Step::Welcome, Step::Components, Step::Shortcut]
        );
    }

    #[test]
    fn the_install_commands_are_the_three_lines_the_user_pastes() {
        assert_eq!(
            install_commands(),
            "sudo snap set system experimental.user-daemons=true\n\
             sudo snap install --edge myna\n\
             sudo snap install --edge myna-parakeet"
        );
    }
}
