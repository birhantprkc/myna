//! GTK/libadwaita wiring for the onboarding wizard.
//!
//! Thin, like [`crate::backend_ui`]: [`crate::onboarding`] decides what is
//! missing, how to install it, and when the flow may advance; this module
//! renders that, connects and restarts what the user installed, and hands
//! control back to the settings window when the user is done.

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gtk::glib;
use gtk4 as gtk;
use libadwaita as adw;
use libadwaita::prelude::*;

use crate::active_backend::{ensure_backend_active, SetupStage, SnapdWait};
use crate::adapters::shell_extensions::GnomeShellExtensions;
use crate::adapters::snap_backend::SnapBackendRepository;
use crate::adapters::system_configurator::PkexecSystemConfigurator;
use crate::command::{CancellationToken, GioCommandRunner};
use crate::domain::BackendSurfaceError;
use crate::onboarding::{
    assess, can_advance, completes, flag_enabled, installs, model_offer, needs_onboarding, polls,
    row_action, unlocked, while_installing, Component, ComponentId, ComponentState, Machine,
    ModelOffer, RowAction, Step, Unavailable, MYNA_DOWNLOAD_BYTES, RECOMMENDED_BACKEND_SNAP,
    SHELL_EXTENSION_UUID,
};
use crate::ports::{
    BackendRepository, ShellExtensions, SystemConfigurator, SystemConfiguratorError,
};
use crate::snap_changes::{pending_install, ApplyProgress};
use crate::snap_install::{follow_change, install, Follow};
use crate::ui;

/// How often the component step re-reads the machine while something is
/// missing: one `snap list` and one discovery each time.
const POLL_INTERVAL: Duration = Duration::from_secs(2);
/// How long setting up waits for snapd to finish installing Myna or a model,
/// which may still be downloading it.
const SNAPD_TIMEOUT: Duration = Duration::from_secs(15 * 60);
/// How long "All required components installed" shows before the wizard moves on by
/// itself.
const BEAT: Duration = Duration::from_secs(1);
/// How often an install's change is read for its progress.
const FOLLOW_INTERVAL: Duration = Duration::from_secs(1);

/// Where a row's install stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Install {
    /// snapd is installing it: its polkit prompt may still be open, and the
    /// percentage appears once a download announces its size.
    Running(Option<u8>),
    /// snapd is done; the row waits for a read that shows it.
    Confirming,
}

/// Where turning snapd's flag on stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FlagWrite {
    Idle,
    /// snapd has not answered: polkit's prompt may be open.
    Asking,
    /// snapd turned it on; the switch waits for a read that shows it.
    Confirming,
}

pub struct OnboardingUi {
    window: ui::OnboardingWindow,
    components_page: ui::OnboardingComponents,
    shortcut_page: ui::OnboardingShortcut,
    shortcut: Rc<crate::shortcut_ui::ShortcutControl>,
    repository: Rc<dyn BackendRepository>,
    configurator: Rc<dyn SystemConfigurator>,
    extensions: Rc<dyn ShellExtensions>,
    step: Cell<Step>,
    components: RefCell<Vec<Component>>,
    offer: Cell<ModelOffer>,
    /// Why the last assessment could not read the machine.
    problem: RefCell<Option<String>>,
    busy: Cell<bool>,
    flag_write: Cell<FlagWrite>,
    flag_cancellation: RefCell<Option<CancellationToken>>,
    /// Bumped by every flag write, so a read that started before one is not
    /// taken for the machine after it.
    epoch: Cell<u64>,
    installs: RefCell<BTreeMap<ComponentId, Install>>,
    /// Stop following installs when the wizard closes; snapd carries on.
    install_cancellation: CancellationToken,
    follow_interval: Cell<Duration>,
    stage: RefCell<Option<SetupStage>>,
    setup_cancellation: RefCell<Option<CancellationToken>>,
    /// The last line logged, so a poll repeats none.
    logged: RefCell<String>,
    assessing: Cell<bool>,
    poll_interval: Cell<Duration>,
    poll: RefCell<Option<glib::SourceId>>,
    beat_length: Cell<Duration>,
    beat: RefCell<Option<glib::SourceId>>,
    finished: RefCell<Option<Box<dyn Fn()>>>,
}

impl OnboardingUi {
    /// Build and present the wizard against the real snapd and snap ports.
    /// `finished` runs once, when the user completes or closes the flow.
    pub fn present(
        application: &adw::Application,
        initial: Vec<Component>,
        finished: Box<dyn Fn()>,
    ) -> Rc<Self> {
        let runner = Arc::new(GioCommandRunner);
        Self::present_with_ports(
            application,
            initial,
            Rc::new(SnapBackendRepository::new(runner.clone())),
            Rc::new(PkexecSystemConfigurator::new(runner)),
            Rc::new(GnomeShellExtensions::new()),
            None,
            finished,
        )
    }

    /// With a `parent`, the wizard is modal over it: the parent's own
    /// operations cannot start while the wizard sets a backend up.
    pub fn present_with_ports(
        application: &adw::Application,
        initial: Vec<Component>,
        repository: Rc<dyn BackendRepository>,
        configurator: Rc<dyn SystemConfigurator>,
        extensions: Rc<dyn ShellExtensions>,
        parent: Option<&gtk::Window>,
        finished: Box<dyn Fn()>,
    ) -> Rc<Self> {
        let window = ui::OnboardingWindow::new(application);
        if let Some(parent) = parent {
            window.set_transient_for(Some(parent));
            window.set_modal(true);
        }
        let welcome = ui::OnboardingWelcome::new();
        let components_page = ui::OnboardingComponents::new();
        let shortcut_page = ui::OnboardingShortcut::new();

        let navigation = window.navigation();
        for (step, content) in [
            (Step::Welcome, welcome.upcast_ref::<gtk::Widget>()),
            (Step::Components, components_page.upcast_ref()),
            (Step::Shortcut, shortcut_page.upcast_ref()),
        ] {
            let toolbar = adw::ToolbarView::new();
            // Each page already heads itself with its title.
            toolbar.add_top_bar(&adw::HeaderBar::builder().show_title(false).build());
            toolbar.set_content(Some(content));
            navigation.add(&adw::NavigationPage::with_tag(
                &toolbar,
                &step_title(step),
                step_name(step),
            ));
        }

        let shortcut = crate::shortcut_ui::ShortcutControl::attach(
            shortcut_page.shortcut_box(),
            shortcut_page.shortcut_button(),
            window.overlay(),
            crate::shortcut_ui::Surface::Onboarding,
            Box::new({
                let description = shortcut_page.description();
                move |state, path| {
                    description.set_label(&crate::shortcut_ui::onboarding_description(state, path))
                }
            }),
        );
        let ui = Rc::new(Self {
            window: window.clone(),
            components_page: components_page.clone(),
            shortcut_page: shortcut_page.clone(),
            shortcut,
            repository,
            configurator,
            extensions,
            step: Cell::new(Step::first()),
            components: RefCell::new(initial),
            offer: Cell::new(model_offer(&Machine {
                nvidia_gpu: crate::machine::has_nvidia_gpu(),
                ..Machine::default()
            })),
            problem: RefCell::default(),
            busy: Cell::new(false),
            flag_write: Cell::new(FlagWrite::Idle),
            flag_cancellation: RefCell::default(),
            epoch: Cell::new(0),
            installs: RefCell::default(),
            install_cancellation: CancellationToken::new(),
            follow_interval: Cell::new(FOLLOW_INTERVAL),
            stage: RefCell::default(),
            setup_cancellation: RefCell::default(),
            logged: RefCell::default(),
            assessing: Cell::new(false),
            poll_interval: Cell::new(POLL_INTERVAL),
            poll: RefCell::new(None),
            beat_length: Cell::new(BEAT),
            beat: RefCell::new(None),
            finished: RefCell::new(Some(finished)),
        });

        // The switch turns the flag on and never off: snapd refuses Myna's
        // refreshes without it. Its state is only ever what snapd reports.
        components_page.flag_switch().connect_state_set({
            let ui = Rc::downgrade(&ui);
            move |switch, requested| {
                let Some(ui) = ui.upgrade() else {
                    return glib::Propagation::Stop;
                };
                let flag = flag_enabled(&ui.components.borrow());
                // A pending write shows the switch on until snapd answers.
                let shown = flag || ui.flag_write.get() != FlagWrite::Idle;
                if requested && !shown {
                    ui.enable_flag();
                } else if requested != shown {
                    let switch = switch.clone();
                    glib::idle_add_local_once(move || switch.set_active(shown));
                }
                glib::Propagation::Stop
            }
        });

        for id in [ComponentId::Myna, ComponentId::Model] {
            let Some(row) = components_page.row(id) else {
                continue;
            };
            row.button.connect_clicked({
                let ui = Rc::downgrade(&ui);
                move |_| {
                    if let Some(ui) = ui.upgrade() {
                        ui.install(id);
                    }
                }
            });
        }

        window.forward_button().connect_clicked({
            let ui = Rc::downgrade(&ui);
            move |_| {
                if let Some(ui) = ui.upgrade() {
                    ui.advance();
                }
            }
        });
        // The visible page is the step: going back is the navigation view's
        // own, from the header bar, Escape or Alt+Left.
        navigation.connect_visible_page_notify({
            let ui = Rc::downgrade(&ui);
            move |navigation| {
                let step = navigation
                    .visible_page()
                    .and_then(|page| page.tag())
                    .and_then(|tag| step_named(&tag));
                if let (Some(ui), Some(step)) = (ui.upgrade(), step) {
                    ui.step.set(step);
                    ui.render();
                }
            }
        });
        // Installing happens in App Center or a terminal, so coming back to
        // the window is the moment to look again.
        window.connect_is_active_notify({
            let ui = Rc::downgrade(&ui);
            move |window| {
                if let Some(ui) = ui.upgrade() {
                    if window.is_active() && ui.step.get() == Step::Components && !ui.busy.get() {
                        ui.refresh_assessment();
                    }
                }
            }
        });
        // The application owns the window, so it outlives this call; without a
        // strong reference living alongside it every button would upgrade a
        // dead weak reference and do nothing. The reference is dropped when
        // the window closes, which breaks the cycle it forms.
        // Closing also stops a setup still waiting on snapd, so nothing is
        // connected or restarted behind a closed wizard.
        window.connect_close_request({
            let held = RefCell::new(Some(ui.clone()));
            move |_| {
                if let Some(ui) = held.borrow_mut().take() {
                    if let Some(cancellation) = ui.flag_cancellation.take() {
                        cancellation.cancel();
                    }
                    ui.install_cancellation.cancel();
                    if let Some(cancellation) = ui.setup_cancellation.take() {
                        ui.log("setup: cancelled, the wizard closed");
                        cancellation.cancel();
                    }
                }
                glib::Propagation::Proceed
            }
        });

        crate::app::install_appearance_policy(window.upcast_ref());
        ui.render();
        window.present();
        ui
    }

    /// Re-read the machine and re-render. Costs one `snap list` and one
    /// discovery, and runs when the window regains focus on the component
    /// step and while that step polls.
    fn refresh_assessment(self: &Rc<Self>) {
        if self.assessing.replace(true) {
            return;
        }
        let epoch = self.epoch.get();
        let ui = Rc::downgrade(self);
        let repository = self.repository.clone();
        let configurator = self.configurator.clone();
        let extensions = self.extensions.clone();
        glib::spawn_future_local(async move {
            let (components, offer, problem) = read_machine(
                repository.as_ref(),
                configurator.as_ref(),
                extensions.as_ref(),
            )
            .await;
            let Some(ui) = ui.upgrade() else {
                return;
            };
            ui.assessing.set(false);
            if ui.epoch.get() != epoch {
                ui.refresh_assessment();
                return;
            }
            if ui.flag_write.get() == FlagWrite::Confirming {
                ui.flag_write.set(FlagWrite::Idle);
            }
            let before = ui.shown();
            ui.installs
                .borrow_mut()
                .retain(|_, install| *install != Install::Confirming);
            ui.offer.set(offer);
            match &problem {
                Some(problem) => ui.log(&format!("assessment: {problem}")),
                None => ui.log(&format!("assessment: {}", describe(&components))),
            }
            ui.problem.replace(problem);
            ui.components.replace(components);
            ui.follow_installs_elsewhere().await;
            // The user installed the last piece while watching: finish for
            // them, as Next would.
            let finish = ui.step.get() == Step::Components
                && !ui.busy.get()
                && completes(&before, &ui.shown());
            if finish {
                ui.finish_setup(Step::Shortcut, true);
            } else {
                ui.render();
            }
        });
    }

    /// Ask snapd to turn the flag on. Its polkit prompt is the only
    /// question; dismissing it puts the switch back silently, a refusal or
    /// failure with a toast whose Details open the report.
    fn enable_flag(self: &Rc<Self>) {
        let cancellation = CancellationToken::new();
        self.flag_cancellation.replace(Some(cancellation.clone()));
        self.flag_write.set(FlagWrite::Asking);
        self.epoch.set(self.epoch.get() + 1);
        self.log("flag: enabling user daemons");
        self.render();
        let ui = Rc::downgrade(self);
        let configurator = self.configurator.clone();
        glib::spawn_future_local(async move {
            let outcome = configurator.enable_user_daemons(cancellation).await;
            let Some(ui) = ui.upgrade() else {
                return;
            };
            ui.flag_cancellation.take();
            ui.epoch.set(ui.epoch.get() + 1);
            match outcome {
                Ok(()) => {
                    ui.log("flag: enabled");
                    ui.flag_write.set(FlagWrite::Confirming);
                    ui.refresh_assessment();
                }
                Err(SystemConfiguratorError::Cancelled) => {
                    ui.log("flag: the prompt was dismissed");
                    ui.flag_write.set(FlagWrite::Idle);
                }
                Err(error) => {
                    ui.log(&format!("flag: failed: {error}"));
                    ui.flag_write.set(FlagWrite::Idle);
                    ui.announce_flag_failure(&error);
                }
            }
            ui.render();
        });
    }

    /// The components as the step shows them: one snapd is still
    /// installing is missing until its change is done.
    fn shown(&self) -> Vec<Component> {
        let installing: Vec<ComponentId> = self.installs.borrow().keys().copied().collect();
        while_installing(&self.components.borrow(), &installing)
    }

    /// Install the snap behind `id` through snapd as the user, one row at a
    /// time. Its polkit prompt is the only question: dismissing it puts the
    /// button back silently, a refusal or a failed change with a toast
    /// whose Details open the report.
    fn install(self: &Rc<Self>, id: ComponentId) {
        let Some((snap, expected)) = installs(id, &self.offer.get()) else {
            return;
        };
        if !self.installs.borrow().is_empty()
            || row_action_of(&self.shown(), id) != Some(RowAction::Install)
        {
            return;
        }
        self.installs
            .borrow_mut()
            .insert(id, Install::Running(None));
        self.epoch.set(self.epoch.get() + 1);
        self.log(&format!("install: installing {snap}"));
        self.render();
        let ui = Rc::downgrade(self);
        let configurator = self.configurator.clone();
        let cancellation = self.install_cancellation.clone();
        let interval = self.follow_interval.get();
        glib::spawn_future_local(async move {
            let sleep = |interval| -> std::pin::Pin<Box<dyn std::future::Future<Output = ()>>> {
                Box::pin(glib::timeout_future(interval))
            };
            let follow = Follow {
                interval,
                sleep: &sleep,
                cancellation,
            };
            let report = progress_reporter(ui.clone(), id);
            let outcome = install(configurator.as_ref(), snap, expected, &follow, &report).await;
            let Some(ui) = ui.upgrade() else {
                return;
            };
            ui.epoch.set(ui.epoch.get() + 1);
            match outcome {
                Ok(()) => {
                    ui.log(&format!("install: {snap} installed"));
                    ui.installs.borrow_mut().insert(id, Install::Confirming);
                    ui.refresh_assessment();
                }
                Err(SystemConfiguratorError::Cancelled) => {
                    ui.log(&format!("install: {snap}: the prompt was dismissed"));
                    ui.installs.borrow_mut().remove(&id);
                }
                Err(error) => {
                    ui.log(&format!("install: {snap} failed: {error}"));
                    ui.installs.borrow_mut().remove(&id);
                    ui.announce_failure(install_failed(id), &error);
                }
            }
            ui.render();
        });
    }

    /// Follow an install snapd is already running for a missing row, one
    /// started in a terminal or by a wizard since closed, rather than offer
    /// to start it again. Costs one read of snapd's changes, and only while
    /// such a row is missing.
    async fn follow_installs_elsewhere(self: &Rc<Self>) {
        let offer = self.offer.get();
        let missing: Vec<(ComponentId, &'static str, u64)> = self
            .shown()
            .iter()
            .filter(|component| component.state == ComponentState::Missing)
            .filter(|component| !self.installs.borrow().contains_key(&component.id))
            .filter_map(|component| {
                installs(component.id, &offer).map(|(snap, bytes)| (component.id, snap, bytes))
            })
            .collect();
        if missing.is_empty() || !flag_enabled(&self.components.borrow()) {
            return;
        }
        let Ok(changes) = self
            .configurator
            .changes_in_progress(CancellationToken::new())
            .await
        else {
            return;
        };
        for (id, snap, expected) in missing {
            if self.installs.borrow().contains_key(&id) {
                continue;
            }
            if let Some(change) = pending_install(&changes, snap) {
                self.follow_elsewhere(id, snap, change.id().to_owned(), expected);
            }
        }
    }

    fn follow_elsewhere(
        self: &Rc<Self>,
        id: ComponentId,
        snap: &str,
        change_id: String,
        expected: u64,
    ) {
        self.installs
            .borrow_mut()
            .insert(id, Install::Running(None));
        self.log(&format!(
            "install: following change {change_id} installing {snap}"
        ));
        let ui = Rc::downgrade(self);
        let configurator = self.configurator.clone();
        let cancellation = self.install_cancellation.clone();
        let interval = self.follow_interval.get();
        let snap = snap.to_owned();
        glib::spawn_future_local(async move {
            let sleep = |interval| -> std::pin::Pin<Box<dyn std::future::Future<Output = ()>>> {
                Box::pin(glib::timeout_future(interval))
            };
            let follow = Follow {
                interval,
                sleep: &sleep,
                cancellation,
            };
            let report = progress_reporter(ui.clone(), id);
            let outcome = follow_change(
                configurator.as_ref(),
                &change_id,
                expected,
                &follow,
                &report,
            )
            .await;
            let Some(ui) = ui.upgrade() else {
                return;
            };
            ui.epoch.set(ui.epoch.get() + 1);
            // Not the user's action here, so a failure only puts Install back.
            match outcome {
                Ok(()) => {
                    ui.log(&format!("install: {snap} installed"));
                    ui.installs.borrow_mut().insert(id, Install::Confirming);
                    ui.refresh_assessment();
                }
                Err(failure) => {
                    ui.log(&format!("install: change {change_id} ended: {failure:?}"));
                    ui.installs.borrow_mut().remove(&id);
                }
            }
            ui.render();
        });
    }

    fn announce_flag_failure(&self, error: &SystemConfiguratorError) {
        self.announce_failure(gettextrs::gettext("Enabling user daemons failed"), error);
    }

    fn announce_failure(&self, heading: String, error: &SystemConfiguratorError) {
        let summary = crate::backend_ui::system_failure_summary(error);
        let details = crate::backend_ui::system_error_details(error);
        let toast = adw::Toast::builder()
            .title(crate::markup::escape_markup(&heading))
            .button_label(gettextrs::gettext("Details"))
            .build();
        toast.connect_button_clicked({
            let window = self.window.clone();
            move |_| {
                ui::OperationErrorDialog::new(&heading, &summary, &details).present(Some(&window));
            }
        });
        self.window.overlay().add_toast(toast);
    }

    fn advance(self: &Rc<Self>) {
        // The gate lives here, not on the button: a step can also be advanced
        // by activating the button from the keyboard or a screen reader, and
        // an insensitive widget still emits `clicked` when told to.
        if self.busy.get() || !can_advance(self.step.get(), &self.shown()) {
            return;
        }
        // Set up already; only the pause before moving on is left.
        if let Some(beat) = self.beat.take() {
            beat.remove();
            self.window
                .navigation()
                .push_by_tag(step_name(Step::Shortcut));
            return;
        }
        match self.step.get().next() {
            Some(step) if self.step.get() == Step::Components => self.finish_setup(step, false),
            Some(step) => self.window.navigation().push_by_tag(step_name(step)),
            None => {
                self.notify_finished();
                self.window.close();
            }
        }
    }

    /// Connect the backend and restart the daemon against it, so the next
    /// step finds dictation running. A store install usually auto-connects
    /// the backend; when it did not, snapd asks polkit once. With `pause`, the
    /// step first shows that everything is installed for a beat.
    fn finish_setup(self: &Rc<Self>, next: Step, pause: bool) {
        if self.busy.replace(true) {
            return;
        }
        let cancellation = CancellationToken::new();
        self.setup_cancellation.replace(Some(cancellation.clone()));
        self.render();
        let ui = Rc::downgrade(self);
        let repository = self.repository.clone();
        let configurator = self.configurator.clone();
        let interval = self.poll_interval.get();
        glib::spawn_future_local(async move {
            let sleep = |interval| -> std::pin::Pin<Box<dyn std::future::Future<Output = ()>>> {
                Box::pin(glib::timeout_future(interval))
            };
            let wait = SnapdWait {
                interval,
                timeout: SNAPD_TIMEOUT,
                sleep: &sleep,
                cancellation,
            };
            let report = {
                let ui = ui.clone();
                move |stage: SetupStage| {
                    if let Some(ui) = ui.upgrade() {
                        ui.log(&format!("setup: {}", stage_log(&stage)));
                        ui.stage.replace(Some(stage));
                        ui.render();
                    }
                }
            };
            let outcome = ensure_backend_active(
                repository.as_ref(),
                configurator.as_ref(),
                RECOMMENDED_BACKEND_SNAP,
                &wait,
                &report,
            )
            .await;
            let Some(ui) = ui.upgrade() else {
                return;
            };
            match &outcome {
                Ok(()) => ui.log("setup: done"),
                Err(message) => ui.log(&format!("setup: failed: {message}")),
            }
            ui.setup_cancellation.take();
            ui.stage.take();
            ui.busy.set(false);
            ui.render();
            if outcome.is_ok() {
                ui.shortcut.install_default();
            }
            match outcome {
                Ok(()) if pause => ui.pause_before(next),
                Ok(()) => ui.window.navigation().push_by_tag(step_name(next)),
                Err(message) => {
                    ui.report_failure(&gettextrs::gettext("Could not set up dictation"), &message)
                }
            }
        });
    }

    /// Move on to `next` once the status has been readable for a beat.
    fn pause_before(self: &Rc<Self>, next: Step) {
        let ui = Rc::downgrade(self);
        let beat = glib::timeout_add_local_once(self.beat_length.get(), move || {
            let Some(ui) = ui.upgrade() else {
                return;
            };
            ui.beat.take();
            ui.window.navigation().push_by_tag(step_name(next));
        });
        self.beat.replace(Some(beat));
    }

    /// The probe polls and pauses shorter than a person needs.
    pub fn set_poll_interval(&self, interval: Duration) {
        self.poll_interval.set(interval);
        self.follow_interval.set(interval);
    }

    pub fn set_beat(&self, beat: Duration) {
        self.beat_length.set(beat);
    }

    /// The widgets the headless probe drives the wizard through. It holds
    /// these and drops the controller, the way the application does.
    pub fn window(&self) -> ui::OnboardingWindow {
        self.window.clone()
    }

    pub fn shortcut_button(&self) -> gtk::Button {
        self.shortcut_page.shortcut_button()
    }

    fn notify_finished(self: &Rc<Self>) {
        if let Some(finished) = self.finished.borrow_mut().take() {
            finished();
        }
    }

    fn render(self: &Rc<Self>) {
        let step = self.step.get();
        let components = self.shown();

        self.window.set_title(Some(&step_title(step)));
        // Setting up restarts the daemon; leaving mid-way would strand it.
        if let Some(page) = self.window.navigation().visible_page() {
            page.set_can_pop(!self.busy.get());
        }

        self.render_components(&components);
        let forward = self.window.forward_button();
        if step.next().is_some() {
            forward.set_label(&gettextrs::gettext("Next"));
            forward.remove_css_class("suggested-action");
            forward.add_css_class("outlined");
        } else {
            forward.set_label(&gettextrs::gettext("Done"));
            forward.remove_css_class("outlined");
            forward.add_css_class("suggested-action");
        }
        forward.set_sensitive(!self.busy.get() && can_advance(step, &components));
        let setting_up = step == Step::Components && self.busy.get();
        let spinner = self.window.setup_spinner();
        spinner.set_visible(setting_up);
        spinner.set_spinning(setting_up);
        let status = match &*self.stage.borrow() {
            Some(stage) if setting_up => Some(stage_text(stage)),
            _ if step == Step::Components && needs_onboarding(&components) => {
                self.problem.borrow().clone()
            }
            _ => None,
        };
        let label = self.window.setup_status();
        label.set_visible(status.is_some());
        label.set_label(status.as_deref().unwrap_or_default());
        self.window
            .installed_status()
            .set_visible(step == Step::Components && !setting_up && !needs_onboarding(&components));
        self.watch(!self.busy.get() && polls(step, &components));
        // Going back during the pause stays back.
        if step != Step::Components {
            if let Some(beat) = self.beat.take() {
                beat.remove();
            }
        }
    }

    /// The flag's switch and one row per component, each with its button or
    /// its check, as the last assessment found them.
    fn render_components(&self, components: &[Component]) {
        let page = &self.components_page;
        let flag = flag_enabled(components);
        let pending = self.flag_write.get() != FlagWrite::Idle;
        let switch = page.flag_switch();
        switch.set_active(flag || pending);
        switch.set_state(flag);
        let flag_row = page.flag_row();
        // Busy, not insensitive: an insensitive row dims its subtitle.
        for widget in [flag_row.upcast_ref::<gtk::Widget>(), switch.upcast_ref()] {
            widget.set_can_target(!pending);
            widget.set_can_focus(!pending);
        }
        let spinner = page.flag_spinner();
        spinner.set_visible(pending);
        spinner.set_spinning(pending);
        let flag_subtitle = if pending {
            gettextrs::gettext("Enabling…")
        } else {
            gettextrs::gettext("Required by the Dictation app")
        };
        flag_row.set_subtitle(&flag_subtitle);
        flag_row.update_property(&[gtk::accessible::Property::Description(&flag_subtitle)]);
        flag_row.update_state(&[gtk::accessible::State::Busy(pending)]);
        page.component_list()
            .set_sensitive(unlocked(ComponentId::Myna, components));
        let installs = self.installs.borrow();
        for component in components {
            let Some(row) = page.row(component.id) else {
                continue;
            };
            let action = row_action(component);
            let subtitle = self.subtitle(component.id, action);
            row.row.set_subtitle(&subtitle);
            // GTK 4.14's AT-SPI reads the subtitle relation as empty.
            row.row
                .update_property(&[gtk::accessible::Property::Description(&subtitle)]);
            let install = installs.get(&component.id).copied();
            row.row
                .update_state(&[gtk::accessible::State::Busy(install.is_some())]);
            if let Some(progress) = &row.installing {
                progress.container.set_visible(install.is_some());
                progress.spinner.set_spinning(install.is_some());
                if let Some(install) = install {
                    progress.label.set_label(&install_text(install));
                }
            }
            if install.is_some() {
                row.installed.set_visible(false);
                row.button.set_visible(false);
                continue;
            }
            // One install at a time: snapd's prompt covers one request.
            row.button.set_sensitive(installs.is_empty());
            row.installed.set_visible(action == RowAction::Installed);
            let label = match action {
                RowAction::Install => {
                    Some((gettextrs::gettext("Install"), install_label(component.id)))
                }
                RowAction::Enable => Some((
                    gettextrs::gettext("Enable"),
                    gettextrs::gettext("Enable the shell extension"),
                )),
                RowAction::Installed | RowAction::Unavailable(_) => None,
            };
            row.button.set_visible(label.is_some());
            if let Some((label, accessible)) = label {
                row.button.set_label(&label);
                // A button is labelled by its label child, over any name set.
                row.button
                    .reset_relation(gtk::AccessibleRelation::LabelledBy);
                row.button
                    .update_property(&[gtk::accessible::Property::Label(&accessible)]);
            }
        }
    }

    fn subtitle(&self, id: ComponentId, action: RowAction) -> String {
        match (id, action) {
            (ComponentId::Myna, _) => download_size(MYNA_DOWNLOAD_BYTES),
            (ComponentId::Model, _) => {
                let offer = self.offer.get();
                let name = crate::model_family::model_family(offer.snap()).name;
                let size = download_size(offer.download_bytes);
                let frame = if offer.upper_bound {
                    // TRANSLATORS: {model} is a model family, such as "Parakeet", and {size} a download size such as "4.2 GB".
                    gettextrs::gettext("{model} · up to {size}")
                } else {
                    // TRANSLATORS: {model} is a model family, such as "Parakeet", and {size} a download size such as "776 MB".
                    gettextrs::gettext("{model} · {size}")
                };
                frame.replace("{model}", &name).replace("{size}", &size)
            }
            (_, RowAction::Unavailable(Unavailable::NeedsRelogin)) => gettextrs::gettext(
                "Log out and back in to use it. Until then, Dictation shows its status in notifications.",
            ),
            (_, RowAction::Unavailable(Unavailable::ShadowedByUserCopy)) => gettextrs::gettext(
                "A copy in ~/.local/share/gnome-shell/extensions hides it. Remove that copy, then log out and back in.",
            ),
            (_, RowAction::Unavailable(Unavailable::NotInstalled)) => gettextrs::gettext(
                "Not available on this system. Dictation still works and shows its status in notifications.",
            ),
            _ => gettextrs::gettext("Recommended"),
        }
    }

    /// Start or stop the component step's poll.
    fn watch(self: &Rc<Self>, wanted: bool) {
        let mut poll = self.poll.borrow_mut();
        if !wanted {
            if let Some(source) = poll.take() {
                source.remove();
            }
            return;
        }
        if poll.is_none() {
            let ui = Rc::downgrade(self);
            *poll = Some(glib::timeout_add_local(
                self.poll_interval.get(),
                move || {
                    if let Some(ui) = ui.upgrade() {
                        ui.refresh_assessment();
                    }
                    glib::ControlFlow::Continue
                },
            ));
        }
    }

    /// Say what the wizard found or did, once per change, to the journal
    /// when launched from the desktop and to stderr from a terminal.
    fn log(&self, line: &str) {
        if *self.logged.borrow() != line {
            glib::g_message!(crate::LOG_DOMAIN, "onboarding {}", line);
            self.logged.replace(line.to_owned());
        }
    }

    fn report_failure(self: &Rc<Self>, title: &str, details: &str) {
        let dialog = ui::OperationErrorDialog::new(title, title, details);
        dialog.present(Some(&self.window));
    }
}

impl Drop for OnboardingUi {
    fn drop(&mut self) {
        for source in [self.poll.take(), self.beat.take()].into_iter().flatten() {
            source.remove();
        }
    }
}

/// What a row offers, by id.
fn row_action_of(components: &[Component], id: ComponentId) -> Option<RowAction> {
    components
        .iter()
        .find(|component| component.id == id)
        .map(row_action)
}

/// What a row's install reports hands to the row.
fn progress_reporter(ui: std::rc::Weak<OnboardingUi>, id: ComponentId) -> impl Fn(Option<u8>) {
    move |percent| {
        if let Some(ui) = ui.upgrade() {
            ui.installs
                .borrow_mut()
                .insert(id, Install::Running(percent));
            ui.render();
        }
    }
}

/// An install as its row says it, beside the spinner.
fn install_text(install: Install) -> String {
    match install {
        Install::Running(Some(percent)) => {
            // TRANSLATORS: {percent} is how much of the download has arrived, a number from 0 to 100.
            let frame = gettextrs::gettext("Installing {percent}%");
            frame.replace("{percent}", &percent.to_string())
        }
        Install::Running(None) | Install::Confirming => gettextrs::gettext("Installing…"),
    }
}

/// The toast's heading when installing `id` fails.
fn install_failed(id: ComponentId) -> String {
    match id {
        ComponentId::Model => gettextrs::gettext("Installing the speech-to-text model failed"),
        _ => gettextrs::gettext("Installing the Dictation app failed"),
    }
}

/// The Install button's name to assistive technology: three rows read
/// "Install" alike.
fn install_label(id: ComponentId) -> String {
    match id {
        ComponentId::Myna => gettextrs::gettext("Install the Dictation app"),
        ComponentId::Model => gettextrs::gettext("Install the speech-to-text model"),
        ComponentId::ShellExtension => gettextrs::gettext("Install the shell extension"),
        ComponentId::UserDaemons => gettextrs::gettext("Enable user daemons experimental support"),
    }
}

/// One assessment of what dictation is missing on this machine: one `snap
/// list`, one discovery, snapd's flags over its socket and one call to
/// gnome-shell. A surface that cannot be read counts as nothing found, which
/// opens the wizard: the flow then shows what it could not verify rather than
/// a settings window with no backends and no explanation.
pub async fn assess_machine(
    repository: &dyn BackendRepository,
    configurator: &dyn SystemConfigurator,
    extensions: &dyn ShellExtensions,
) -> Vec<Component> {
    let (components, _, problem) = read_machine(repository, configurator, extensions).await;
    let found = problem.unwrap_or_else(|| describe(&components));
    glib::g_message!(crate::LOG_DOMAIN, "onboarding assessment: {found}");
    components
}

/// [`assess_machine`], the model it would install, and what it could not
/// read.
async fn read_machine(
    repository: &dyn BackendRepository,
    configurator: &dyn SystemConfigurator,
    extensions: &dyn ShellExtensions,
) -> (Vec<Component>, ModelOffer, Option<String>) {
    let cancellation = CancellationToken::new();
    let mut problems = Vec::new();
    let user_daemons = configurator
        .user_daemons_enabled(cancellation.clone())
        .await
        .unwrap_or_else(|error| {
            problems.push(error);
            false
        });
    let installed = repository
        .installed_snaps(cancellation.clone())
        .await
        .unwrap_or_else(|error| {
            problems.push(reason(&error));
            Vec::new()
        });
    let backends = repository
        .discover(cancellation)
        .await
        .map(|snapshot| snapshot.backends().len())
        .unwrap_or_else(|error| {
            problems.push(reason(&error));
            0
        });
    problems.dedup();
    let problem = (!problems.is_empty()).then(|| {
        // TRANSLATORS: {error} is snapd's own message, in English.
        let frame = gettextrs::gettext("Cannot read what snapd has set up: {error}");
        frame.replace("{error}", &problems.join("; "))
    });
    let machine = Machine {
        user_daemons,
        extension: extensions.extension_state(SHELL_EXTENSION_UUID).await,
        nvidia_gpu: crate::machine::has_nvidia_gpu(),
        ..Machine::new(&installed, backends)
    };
    (assess(machine), model_offer(&machine), problem)
}

/// What snap said, which names the cause, over how it exited.
fn reason(error: &BackendSurfaceError) -> String {
    match error.stderr().trim() {
        "" => error.message().to_owned(),
        stderr => stderr.to_owned(),
    }
}

fn describe(components: &[Component]) -> String {
    components
        .iter()
        .map(|component| {
            let state = match component.state {
                ComponentState::Satisfied => "found".to_owned(),
                ComponentState::Missing => "missing".to_owned(),
                ComponentState::Unavailable(why) => format!("unavailable ({why:?})"),
            };
            format!("{:?} {state}", component.id)
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// A download's size as the rows show it: in whole megabytes from 100 MB to
/// 1 GB, where a decimal only adds noise.
fn download_size(bytes: u64) -> String {
    if !(100_000_000..999_500_000).contains(&bytes) {
        return glib::format_size(bytes).to_string();
    }
    // TRANSLATORS: a download size in megabytes, such as "776 MB"; the space is a no-break space.
    gettextrs::gettext("{megabytes} MB")
        .replace("{megabytes}", &((bytes + 500_000) / 1_000_000).to_string())
}

/// A stage as the footer says it, beside the spinner.
fn stage_text(stage: &SetupStage) -> String {
    match stage {
        SetupStage::Checking => gettextrs::gettext("Checking the installation…"),
        SetupStage::Waiting(progress @ ApplyProgress::Download { .. }) => {
            crate::backend_ui::apply_progress_text(progress)
        }
        SetupStage::Waiting(ApplyProgress::Change { .. }) => {
            gettextrs::gettext("Waiting for other software changes to finish…")
        }
        SetupStage::Connecting(snap) => {
            // TRANSLATORS: {model} is a model family, such as "Parakeet".
            let frame = gettextrs::gettext("Connecting {model}. Authorize it if asked.");
            frame.replace("{model}", &crate::model_family::model_family(snap).name)
        }
        SetupStage::Restarting => gettextrs::gettext("Starting dictation…"),
    }
}

/// A stage as the log says it: a download once, not at every byte count.
fn stage_log(stage: &SetupStage) -> String {
    match stage {
        SetupStage::Checking => "checking the connections and snapd".to_owned(),
        SetupStage::Waiting(ApplyProgress::Download { name, total, .. }) => {
            format!(
                "waiting for snapd to download {name} ({})",
                glib::format_size(*total)
            )
        }
        SetupStage::Waiting(ApplyProgress::Change { summary }) => {
            format!("waiting for snapd: {summary}")
        }
        SetupStage::Connecting(snap) => format!("connecting myna:backend to {snap}"),
        SetupStage::Restarting => "restarting snap.myna.myna.service".to_owned(),
    }
}

fn step_name(step: Step) -> &'static str {
    match step {
        Step::Welcome => "welcome",
        Step::Components => "components",
        Step::Shortcut => "shortcut",
    }
}

fn step_named(name: &str) -> Option<Step> {
    [Step::Welcome, Step::Components, Step::Shortcut]
        .into_iter()
        .find(|step| step_name(*step) == name)
}

fn step_title(step: Step) -> String {
    match step {
        Step::Welcome => gettextrs::gettext("Dictation"),
        Step::Components => gettextrs::gettext("Install components"),
        Step::Shortcut => gettextrs::gettext("How to dictate"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wait_on_snapd_keeps_its_english_summary_for_the_log() {
        let stage = SetupStage::Waiting(ApplyProgress::Change {
            summary: "Auto-refresh snap \"myna\"".to_owned(),
        });

        assert_eq!(
            stage_text(&stage),
            "Waiting for other software changes to finish…"
        );
        assert_eq!(
            stage_log(&stage),
            "waiting for snapd: Auto-refresh snap \"myna\""
        );
    }

    #[test]
    fn a_download_over_100_mb_is_sized_in_whole_megabytes() {
        assert_eq!(download_size(10_133_504), glib::format_size(10_133_504));
        assert_eq!(download_size(775_593_984), "776\u{a0}MB");
        assert_eq!(download_size(100_000_000), "100\u{a0}MB");
        assert_eq!(download_size(99_999_999), glib::format_size(99_999_999));
        assert_eq!(download_size(999_499_999), "999\u{a0}MB");
        assert_eq!(download_size(999_500_000), glib::format_size(999_500_000));
        assert_eq!(
            download_size(4_172_693_504),
            glib::format_size(4_172_693_504)
        );
    }
}
