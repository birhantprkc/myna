# Preface

Read this file before changing Myna Settings: the GTK application that onboards a machine, switches and configures inference backends, and edits the dictation settings the daemon reads live.

Read the top-level `.kb/agents.md` file before continuing below.

# Overview

Myna Settings is a host application, not a snap. It talks to snapd on the user's behalf and escalates to root through polkit for the few `snap` commands that need it, so it is packaged as a deb from `myna-config-deb/` and only shares the `myna-core` crate with the confined client. Everything the user can trigger is expressed as a plan of exact commands first, confirmed by the user in plain words, and only then executed.

# Important

- One user action costs at most one polkit prompt. Two privilege paths exist and they are not interchangeable. Backend switching (`connect`, `disconnect`) goes straight to `/run/snapd.socket` as the user and snapd asks polkit itself; its `auth_admin_keep` is per action, so every call in one operation must use the same snapd action. The daemon restart that follows goes through `systemctl --user`, which needs no authorization. Backend configuration (`snap run <backend>.modelctl ...`) runs as root through one `pkexec myna-config --apply-plan` invocation, whose prompt shows the message of the polkit action in `data/com.canonical.Myna.Config.policy`. Do not route a new operation through `pkexec` when snapd's REST API can do it unprivileged.
- The application installs no snaps. Onboarding sends the user to App Center or a copyable `snap install --edge` command (`docs/onboarding.md`).
- The `--apply-plan` executor is the trust boundary. It accepts only operations matching the exact shapes the UI produces (`apply_plan.rs`, `system_configurator.rs`). Extend the whitelist deliberately and with a test; never pass free-form argv through it.
- Nothing runs through a shell. Build every subprocess as a `CommandRequest` and run it through the `CommandRunner` port so tests can substitute a fixture.
- Subprocess spawning is budgeted per refresh reason (`docs/refresh-budget.md`). A new `snap` read must fit the budget or change it explicitly.
- The diagnostics page measures CPU clock under load and pressure stalls on every refresh (`docs/performance-warnings.md`). Verdicts are enums in `performance.rs`; wording lives in the presenter. A host the reader cannot understand must render as unknown, never as a warning.
- Domain and controller modules are GTK-free and tested headlessly. Keep GTK to `ui/`, `*_ui.rs`, and `app.rs`.
- Every user-visible string goes through gettext. Adding or changing one requires `make i18n` and committing the template; `make check` fails while it drifts.
- Strict confinement was measured and rejected (`docs/confinement.md`). Do not reopen it without new evidence.
- The GSettings schema this application writes is owned by `client/data/` and shared with the daemon.

# Architecture

Hexagonal. `ports.rs` declares the traits the application depends on (backend repository, system configurator, client settings). `adapters/` implements them against real snapd, `snap`, `pkexec`, and Gio. `domain.rs`, `active_backend.rs`, `backend_apply.rs`, `onboarding.rs`, `shortcut.rs`, and `machine.rs` hold the pure decision logic. The `*_controller.rs` and `*_ui.rs` pairs bind that logic to GTK, and `operation_gate.rs` ensures one privileged operation runs at a time.

# Directory

- `build/` - Build-script logic that `tests/` pulls in with `include!`, such as the minimum `blueprint-compiler` version.
- `src/adapters/` - snapd REST client, `snap` CLI repository, pkexec configurator, Gio settings.
- `src/ui/` - One module per Blueprint template in `data/`.
- `src/bin/` - Test fixture that stands in for a real command runner.
- `data/` - Blueprint templates, CSS, desktop entry, polkit action, man page, gresource manifest.
- `docs/` - Decision records: confinement gate, onboarding flow, refresh budget, performance warnings.
- `po/` - gettext template and translator instructions.
- `tests/` - Contract tests per port and adapter. `snap_packaging.rs` covers the `myna.config` gsettings wrapper the snap ships, which is a shell script and not this application.
