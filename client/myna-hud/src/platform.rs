//! platform — runtime probing for the two desktop preferences the HUD
//! honours besides the theme itself: **reduced motion** (E2b, FR-022a) and
//! **high contrast** (FR-022). The motion rule lives in [`crate::motion`];
//! this module only reads the live sources and feeds them. The accent needs
//! no probing: the indicators' CSS resolves `--accent-bg-color` and GTK
//! restyles them when it changes.
//!
//! Only *host* preferences are read here. Myna's own settings are not: the
//! HUD is told what to draw by the publisher (`HudStyle`), because this
//! process runs on the desktop's GSettings backend while the rest of the
//! client runs on the snap's keyfile store, and a reader on the wrong side of
//! that line silently returns the schema default. See
//! `myna_desktop::dbus::hud_style`.
//!
//! ## No compile-time version features
//!
//! The newer sources are looked up by *runtime GObject property name*
//! rather than a `gtk4/v4_22` cargo feature, because a single binary must
//! serve a runtime matrix that spans the snap's gnome-46-2404 SDK (GTK 4.18)
//! and 26.04 hosts (GTK 4.22). A compile-time feature would either raise the
//! floor or forfeit the newer source; `find_property` costs nothing and
//! degrades exactly.
//!
//! ## Crash guard (E2b)
//!
//! `org.gnome.desktop.a11y.interface reduced-motion` is NEVER read. It is
//! new in gsettings-desktop-schemas, and constructing a `gio::Settings` for
//! a missing schema — or reading a missing key — **aborts the process**.
//! Every GSettings access below is guarded through
//! [`settings_for_schema_key`], which consults the schema source first.

use glib::translate::ToGlibPtr;
use gtk::glib;
use gtk::prelude::*;
use gtk4 as gtk;
use libadwaita as adw;

use crate::motion::{reduced_motion, MotionReadings};

/// `org.gnome.desktop.interface`, home of the `enable-animations` fallback.
const INTERFACE_SCHEMA: &str = "org.gnome.desktop.interface";
const ANIMATIONS_KEY: &str = "enable-animations";

/// `GtkSettings`' reduced-motion property (GTK ≥ 4.22).
const GTK_REDUCED_MOTION_PROPERTY: &str = "gtk-interface-reduced-motion";
/// `GtkReducedMotion.no_preference` — the one value meaning "full motion".
const GTK_REDUCED_MOTION_NO_PREFERENCE: i32 = 0;

/// Build a [`gio::Settings`] for `schema` only if the schema **and** `key`
/// both exist on this system; otherwise `None`.
///
/// This is the guard that keeps a missing schema/key from aborting the
/// process (E2b) — the reason the HUD may never touch a GSettings key
/// without asking first.
pub fn settings_for_schema_key(schema: &str, key: &str) -> Option<gtk::gio::Settings> {
    let source = gtk::gio::SettingsSchemaSource::default()?;
    let schema_obj = source.lookup(schema, true)?;
    if !schema_obj.has_key(key) {
        return None;
    }
    Some(gtk::gio::Settings::new(schema))
}

/// Read `GtkSettings:gtk-interface-reduced-motion` if this GTK has it.
///
/// Returns `None` on GTK < 4.22, where [`crate::motion`] falls back to the
/// inverted `enable-animations`.
///
/// The property is a **`GtkReducedMotion` enum**, not a boolean
/// (`no_preference = 0`, `reduce = 1`) — reading it as a `bool` fails and
/// looks exactly like "the property is absent", silently forfeiting the
/// primary source on precisely the systems that have it. It is read through
/// `g_value_get_enum` rather than a bound Rust enum type both because the
/// binding lacks one without a `v4_22` feature and because that tolerates
/// additive values: anything other than `no_preference` counts as reduced
/// motion, so a future stronger level errs toward less animation.
pub fn probe_gtk_reduced_motion() -> Option<bool> {
    let settings = gtk::Settings::default()?;
    let property = settings
        .find_property(GTK_REDUCED_MOTION_PROPERTY)
        .map(|p| p.name().to_string())?;
    decode_reduced_motion(&settings.property_value(&property))
}

/// Decode whatever `gtk-interface-reduced-motion` holds into the boolean
/// [`crate::motion`] expects. Split out from the probe so the enum handling
/// is testable without a display.
pub fn decode_reduced_motion(value: &glib::Value) -> Option<bool> {
    if let Ok(flag) = value.get::<bool>() {
        return Some(flag);
    }
    if value.type_().is_a(glib::Type::ENUM) {
        // SAFETY: the GValue is known to hold an enum.
        let raw = unsafe { glib::gobject_ffi::g_value_get_enum(value.to_glib_none().0) };
        return Some(raw != GTK_REDUCED_MOTION_NO_PREFERENCE);
    }
    None
}

/// Read `org.gnome.desktop.interface enable-animations` (raw, NOT inverted
/// — [`crate::motion::reduced_motion`] owns that), schema/key guarded.
pub fn probe_enable_animations() -> Option<bool> {
    let settings = settings_for_schema_key(INTERFACE_SCHEMA, ANIMATIONS_KEY)?;
    Some(settings.boolean(ANIMATIONS_KEY))
}

/// Whether the desktop requests a higher-contrast UI (FR-022).
///
/// `Adw.StyleManager:high-contrast` — a plain bool libadwaita exposes (and
/// itself derives from `GtkSettings:gtk-interface-contrast` where available,
/// i.e. `gtk-interface-contrast` is just the GTK plumbing Adw builds on).
/// Looked up by runtime property name so the same binary works on older
/// libadwaita; `false` when the property is missing.
pub fn probe_high_contrast() -> bool {
    let manager = adw::StyleManager::default();
    let Some(property) = manager
        .find_property("high-contrast")
        .map(|p| p.name().to_string())
    else {
        return false;
    };
    manager
        .property_value(&property)
        .get::<bool>()
        .unwrap_or(false)
}

/// The live reduced-motion preference, resolved through both safe sources.
pub fn probe_reduced_motion() -> bool {
    reduced_motion(&MotionReadings {
        gtk_reduced_motion: probe_gtk_reduced_motion(),
        enable_animations: probe_enable_animations(),
    })
}

/// Call `on_change` whenever a preference that affects the HUD may have
/// changed.
///
/// The returned guard owns the subscriptions; dropping it disconnects
/// everything, so no callback can outlive the window.
pub fn watch_preferences<F: Fn() + 'static + Clone>(on_change: F) -> PreferenceWatch {
    let settings = settings_for_schema_key(INTERFACE_SCHEMA, ANIMATIONS_KEY);
    if let Some(settings) = &settings {
        let cb = on_change.clone();
        settings.connect_changed(Some(ANIMATIONS_KEY), move |_, _| cb());
    }

    let manager = adw::StyleManager::default();

    let mut gtk_handles = Vec::new();
    let gtk_settings = gtk::Settings::default();
    if let Some(settings) = &gtk_settings {
        // Reduced motion (GTK ≥ 4.22).
        if settings
            .find_property(GTK_REDUCED_MOTION_PROPERTY)
            .is_some()
        {
            let cb = on_change.clone();
            gtk_handles.push(
                settings.connect_notify_local(Some(GTK_REDUCED_MOTION_PROPERTY), move |_, _| cb()),
            );
        }
    }

    // High contrast — Adw tracks it (and itself follows
    // GtkSettings:gtk-interface-contrast where it exists).
    let mut adw_high_contrast_handle = None;
    if manager.find_property("high-contrast").is_some() {
        let cb = on_change.clone();
        adw_high_contrast_handle =
            Some(manager.connect_notify_local(Some("high-contrast"), move |_, _| cb()));
    }

    PreferenceWatch {
        _settings: settings,
        manager,
        gtk_settings,
        gtk_handles,
        adw_high_contrast_handle,
    }
}

/// Owns the preference subscriptions; disconnects them on drop.
pub struct PreferenceWatch {
    _settings: Option<gtk::gio::Settings>,
    manager: adw::StyleManager,
    gtk_settings: Option<gtk::Settings>,
    gtk_handles: Vec<glib::SignalHandlerId>,
    adw_high_contrast_handle: Option<glib::SignalHandlerId>,
}

impl Drop for PreferenceWatch {
    fn drop(&mut self) {
        if let Some(handle) = self.adw_high_contrast_handle.take() {
            self.manager.disconnect(handle);
        }
        if let Some(settings) = &self.gtk_settings {
            for handle in self.gtk_handles.drain(..) {
                settings.disconnect(handle);
            }
        }
        // The gio::Settings object drops with its handler attached; it is
        // owned here and released now.
    }
}
