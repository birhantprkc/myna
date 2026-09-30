//! GTK/libadwaita wiring for the onboarding wizard.
//!
//! Thin, like [`crate::backend_ui`]: [`crate::onboarding`] decides what is
//! missing, how to install it, and when the flow may advance; this module
//! renders that, connects and restarts what the user installed, and hands
//! control back to the settings window when the user is done.

use std::cell::{Cell, RefCell};
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
    assess, can_advance, completes, needs_onboarding, polls, Component, ComponentState, Machine,
    Step, RECOMMENDED_BACKEND_SNAP, SHELL_EXTENSION_UUID,
};
use crate::ports::{BackendRepository, ShellExtensions, SystemConfigurator};
use crate::snap_changes::ApplyProgress;
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

pub struct OnboardingUi {
    window: ui::OnboardingWindow,
    shortcut_page: ui::OnboardingShortcut,
    shortcut: Rc<crate::shortcut_ui::ShortcutControl>,
    repository: Rc<dyn BackendRepository>,
    configurator: Rc<dyn SystemConfigurator>,
    extensions: Rc<dyn ShellExtensions>,
    step: Cell<Step>,
    components: RefCell<Vec<Component>>,
    /// Why the last assessment could not read the machine.
    problem: RefCell<Option<String>>,
    busy: Cell<bool>,
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

        let commands = crate::onboarding::install_commands();
        components_page.commands().set_label(&commands);
        components_page.copy_button().connect_clicked({
            let window = window.clone();
            move |_| copy_command(&window, &commands)
        });

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
            shortcut_page: shortcut_page.clone(),
            shortcut,
            repository,
            configurator,
            extensions,
            step: Cell::new(Step::first()),
            components: RefCell::new(initial),
            problem: RefCell::default(),
            busy: Cell::new(false),
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
        let ui = Rc::downgrade(self);
        let repository = self.repository.clone();
        let configurator = self.configurator.clone();
        let extensions = self.extensions.clone();
        glib::spawn_future_local(async move {
            let (components, problem) = read_machine(
                repository.as_ref(),
                configurator.as_ref(),
                extensions.as_ref(),
            )
            .await;
            let Some(ui) = ui.upgrade() else {
                return;
            };
            ui.assessing.set(false);
            match &problem {
                Some(problem) => ui.log(&format!("assessment: {problem}")),
                None => ui.log(&format!("assessment: {}", describe(&components))),
            }
            ui.problem.replace(problem);
            let before = ui.components.replace(components);
            // The user installed the last piece while watching: finish for
            // them, as Next would.
            let finish = ui.step.get() == Step::Components
                && !ui.busy.get()
                && completes(&before, &ui.components.borrow());
            if finish {
                ui.finish_setup(Step::Shortcut, true);
            } else {
                ui.render();
            }
        });
    }

    fn advance(self: &Rc<Self>) {
        // The gate lives here, not on the button: a step can also be advanced
        // by activating the button from the keyboard or a screen reader, and
        // an insensitive widget still emits `clicked` when told to.
        if self.busy.get() || !can_advance(self.step.get(), &self.components.borrow()) {
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
        let components = self.components.borrow().clone();

        self.window.set_title(Some(&step_title(step)));
        // Setting up restarts the daemon; leaving mid-way would strand it.
        if let Some(page) = self.window.navigation().visible_page() {
            page.set_can_pop(!self.busy.get());
        }

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

fn copy_command(window: &ui::OnboardingWindow, command: &str) {
    gtk::prelude::WidgetExt::display(window)
        .clipboard()
        .set_text(command);
    window
        .overlay()
        .add_toast(adw::Toast::new(&gettextrs::gettext(
            "Commands copied. Paste them into a terminal.",
        )));
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
    let (components, problem) = read_machine(repository, configurator, extensions).await;
    let found = problem.unwrap_or_else(|| describe(&components));
    glib::g_message!(crate::LOG_DOMAIN, "onboarding assessment: {found}");
    components
}

/// [`assess_machine`], and what it could not read.
async fn read_machine(
    repository: &dyn BackendRepository,
    configurator: &dyn SystemConfigurator,
    extensions: &dyn ShellExtensions,
) -> (Vec<Component>, Option<String>) {
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
    (assess(machine), problem)
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

/// A stage as the footer says it, beside the spinner.
fn stage_text(stage: &SetupStage) -> String {
    match stage {
        SetupStage::Checking => gettextrs::gettext("Checking the installation…"),
        SetupStage::Waiting(progress @ ApplyProgress::Download { .. }) => {
            crate::backend_ui::apply_progress_text(progress)
        }
        SetupStage::Waiting(ApplyProgress::Change { summary }) => {
            // TRANSLATORS: {change} is snapd's own summary of what it is doing, in English.
            let frame = gettextrs::gettext("Waiting for snapd: {change}");
            frame.replace("{change}", summary)
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
