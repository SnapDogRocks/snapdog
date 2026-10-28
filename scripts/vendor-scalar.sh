#!/bin/sh
# Fetch a release of the Scalar API Reference browser bundle for the API console and
# prepare it for snapdog/assets/scalar/. Read-only against the network; it writes only the
# compressed bundle. See snapdog/assets/scalar/NOTICE.md for what has to be updated by hand.
#
#   scripts/vendor-scalar.sh 1.74.0
#
# Pick a version that is at least two weeks old (npm view @scalar/api-reference time).
set -eu

version=${1:?usage: scripts/vendor-scalar.sh <@scalar/api-reference version>}
root=$(cd "$(dirname "$0")/.." && pwd)
dir=$(mktemp -d)
trap 'rm -rf "$dir"' EXIT

url="https://cdn.jsdelivr.net/npm/@scalar/api-reference@${version}/dist/browser/standalone.js"
curl -fsS --max-time 120 -o "$dir/standalone.js" "$url"

out="$root/snapdog/assets/scalar/standalone-${version}.js.gz"
gzip -9 -n -c "$dir/standalone.js" > "$out"

sha256() { shasum -a 256 "$1" | cut -d' ' -f1; }
echo "source:            $url"
echo "original size:     $(wc -c < "$dir/standalone.js" | tr -d ' ') bytes"
echo "original sha256:   $(sha256 "$dir/standalone.js")"
echo "embedded file:     $out"
echo "embedded size:     $(wc -c < "$out" | tr -d ' ') bytes"
echo "embedded sha256:   $(sha256 "$out")"
echo
echo "Now update: SCRIPT_GZ and SCALAR_VERSION in snapdog/src/api/console.rs, the table in"
echo "snapdog/assets/scalar/NOTICE.md, the pinned values in snapdog/tests/api_console.rs,"
echo "and the README. Check the licence of the new version and delete the old .gz."
echo "Open /docs in a browser once: the page must contact no host but this server."
