# Onboarding

The settings application ships separately from the `myna` snap, so it can be
opened on a machine where dictation is not installed at all. When that is the
case it opens a three-step wizard instead of the settings window.

## What opens it

`myna_config::onboarding` assesses three components from the observations the
application already makes at startup (`snap list`, `snap connections` and
`snap interface content`) plus a directory probe:

| Component       | Required | Satisfied when                             | Remedy       |
| --------------- | -------- | ------------------------------------------ | ------------ |
| Myna            | yes      | the `myna` snap is installed               | command      |
| Model           | yes      | discovery reports at least one backend     | App Center   |
| Shell extension | no       | `myna-shell@canonical.com` is in a data dir | instructions |

The wizard opens when a **required** component is missing. "Model" is satisfied
by discovery rather than by a snap name: which snaps are backends is a property
of the socket interface they publish, not of their name.

The settings window's main menu reopens the wizard (Set Up Dictation), modal
over the window. It refuses while a backend operation is in flight: the wizard
connects a backend and restarts the daemon, and the window's operation gate does
not cover it. Closing the wizard rediscovers, since it may have changed both.

## Installing

The application installs nothing itself. Both snaps come from the store, on
`edge`, the only channel they are published to.

- **Model** - Install opens App Center at `snap://myna-parakeet`. The URI
  carries no channel (App Center reads everything after the scheme as the
  name), and App Center picks the only published one. A copy button gives the
  equivalent `sudo snap install --edge myna-parakeet`. A plain install is a
  working backend: the install hook selects an engine, and selecting one
  installs its model component.
- **Myna** - only a copyable command. snapd refuses to install a snap declaring
  a user daemon unless `experimental.user-daemons` is set or its snap-id is on
  the hardcoded allowlist in snapd's `overlord/snapstate/snapstate.go`, so an
  App Center install fails on every stock machine. The command sets the flag
  first.
- **Shell extension** - not published anywhere snapd can reach; it is copied
  into `~/.local/share/gnome-shell/extensions` by hand. It is also not required:
  the daemon falls back to desktop notifications without it, and gating the flow
  on a manual copy would strand anyone who cannot perform it.

The installs happen in another window, so the component step re-assesses the
machine whenever the wizard regains focus.

## Finishing setup

Leaving the component step makes a backend active and restarts the daemon, so
the shortcut step finds dictation running. Both snaps share a publisher, so
snapd's base declaration auto-connects `myna:backend` to the new backend's
slot and the step only restarts. Otherwise it runs the active-backend switch,
which costs one polkit prompt: snapd's `manage-interfaces` action is
`auth_admin_keep`, and the restart goes through `systemctl --user`, which needs
none.

The store auto-connects the backend's `hardware-observe` and
`system-observe` plugs (granted 2026-09-23), so the wizard connects neither.
With `hardware-observe` connected at install, the install hook's
`use-engine --auto` can pick the GPU engine, and on an NVIDIA machine the
install downloads the GPU components rather than the int8 model.

## The keyboard shortcut

The daemon publishes `Activation` (`portal` or `control`) and, under the portal,
the portal's description of the binding as `Shortcut`. The last step and the Myna
page follow both through a live proxy, so a rebind elsewhere shows up at once.
Not running disables the button; no `Shortcut` from an older daemon is treated
as bound.

**Portal.** Set Up Shortcut calls `BindShortcut("")`: the daemon offers `LOGO+j`
(Super+J) to the portal's dialog, because the portal files a binding under the
caller's app id and grants one only through that dialog. The description
(`Press <Super>j`) becomes key caps. Change Shortcut opens
`gnome-control-center applications myna_myna`: GlobalShortcuts 1 has no
`ConfigureShortcuts` and no unbind. A refused bind shows the error dialog.

**Control (Noble).** The key is a GNOME custom shortcut to
`/snap/bin/myna.toggle`, the entry `myna.install-shortcut` writes; this
application is unconfined and writes it itself. Set Up installs Super+J;
Change captures a key in a dialog that:

- takes a chord with Ctrl, Alt or Super, or a lone function or media key, so
  typing is never hijacked; media keys are stored as `XF86<Name>`, the only
  spelling the desktop resolves;
- inhibits the desktop's shortcuts while open, as GNOME Settings does, so keys
  GNOME uses reach it (GNOME asks once to allow this);
- asks before taking a key from a desktop action or another custom shortcut,
  and removes it there;
- refuses keys gsd-media-keys binds as `-static` (Super+O for rotation lock):
  it grabs those at login and holds them until logout whatever the setting
  says, so a replaced one would never fire.

## Cost

The startup assessment is the same two subprocesses as a `RefreshReason::Startup`
refresh, run before any window exists, and it is handed to the wizard rather
than repeated there. The shortcut proxy spawns nothing: it is one D-Bus match
per surface. Regaining focus on the component step costs another `snap list`
plus a discovery. Reopening the wizard from the menu costs one assessment, and
closing it one startup-sized refresh of the settings window.
