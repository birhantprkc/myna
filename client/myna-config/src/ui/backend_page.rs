use adw::subclass::prelude::*;
use glib::subclass::types::ObjectSubclassIsExt;
use gtk::{glib, CompositeTemplate};
use gtk4 as gtk;
use libadwaita as adw;
use libadwaita::prelude::*;

mod imp {
    use super::*;

    #[derive(Default, CompositeTemplate)]
    #[template(resource = "/com/canonical/Myna/Config/ui/backend-page.ui")]
    pub struct BackendPage {
        #[template_child]
        pub preferences_page: gtk::TemplateChild<adw::PreferencesPage>,
        pub groups: std::cell::RefCell<Vec<adw::PreferencesGroup>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for BackendPage {
        const NAME: &'static str = "BackendPage";
        type Type = super::BackendPage;
        type ParentType = adw::NavigationPage;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for BackendPage {}
    impl WidgetImpl for BackendPage {}
    impl NavigationPageImpl for BackendPage {}
}

glib::wrapper! {
    pub struct BackendPage(ObjectSubclass<imp::BackendPage>)
        @extends gtk::Widget, adw::NavigationPage,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl BackendPage {
    pub fn new() -> Self {
        super::register_resources();
        glib::Object::builder().build()
    }

    pub fn preferences_page(&self) -> adw::PreferencesPage {
        self.imp().preferences_page.get()
    }

    pub fn set_display_title(&self, title: &str) {
        self.set_title(title);
        self.preferences_page().set_title(title);
    }

    /// Shows `groups` in place of the ones set before. The page itself stays,
    /// and with it where the user scrolled: GTK lays out the new groups
    /// before the scroll position is next clamped.
    pub fn set_groups(&self, groups: Vec<adw::PreferencesGroup>) {
        let page = self.preferences_page();
        for group in self.imp().groups.take() {
            let on_page = group
                .ancestor(adw::PreferencesPage::static_type())
                .is_some_and(|ancestor| ancestor == page);
            if on_page {
                page.remove(&group);
            }
        }
        for group in &groups {
            page.add(group);
        }
        self.imp().groups.replace(groups);
    }
}
impl Default for BackendPage {
    fn default() -> Self {
        Self::new()
    }
}
