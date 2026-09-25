//! The GNOME custom shortcut that pokes the daemon's control socket: the
//! dictation key where the portal has no GlobalShortcuts. The same entry the
//! snap's `myna.install-shortcut` writes.

use gio::glib;
use gio::prelude::*;

const MEDIA_KEYS: &str = "org.gnome.settings-daemon.plugins.media-keys";
const CUSTOM_KEYBINDING: &str = "org.gnome.settings-daemon.plugins.media-keys.custom-keybinding";
const LIST_KEY: &str = "custom-keybindings";
const PATH: &str = "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/myna/";

pub struct DesktopShortcut {
    list: gio::Settings,
    entry: gio::Settings,
}

impl DesktopShortcut {
    /// `None` off GNOME, where there is no media-keys schema.
    pub fn open() -> Option<Self> {
        Self::open_with(&gio::SettingsSchemaSource::default()?, None)
    }

    pub fn open_with(
        source: &gio::SettingsSchemaSource,
        backend: Option<&gio::SettingsBackend>,
    ) -> Option<Self> {
        let list = source.lookup(MEDIA_KEYS, true)?;
        let entry = source.lookup(CUSTOM_KEYBINDING, true)?;
        Some(Self {
            list: gio::Settings::new_full(&list, backend, None),
            entry: gio::Settings::new_full(&entry, backend, Some(PATH)),
        })
    }

    /// The installed accelerator. An entry GNOME does not list binds nothing.
    pub fn binding(&self) -> Option<String> {
        let binding = self.entry.string("binding");
        (self.listed() && !binding.is_empty()).then(|| binding.to_string())
    }

    /// Bind `binding` to `command`, keeping every other custom shortcut.
    pub fn install(&self, name: &str, command: &str, binding: &str) -> Result<(), glib::BoolError> {
        self.entry.set_string("name", name)?;
        self.entry.set_string("command", command)?;
        self.entry.set_string("binding", binding)?;
        if !self.listed() {
            let mut paths: Vec<String> = self
                .list
                .strv(LIST_KEY)
                .iter()
                .map(|path| path.to_string())
                .collect();
            paths.push(PATH.to_owned());
            self.list.set_strv(LIST_KEY, paths)?;
        }
        Ok(())
    }

    /// Run `changed` whenever the binding may have changed, from here or from
    /// the desktop's keyboard settings.
    pub fn connect_changed(&self, changed: impl Fn() + Clone + 'static) {
        for settings in [&self.list, &self.entry] {
            let changed = changed.clone();
            settings.connect_changed(None, move |_, _| changed());
        }
    }

    fn listed(&self) -> bool {
        self.list.strv(LIST_KEY).iter().any(|path| path == PATH)
    }
}
