//! gnome-shell's extension list over `org.gnome.Shell.Extensions` on the
//! session bus.
//!
//! gnome-shell scans the extension directories at login only, so what it
//! reports is cross-checked against the disk: a system copy it does not list
//! was installed after login, unless a user copy of the same uuid hides it.

use std::path::PathBuf;
use std::time::Duration;

use async_trait::async_trait;
use gio::glib::{self, Variant, VariantDict, VariantTy};
use gio::prelude::*;

use crate::onboarding::{
    extension_state, ExtensionCopies, ExtensionInfo, ExtensionRun, ExtensionState,
};
use crate::ports::ShellExtensions;

const SHELL_NAME: &str = "org.gnome.Shell";
const SHELL_PATH: &str = "/org/gnome/Shell";
const EXTENSIONS_INTERFACE: &str = "org.gnome.Shell.Extensions";
/// A shell that does not answer within this is treated as absent.
const CALL_TIMEOUT: Duration = Duration::from_secs(2);

/// `ExtensionType.SYSTEM` in gnome-shell's `extensionUtils.js`.
const TYPE_SYSTEM: f64 = 1.0;

pub struct GnomeShellExtensions {
    connection: Option<gio::DBusConnection>,
    data_dirs: Vec<PathBuf>,
    user_data_dir: PathBuf,
}

impl GnomeShellExtensions {
    /// The session bus and the system data directories.
    pub fn new() -> Self {
        Self {
            connection: None,
            data_dirs: glib::system_data_dirs(),
            user_data_dir: glib::user_data_dir(),
        }
    }

    pub fn with_connection(
        connection: gio::DBusConnection,
        data_dirs: Vec<PathBuf>,
        user_data_dir: PathBuf,
    ) -> Self {
        Self {
            connection: Some(connection),
            data_dirs,
            user_data_dir,
        }
    }

    async fn info(&self, uuid: &str) -> Option<ExtensionInfo> {
        let connection = match &self.connection {
            Some(connection) => connection.clone(),
            None => gio::bus_get_future(gio::BusType::Session).await.ok()?,
        };
        let reply = connection
            .call_future(
                // A peer-to-peer connection has no bus to route by name.
                connection
                    .flags()
                    .contains(gio::DBusConnectionFlags::MESSAGE_BUS_CONNECTION)
                    .then_some(SHELL_NAME),
                SHELL_PATH,
                EXTENSIONS_INTERFACE,
                "GetExtensionInfo",
                Some(&(uuid,).to_variant()),
                Some(VariantTy::new("(a{sv})").expect("valid type")),
                gio::DBusCallFlags::NO_AUTO_START,
                CALL_TIMEOUT.as_millis() as i32,
            )
            .await
            .map_err(|error| {
                glib::g_debug!(crate::LOG_DOMAIN, "no extension info for {uuid}: {error}");
            })
            .ok()?;
        parse_info(&reply.child_value(0))
    }

    fn copies_on_disk(&self, uuid: &str) -> ExtensionCopies {
        let has_copy = |dir: &PathBuf| {
            dir.join("gnome-shell/extensions")
                .join(uuid)
                .join("metadata.json")
                .is_file()
        };
        ExtensionCopies {
            system: self.data_dirs.iter().any(has_copy),
            user: has_copy(&self.user_data_dir),
        }
    }
}

impl Default for GnomeShellExtensions {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait(?Send)]
impl ShellExtensions for GnomeShellExtensions {
    async fn extension_state(&self, uuid: &str) -> ExtensionState {
        let info = self.info(uuid).await;
        extension_state(info, self.copies_on_disk(uuid))
    }
}

/// One `GetExtensionInfo` reply. An unknown uuid is an empty dictionary.
/// gnome-shell sends `type` and `state` as doubles.
pub fn parse_info(info: &Variant) -> Option<ExtensionInfo> {
    let info = VariantDict::new(Some(info));
    let kind = info.lookup::<f64>("type").ok()??;
    let state = info.lookup::<f64>("state").ok()??;
    // `ExtensionState` in gnome-shell's `extensionUtils.js`.
    let run = match state as i64 {
        // 8 and 7 are ACTIVATING and DEACTIVATING: read where it is heading.
        1 | 8 => ExtensionRun::Enabled,
        2 | 6 | 7 => ExtensionRun::Disabled,
        _ => ExtensionRun::Broken,
    };
    Some(ExtensionInfo {
        system: kind == TYPE_SYSTEM,
        run,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reply(entries: &[(&str, Variant)]) -> Variant {
        let dict = VariantDict::new(None);
        for (key, value) in entries {
            dict.insert_value(key, value);
        }
        dict.end()
    }

    #[test]
    fn an_unknown_uuid_is_no_info() {
        assert_eq!(parse_info(&reply(&[])), None);
    }

    #[test]
    fn type_and_state_are_read_as_doubles() {
        let info = |kind: f64, state: f64| {
            parse_info(&reply(&[
                ("type", kind.to_variant()),
                ("state", state.to_variant()),
                ("uuid", "myna-shell@canonical.com".to_variant()),
            ]))
        };
        assert_eq!(
            info(1.0, 1.0),
            Some(ExtensionInfo {
                system: true,
                run: ExtensionRun::Enabled
            })
        );
        assert_eq!(
            info(2.0, 2.0),
            Some(ExtensionInfo {
                system: false,
                run: ExtensionRun::Disabled
            })
        );
        assert_eq!(
            info(1.0, 6.0).map(|info| info.run),
            Some(ExtensionRun::Disabled)
        );
        // Mid-toggle: read the state it is heading for.
        assert_eq!(
            info(1.0, 8.0).map(|info| info.run),
            Some(ExtensionRun::Enabled)
        );
        assert_eq!(
            info(1.0, 7.0).map(|info| info.run),
            Some(ExtensionRun::Disabled)
        );
        for broken in [3.0, 4.0, 99.0] {
            assert_eq!(
                info(1.0, broken).map(|info| info.run),
                Some(ExtensionRun::Broken)
            );
        }
    }

    #[test]
    fn integer_fields_are_not_gnome_shells() {
        let info = reply(&[("type", 1i32.to_variant()), ("state", 1i32.to_variant())]);
        assert_eq!(parse_info(&info), None);
    }
}
