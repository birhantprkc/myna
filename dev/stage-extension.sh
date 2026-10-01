#!/bin/sh
# Stage the myna-shell extension as gnome-shell loads it: <dest>/<uuid>/ with
# the .js files and a metadata.json stamped with version-name=<version>.
# The directory name is the uuid read from metadata.json, since gnome-shell
# keys the extension on it and silently skips a mismatch.
#
# Usage: dev/stage-extension.sh <src-dir> <dest-dir> <version>
set -eu

if [ $# -ne 3 ]; then
    echo "usage: $0 <src-dir> <dest-dir> <version>" >&2
    exit 2
fi
src=$1
dest=$2
version=$3

uuid=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["uuid"])' "$src/metadata.json")
rm -rf "${dest:?}/$uuid"
mkdir -p "$dest/$uuid"
cp "$src"/*.js "$dest/$uuid/"
python3 -c 'import json,sys; m=json.load(open(sys.argv[1])); m["version-name"]=sys.argv[2]
json.dump(m, open(sys.argv[3], "w"), indent=2)' \
    "$src/metadata.json" "$version" "$dest/$uuid/metadata.json"
echo "$dest/$uuid"
