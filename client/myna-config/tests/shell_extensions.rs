//! The gnome-shell extension adapter against a stand-in shell on a private
//! peer-to-peer D-Bus connection: no bus daemon, no real shell.

use std::cell::RefCell;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::rc::Rc;

use gio::glib::{MainContext, Variant, VariantDict};
use gio::prelude::*;
use myna_config::adapters::shell_extensions::GnomeShellExtensions;
use myna_config::onboarding::{ExtensionState, SHELL_EXTENSION_UUID};
use myna_config::ports::ShellExtensions;

const SHELL_XML: &str = "<node>\
  <interface name='org.gnome.Shell.Extensions'>\
    <method name='GetExtensionInfo'>\
      <arg type='s' name='uuid' direction='in'/>\
      <arg type='a{sv}' name='info' direction='out'/>\
    </method>\
  </interface>\
</node>";

fn socket(stream: UnixStream) -> gio::IOStream {
    let socket = gio::Socket::from_fd(stream.into()).expect("wrap the socket");
    socket.connection_factory_create_connection().upcast()
}

/// A connected pair: the adapter's end and the stand-in shell's, which
/// answers `GetExtensionInfo` with whatever `known` holds for the uuid.
fn shell(known: Rc<RefCell<Vec<(String, Variant)>>>) -> (gio::DBusConnection, gio::DBusConnection) {
    let (client, server) = UnixStream::pair().expect("socket pair");
    let guid = gio::dbus_generate_guid();
    let (client, server) = MainContext::ref_thread_default().block_on(async {
        futures_join(
            gio::DBusConnection::new_future(
                &socket(client),
                None,
                gio::DBusConnectionFlags::AUTHENTICATION_CLIENT,
                None,
            ),
            gio::DBusConnection::new_future(
                &socket(server),
                Some(&guid),
                gio::DBusConnectionFlags::AUTHENTICATION_SERVER
                    | gio::DBusConnectionFlags::AUTHENTICATION_ALLOW_ANONYMOUS,
                None,
            ),
        )
        .await
    });
    let (client, server) = (client.expect("client end"), server.expect("server end"));
    let interface = gio::DBusNodeInfo::for_xml(SHELL_XML)
        .unwrap()
        .lookup_interface("org.gnome.Shell.Extensions")
        .unwrap();
    server
        .register_object("/org/gnome/Shell", &interface)
        .method_call(move |_, _, _, _, _, parameters, invocation| {
            let (uuid,) = parameters.get::<(String,)>().unwrap();
            let dict = VariantDict::new(None);
            for (key, value) in known.borrow().iter() {
                if uuid == SHELL_EXTENSION_UUID {
                    dict.insert_value(key, value);
                }
            }
            invocation.return_value(Some(&Variant::tuple_from_iter([dict.end()])));
        })
        .build()
        .expect("register the stand-in shell");
    (client, server)
}

/// Both handshakes must progress together: each side waits on the other.
async fn futures_join<A, B: 'static>(
    a: impl std::future::Future<Output = A>,
    b: impl std::future::Future<Output = B> + 'static,
) -> (A, B) {
    let b = MainContext::ref_thread_default().spawn_local(b);
    let a = a.await;
    (a, b.await.expect("server handshake"))
}

fn install_copy(data_dir: &std::path::Path) {
    let extension = data_dir
        .join("gnome-shell/extensions")
        .join(SHELL_EXTENSION_UUID);
    std::fs::create_dir_all(&extension).unwrap();
    std::fs::write(extension.join("metadata.json"), "{}").unwrap();
}

/// The system data dirs and the user's, holding the copies asked for.
struct DataDirs {
    _system: tempdir::Dir,
    _user: tempdir::Dir,
    system_dirs: Vec<PathBuf>,
    user_dir: PathBuf,
}

fn data_dirs(with_system_copy: bool, with_user_copy: bool) -> DataDirs {
    let (system, user) = (tempdir::Dir::new(), tempdir::Dir::new());
    if with_system_copy {
        install_copy(system.path());
    }
    if with_user_copy {
        install_copy(user.path());
    }
    DataDirs {
        system_dirs: vec![system.path().to_owned()],
        user_dir: user.path().to_owned(),
        _system: system,
        _user: user,
    }
}

mod tempdir {
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT: AtomicUsize = AtomicUsize::new(0);

    pub struct Dir(PathBuf);

    impl Dir {
        pub fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "myna-shell-extensions-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::SeqCst)
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        pub fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}

/// Everything on one context: the stand-in shell answers on the context it
/// was registered from, and the test blocks on that one.
fn on_own_context<T>(test: impl FnOnce() -> T) -> T {
    MainContext::new()
        .with_thread_default(test)
        .expect("own the test's context")
}

fn block_on<T>(future: impl std::future::Future<Output = T>) -> T {
    MainContext::ref_thread_default().block_on(future)
}

fn state(reported: &[(&str, f64)], with_system_copy: bool) -> ExtensionState {
    state_with(reported, with_system_copy, false)
}

fn state_with(
    reported: &[(&str, f64)],
    with_system_copy: bool,
    with_user_copy: bool,
) -> ExtensionState {
    on_own_context(|| {
        let known = Rc::new(RefCell::new(
            reported
                .iter()
                .map(|(key, value)| (key.to_string(), value.to_variant()))
                .collect(),
        ));
        let dirs = data_dirs(with_system_copy, with_user_copy);
        let (client, _server) = shell(known);
        let extensions = GnomeShellExtensions::with_connection(
            client,
            dirs.system_dirs.clone(),
            dirs.user_dir.clone(),
        );
        block_on(extensions.extension_state(SHELL_EXTENSION_UUID))
    })
}

#[test]
fn an_enabled_system_copy_is_enabled() {
    assert_eq!(
        state(&[("type", 1.0), ("state", 1.0)], true),
        ExtensionState::Enabled
    );
}

#[test]
fn a_disabled_system_copy_can_be_enabled() {
    assert_eq!(
        state(&[("type", 1.0), ("state", 2.0)], true),
        ExtensionState::Disabled
    );
}

#[test]
fn a_user_copy_is_not_the_packaged_extension() {
    assert_eq!(
        state_with(&[("type", 2.0), ("state", 1.0)], false, true),
        ExtensionState::Unavailable
    );
}

#[test]
fn a_user_copy_on_disk_shadows_a_system_copy() {
    assert_eq!(
        state_with(&[("type", 2.0), ("state", 1.0)], true, true),
        ExtensionState::ShadowedByUserCopy
    );
    assert_eq!(
        state_with(&[], true, true),
        ExtensionState::ShadowedByUserCopy
    );
}

#[test]
fn a_system_copy_the_shell_does_not_list_needs_a_relogin() {
    assert_eq!(state(&[], true), ExtensionState::NeedsRelogin);
}

#[test]
fn an_extension_neither_listed_nor_installed_is_unavailable() {
    assert_eq!(state(&[], false), ExtensionState::Unavailable);
}

#[test]
fn a_shell_that_is_gone_is_no_shell() {
    let state = on_own_context(|| {
        let dirs = data_dirs(false, false);
        let (client, server) = shell(Rc::default());
        block_on(server.close_future()).unwrap();
        let extensions = GnomeShellExtensions::with_connection(
            client,
            dirs.system_dirs.clone(),
            dirs.user_dir.clone(),
        );
        block_on(extensions.extension_state(SHELL_EXTENSION_UUID))
    });
    assert_eq!(state, ExtensionState::Unavailable);
}
