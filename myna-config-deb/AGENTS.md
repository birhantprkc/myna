# Preface

Read this file before touching the Debian packaging of Myna Settings, its version scheme, or the PPA upload.

Read the top-level `.kb/agents.md` file before continuing below.

# Overview

This directory turns `client/myna-core` and `client/myna-config` into the `myna-config` source package. The application must run unconfined because it drives snapd and escalates through polkit, which the `myna` snap cannot do, so it is the one piece of Myna shipped as a deb. `README.md` here is the human build recipe; this file carries what an agent must not get wrong.

# Important

- `build-source.sh` reads HEAD, not the working tree. Commit before staging or the tarball silently lacks the change.
- The orig tarball is reproducible and vendors every crates.io dependency, so the build runs offline. `debian/copyright` is generated from the vendor tree by `vendor-copyright.py`; edit `debian/copyright.in`, never the generated file.
- Versioning: the upstream version is `dev/version.sh`'s, shared with the snaps: `X.Y.Z` on the tag `vX.Y.Z`, where it must equal the changelog's, and `X.Y.Z+git<n>.<sha>` past it. A snapshot sorts by commits since the tag, so a PPA upload from a branch with fewer of them than the last upload is rejected as a downgrade. `PPA=N` appends `~ppaN`, `SERIES=<name>` retargets a series and inserts `~<release>` so older series sort below newer ones. Do not hand-edit the changelog to fake a snapshot.
- The unshare sbuild chroot unpacks under `$TMPDIR`. On a tmpfs `/tmp` a debuginfo build fills it; use `TMPDIR=/var/tmp`. Aborted builds leave multi-gigabyte directories there.
- The binary package is `Architecture: amd64`. Nothing has been tested on another architecture, and resolute's ppc64el build segfaulted in the GTK widget tests. Widen it only after someone runs the application there.
- Keep the source lintian-clean. Run `lintian` on the staged source before uploading and fix tags rather than overriding them.
- The GSettings schema, the desktop entry and the AppStream metainfo are installed by this package, not by the snap. Changing any of them in `client/` changes what this deb ships.
- The vendored closure needs Rust 1.85, which the workspace `rust-version` and `debian/control` state. gtk-rs is held at 0.21 (gtk4 0.10) because 0.22 needs 1.92 and noble's newest toolchain is `rustc-1.91`. Noble's default `rustc` is 1.75, so `build-source.sh` swaps in the versioned `rustc-1.91`/`cargo-1.91` for `SERIES=noble` and `debian/rules` puts its `bin/` first on `PATH`.
- The autopkgtest in `debian/tests/` is the only CI that exercises the installed binary. Extend it when the CLI surface changes.

# Directory

- `build-source.sh` - Stages the orig tarball and debianised tree into `target/deb/`.
- `vendor-copyright.py` - Generates `debian/copyright` from the vendored crates.
- `debian/` - Packaging: `rules` builds offline with `--locked`, `install` places the schema and icons, `rules` installs the catalogs and the translated desktop entry and metainfo, `tests/` is the autopkgtest.
