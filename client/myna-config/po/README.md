# myna-config translations

User-visible strings use the `myna-config` gettext domain: `gettext()` in
Rust, `_()` in Blueprint, the `<summary>` elements of the shared GSettings
schema, and the AppStream metainfo's name, summary and description. Regenerate the template with `make i18n` from the repository root
(`dev/i18n.sh` lists the crate); `make check` fails while the committed
template is stale.

Set `MYNA_CONFIG_LOCALEDIR` to test a catalog outside the system locale
directories. Without an installed catalog, gettext safely returns each source
string unchanged.
