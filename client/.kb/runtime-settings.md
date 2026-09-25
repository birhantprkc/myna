# Preface

Read this document when changing persisted client preferences, the streaming mode, or live settings reload.

Read the top-level `.kb/agents.md` file before continuing below.

# Overview

Client settings use GSettings schema `com.canonical.Myna.Dictation` with the keyfile backend. Packaged and unpackaged clients use the same schema and storage shape.

# Architecture

The snap stores settings below `$SNAP_USER_COMMON/.config`; unpackaged development uses the host configuration directory and requires `make install-schema`. `myna_core::Settings` is the shared access layer.

`streaming-mode` is used as stored by `myna-testbed` and `myna-desktop`:

- `streaming` (the default) displays committed deltas as they arrive and enables preedit when supported.
- `batch` delays display and injection until the utterance completes.

A stored value outside the schema enum, such as the retired `auto`, reads as the default. There is no hardware gate; a capability-based default is a future redesign.

The mode is a client presentation preference, not wire negotiation. A streaming backend can feed a batch client, which accumulates committed deltas until completion.

`silence-timeout` (unsigned seconds, 0 = off, schema default 30) is the toggle session's idle limit. `Settings::default()` carries the schema default too, so a machine without the schema still ends a forgotten session. The daemon reads it live at every stats tick, so a change applies to the session in progress. Myna Settings renders any bounded integer key as a spin row; a key with a GVariant type the adapter cannot widen to `i64` fails the whole page, so add the conversion before adding such a key.

# Important

- Apply live-reloadable settings without restart. Activation and hotkey changes require rebinding and must report that limitation.
- Use command-line overrides for debugging without mutating persisted preferences.
