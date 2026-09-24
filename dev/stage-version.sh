#!/bin/bash
# Stage the checkout's version (dev/version.sh) into a snap project, and print it.
#
#   dev/stage-version.sh <snap-dir> <path>...
#
# Writes <snap-dir>/version/version.metainfo.xml, which the snap's `version`
# part adopts through parse-info. Not `craftctl set version`: when craft-parts
# updates rather than reruns a step it keeps the step's old project variables,
# so an incremental pack ships the previous version; parse-info is re-read on
# every pack. The paths are what the snap packs, for -dirty.
set -euo pipefail

snap_dir="$(cd "$1" && pwd)"
shift
repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

version=$("$repo_root/dev/version.sh" "$@")
mkdir -p "$snap_dir/version"
printf '<component><releases><release version="%s"/></releases></component>\n' \
    "$version" >"$snap_dir/version/version.metainfo.xml"
echo "$version"
