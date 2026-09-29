# Onboarding

The settings application ships separately from the `myna` snap, so it can be
opened on a machine where dictation is not installed at all. When that is the
case it opens a three-step wizard instead of the settings window.

Every step leaves through one footer button: Next, and Done on the last step.
Done closes the wizard onto the settings window. The design quits the
application instead; the window stays because it is where the key and the
backend are changed later.
The welcome step loads the application icon straight from the application's
own resources, not by name through the icon theme: a stale icon cache that
still lists a deleted hicolor copy made the theme fail without falling back.

## What opens it

`myna_config::onboarding` assesses two components from the observations the
application already makes at startup (`snap list`, `snap connections` and
`snap interface content`):

| Component | Satisfied when                         |
| --------- | -------------------------------------- |
| Myna      | the `myna` snap is installed           |
| Model     | discovery reports at least one backend |

The wizard opens when either is missing. "Model" is satisfied by discovery
rather than by a snap name: which snaps are backends is a property of the
socket interface they publish, not of their name. The GNOME Shell extension is
not assessed: dictation works without it (the daemon falls back to desktop
notifications), and it is not published anywhere snapd can reach.

The settings window's main menu reopens the wizard (Set Up Dictation), modal
over the window. It refuses while a backend operation is in flight: the wizard
connects a backend and restarts the daemon, and the window's operation gate does
not cover it. Closing the wizard rediscovers, since it may have changed both.

## Installing

The application installs nothing itself. The component step shows one block of
three commands, always all three, with a copy button that puts them on the
clipboard as they are shown. Each command stays on one line; a window too
narrow for one scrolls the block sideways, since a command broken across
lines reads as two:

    sudo snap set system experimental.user-daemons=true
    sudo snap install --edge myna
    sudo snap install --edge myna-parakeet

Both snaps come from the store on `edge`, the only channel they are published
to. The flag comes first: snapd refuses to install a snap declaring a user
daemon unless `experimental.user-daemons` is set or its snap-id is on the
hardcoded allowlist in snapd's `overlord/snapstate/snapstate.go`, so an App
Center install of Myna fails on every stock machine, and one terminal session
beats splitting the install between a terminal and App Center. Rerunning a
command for a snap already installed is harmless. A plain install of the model
is a working backend: the install hook selects an engine, and selecting one
installs its model component.

The installs happen in another window, so the component step re-assesses the
machine whenever the wizard regains focus, and every 2 s while something is
missing: a terminal beside the wizard may never take its focus. Next stays insensitive until both
components are found; then the footer shows a success checkmark and "All
components installed" left of it.

## Finishing setup

Leaving the component step makes a backend active and restarts the daemon, so
the shortcut step finds dictation running. When a re-assessment finds the last
missing component while the step shows, the step does this by itself: a
spinner takes the footer status's place, then "All components installed" shows
for a second and the wizard moves on. Only that transition counts: opening the
step with everything already installed waits for Next, so a re-run of the
wizard does not rush past it. Next during the pause moves on at once without
setting up again; a failed setup shows the error dialog and leaves Next to
retry. Both snaps share a publisher, so
snapd's base declaration auto-connects `myna:backend` to the new backend's
slot and the step only restarts.

snapd shows that connection while the install change is still fetching the
model, and mounts the backend into Myna's namespace only as the change's
last task. A daemon restarted before then never finds the backend, although
it looks for it at every utterance. So setup first waits, spinner spinning,
until snapd's `/v2/changes?select=in-progress`, read as the user, lists no
change on Myna or a discovered backend, then decides on a fresh discovery. After 15 min it gives up with the
error dialog, naming the change, and Next retries. Otherwise it runs the active-backend switch,
which costs one polkit prompt: snapd's `manage-interfaces` action is
`auth_admin_keep`, and the restart goes through `systemctl --user`, which needs
none.

The store auto-connects the backend's `hardware-observe` and
`system-observe` plugs (granted 2026-09-23), so the wizard connects neither.
With `hardware-observe` connected at install, the install hook's
`use-engine --auto` can pick the GPU engine, and on an NVIDIA machine the
install downloads the GPU components rather than the int8 model.

A spinner alone read as a hang, so beside it a line says what setup is
doing as it starts doing it (`active_backend::SetupStage`): checking, the
download in bytes or snapd's summary of the change it waits on, connecting
the model, with a reminder to authorize it since polkit's dialog can open
behind the wizard, and starting dictation. Closing the wizard stops a setup
still waiting on snapd, so nothing is connected or restarted behind it.
While the step polls, the same line shows snap's own error when it cannot
read the machine, rather than only reporting a component missing that it
could not check.

Each assessment that differs from the last, and each setup stage and
outcome, is logged once as a GLib message in the `myna-config` domain: to
the journal when launched from the desktop (`journalctl --user -b | grep
myna-config`), and to stderr from a terminal.

## The keyboard shortcut

The daemon publishes `Activation` (`portal` or `control`) and, under the portal,
the portal's description of the binding as `Shortcut`. The last step and the Myna
page follow both through a live proxy, so a rebind elsewhere shows up at once.
Not running disables the button; no `Shortcut` from an older daemon is treated
as bound. A bound key reads "You can trigger Dictation anytime by using the
keyboard shortcut:" over its key caps on the last step; the other states say
what is missing instead.

**Portal.** Set up shortcut calls `BindShortcut("")`: the daemon offers `LOGO+j`
(Super+J) to the portal's dialog, because the portal files a binding under the
caller's app id and grants one only through that dialog. The description
(`Press <Super>j`) becomes key caps. Change shortcut opens
`gnome-control-center applications myna_myna`: GlobalShortcuts 1 has no
`ConfigureShortcuts` and no unbind. A refused bind shows the error dialog.

**Control (Noble).** The key is a GNOME custom shortcut to
`/snap/bin/myna.toggle`, the entry `myna.install-shortcut` writes; this
application is unconfined and writes it itself. Finishing setup installs
Super+J without asking once the restarted daemon reports `control`, unless
Myna's entry already has a key or another shortcut holds Super+J: a key the
user chose is never replaced silently. Under the portal setup binds nothing,
because only the portal's own dialog may, and the shortcut step's button
raises it. Set Up installs Super+J;
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

The startup assessment is the same two subprocesses as a startup
refresh, run before any window exists, and it is handed to the wizard rather
than repeated there. The shortcut proxy spawns nothing: it is one D-Bus match
per surface. Regaining focus on the component step costs another `snap list`
plus a discovery, and so does each poll while a component is missing.
Setting up reads snapd's changes over its socket once, and again every 2 s
while snapd is still changing Myna or a backend, plus one discovery after
such a wait.
Reopening the wizard from the menu costs one assessment, and closing it one
startup-sized refresh of the settings window.
