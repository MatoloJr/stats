#!/usr/bin/env bash
# Fails when VERSION, the Cargo workspace version and linux/metadata.json
# "version-name" disagree. With a tag argument (v1.2.3) it checks that too.
set -euo pipefail
cd "$(dirname "$0")/.."
want=$(tr -d '[:space:]' < VERSION)
cargo=$(sed -n '/^\[workspace.package\]/,/^\[/{s/^version *= *"\(.*\)"/\1/p}' Cargo.toml)
gnome=$(sed -n 's/.*"version-name": *"\([^"]*\)".*/\1/p' linux/metadata.json)
status=0
for pair in "Cargo.toml:$cargo" "linux/metadata.json:$gnome" ${1:+"tag:${1#v}"}; do
    if [ "${pair#*:}" != "$want" ]; then
        echo "FAIL ${pair%%:*} has '${pair#*:}' but VERSION is '$want'" >&2
        status=1
    fi
done
[ "$status" -eq 0 ] && echo "ok: version $want everywhere"
exit "$status"
