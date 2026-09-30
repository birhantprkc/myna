use adw::subclass::prelude::*;
use glib::subclass::types::ObjectSubclassIsExt;
use gtk::{glib, CompositeTemplate};
use gtk4 as gtk;
use libadwaita as adw;

use crate::onboarding::ComponentId;

mod imp {
    use super::*;

    #[derive(Default, CompositeTemplate)]
    #[template(resource = "/com/canonical/Myna/Config/ui/onboarding-components.ui")]
    pub struct OnboardingComponents {
        #[template_child]
        pub flag_row: gtk::TemplateChild<adw::ActionRow>,
        #[template_child]
        pub flag_spinner: gtk::TemplateChild<gtk::Spinner>,
        #[template_child]
        pub flag_switch: gtk::TemplateChild<gtk::Switch>,
        #[template_child]
        pub component_list: gtk::TemplateChild<gtk::ListBox>,
        #[template_child]
        pub myna_row: gtk::TemplateChild<adw::ActionRow>,
        #[template_child]
        pub myna_button: gtk::TemplateChild<gtk::Button>,
        #[template_child]
        pub myna_installed: gtk::TemplateChild<gtk::Box>,
        #[template_child]
        pub myna_installing: gtk::TemplateChild<gtk::Box>,
        #[template_child]
        pub myna_spinner: gtk::TemplateChild<gtk::Spinner>,
        #[template_child]
        pub myna_progress: gtk::TemplateChild<gtk::Label>,
        #[template_child]
        pub model_row: gtk::TemplateChild<adw::ActionRow>,
        #[template_child]
        pub model_button: gtk::TemplateChild<gtk::Button>,
        #[template_child]
        pub model_installed: gtk::TemplateChild<gtk::Box>,
        #[template_child]
        pub model_installing: gtk::TemplateChild<gtk::Box>,
        #[template_child]
        pub model_spinner: gtk::TemplateChild<gtk::Spinner>,
        #[template_child]
        pub model_progress: gtk::TemplateChild<gtk::Label>,
        #[template_child]
        pub extension_row: gtk::TemplateChild<adw::ActionRow>,
        #[template_child]
        pub extension_button: gtk::TemplateChild<gtk::Button>,
        #[template_child]
        pub extension_installed: gtk::TemplateChild<gtk::Box>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for OnboardingComponents {
        const NAME: &'static str = "OnboardingComponents";
        type Type = super::OnboardingComponents;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            <crate::ui::BalancedLabel as glib::prelude::StaticTypeExt>::ensure_type();
            klass.bind_template();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for OnboardingComponents {}
    impl WidgetImpl for OnboardingComponents {}
    impl BinImpl for OnboardingComponents {}
}

glib::wrapper! {
    pub struct OnboardingComponents(ObjectSubclass<imp::OnboardingComponents>)
        @extends gtk::Widget, adw::Bin,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

/// One installable component's row: its button, or the check that replaces it.
#[derive(Clone)]
pub struct ComponentRow {
    pub row: adw::ActionRow,
    pub button: gtk::Button,
    pub installed: gtk::Box,
    /// What replaces the button while snapd installs it; the extension is
    /// not installed through snapd.
    pub installing: Option<RowProgress>,
}

/// A spinner and how far the install has come.
#[derive(Clone)]
pub struct RowProgress {
    pub container: gtk::Box,
    pub spinner: gtk::Spinner,
    pub label: gtk::Label,
}

impl OnboardingComponents {
    pub fn new() -> Self {
        super::register_resources();
        glib::Object::builder().build()
    }

    pub fn flag_row(&self) -> adw::ActionRow {
        self.imp().flag_row.get()
    }

    pub fn flag_spinner(&self) -> gtk::Spinner {
        self.imp().flag_spinner.get()
    }

    pub fn flag_switch(&self) -> gtk::Switch {
        self.imp().flag_switch.get()
    }

    pub fn component_list(&self) -> gtk::ListBox {
        self.imp().component_list.get()
    }

    /// The row of `id`; the flag has a switch row of its own instead.
    pub fn row(&self, id: ComponentId) -> Option<ComponentRow> {
        let imp = self.imp();
        let progress = |container: &gtk::TemplateChild<gtk::Box>,
                        spinner: &gtk::TemplateChild<gtk::Spinner>,
                        label: &gtk::TemplateChild<gtk::Label>| {
            Some(RowProgress {
                container: container.get(),
                spinner: spinner.get(),
                label: label.get(),
            })
        };
        let (row, button, installed, installing) = match id {
            ComponentId::UserDaemons => return None,
            ComponentId::Myna => (
                &imp.myna_row,
                &imp.myna_button,
                &imp.myna_installed,
                progress(&imp.myna_installing, &imp.myna_spinner, &imp.myna_progress),
            ),
            ComponentId::Model => (
                &imp.model_row,
                &imp.model_button,
                &imp.model_installed,
                progress(
                    &imp.model_installing,
                    &imp.model_spinner,
                    &imp.model_progress,
                ),
            ),
            ComponentId::ShellExtension => (
                &imp.extension_row,
                &imp.extension_button,
                &imp.extension_installed,
                None,
            ),
        };
        Some(ComponentRow {
            row: row.get(),
            button: button.get(),
            installed: installed.get(),
            installing,
        })
    }
}

impl Default for OnboardingComponents {
    fn default() -> Self {
        Self::new()
    }
}
