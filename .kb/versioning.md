# Preface

How every artifact built from this tree gets its version, and why the manifests carry `0.0.0`. Read before touching packaging, build scripts or anything that reports a version.

Read the top-level `.kb/agents.md` file before continuing below.


# Overview

The last annotated `vX.Y.Z` tag is the only version source. `dev/version.sh` turns it into `X.Y.Z` on the tag, `X.Y.Z+git<n>.<sha>` past it and `0+git.<sha>` with no tag, appending `-dirty` only when the paths a build packs differ from HEAD. Releasing is tagging; nothing in the tree is bumped.


# Important

- `version` in `client/Cargo.toml` and `server/pyproject.toml` is a required-field placeholder, `0.0.0` forever. Nothing reads it: the client reports `MYNA_VERSION`, and the server reports no version at runtime.
- The inference snaps name the wheel `myna-0.0.0-py3-none-any.whl` literally, because `python-packages` cannot glob and installing `myna` by name through find-links would let a same-named index package win. Stamping the real version into the wheel would buy nothing visible and force templating those yamls.
- A client build with neither a staged `.version` nor a reachable checkout fails. It never falls back to the Cargo version: a silent `0.0.0` in About and diagnostics is worse than a broken build.
- Never hardcode a version in packaging, and never hand-edit a changelog to fake one.


# Architecture

Each artifact receives the version by the route its build environment allows:

- **Snaps** - a snap build instance mounts only the snap directory, so `version: git` cannot run. `dev/prepare.sh` calls `dev/stage-version.sh`, which writes `version/version.metainfo.xml` for the `version` part to adopt through `parse-info`. Not `craftctl set version`: an incremental pack that updates rather than reruns a step keeps the old value.
- **Client binaries** - `client/build-support/version.rs` emits `MYNA_VERSION` from a staged `client/.version` if present (the myna snap and the deb stage one), else from `dev/version.sh` in the checkout. The checkout is `MYNA_REPO_ROOT` when set, which cargo-mutants needs because it builds a copy of `client/` alone, else two levels above the crate. Only HEAD and refs are watched, so a dev build never reports `-dirty`.
- **Deb** - `myna-config-deb/build-source.sh` stages `.version` into the source tree it exports; `myna-config-deb/AGENTS.md` has the Debian revision scheme.
- **Extension tarball** - the Makefile calls `dev/version.sh` over the extension directory.
