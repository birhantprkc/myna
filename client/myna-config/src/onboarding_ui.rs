//! GTK/libadwaita wiring for the onboarding wizard.
//!
//! Thin, like [`crate::backend_ui`]: [`crate::onboarding`] decides what is
//! missing, how to install it, and when the flow may advance; this module
//! renders that, connects and restarts what the user installed, and hands
//! control back to the settings window when the user is done.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use gtk::glib;
use gtk4 as gtk;
use libadwaita as adw;
use libadwaita::prelude::*;

use crate::active_backend::ensure_backend_active;
use crate::adapters::snap_backend::SnapBackendRepository;
use crate::adapters::system_configurator::PkexecSystemConfigurator;
use crate::command::{CancellationToken, GioCommandRunner};
use crate::onboarding::{
    assess, can_advance, needs_onboarding, Component, Machine, Step, RECOMMENDED_BACKEND_SNAP,
};
use crate::ports::{BackendRepository, SystemConfigurator};
use crate::ui;

pub struct OnboardingUi {
    window: ui::OnboardingWindow,
    components_page: ui::OnboardingComponents,
    shortcut_page: ui::OnboardingShortcut,
    repository: Rc<dyn BackendRepository>,
    configurator: Rc<dyn SystemConfigurator>,
    step: Cell<Step>,
    components: RefCell<Vec<Component>>,
    busy: Cell<bool>,
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

        let ui = Rc::new(Self {
            window: window.clone(),
            components_page,
            shortcut_page: shortcut_page.clone(),
            repository,
            configurator,
            step: Cell::new(Step::first()),
            components: RefCell::new(initial),
            busy: Cell::new(false),
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
        crate::shortcut_ui::ShortcutControl::attach(
            shortcut_page.shortcut_box(),
            shortcut_page.shortcut_button(),
            window.overlay(),
            false,
            Box::new({
                let description = shortcut_page.description();
                move |state, path| {
                    description.set_label(&crate::shortcut_ui::onboarding_description(state, path))
                }
            }),
        );
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
        window.connect_close_request({
            let held = RefCell::new(Some(ui.clone()));
            move |_| {
                held.borrow_mut().take();
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
    /// step.
    fn refresh_assessment(self: &Rc<Self>) {
        let ui = Rc::downgrade(self);
        let repository = self.repository.clone();
        glib::spawn_future_local(async move {
            let components = assess_machine(repository.as_ref()).await;
            let Some(ui) = ui.upgrade() else {
                return;
            };
            ui.components.replace(components);
            ui.render();
        });
    }

    fn advance(self: &Rc<Self>) {
        // The gate lives here, not on the button: a step can also be advanced
        // by activating the button from the keyboard or a screen reader, and
        // an insensitive widget still emits `clicked` when told to.
        if self.busy.get() || !can_advance(self.step.get(), &self.components.borrow()) {
            return;
        }
        match self.step.get().next() {
            Some(step) if self.step.get() == Step::Components => self.finish_setup(step),
            Some(step) => self.window.navigation().push_by_tag(step_name(step)),
            None => {
                self.notify_finished();
                self.window.close();
            }
        }
    }

    /// Connect the backend and restart the daemon against it, so the next
    /// step finds dictation running. A store install usually auto-connects
    /// the backend; when it did not, snapd asks polkit once.
    fn finish_setup(self: &Rc<Self>, next: Step) {
        if self.busy.replace(true) {
            return;
        }
        self.render();
        let ui = Rc::downgrade(self);
        let repository = self.repository.clone();
        let configurator = self.configurator.clone();
        glib::spawn_future_local(async move {
            let outcome = ensure_backend_active(
                repository.as_ref(),
                configurator.as_ref(),
                RECOMMENDED_BACKEND_SNAP,
            )
            .await;
            let Some(ui) = ui.upgrade() else {
                return;
            };
            ui.busy.set(false);
            ui.render();
            match outcome {
                Ok(()) => ui.window.navigation().push_by_tag(step_name(next)),
                Err(message) => {
                    ui.report_failure(&gettextrs::gettext("Could not set up dictation"), &message)
                }
            }
        });
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
        } else {
            forward.set_label(&gettextrs::gettext("Done"));
            forward.add_css_class("suggested-action");
        }
        forward.set_sensitive(!self.busy.get() && can_advance(step, &components));
        self.window
            .installed_status()
            .set_visible(step == Step::Components && !needs_onboarding(&components));

        if step == Step::Components {
            self.render_components(&components);
        }
    }

    fn render_components(self: &Rc<Self>, components: &[Component]) {
        self.components_page
            .subtitle()
            .set_label(&if !needs_onboarding(components) {
                gettextrs::gettext("Everything Dictation needs is installed.")
            } else {
                gettextrs::gettext("You need to install some components for Dictation to work.")
            });
    }

    fn report_failure(self: &Rc<Self>, title: &str, details: &str) {
        let dialog = ui::OperationErrorDialog::new(title, title, details);
        dialog.present(Some(&self.window));
    }
}

fn copy_command(window: &ui::OnboardingWindow, command: &str) {
    gtk::prelude::WidgetExt::display(window)
        .clipboard()
        .set_text(command);
    window
        .overlay()
        .add_toast(adw::Toast::new(&gettextrs::gettext(
            "Command copied. Paste it into a terminal.",
        )));
}

/// One assessment of what dictation is missing on this machine: one `snap
/// list` and one discovery. A surface that cannot be read counts as nothing
/// found, which opens the wizard: the flow then shows what it could not verify
/// rather than a settings window with no backends and no explanation.
pub async fn assess_machine(repository: &dyn BackendRepository) -> Vec<Component> {
    let cancellation = CancellationToken::new();
    let installed = repository
        .installed_snaps(cancellation.clone())
        .await
        .unwrap_or_default();
    let backends = repository
        .discover(cancellation)
        .await
        .map(|snapshot| snapshot.backends().len())
        .unwrap_or_default();
    assess(Machine::new(&installed, backends))
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
