#!/bin/sh
# install-translations.sh PO_DIR DEST_DIR
#
# Compiles every catalog PO_DIR/LINGUAS names into DEST_DIR/usr/share/locale,
# and merges them into the desktop entry and AppStream metainfo it installs
# under DEST_DIR/usr/share. The deb build runs it on po/; the translations
# contract test runs it on a scratch catalog.
set -eu

po=$1
dest=$2
data=$(dirname "$0")/../data
id=com.canonical.Myna.Config

# shellcheck disable=SC2013  # LINGUAS separates codes by any whitespace
for lang in $(sed 's/#.*//' "$po/LINGUAS"); do
    install -d "$dest/usr/share/locale/$lang/LC_MESSAGES"
    msgfmt --check-format -o "$dest/usr/share/locale/$lang/LC_MESSAGES/myna-config.mo" "$po/$lang.po"
done

install -d "$dest/usr/share/applications" "$dest/usr/share/metainfo"
msgfmt --desktop -d "$po" --template "$data/$id.desktop" \
    -o "$dest/usr/share/applications/$id.desktop"
msgfmt --xml -d "$po" --template "$data/$id.metainfo.xml" \
    -o "$dest/usr/share/metainfo/$id.metainfo.xml"
