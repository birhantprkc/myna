use adw::subclass::prelude::*;
use glib::subclass::types::ObjectSubclassIsExt;
use gtk::{glib, CompositeTemplate};
use gtk4 as gtk;
use libadwaita as adw;

mod imp {
    use super::*;

    #[derive(Default, CompositeTemplate)]
    #[template(resource = "/com/canonical/Myna/Config/ui/install-models-dialog.ui")]
    pub struct InstallModelsDialog {
        #[template_child]
        pub families: gtk::TemplateChild<adw::PreferencesGroup>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for InstallModelsDialog {
        const NAME: &'static str = "InstallModelsDialog";
        type Type = super::InstallModelsDialog;
        type ParentType = adw::Dialog;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for InstallModelsDialog {}
    impl WidgetImpl for InstallModelsDialog {}
    impl AdwDialogImpl for InstallModelsDialog {}
}

glib::wrapper! {
    /// The model families Myna knows that are not installed, each opening
    /// its App Center page.
    pub struct InstallModelsDialog(ObjectSubclass<imp::InstallModelsDialog>)
        @extends gtk::Widget, adw::Dialog,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl InstallModelsDialog {
    pub fn new() -> Self {
        super::register_resources();
        glib::Object::builder().build()
    }

    pub fn families(&self) -> adw::PreferencesGroup {
        self.imp().families.get()
    }
}

impl Default for InstallModelsDialog {
    fn default() -> Self {
        Self::new()
    }
}
