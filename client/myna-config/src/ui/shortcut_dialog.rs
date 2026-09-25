use std::cell::RefCell;

use adw::subclass::prelude::*;
use glib::subclass::types::ObjectSubclassIsExt;
use gtk::{gdk, glib, glib::translate::IntoGlib, CompositeTemplate};
use gtk4 as gtk;
use libadwaita as adw;
use libadwaita::prelude::*;

type Captured = Box<dyn Fn(&str)>;

mod imp {
    use super::*;

    #[derive(Default, CompositeTemplate)]
    #[template(resource = "/com/canonical/Myna/Config/ui/shortcut-dialog.ui")]
    pub struct ShortcutDialog {
        pub captured: RefCell<Option<Captured>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ShortcutDialog {
        const NAME: &'static str = "ShortcutDialog";
        type Type = super::ShortcutDialog;
        type ParentType = adw::Dialog;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for ShortcutDialog {
        fn constructed(&self) {
            self.parent_constructed();
            let keys = gtk::EventControllerKey::new();
            keys.set_propagation_phase(gtk::PropagationPhase::Capture);
            let dialog = self.obj().downgrade();
            keys.connect_key_pressed(move |_, key, _, state| {
                dialog
                    .upgrade()
                    .map_or(glib::Propagation::Proceed, |dialog| {
                        dialog.press(key, state)
                    })
            });
            self.obj().add_controller(keys);
            // The desktop grabs the keys it uses (Super+L, the Calculator
            // key) before any window sees them, so it pauses those while
            // capturing, as GNOME Settings does.
            self.obj().connect_map(|dialog| {
                if let Some(toplevel) = toplevel(dialog) {
                    toplevel.inhibit_system_shortcuts(None::<&gdk::Event>);
                }
            });
            self.obj().connect_unmap(|dialog| {
                if let Some(toplevel) = toplevel(dialog) {
                    toplevel.restore_system_shortcuts();
                }
            });
        }
    }

    fn toplevel(dialog: &super::ShortcutDialog) -> Option<gdk::Toplevel> {
        dialog.root()?.surface()?.downcast::<gdk::Toplevel>().ok()
    }
    impl WidgetImpl for ShortcutDialog {}
    impl AdwDialogImpl for ShortcutDialog {}
}

glib::wrapper! {
    /// Captures the key combination for the desktop dictation shortcut.
    pub struct ShortcutDialog(ObjectSubclass<imp::ShortcutDialog>)
        @extends gtk::Widget, adw::Dialog,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl ShortcutDialog {
    pub fn new() -> Self {
        super::register_resources();
        glib::Object::builder().build()
    }

    /// Run `captured` with the accelerator the user presses.
    pub fn connect_captured(&self, captured: impl Fn(&str) + 'static) {
        self.imp().captured.replace(Some(Box::new(captured)));
    }

    /// Escape cancels. A combination with Ctrl, Alt or Super is captured, and
    /// so is a lone function or media key, which closes the dialog. Anything
    /// else, such as a bare letter that would take over typing, is ignored.
    pub fn press(&self, key: gdk::Key, state: gdk::ModifierType) -> glib::Propagation {
        let modifiers = state & gtk::accelerator_get_default_mod_mask();
        if key == gdk::Key::Escape && modifiers.is_empty() {
            self.close();
            return glib::Propagation::Stop;
        }
        let key = key.to_lower();
        let chord = gdk::ModifierType::CONTROL_MASK
            | gdk::ModifierType::ALT_MASK
            | gdk::ModifierType::SUPER_MASK;
        if !(modifiers.intersects(chord) || stands_alone(key))
            || !gtk::accelerator_valid(key, modifiers)
        {
            return glib::Propagation::Proceed;
        }
        if let Some(captured) = self.imp().captured.borrow().as_ref() {
            captured(&accelerator_name(key, modifiers));
        }
        self.close();
        glib::Propagation::Stop
    }
}

/// The XF86 media keys: Calculator, Mail, ...
const MEDIA_KEYS: std::ops::RangeInclusive<u32> = 0x1008_ff00..=0x1008_ffff;

/// Keys that type nothing and move nothing: F1 to F35 and the media keys.
fn stands_alone(key: gdk::Key) -> bool {
    (gdk::Key::F1.into_glib()..=gdk::Key::F35.into_glib()).contains(&key.into_glib())
        || MEDIA_KEYS.contains(&key.into_glib())
}

/// GTK names a media key without its `XF86` prefix, which the desktop's
/// keysym lookup does not know, so the binding would never fire.
fn accelerator_name(key: gdk::Key, modifiers: gdk::ModifierType) -> String {
    let name = gtk::accelerator_name(key, modifiers).to_string();
    match key.name() {
        Some(bare) if MEDIA_KEYS.contains(&key.into_glib()) && !bare.starts_with("XF86") => name
            .strip_suffix(bare.as_str())
            .map_or(name.clone(), |modifiers| format!("{modifiers}XF86{bare}")),
        _ => name,
    }
}

impl Default for ShortcutDialog {
    fn default() -> Self {
        Self::new()
    }
}
