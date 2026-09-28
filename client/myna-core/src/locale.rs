//! The languages the user reads their desktop in, most preferred first.
//!
//! The locale environment decides, as `g_get_language_names()` orders it
//! (`LANGUAGE`, `LC_ALL`, `LC_MESSAGES`, `LANG`): the same list that picks
//! the translation Settings is shown in. The `LC_*` format categories are
//! never read, since they carry the Formats region, not the language. Only
//! a session with no language at all (`LANG=C`) asks AccountsService for the
//! account's language, and failing that assumes English.

use gio::glib;
use gio::prelude::*;

/// Where the preferred languages come from; the system reads the
/// environment and AccountsService, tests stand in for both.
pub trait LocaleSource {
    /// Locale names in preference order, as `g_get_language_names()` lists
    /// them: variants expanded, `C` last.
    fn language_names(&self) -> Vec<String>;
    /// AccountsService's `User.Language` for this user, if set.
    fn account_language(&self) -> Option<String>;
}

/// The user's languages, most preferred first, as `ll` or `ll_CC` without
/// codeset or modifier. Never empty.
pub fn preferred_languages(source: &dyn LocaleSource) -> Vec<String> {
    let mut languages: Vec<String> = Vec::new();
    for name in source.language_names() {
        if let Some(language) = normalize_locale(&name) {
            if !languages.contains(&language) {
                languages.push(language);
            }
        }
    }
    if languages.is_empty() {
        languages.extend(
            source
                .account_language()
                .and_then(|language| normalize_locale(&language)),
        );
    }
    if languages.is_empty() {
        languages.push("en".to_owned());
    }
    languages
}

/// `zh_TW.UTF-8` reads as `zh_TW`; `C`, `POSIX` and anything that is not a
/// language read as `None`.
pub fn normalize_locale(locale: &str) -> Option<String> {
    let name = locale.split(['.', '@']).next().unwrap_or_default();
    let mut parts = name.splitn(2, ['_', '-']);
    let language = parts.next().unwrap_or_default();
    if !(2..=3).contains(&language.len()) || !language.chars().all(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    let language = language.to_ascii_lowercase();
    match parts.next() {
        Some(region) if !region.is_empty() => {
            Some(format!("{language}_{}", region.to_ascii_uppercase()))
        }
        _ => Some(language),
    }
}

/// The bare language of a locale: `pt_BR.UTF-8` reads as `pt`.
pub fn language_code(locale: &str) -> Option<String> {
    let mut language = normalize_locale(locale)?;
    language.truncate(language.find('_').unwrap_or(language.len()));
    Some(language)
}

/// The running session's locale and the account's AccountsService record.
/// Under `LANG=C` it makes blocking system-bus calls (up to ~2 s), so keep it
/// off the GTK main thread.
pub struct SystemLocale {
    accounts_bus: Box<dyn Fn() -> Option<gio::DBusConnection>>,
}

impl Default for SystemLocale {
    fn default() -> Self {
        Self {
            accounts_bus: Box::new(|| {
                gio::bus_get_sync(gio::BusType::System, gio::Cancellable::NONE).ok()
            }),
        }
    }
}

impl LocaleSource for SystemLocale {
    fn language_names(&self) -> Vec<String> {
        glib::language_names()
            .into_iter()
            .map(String::from)
            .collect()
    }

    fn account_language(&self) -> Option<String> {
        accounts_service_language(&(self.accounts_bus)()?, glib::user_name().to_str()?)
    }
}

const ACCOUNTS_SERVICE: &str = "org.freedesktop.Accounts";
const ACCOUNTS_TIMEOUT_MS: i32 = 1_000;

fn accounts_service_language(bus: &gio::DBusConnection, user: &str) -> Option<String> {
    let call =
        |path: &str, interface: &str, method: &str, arguments: glib::Variant, reply: &str| {
            bus.call_sync(
                Some(ACCOUNTS_SERVICE),
                path,
                interface,
                method,
                Some(&arguments),
                Some(glib::VariantTy::new(reply).ok()?),
                gio::DBusCallFlags::NONE,
                ACCOUNTS_TIMEOUT_MS,
                gio::Cancellable::NONE,
            )
            .ok()
        };
    let (user_path,) = call(
        "/org/freedesktop/Accounts",
        ACCOUNTS_SERVICE,
        "FindUserByName",
        (user,).to_variant(),
        "(o)",
    )?
    .get::<(glib::variant::ObjectPath,)>()?;
    let (language,) = call(
        user_path.as_str(),
        "org.freedesktop.DBus.Properties",
        "Get",
        ("org.freedesktop.Accounts.User", "Language").to_variant(),
        "(v)",
    )?
    .get::<(glib::Variant,)>()?;
    language
        .get::<String>()
        .filter(|language| !language.is_empty())
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::os::unix::net::UnixStream;

    use super::*;

    struct Session {
        names: &'static [&'static str],
        account: Option<&'static str>,
        account_reads: Cell<usize>,
    }

    impl Session {
        fn new(names: &'static [&'static str], account: Option<&'static str>) -> Self {
            Self {
                names,
                account,
                account_reads: Cell::new(0),
            }
        }
    }

    impl LocaleSource for Session {
        fn language_names(&self) -> Vec<String> {
            self.names.iter().map(|name| name.to_string()).collect()
        }

        fn account_language(&self) -> Option<String> {
            self.account_reads.set(self.account_reads.get() + 1);
            self.account.map(str::to_owned)
        }
    }

    fn preferred(names: &'static [&'static str], account: Option<&'static str>) -> Vec<String> {
        preferred_languages(&Session::new(names, account))
    }

    #[test]
    fn locales_keep_their_region_and_lose_codeset_and_modifier() {
        let cases = [
            ("zh_TW.UTF-8", Some("zh_TW")),
            ("pt_BR", Some("pt_BR")),
            ("de_DE.UTF-8@euro", Some("de_DE")),
            ("sr_RS@latin", Some("sr_RS")),
            ("ca_ES@valencia.UTF-8", Some("ca_ES")),
            ("es_419", Some("es_419")),
            ("en", Some("en")),
            ("en_", Some("en")),
            ("yue", Some("yue")),
            ("EN_gb", Some("en_GB")),
            ("pt-BR", Some("pt_BR")),
            ("C", None),
            ("C.UTF-8", None),
            ("POSIX", None),
            ("", None),
            ("_US", None),
            ("e1_US", None),
            ("english", None),
        ];
        for (locale, expected) in cases {
            assert_eq!(normalize_locale(locale).as_deref(), expected, "{locale:?}");
        }
    }

    #[test]
    fn the_language_code_drops_the_region() {
        assert_eq!(language_code("zh_TW.UTF-8").as_deref(), Some("zh"));
        assert_eq!(language_code("pt_BR").as_deref(), Some("pt"));
        assert_eq!(language_code("Ja").as_deref(), Some("ja"));
        assert_eq!(language_code("POSIX"), None);
    }

    #[test]
    fn the_environment_order_is_kept_and_variants_collapse() {
        assert_eq!(
            preferred(&["zh_TW.UTF-8", "zh_TW", "zh.UTF-8", "zh", "en", "C"], None),
            ["zh_TW", "zh", "en"]
        );
        assert_eq!(
            preferred(&["pt_BR.UTF-8", "pt_BR", "pt.UTF-8", "pt", "C"], None),
            ["pt_BR", "pt"]
        );
    }

    #[test]
    fn a_multi_entry_language_list_keeps_its_order() {
        assert_eq!(
            preferred(&["fr_CA", "fr", "en_GB", "en", "C"], Some("de_DE")),
            ["fr_CA", "fr", "en_GB", "en"]
        );
    }

    #[test]
    fn the_account_is_asked_only_when_the_session_has_no_language() {
        let session = Session::new(&["en_GB.UTF-8", "en_GB", "en", "C"], Some("de_DE"));
        preferred_languages(&session);
        assert_eq!(session.account_reads.get(), 0);

        let session = Session::new(&["C"], Some("de_DE.UTF-8"));
        assert_eq!(preferred_languages(&session), ["de_DE"]);
        assert_eq!(session.account_reads.get(), 1);
    }

    #[test]
    fn a_c_or_posix_session_uses_the_account_language() {
        assert_eq!(preferred(&["C"], Some("ja_JP.UTF-8")), ["ja_JP"]);
        assert_eq!(preferred(&["POSIX", "C"], Some("ko_KR")), ["ko_KR"]);
        assert_eq!(preferred(&["C.UTF-8", "C"], Some("pt_BR")), ["pt_BR"]);
    }

    #[test]
    fn with_no_language_anywhere_english_is_assumed() {
        assert_eq!(preferred(&["C"], None), ["en"]);
        assert_eq!(preferred(&["C"], Some("")), ["en"]);
        assert_eq!(preferred(&["POSIX"], Some("C")), ["en"]);
        assert_eq!(preferred(&[], None), ["en"]);
    }

    #[test]
    fn the_session_lists_what_glib_lists() {
        let names = SystemLocale::default().language_names();
        assert_eq!(names.last().map(String::as_str), Some("C"));
    }

    const ACCOUNTS_XML: &str = "<node>\
  <interface name='org.freedesktop.Accounts'>\
    <method name='FindUserByName'>\
      <arg name='name' type='s' direction='in'/>\
      <arg name='user' type='o' direction='out'/>\
    </method>\
  </interface>\
  <interface name='org.freedesktop.Accounts.User'>\
    <property name='Language' type='s' access='read'/>\
  </interface>\
</node>";

    /// A connection to a stand-in AccountsService over a socket pair, no
    /// bus involved. It knows one user, whose language is `language`;
    /// `None` serves nothing at all.
    fn accounts_service(user: &str, language: Option<&'static str>) -> gio::DBusConnection {
        let user = user.to_owned();
        let (client, server) = UnixStream::pair().unwrap();
        let guid = gio::dbus_generate_guid();
        std::thread::spawn(move || {
            let context = glib::MainContext::new();
            let main_loop = glib::MainLoop::new(Some(&context), false);
            context
                .with_thread_default(|| {
                    let stream = gio::Socket::from_fd(server.into())
                        .unwrap()
                        .connection_factory_create_connection();
                    let connection = gio::DBusConnection::new_sync(
                        &stream,
                        Some(&guid),
                        gio::DBusConnectionFlags::AUTHENTICATION_SERVER,
                        None,
                        gio::Cancellable::NONE,
                    )
                    .unwrap();
                    let _objects = language.map(|language| serve(&connection, user, language));
                    main_loop.run();
                })
                .unwrap();
        });
        let stream = gio::Socket::from_fd(client.into())
            .unwrap()
            .connection_factory_create_connection();
        gio::DBusConnection::new_sync(
            &stream,
            None,
            gio::DBusConnectionFlags::AUTHENTICATION_CLIENT,
            None,
            gio::Cancellable::NONE,
        )
        .unwrap()
    }

    fn serve(
        connection: &gio::DBusConnection,
        user: String,
        language: &'static str,
    ) -> Vec<gio::RegistrationId> {
        let node = gio::DBusNodeInfo::for_xml(ACCOUNTS_XML).unwrap();
        let accounts = connection
            .register_object(
                "/org/freedesktop/Accounts",
                &node.lookup_interface("org.freedesktop.Accounts").unwrap(),
            )
            .method_call(move |_, _, _, _, _, parameters, invocation| {
                match parameters.get::<(String,)>() {
                    Some((name,)) if name == user => {
                        let path = glib::variant::ObjectPath::try_from(
                            "/org/freedesktop/Accounts/User1000".to_owned(),
                        )
                        .unwrap();
                        invocation.return_value(Some(&(path,).to_variant()));
                    }
                    _ => invocation
                        .return_dbus_error("org.freedesktop.Accounts.Error.Failed", "no such user"),
                }
            })
            .build()
            .unwrap();
        let user = connection
            .register_object(
                "/org/freedesktop/Accounts/User1000",
                &node
                    .lookup_interface("org.freedesktop.Accounts.User")
                    .unwrap(),
            )
            .property(move |_, _, _, _, _| language.to_variant())
            .build()
            .unwrap();
        vec![accounts, user]
    }

    #[test]
    fn accounts_service_answers_the_users_language() {
        let bus = accounts_service("ada", Some("fr_FR.UTF-8"));
        assert_eq!(
            accounts_service_language(&bus, "ada").as_deref(),
            Some("fr_FR.UTF-8")
        );
    }

    #[test]
    fn an_unset_account_language_reads_as_none() {
        let bus = accounts_service("ada", Some(""));
        assert_eq!(accounts_service_language(&bus, "ada"), None);
    }

    #[test]
    fn an_unknown_user_reads_as_none() {
        let bus = accounts_service("ada", Some("fr_FR"));
        assert_eq!(accounts_service_language(&bus, "grace"), None);
    }

    #[test]
    fn a_missing_accounts_service_reads_as_none() {
        let bus = accounts_service("ada", None);
        assert_eq!(accounts_service_language(&bus, "ada"), None);
    }

    #[test]
    fn the_system_asks_accounts_service_about_the_running_user() {
        let user = glib::user_name().into_string().unwrap();
        let bus = accounts_service(&user, Some("ko_KR.UTF-8"));
        let system = SystemLocale {
            accounts_bus: Box::new(move || Some(bus.clone())),
        };
        assert_eq!(system.account_language().as_deref(), Some("ko_KR.UTF-8"));

        let without_bus = SystemLocale {
            accounts_bus: Box::new(|| None),
        };
        assert_eq!(without_bus.account_language(), None);
    }
}
