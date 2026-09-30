# Onboarding

The settings application ships separately from the `myna` snap, so it can be
opened on a machine where dictation is not installed at all. When that is the
case it opens a three-step wizard instead of the settings window.

Every step leaves through one footer button: an outlined Next, and a suggested
Done on the last step. The window opens at the design's 800x600, and every
step's header is flat and untitled; each step after the first carries a back
arrow to the step before it. Done closes the wizard onto the settings window.
The design quits the application instead; the window stays because it is where the key and the
backend are changed later.
The welcome step loads the application icon straight from the application's
own resources, not by name through the icon theme: a stale icon cache that
still lists a deleted hicolor copy made the theme fail without falling back.

## What opens it

`myna_config::onboarding` assesses four components from the observations the
application already makes at startup (`snap list`, `snap connections` and
`snap interface content`), snapd's `/v2/system-info` read as the user, and
gnome-shell's `org.gnome.Shell.Extensions`:

| Component       | Required | Satisfied when                                          |
| --------------- | -------- | ------------------------------------------------------- |
| User daemons    | yes      | snapd's `experimental.user-daemons` is on               |
| Myna            | yes      | the `myna` snap is installed                            |
| Model           | yes      | discovery reports at least one backend                  |
| Shell extension | no       | gnome-shell runs the system copy of `myna-shell@canonical.com` |

The wizard opens when a required component is missing. The flag stays
required once Myna is installed: an installed Myna keeps running with the
flag unset, but snapd refuses its refreshes. "Model" is satisfied by
discovery rather than by a snap name: which snaps are backends is a property
of the socket interface they publish, not of their name. Every other row
waits for the flag, since snapd refuses Myna without it.

The extension is optional because dictation works without it: the daemon
falls back to desktop notifications. It ships in the
`gnome-shell-ubuntu-extensions` deb, not through snapd, so the wizard never
installs it. Only a system copy counts, not a development copy in `~/.local`.
Its state is one of:

- enabled: satisfied;
- disabled: gnome-shell lists the system copy but does not run it, and one
  `EnableExtension` call fixes it;
- installed after login: the copy is under a system data directory but
  gnome-shell, which scans them only at login, does not list it;
- shadowed: a system copy is on disk and so is a user copy of the same uuid
  under `~/.local/share/gnome-shell/extensions`. gnome-shell loads the user
  directory first and skips a uuid it already has, so the system copy never
  runs and a re-login does not help; removing the user copy does. Checked on
  disk, since gnome-shell keeps listing a user copy deleted after login;
- unavailable: no system copy, one gnome-shell cannot run (error, out of
  date), or no gnome-shell answering on the session bus within 2 s.

gnome-shell sends `type` and `state` as doubles; `type` 1 is a system copy,
`state` 1 enabled, 2 and 6 disabled and never enabled. The transient 8
(activating) and 7 (deactivating) read as the state they are heading for, so
a re-read right after `EnableExtension` does not flash the row unavailable.

The settings window's main menu reopens the wizard (Set Up Dictation), modal
over the window. It refuses while a backend operation is in flight: the wizard
connects a backend and restarts the daemon, and the window's operation gate does
not cover it. Closing the wizard rediscovers, since it may have changed both.

## Installing

The component step titles itself "Install components" and lists every
component in its own row; nothing is left for a terminal. The flag gets a boxed
row of its own with a switch, "Enable user daemons experimental support",
because it is a system setting rather than something to install. The other
three share one boxed list below it: Dictation app, Speech-to-text model and
Shell extension. The whole list is insensitive until the flag is on, since
snapd refuses Myna without it.

Each row ends in what the wizard can do about it (`onboarding::row_action`):

- Install, for a missing snap;
- Enable, for a system copy of the extension gnome-shell is not running;
- a check and "Installed" once it is in place;
- nothing, for an extension out of the wizard's reach. The row stays
  sensitive, since an insensitive row dims its subtitle past reading, and the
  subtitle says why: not on this system (dictation still works and
  shows its status in notifications), installed after login (log out and back
  in), or hidden by a copy in `~/.local/share/gnome-shell/extensions`.

The switch turns the flag on through snapd's REST API as the user
(`PUT /v2/snaps/system/conf`), and snapd raises polkit's prompt for
`io.snapcraft.snapd.manage-configuration` itself: no root code of ours, one
prompt. The prompt therefore shows snapd's wording ("access or modify snap
configuration"), not a Myna one. While snapd has not answered, the switch
shows on but not yet active, a spinner sits beside it and the subtitle reads
"Enabling…"; the row stops taking input but stays sensitive, as the settings
window's busy rows do. snapd answers only once the prompt is, 40 s for one
left open on Noble, so the write waits up to 10 min for that answer
(`SnapdTimeouts::authorization`) before following the change; interface
connects wait the same way. Dismissing the prompt puts the switch back
silently; a refusal or a failed change puts it back with a toast whose
Details open the report, which names the snapd request and its HTTP
status rather than a command. Success keeps the switch pending until a fresh read
shows the flag, then unlocks the list. A read that started before the write
is discarded rather than taken for the machine after it. The switch never
turns the flag off: activating it again springs back, since snapd refuses
Myna's refreshes without the flag.

Install asks snapd for the snap as the user
(`POST /v2/snaps/<name>`, `{"action":"install","channel":"latest/edge"}`),
the same install `snap install --edge` makes, and snapd raises polkit's prompt
for `io.snapcraft.snapd.manage` itself. That action is `auth_admin_keep` per
process, so installing the model within five minutes of the app asks nothing
more. snapd answers once the prompt is answered; the row then follows the
change `GET /v2/changes/<id>` once a second (`snap_install.rs`); ten failed
reads in a row end it, so snapd restarting mid-install is no failure. The button
gives way to a spinner and "Installing…", then "Installing 42%" once a
download announces its size: the bytes of every download task in the change
over what they announce or what the row expected to fetch, whichever is
more, never going down. The percentage shows only while a download runs:
mounting, hooks and services take 15 s or more after the app's download,
and "Installing 100%" there read as stuck (seen on Noble). The model's
component download runs inside the backend's own install change, fetched by
its install hook's engine choice, so the model row follows it to the end,
the percentage resuming where the snap's own download left it. A download
served from snapd's cache reports no bytes, so a cached install shows no
percentage.

A row snapd is still installing counts as missing (`onboarding::while_installing`)
whatever a read finds half-way: the backend's slot is published, and
discovery finds it, minutes before its model has arrived. Next and the
automatic move on therefore wait for the change. Once it is done the row
waits for a fresh read, which shows Installed. One install runs at a time,
since one request raises one prompt; the other Install buttons are
insensitive meanwhile. Dismissing the prompt puts the button back silently;
a refusal, a request snapd rejects (Myna without the flag: "feature flag
validation failed") or a change that fails puts it back with a toast whose
Details name the request, snapd's HTTP status (202 for a change that failed
after snapd accepted it) and snapd's error. Closing the wizard stops
following; snapd's change carries on.

A change snapd is already running to install a missing row's snap, started
in a terminal or by a wizard since closed, is followed the same way instead
of offering Install again. It is found in `/v2/changes?select=in-progress` as
an unfinished `install-snap` change whose summary names the snap. Such a change failing only puts Install back, since it
was not this window's action.

Enable does not act yet.

The subtitles size the download, in whole megabytes from 100 MB to 1 GB,
where a decimal is noise. The app's is the store's size of the `myna`
snap. The model's names the family and the size of what its install fetches:
the snap and the int8 model, or, with an NVIDIA GPU, the CUDA runtime and the
fp32 model, said as "up to" because the install hook falls back to the CPU
engine when the GPU has no driver. The sizes are fixed per store revision in
`onboarding.rs`, not read from the store.

Installing may still happen elsewhere, so the component step re-assesses the
machine whenever the wizard regains focus, and every 2 s while something
required is missing. Next stays insensitive until the required components are
found; then the footer shows a success checkmark and "All required components
installed" left of it.

The store commands the diagnostics page suggests (`MYNA_INSTALL_COMMAND`,
`MODEL_INSTALL_COMMAND`) ask for `edge`, the only channel both snaps are
published to, and set the flag first: snapd refuses to install a snap
declaring a user daemon unless `experimental.user-daemons` is set or its
snap-id is on the hardcoded allowlist in snapd's
`overlord/snapstate/snapstate.go`, so an App Center install of Myna fails on
every stock machine. A plain install of the model is a working backend: the
install hook selects an engine, and selecting one installs its model
component.

## Finishing setup

Leaving the component step makes a backend active and restarts the daemon, so
the shortcut step finds dictation running. When a re-assessment finds the last
missing component, the optional extension included, while the step shows,
the step does this by itself: a
spinner takes the footer status's place, then "All required components installed" shows
for a second and the wizard moves on. Only that transition counts: opening the
step with everything already installed waits for Next, so a re-run of the
wizard does not rush past it, and a machine whose extension the wizard
cannot install leaves the move to Next. Next during the pause moves on at once without
setting up again; a failed setup shows the error dialog and leaves Next to
retry. Both snaps share a publisher, so
snapd's base declaration auto-connects `myna:backend` to the new backend's
slot and the step only restarts.

snapd shows that connection while the install change is still fetching the
model, and mounts the backend into Myna's namespace only as the change's
last task. A daemon restarted before then never finds the backend, although
it looks for it at every utterance. So setup first waits, spinner spinning,
until snapd's `/v2/changes?select=in-progress`, read as the user, lists no
unfinished change on Myna or a discovered backend, then decides on a fresh discovery. After 15 min it gives up with the
error dialog, naming the change, and Next retries. `snap_changes::parse_changes`
drops every ready change: that listing also returns a held one, `Hold` and
ready with no tasks, such as the auto-refresh of a snap removed while its
refresh was inhibited (stonking, 2026-09-30), and setup sat under it. Otherwise it runs the active-backend switch,
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
download in bytes or "Waiting for other software changes to finish…"
(snapd's summary of the change goes to the log only: it is English and
names snapd's internals), connecting
the model, with a reminder to authorize it since polkit's dialog can open
behind the wizard, and starting dictation. Closing the wizard stops a setup
still waiting on snapd, so nothing is connected or restarted behind it.
While the step polls, the same line shows snap's own error when it cannot
read the machine, rather than only reporting a component missing that it
could not check.

The assessment that opens the wizard or the settings window, each later one
that differs from the last, and each setup stage and outcome, is logged once as a GLib message in the `myna-config` domain: to
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
refresh, plus one read of snapd's socket and one D-Bus call to gnome-shell,
run before any window exists, and it is handed to the wizard rather than
repeated there. The shortcut proxy spawns nothing: it is one D-Bus match
per surface. Regaining focus on the component step costs a full re-assessment,
and so does each poll while a component is missing: a `snap list`, a
discovery, one read of snapd's socket for the flag, one `GetExtensionInfo`
call to gnome-shell (at most 2 s when it does not answer), a stat of each
extension directory and a scan of `/sys/bus/pci/devices` for an NVIDIA GPU.
While a snap row is missing and the flag is on, each re-assessment also
reads snapd's in-progress changes once, to follow an install started
elsewhere. An install reads its change once a second until snapd is done.
Setting up reads snapd's changes over its socket once, and again every 2 s
while snapd is still changing Myna or a backend, plus one discovery after
such a wait.
Reopening the wizard from the menu costs one assessment, and closing it one
startup-sized refresh of the settings window.
