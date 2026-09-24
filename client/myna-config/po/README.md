# myna-config translations

User-visible strings use the `myna-config` gettext domain: `gettext()` in
Rust, `_()` in Blueprint, the `<summary>` elements of the shared GSettings
schema, the AppStream metainfo's name, summary and description, and the
desktop entry. Regenerate the template with `make i18n` from the repository
root (`dev/i18n.sh` lists the crate); `make check` fails while the committed
template is stale.

`LINGUAS` lists the shipped languages. The deb build compiles each catalog and
merges it into the desktop entry and metainfo
(`build/install-translations.sh`).

Set `MYNA_CONFIG_LOCALEDIR` to test a catalog outside the system locale
directories. Without an installed catalog, gettext safely returns each source
string unchanged.
