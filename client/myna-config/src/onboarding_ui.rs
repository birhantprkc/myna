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
    assess, can_advance, outstanding, Component, ComponentId, Machine, Remedy, Step, StoreSnap,
    RECOMMENDED_BACKEND_SNAP, RECOMMENDED_MODEL_MEGABYTES, SHELL_EXTENSION_UUID,
};
use crate::ports::{BackendRepository, SystemConfigurator};
use crate::ui;

pub struct OnboardingUi {
    window: ui::OnboardingWindow,
    welcome: ui::OnboardingWelcome,
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
            finished,
        )
    }

    pub fn present_with_ports(
        application: &adw::Application,
        initial: Vec<Component>,
        repository: Rc<dyn BackendRepository>,
        configurator: Rc<dyn SystemConfigurator>,
        finished: Box<dyn Fn()>,
    ) -> Rc<Self> {
        let window = ui::OnboardingWindow::new(application);
        let welcome = ui::OnboardingWelcome::new();
        let components_page = ui::OnboardingComponents::new();
        let shortcut_page = ui::OnboardingShortcut::new();

        let stack = window.stack();
        stack.add_named(&welcome, Some(step_name(Step::Welcome)));
        stack.add_named(&components_page, Some(step_name(Step::Components)));
        stack.add_named(&shortcut_page, Some(step_name(Step::Shortcut)));

        let ui = Rc::new(Self {
            window: window.clone(),
            welcome: welcome.clone(),
            components_page,
            shortcut_page: shortcut_page.clone(),
            repository,
            configurator,
            step: Cell::new(Step::first()),
            components: RefCell::new(initial),
            busy: Cell::new(false),
            finished: RefCell::new(Some(finished)),
        });

        welcome.start_button().connect_clicked({
            let ui = Rc::downgrade(&ui);
            move |_| {
                if let Some(ui) = ui.upgrade() {
                    ui.advance();
                }
            }
        });
        window.forward_button().connect_clicked({
            let ui = Rc::downgrade(&ui);
            move |_| {
                if let Some(ui) = ui.upgrade() {
                    ui.advance();
                }
            }
        });
        window.back_button().connect_clicked({
            let ui = Rc::downgrade(&ui);
            move |_| {
                if let Some(ui) = ui.upgrade() {
                    ui.retreat();
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
                move |state| {
                    description.set_label(&crate::shortcut_ui::onboarding_description(state))
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
            let Some(ui) = ui.upgrade() else {
                return;
            };
            let machine = Machine::new(&installed, backends, shell_extension_installed());
            ui.components.replace(assess(machine));
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
            Some(step) => {
                self.step.set(step);
                self.render();
            }
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
            match outcome {
                Ok(()) => ui.step.set(next),
                Err(message) => {
                    ui.report_failure(&gettextrs::gettext("Could not set up dictation"), &message)
                }
            }
            ui.render();
        });
    }

    fn retreat(self: &Rc<Self>) {
        if let Some(step) = self.step.get().previous() {
            self.step.set(step);
            self.render();
        }
    }

    /// The widgets the headless probe drives the wizard through. It holds
    /// these and drops the controller, the way the application does.
    pub fn window(&self) -> ui::OnboardingWindow {
        self.window.clone()
    }

    pub fn start_button(&self) -> gtk::Button {
        self.welcome.start_button()
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

        self.window.window_title().set_title(&step_title(step));
        self.window.set_title(Some(&step_title(step)));
        self.window.stack().set_visible_child_name(step_name(step));

        let back = self.window.back_button();
        back.set_visible(step.previous().is_some());
        back.set_sensitive(!self.busy.get());

        let forward = self.window.forward_button();
        // The welcome step has its own button in the middle of the page, so
        // the action bar carries nothing there.
        forward.set_visible(step != Step::Welcome);
        forward.set_label(&if step.next().is_some() {
            gettextrs::gettext("Next")
        } else {
            gettextrs::gettext("Done")
        });
        forward.set_sensitive(!self.busy.get() && can_advance(step, &components));

        if step == Step::Components {
            self.render_components(&components);
        }
    }

    fn render_components(self: &Rc<Self>, components: &[Component]) {
        let list = self.components_page.list();
        while let Some(child) = list.first_child() {
            list.remove(&child);
        }

        let missing = outstanding(components);
        if missing.is_empty() {
            self.components_page
                .subtitle()
                .set_label(&gettextrs::gettext(
                    "Everything Dictation needs is installed.",
                ));
            return;
        }
        self.components_page
            .subtitle()
            .set_label(&gettextrs::gettext(
                "You need to install some components for Dictation to work.",
            ));

        for component in missing {
            list.append(&self.component_row(component));
        }
    }

    fn component_row(self: &Rc<Self>, component: Component) -> adw::ActionRow {
        let row = adw::ActionRow::builder()
            .title(component_title(component.id))
            .subtitle(component_detail(component.id))
            .build();
        let primary = match component.remedy {
            Remedy::Store(snap) if snap.installs_from_app_center() => {
                let copy = gtk::Button::builder()
                    .icon_name("edit-copy-symbolic")
                    .tooltip_text(snap.install_command())
                    .valign(gtk::Align::Center)
                    .css_classes(["flat"])
                    .build();
                copy.update_property(&[gtk::accessible::Property::Label(&gettextrs::gettext(
                    "Copy install command",
                ))]);
                copy.connect_clicked(self.copy_handler(snap.install_command()));
                row.add_suffix(&copy);

                let install = gtk::Button::builder()
                    .label(gettextrs::gettext("Install"))
                    .valign(gtk::Align::Center)
                    .build();
                install.connect_clicked({
                    let window = self.window.clone();
                    move |_| open_app_center(&window, snap)
                });
                install
            }
            Remedy::Store(snap) => {
                let copy = gtk::Button::builder()
                    .label(gettextrs::gettext("Copy Command"))
                    .tooltip_text(snap.install_command())
                    .valign(gtk::Align::Center)
                    .build();
                copy.connect_clicked(self.copy_handler(snap.install_command()));
                copy
            }
            Remedy::Explain => {
                let button = gtk::Button::builder()
                    .label(gettextrs::gettext("How to install"))
                    .valign(gtk::Align::Center)
                    .build();
                button.connect_clicked({
                    let ui = Rc::downgrade(self);
                    move |_| {
                        if let Some(ui) = ui.upgrade() {
                            ui.show_extension_instructions();
                        }
                    }
                });
                button
            }
        };
        row.add_suffix(&primary);
        row.set_activatable_widget(Some(&primary));
        row
    }

    fn copy_handler(&self, command: String) -> impl Fn(&gtk::Button) + 'static {
        let window = self.window.clone();
        move |_| copy_command(&window, &command)
    }

    fn show_extension_instructions(self: &Rc<Self>) {
        let dialog = adw::AlertDialog::new(
            Some(&gettextrs::gettext("Install the shell extension")),
            Some(&gettextrs::gettext(
                "The extension is not published in a store yet. Copy it into the extensions directory and enable it, then log out and back in.",
            )),
        );
        dialog.add_response("close", &gettextrs::gettext("Close"));
        dialog.add_response("copy", &gettextrs::gettext("Copy command"));
        dialog.set_response_appearance("copy", adw::ResponseAppearance::Suggested);
        dialog.connect_response(None, {
            let window = self.window.clone();
            move |_, response| {
                if response == "copy" {
                    copy_command(
                        &window,
                        &format!("gnome-extensions enable {SHELL_EXTENSION_UUID}"),
                    );
                }
            }
        });
        dialog.present(Some(&self.window));
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

fn open_app_center(window: &ui::OnboardingWindow, snap: StoreSnap) {
    let launcher = gtk::UriLauncher::new(&snap.app_center_uri());
    launcher.launch(Some(window), None::<&gio::Cancellable>, {
        let window = window.clone();
        move |result| {
            if result.is_err() {
                copy_command(&window, &snap.install_command());
            }
        }
    });
}

/// Whether the HUD's GNOME Shell extension is installed for this user or
/// system-wide.
pub fn shell_extension_installed() -> bool {
    let data_home = gio::glib::user_data_dir();
    let system: Vec<std::path::PathBuf> = gio::glib::system_data_dirs();
    crate::onboarding::shell_extension_directories(&data_home, &system)
        .iter()
        .any(|directory| directory.is_dir())
}

fn step_name(step: Step) -> &'static str {
    match step {
        Step::Welcome => "welcome",
        Step::Components => "components",
        Step::Shortcut => "shortcut",
    }
}

fn step_title(step: Step) -> String {
    match step {
        Step::Welcome => gettextrs::gettext("Dictation"),
        Step::Components => gettextrs::gettext("Install components"),
        Step::Shortcut => gettextrs::gettext("How to dictate"),
    }
}

fn component_title(id: ComponentId) -> String {
    match id {
        ComponentId::Myna => gettextrs::gettext("Myna"),
        ComponentId::Model => gettextrs::gettext("Recommended speech-to-text model"),
        ComponentId::ShellExtension => gettextrs::gettext("Shell extension"),
    }
}

fn component_detail(id: ComponentId) -> String {
    match id {
        ComponentId::Myna => gettextrs::gettext("The dictation client itself"),
        ComponentId::Model => {
            // Translators: the model name, then its installed size.
            gettextrs::gettext("Parakeet · {size} MB")
                .replace("{size}", &RECOMMENDED_MODEL_MEGABYTES.to_string())
        }
        ComponentId::ShellExtension => {
            gettextrs::gettext("Needed to show dictation status in the desktop")
        }
    }
}
