//! The dictation shortcut as Myna Settings sees it, without GTK.
//!
//! The portal owns the key, and the daemon republishes the portal's own
//! description of it as `Shortcut` on `com.canonical.Myna.Dictation`. Where the
//! portal has no GlobalShortcuts the daemon says `Activation` is `control`, and
//! the key is a desktop custom shortcut instead.

/// The key a desktop shortcut is installed with, the daemon's portal default.
pub const DEFAULT_ACCELERATOR: &str = "<Super>j";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShortcutState {
    /// Nothing owns the daemon's bus name.
    NotRunning,
    /// A daemon that predates the `Shortcut` property.
    Unpublished,
    /// The daemon holds no binding.
    Unbound,
    /// The portal's description of the binding, such as `Press <Super>j`.
    Bound(String),
}

impl ShortcutState {
    /// `owned` is whether the bus name has an owner; `shortcut` is the
    /// property's value when the daemon publishes one.
    pub fn observe(owned: bool, shortcut: Option<&str>) -> Self {
        match (owned, shortcut) {
            (false, _) => Self::NotRunning,
            (true, None) => Self::Unpublished,
            (true, Some(shortcut)) if shortcut.trim().is_empty() => Self::Unbound,
            (true, Some(shortcut)) => Self::Bound(shortcut.to_owned()),
        }
    }

    /// The control path's state: `binding` is the desktop shortcut's
    /// accelerator, when one is installed.
    pub fn observe_control(owned: bool, binding: Option<&str>) -> Self {
        match (owned, binding) {
            (false, _) => Self::NotRunning,
            (true, Some(binding)) if !binding.trim().is_empty() => Self::Bound(binding.to_owned()),
            (true, _) => Self::Unbound,
        }
    }
}

/// How the key reaches the daemon, from its `Activation` property.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShortcutPath {
    /// The portal's binding. Also a daemon that has not decided yet.
    Portal,
    /// A desktop custom shortcut that pokes the control socket, where the
    /// portal has no GlobalShortcuts.
    Control,
}

impl ShortcutPath {
    pub fn from_activation(activation: Option<&str>) -> Self {
        match activation {
            Some("control") => Self::Control,
            _ => Self::Portal,
        }
    }
}

/// The GTK accelerators inside a portal trigger description.
///
/// GNOME's portal wraps the accelerator in a translated sentence
/// (`Press <Super>j`), so the accelerator is the part to render as keys. Other
/// portals describe a binding however they like and yield none.
pub fn accelerators(description: &str) -> Vec<&str> {
    description
        .split_whitespace()
        .filter(|token| is_accelerator(token))
        .collect()
}

fn is_accelerator(token: &str) -> bool {
    let mut rest = token;
    let mut modifiers = 0;
    while let Some(tail) = rest.strip_prefix('<') {
        let Some((name, after)) = tail.split_once('>') else {
            return false;
        };
        if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric()) {
            return false;
        }
        modifiers += 1;
        rest = after;
    }
    modifiers > 0 && !rest.is_empty() && !rest.contains(['<', '>'])
}
