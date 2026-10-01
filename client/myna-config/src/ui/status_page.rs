use adw::subclass::prelude::*;
use glib::subclass::types::ObjectSubclassIsExt;
use gtk::{glib, CompositeTemplate};
use gtk4 as gtk;
use libadwaita as adw;
use libadwaita::prelude::*;

mod imp {
    use super::*;

    #[derive(Default, CompositeTemplate)]
    #[template(resource = "/com/canonical/Myna/Config/ui/status-page.ui")]
    pub struct StatusPage {
        #[template_child]
        pub status: gtk::TemplateChild<adw::StatusPage>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for StatusPage {
        const NAME: &'static str = "StatusPage";
        type Type = super::StatusPage;
        type ParentType = adw::NavigationPage;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for StatusPage {}
    impl WidgetImpl for StatusPage {}
    impl NavigationPageImpl for StatusPage {}
}

glib::wrapper! {
    pub struct StatusPage(ObjectSubclass<imp::StatusPage>)
        @extends gtk::Widget, adw::NavigationPage,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl StatusPage {
    pub fn new() -> Self {
        super::register_resources();
        glib::Object::builder().build()
    }

    pub fn set_status(&self, title: &str, description: &str, icon_name: &str) {
        self.set_title(title);
        let status = self.imp().status.get();
        status.set_title(title);
        status.set_description(Some(&crate::markup::escape_markup(description)));
        status.set_icon_name(Some(icon_name));
    }

    /// Put `details` - raw error text, for a bug report - under a collapsed
    /// "Details" expander below the description.
    pub fn set_details(&self, details: &str) {
        let label = gtk::Label::builder()
            .label(details)
            .selectable(true)
            .wrap(true)
            .wrap_mode(gtk::pango::WrapMode::WordChar)
            .xalign(0.0)
            .css_classes(["monospace"])
            .build();
        let expander = gtk::Expander::builder()
            .label(gettextrs::gettext("Details"))
            .child(&label)
            .build();
        self.imp().status.get().set_child(Some(&expander));
    }

    pub fn status(&self) -> adw::StatusPage {
        self.imp().status.get()
    }
}
impl Default for StatusPage {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn has_header_bar(widget: &gtk::Widget) -> bool {
        widget.is::<adw::HeaderBar>()
            || std::iter::successors(widget.first_child(), |child| child.next_sibling())
                .any(|child| has_header_bar(&child))
    }

    #[test]
    fn a_status_page_leaves_the_header_to_its_window() {
        crate::ui::on_gtk_thread(|| {
            let page = StatusPage::new();
            assert!(!has_header_bar(page.upcast_ref()));
        });
    }

    #[test]
    fn details_wait_collapsed_under_the_description() {
        crate::ui::on_gtk_thread(|| {
            let page = StatusPage::new();
            page.set_details("schema com.canonical.Myna not found");
            let expander = page
                .status()
                .child()
                .and_downcast::<gtk::Expander>()
                .expect("an expander");
            assert!(!expander.is_expanded());
            let label = expander
                .child()
                .and_downcast::<gtk::Label>()
                .expect("a label");
            assert_eq!(label.label(), "schema com.canonical.Myna not found");
        });
    }
}
