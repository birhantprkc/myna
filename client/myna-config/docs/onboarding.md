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

Under portal activation the accelerator belongs to the compositor, and only the
daemon holding the portal session sees what was granted. The daemon republishes
the portal's own description of the binding as the `Shortcut` property on
`com.canonical.Myna.Dictation`. The last step and the Myna page follow it
through a live proxy, so a daemon starting or a rebind in Settings shows up
without a refresh:

| Daemon                       | Shows                       | Button                    |
| ---------------------------- | --------------------------- | ------------------------- |
| not running                  | that it has to start first  | Set Up Shortcut, disabled |
| `Shortcut` empty             | that no key is bound        | Set Up Shortcut           |
| `Shortcut` set               | key caps                    | Change Shortcut           |
| no `Shortcut` (older daemon) | where the key is listed     | Change Shortcut           |

Set Up Shortcut calls `BindShortcut("")`, and the daemon offers `LOGO+j`
(Super+J) to the portal's dialog. There is no silent default: GNOME grants a new
binding only through that dialog. Seeding gnome-settings-daemon's store instead
would bypass the consent, depend on a private schema, and key on an app id that
has already regressed once. The call goes through the daemon because the portal
files a binding under the caller's app id.

GNOME describes a binding as a translated sentence around a GTK accelerator
(`Press <Super>j`). The accelerator becomes key caps; a description without one
is shown verbatim. Change Shortcut opens `gnome-control-center applications
myna_myna`, where GNOME rebinds portal shortcuts: GlobalShortcuts version 1 has
no `ConfigureShortcuts`, and the portal has no unbind.

Where the portal has no GlobalShortcuts (Noble), the daemon publishes
`Activation` as `control` and listens on its control socket. The key is then a
GNOME custom shortcut to `/snap/bin/myna.toggle`, the entry
`myna.install-shortcut` writes. Set Up Shortcut installs it with Super+J, the
application being unconfined, and Change Shortcut captures a new combination
in a dialog. It must include Ctrl, Alt or Super, so a bare key cannot take over
typing, unless it is a function or media key. While the dialog is open it
inhibits the desktop's shortcuts, as GNOME Settings does, so keys GNOME already
uses reach it; GNOME asks once whether to allow that. A key already bound to a
desktop action or another custom shortcut is taken only after the user agrees to
replace it, which removes it from there.

## Cost

The startup assessment is the same two subprocesses as a `RefreshReason::Startup`
refresh, run before any window exists, and it is handed to the wizard rather
than repeated there. The shortcut proxy spawns nothing: it is one D-Bus match
per surface. Regaining focus on the component step costs another `snap list`
plus a discovery. Reopening the wizard from the menu costs one assessment, and
closing it one startup-sized refresh of the settings window.
