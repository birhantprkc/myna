#!/bin/sh
# Check that the extension's metadata.json shell-version covers the GNOME
# Shell major a series ships. gnome-shell marks a copy that lacks it
# OUT_OF_DATE, and the deb's copy registers before Ubuntu's own, so a missing
# major hides the indicator rather than falling back to the archive's copy.
#
# The target series failing is an error. The devel series is checked too, as
# a warning only: that gap is what a release upgrade lands in, and it closes
# once `make test-extension-next` is green against the new major. A failed
# archive query is a warning, so offline builds still stage.
#
# Usage: dev/check-shell-version.sh <metadata.json> <series>
set -eu

if [ $# -ne 2 ]; then
    echo "usage: $0 <metadata.json> <series>" >&2
    exit 2
fi
metadata=$1
series=$2

supported=$(python3 -c 'import json,sys; print(" ".join(json.load(open(sys.argv[1]))["shell-version"]))' "$metadata")

# Prints the highest gnome-shell major in <series>, or nothing.
shell_major() {
    rmadison -s "$1" gnome-shell 2>/dev/null \
        | awk -F'|' '{ v = $2; gsub(/ /, "", v); sub(/^[0-9]+:/, "", v); sub(/[.~-].*/, "", v); print v }' \
        | sort -n | tail -n 1
}

# check <series> <error|warning>
check() {
    major=$(shell_major "$1")
    if [ -z "$major" ]; then
        echo "warning: could not look up gnome-shell in $1; shell-version unchecked" >&2
        return 0
    fi
    for v in $supported; do
        [ "$v" = "$major" ] && return 0
    done
    echo "$2: $1 ships gnome-shell $major, not in shell-version ($supported) of $metadata" >&2
    [ "$2" = warning ]
}

check "$series" error
devel=$(ubuntu-distro-info --devel 2>/dev/null || true)
if [ -n "$devel" ] && [ "$devel" != "$series" ]; then
    check "$devel" warning
fi
