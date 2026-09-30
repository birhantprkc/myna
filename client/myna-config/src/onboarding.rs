//! What a machine is missing before dictation works, and the three-step flow
//! that fixes it.
//!
//! GTK-independent so it can be exercised headlessly. The rules here decide
//! *what* is missing, which of it the flow waits for, and when it moves on;
//! the strings the user reads live in `onboarding_ui`.

use myna_core::language::ModelFamily;

use crate::diagnostics::InstalledSnap;

/// The client snap.
pub const MYNA_SNAP: &str = "myna";

/// The recommended backend. Its install hook selects an engine, which installs
/// the matching model component, so a plain install is a working backend.
pub const RECOMMENDED_BACKEND_SNAP: &str = "myna-parakeet";

/// The GNOME Shell extension, as `gnome-shell-ubuntu-extensions` ships it.
pub const SHELL_EXTENSION_UUID: &str = "myna-shell@canonical.com";

/// Installs Myna from a terminal. The flag comes first: snapd refuses a snap
/// declaring a user daemon unless `experimental.user-daemons` is set or its
/// snap-id is on snapd's hardcoded allowlist.
pub const MYNA_INSTALL_COMMAND: &str =
    "sudo snap set system experimental.user-daemons=true\nsudo snap install --edge myna";

/// Installs the recommended backend from a terminal.
pub const MODEL_INSTALL_COMMAND: &str = "sudo snap install --edge myna-parakeet";

/// One thing onboarding checks for, in the order the component step lists
/// them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ComponentId {
    /// snapd's `experimental.user-daemons`, without which snapd refuses to
    /// install or refresh Myna.
    UserDaemons,
    /// The dictation client itself.
    Myna,
    /// A speech-to-text backend with its model.
    Model,
    /// Myna's GNOME Shell extension. Dictation works without it, falling back
    /// to desktop notifications.
    ShellExtension,
}

impl ComponentId {
    pub const ALL: [Self; 4] = [
        Self::UserDaemons,
        Self::Myna,
        Self::Model,
        Self::ShellExtension,
    ];

    /// Whether dictation needs it. Only these gate the component step and
    /// open the wizard at startup.
    pub const fn required(self) -> bool {
        !matches!(self, Self::ShellExtension)
    }
}

/// Where one component stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ComponentState {
    Satisfied,
    /// Missing, and the wizard can install or turn it on.
    Missing,
    /// Missing, and out of the wizard's reach.
    Unavailable(Unavailable),
}

/// Why a component is out of the wizard's reach.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unavailable {
    /// Nothing on this system provides it.
    NotInstalled,
    /// Installed after gnome-shell started, which only rescans at login.
    NeedsRelogin,
    /// Hidden by a copy in the user's data dir, which gnome-shell loads
    /// first; a re-login does not help, removing that copy does.
    ShadowedByUserCopy,
    /// The user turned all extensions off (the Extensions app's switch).
    ExtensionsOff,
}

/// One assessed component.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Component {
    pub id: ComponentId,
    pub state: ComponentState,
}

impl Component {
    pub fn satisfied(&self) -> bool {
        self.state == ComponentState::Satisfied
    }
}

/// Where Myna's extension stands, from gnome-shell's view and the disk.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ExtensionState {
    Enabled,
    /// gnome-shell has the system copy and is not running it; enabling it
    /// is one call.
    Disabled,
    /// A system copy is on disk that gnome-shell has not scanned.
    NeedsRelogin,
    /// A system copy is on disk, and a user copy of the same uuid hides it.
    ShadowedByUserCopy,
    /// A system copy gnome-shell will not run while extensions are off.
    TurnedOff,
    /// No system copy, or one gnome-shell cannot run or the user may not
    /// change.
    #[default]
    Unavailable,
}

/// How gnome-shell runs an extension (`GetExtensionInfo`'s `state`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExtensionRun {
    Enabled,
    /// Disabled, or initialized and never enabled.
    Disabled,
    /// Not running because the user turned all extensions off.
    TurnedOff,
    /// Errored, out of date, uninstalled or locked down: enabling it would
    /// not run it.
    Broken,
}

/// What gnome-shell reports for an extension it knows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExtensionInfo {
    /// Installed under a system data directory rather than the user's.
    pub system: bool,
    pub run: ExtensionRun,
}

/// Which copies of the extension are on disk.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ExtensionCopies {
    /// Under a system data dir: what `gnome-shell-ubuntu-extensions` ships.
    pub system: bool,
    /// Under the user's data dir, as the development tree installs it.
    pub user: bool,
}

/// Classify the extension. Only a system copy counts: the user copy the
/// development tree installs is not what ships.
pub fn extension_state(
    reported: Option<ExtensionInfo>,
    on_disk: ExtensionCopies,
) -> ExtensionState {
    match reported {
        Some(ExtensionInfo {
            system: true,
            run: ExtensionRun::Enabled,
        }) => ExtensionState::Enabled,
        Some(ExtensionInfo {
            system: true,
            run: ExtensionRun::Disabled,
        }) => ExtensionState::Disabled,
        Some(ExtensionInfo {
            system: true,
            run: ExtensionRun::TurnedOff,
        }) => ExtensionState::TurnedOff,
        Some(ExtensionInfo {
            system: true,
            run: ExtensionRun::Broken,
        }) => ExtensionState::Unavailable,
        _ if on_disk.system && on_disk.user => ExtensionState::ShadowedByUserCopy,
        _ if on_disk.system => ExtensionState::NeedsRelogin,
        _ => ExtensionState::Unavailable,
    }
}

/// The model the wizard installs and what installing it downloads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModelOffer {
    pub family: ModelFamily,
    pub download_bytes: u64,
    /// The install hook may still pick the CPU engine, as it does for a GPU
    /// with no driver, and download less.
    pub upper_bound: bool,
}

impl ModelOffer {
    pub fn snap(&self) -> &'static str {
        self.family.snap_name()
    }
}

/// What installing Myna downloads.
pub const MYNA_DOWNLOAD_BYTES: u64 = 10_133_504;

const PARAKEET_SNAP_BYTES: u64 = 46_194_688;
const PARAKEET_INT8_BYTES: u64 = 729_399_296;
const PARAKEET_FP32_BYTES: u64 = 2_549_030_912;
const ONNXRUNTIME_CUDA_BYTES: u64 = 1_577_467_904;

/// Parakeet, sized for the engine its install hook will pick: with an
/// NVIDIA GPU it fetches the CUDA runtime and the fp32 model instead of int8.
pub fn model_offer(machine: &Machine) -> ModelOffer {
    let components = if machine.nvidia_gpu {
        ONNXRUNTIME_CUDA_BYTES + PARAKEET_FP32_BYTES
    } else {
        PARAKEET_INT8_BYTES
    };
    ModelOffer {
        family: ModelFamily::Parakeet,
        download_bytes: PARAKEET_SNAP_BYTES + components,
        upper_bound: machine.nvidia_gpu,
    }
}

/// The observations onboarding is assessed from. `backend_discovered` is
/// discovery's answer, not a name match on the snap inventory: which snaps are
/// backends is a property of the interfaces they publish.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Machine {
    pub user_daemons: bool,
    pub myna_installed: bool,
    pub backend_discovered: bool,
    pub extension: ExtensionState,
    pub nvidia_gpu: bool,
}

impl Machine {
    /// The snap observations; the rest defaults to absent.
    pub fn new(installed_snaps: &[InstalledSnap], backends_discovered: usize) -> Self {
        Self {
            myna_installed: installed_snaps.iter().any(|snap| snap.name == MYNA_SNAP),
            backend_discovered: backends_discovered > 0,
            ..Self::default()
        }
    }
}

/// Assess every component, in [`ComponentId::ALL`]'s order.
pub fn assess(machine: Machine) -> Vec<Component> {
    let installed = |satisfied: bool| {
        if satisfied {
            ComponentState::Satisfied
        } else {
            ComponentState::Missing
        }
    };
    ComponentId::ALL
        .into_iter()
        .map(|id| Component {
            id,
            state: match id {
                ComponentId::UserDaemons => installed(machine.user_daemons),
                ComponentId::Myna => installed(machine.myna_installed),
                ComponentId::Model => installed(machine.backend_discovered),
                ComponentId::ShellExtension => match machine.extension {
                    ExtensionState::Enabled => ComponentState::Satisfied,
                    ExtensionState::Disabled => ComponentState::Missing,
                    ExtensionState::NeedsRelogin => {
                        ComponentState::Unavailable(Unavailable::NeedsRelogin)
                    }
                    ExtensionState::ShadowedByUserCopy => {
                        ComponentState::Unavailable(Unavailable::ShadowedByUserCopy)
                    }
                    ExtensionState::TurnedOff => {
                        ComponentState::Unavailable(Unavailable::ExtensionsOff)
                    }
                    ExtensionState::Unavailable => {
                        ComponentState::Unavailable(Unavailable::NotInstalled)
                    }
                },
            },
        })
        .collect()
}

/// The components as the step shows them while snapd installs `installing`:
/// missing until its change is done, whatever a read found half-way through
/// it. A backend's slot is published before its install has fetched the
/// model.
pub fn while_installing(components: &[Component], installing: &[ComponentId]) -> Vec<Component> {
    components
        .iter()
        .map(|component| Component {
            state: if installing.contains(&component.id) {
                ComponentState::Missing
            } else {
                component.state
            },
            ..*component
        })
        .collect()
}

/// The snap a row's Install button installs and what that is expected to
/// download; the flag and the extension are not snaps.
pub fn installs(id: ComponentId, offer: &ModelOffer) -> Option<(&'static str, u64)> {
    match id {
        ComponentId::Myna => Some((MYNA_SNAP, MYNA_DOWNLOAD_BYTES)),
        ComponentId::Model => Some((offer.snap(), offer.download_bytes)),
        ComponentId::UserDaemons | ComponentId::ShellExtension => None,
    }
}

/// Whether a required component is missing: what opens the wizard at
/// startup and holds its component step.
pub fn needs_onboarding(components: &[Component]) -> bool {
    components
        .iter()
        .any(|component| component.id.required() && !component.satisfied())
}

/// Whether every component, the optional ones too, is in place.
pub fn fully_installed(components: &[Component]) -> bool {
    components.iter().all(Component::satisfied)
}

/// Whether a component's row takes input. snapd refuses Myna without the
/// flag, so the whole list waits for it.
pub fn unlocked(id: ComponentId, components: &[Component]) -> bool {
    id == ComponentId::UserDaemons || flag_enabled(components)
}

/// Whether snapd's `experimental.user-daemons` flag is on.
pub fn flag_enabled(components: &[Component]) -> bool {
    components
        .iter()
        .any(|component| component.id == ComponentId::UserDaemons && component.satisfied())
}

/// What a component's row offers in place of a button, or the button.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowAction {
    Install,
    /// The extension's system copy is there but not running.
    Enable,
    Installed,
    Unavailable(Unavailable),
}

/// What the row of a snap or the extension offers; the flag's row is a
/// switch instead.
pub fn row_action(component: &Component) -> RowAction {
    match component.state {
        ComponentState::Satisfied => RowAction::Installed,
        ComponentState::Missing if component.id == ComponentId::ShellExtension => RowAction::Enable,
        ComponentState::Missing => RowAction::Install,
        ComponentState::Unavailable(why) => RowAction::Unavailable(why),
    }
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
/// gates, and only on the required components: the extension is optional.
pub fn can_advance(step: Step, components: &[Component]) -> bool {
    match step {
        Step::Welcome | Step::Shortcut => true,
        Step::Components => !needs_onboarding(components),
    }
}

/// Whether the step re-reads the machine on its own. The component step does
/// while something required is missing: the user installs in another window,
/// which the wizard may never lose focus to.
pub fn polls(step: Step, components: &[Component]) -> bool {
    step == Step::Components && needs_onboarding(components)
}

/// Whether a re-assessment found the last missing component, optional ones
/// included, the moment the component step moves on by itself. Arriving
/// with everything installed is not one, so a re-run of the wizard does not
/// rush past it, and an extension the wizard cannot install leaves the move
/// to Next.
pub fn completes(before: &[Component], after: &[Component]) -> bool {
    !fully_installed(before) && fully_installed(after)
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

    fn ready() -> Machine {
        Machine {
            user_daemons: true,
            ..Machine::new(&[snap("myna")], 1)
        }
    }

    fn with_extension(extension: ExtensionState) -> Vec<Component> {
        assess(Machine {
            extension,
            ..ready()
        })
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
    fn every_component_is_assessed_in_the_order_the_step_lists_them() {
        let ids: Vec<ComponentId> = assess(Machine::default())
            .iter()
            .map(|component| component.id)
            .collect();
        assert_eq!(ids, ComponentId::ALL);
    }

    #[test]
    fn only_the_extension_is_optional() {
        let optional: Vec<ComponentId> = ComponentId::ALL
            .into_iter()
            .filter(|id| !id.required())
            .collect();
        assert_eq!(optional, [ComponentId::ShellExtension]);
    }

    #[test]
    fn the_flag_is_required_even_with_myna_installed() {
        // An installed Myna keeps running with the flag unset, but snapd then
        // refuses its refreshes (measured on Noble).
        let machine = Machine {
            user_daemons: false,
            ..ready()
        };
        assert!(needs_onboarding(&assess(machine)));
    }

    #[test]
    fn a_missing_extension_does_not_open_the_wizard() {
        for extension in [
            ExtensionState::Unavailable,
            ExtensionState::Disabled,
            ExtensionState::NeedsRelogin,
            ExtensionState::ShadowedByUserCopy,
        ] {
            assert!(
                !needs_onboarding(&with_extension(extension)),
                "{extension:?}"
            );
        }
    }

    #[test]
    fn the_component_step_gates_on_the_required_components_only() {
        let bare = assess(Machine::default());
        assert!(can_advance(Step::Welcome, &bare));
        assert!(!can_advance(Step::Components, &bare));

        let model_missing = assess(Machine {
            backend_discovered: false,
            ..ready()
        });
        assert!(!can_advance(Step::Components, &model_missing));
        let flag_missing = assess(Machine {
            user_daemons: false,
            ..ready()
        });
        assert!(!can_advance(Step::Components, &flag_missing));
        assert!(can_advance(
            Step::Components,
            &with_extension(ExtensionState::Unavailable)
        ));
        assert!(can_advance(
            Step::Components,
            &with_extension(ExtensionState::Disabled)
        ));
    }

    #[test]
    fn the_extension_is_satisfied_only_when_enabled() {
        let state = |extension| with_extension(extension)[3].state;
        assert_eq!(state(ExtensionState::Enabled), ComponentState::Satisfied);
        assert_eq!(state(ExtensionState::Disabled), ComponentState::Missing);
        assert_eq!(
            state(ExtensionState::NeedsRelogin),
            ComponentState::Unavailable(Unavailable::NeedsRelogin)
        );
        assert_eq!(
            state(ExtensionState::ShadowedByUserCopy),
            ComponentState::Unavailable(Unavailable::ShadowedByUserCopy)
        );
        assert_eq!(
            state(ExtensionState::TurnedOff),
            ComponentState::Unavailable(Unavailable::ExtensionsOff)
        );
        assert_eq!(
            state(ExtensionState::Unavailable),
            ComponentState::Unavailable(Unavailable::NotInstalled)
        );
    }

    fn copies(system: bool, user: bool) -> ExtensionCopies {
        ExtensionCopies { system, user }
    }

    #[test]
    fn only_a_system_copy_counts() {
        let info = |system, run| Some(ExtensionInfo { system, run });
        let packaged = copies(true, false);
        assert_eq!(
            extension_state(info(true, ExtensionRun::Enabled), packaged),
            ExtensionState::Enabled
        );
        assert_eq!(
            extension_state(info(true, ExtensionRun::Disabled), packaged),
            ExtensionState::Disabled
        );
        assert_eq!(
            extension_state(info(true, ExtensionRun::Broken), packaged),
            ExtensionState::Unavailable
        );
        assert_eq!(
            extension_state(info(true, ExtensionRun::TurnedOff), packaged),
            ExtensionState::TurnedOff
        );
        // A development copy in ~/.local, enabled or not, is not what ships.
        assert_eq!(
            extension_state(info(false, ExtensionRun::Enabled), copies(false, true)),
            ExtensionState::Unavailable
        );
        assert_eq!(
            extension_state(None, ExtensionCopies::default()),
            ExtensionState::Unavailable
        );
    }

    #[test]
    fn a_system_copy_the_shell_has_not_scanned_needs_a_relogin() {
        assert_eq!(
            extension_state(None, copies(true, false)),
            ExtensionState::NeedsRelogin
        );
        // gnome-shell still lists a deleted user copy it scanned at login.
        assert_eq!(
            extension_state(
                Some(ExtensionInfo {
                    system: false,
                    run: ExtensionRun::Disabled
                }),
                copies(true, false)
            ),
            ExtensionState::NeedsRelogin
        );
    }

    #[test]
    fn a_user_copy_on_disk_shadows_the_system_copy_past_a_relogin() {
        // gnome-shell loads the user data dir first and skips a uuid it has
        // already loaded, so the system copy never runs while this is there.
        for reported in [
            None,
            Some(ExtensionInfo {
                system: false,
                run: ExtensionRun::Disabled,
            }),
            Some(ExtensionInfo {
                system: false,
                run: ExtensionRun::Enabled,
            }),
        ] {
            assert_eq!(
                extension_state(reported, copies(true, true)),
                ExtensionState::ShadowedByUserCopy,
                "{reported:?}"
            );
        }
    }

    #[test]
    fn a_row_offers_what_the_wizard_can_do_about_it() {
        let bare = assess(Machine::default());
        assert_eq!(row_action(&bare[1]), RowAction::Install);
        assert_eq!(row_action(&bare[2]), RowAction::Install);
        let installed = with_extension(ExtensionState::Enabled);
        for component in &installed[1..] {
            assert_eq!(row_action(component), RowAction::Installed, "{component:?}");
        }
        // The extension ships in a deb: a disabled system copy is enabled,
        // never installed.
        assert_eq!(
            row_action(&with_extension(ExtensionState::Disabled)[3]),
            RowAction::Enable
        );
        for (extension, why) in [
            (ExtensionState::Unavailable, Unavailable::NotInstalled),
            (ExtensionState::NeedsRelogin, Unavailable::NeedsRelogin),
            (
                ExtensionState::ShadowedByUserCopy,
                Unavailable::ShadowedByUserCopy,
            ),
            (ExtensionState::TurnedOff, Unavailable::ExtensionsOff),
        ] {
            assert_eq!(
                row_action(&with_extension(extension)[3]),
                RowAction::Unavailable(why)
            );
        }
    }

    #[test]
    fn the_model_size_is_an_upper_bound_only_with_an_nvidia_gpu() {
        // The install hook falls back to the CPU engine when the GPU has no
        // driver, so the GPU total is the most it downloads.
        assert!(!model_offer(&Machine::default()).upper_bound);
        assert!(
            model_offer(&Machine {
                nvidia_gpu: true,
                ..Machine::default()
            })
            .upper_bound
        );
    }

    #[test]
    fn the_list_waits_for_the_flag() {
        let bare = assess(Machine::default());
        assert!(unlocked(ComponentId::UserDaemons, &bare));
        for id in [
            ComponentId::Myna,
            ComponentId::Model,
            ComponentId::ShellExtension,
        ] {
            assert!(!unlocked(id, &bare), "{id:?}");
        }
        let flagged = assess(Machine {
            user_daemons: true,
            ..Machine::default()
        });
        assert!(ComponentId::ALL.iter().all(|id| unlocked(*id, &flagged)));
    }

    #[test]
    fn only_the_component_step_polls_and_only_while_something_required_is_missing() {
        let bare = assess(Machine::default());
        assert!(polls(Step::Components, &bare));
        assert!(!polls(
            Step::Components,
            &with_extension(ExtensionState::Unavailable)
        ));
        assert!(!polls(Step::Welcome, &bare));
        assert!(!polls(Step::Shortcut, &bare));
    }

    #[test]
    fn only_the_last_component_appearing_moves_on_by_itself() {
        let bare = assess(Machine::default());
        let complete = with_extension(ExtensionState::Enabled);
        let required_only = with_extension(ExtensionState::Unavailable);
        let disabled = with_extension(ExtensionState::Disabled);
        assert!(completes(&bare, &complete));
        assert!(completes(&disabled, &complete));
        assert!(!completes(&bare, &required_only));
        assert!(!completes(&disabled, &required_only));
        assert!(!completes(&complete, &complete));
        assert!(!completes(&complete, &bare));
    }

    #[test]
    fn the_model_offer_is_parakeet_sized_for_the_engine_it_will_pick() {
        let cpu = model_offer(&Machine::default());
        assert_eq!(cpu.family, ModelFamily::Parakeet);
        assert_eq!(cpu.snap(), RECOMMENDED_BACKEND_SNAP);
        assert_eq!(cpu.download_bytes, 775_593_984);
        let gpu = model_offer(&Machine {
            nvidia_gpu: true,
            ..Machine::default()
        });
        assert_eq!(gpu.download_bytes, 4_172_693_504);
    }

    #[test]
    fn a_component_snapd_is_still_installing_shows_missing() {
        // A backend's slot is published before its install has fetched the
        // model, so discovery finds it half-way through the change.
        let found = with_extension(ExtensionState::Enabled);
        let shown = while_installing(&found, &[ComponentId::Model]);
        assert_eq!(shown[2].state, ComponentState::Missing);
        assert!(!can_advance(Step::Components, &shown));
        assert!(!fully_installed(&shown));
        assert_eq!(while_installing(&found, &[]), found);
        for index in [0, 1, 3] {
            assert_eq!(shown[index], found[index]);
        }
    }

    #[test]
    fn only_the_app_and_the_model_are_installed_from_snapd() {
        let offer = model_offer(&Machine::default());
        assert_eq!(
            installs(ComponentId::Myna, &offer),
            Some((MYNA_SNAP, MYNA_DOWNLOAD_BYTES))
        );
        assert_eq!(
            installs(ComponentId::Model, &offer),
            Some((RECOMMENDED_BACKEND_SNAP, offer.download_bytes))
        );
        assert_eq!(installs(ComponentId::UserDaemons, &offer), None);
        assert_eq!(installs(ComponentId::ShellExtension, &offer), None);
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
}
