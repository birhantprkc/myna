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
        pub description: gtk::TemplateChild<crate::ui::BalancedLabel>,
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
        pub myna_control: gtk::TemplateChild<crate::ui::InstallControl>,
        #[template_child]
        pub model_row: gtk::TemplateChild<adw::ActionRow>,
        #[template_child]
        pub model_control: gtk::TemplateChild<crate::ui::InstallControl>,
        #[template_child]
        pub extension_row: gtk::TemplateChild<adw::ActionRow>,
        #[template_child]
        pub extension_control: gtk::TemplateChild<crate::ui::InstallControl>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for OnboardingComponents {
        const NAME: &'static str = "OnboardingComponents";
        type Type = super::OnboardingComponents;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            <crate::ui::BalancedLabel as glib::prelude::StaticTypeExt>::ensure_type();
            <crate::ui::InstallControl as glib::prelude::StaticTypeExt>::ensure_type();
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

/// One installable component's row and its install control.
#[derive(Clone)]
pub struct ComponentRow {
    pub row: adw::ActionRow,
    pub control: crate::ui::InstallControl,
    pub button: gtk::Button,
    pub installed: gtk::Box,
    /// What replaces the button while snapd installs it, or gnome-shell
    /// enables the extension.
    pub installing: crate::ui::RowProgress,
}

impl OnboardingComponents {
    pub fn new() -> Self {
        super::register_resources();
        glib::Object::builder().build()
    }

    /// The line under the title.
    pub fn description(&self) -> gtk::Label {
        self.imp().description.text_label()
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
        let (row, control) = match id {
            ComponentId::UserDaemons => return None,
            ComponentId::Myna => (&imp.myna_row, &imp.myna_control),
            ComponentId::Model => (&imp.model_row, &imp.model_control),
            ComponentId::ShellExtension => (&imp.extension_row, &imp.extension_control),
        };
        let control = control.get();
        Some(ComponentRow {
            row: row.get(),
            button: control.button(),
            installed: control.installed(),
            installing: control.progress(),
            control,
        })
    }
}

impl Default for OnboardingComponents {
    fn default() -> Self {
        Self::new()
    }
}
