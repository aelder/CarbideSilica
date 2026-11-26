#!/bin/sh
# Takes a picture of the Linux app, headless, in Docker.
#
#   ./shot.sh <object-or-empty> <out.png>
#
# The app runs on a virtual X display of 1281x1084 (the logical size of the pictures in
# macos/docs/images) with the node configuration of the Mac mounted read-only, and the
# picture is the whole display. <object> is opened as Carbide's -open= does: a height, a
# hash, a transaction ID, name@ or currency@@. An empty object shows the Dashboard.
#
# Environment:
#   WAIT=60          seconds to wait for data before the picture
#   KEYS="j j"       keys to press (xdotool names) before the picture
#   KEYWAIT=3        seconds to wait after the keys
#   MOUSE="x y"      a place for the pointer, to show a hover state
#   CONF=<file>      the node configuration (default: the VRSC.conf of the Extreme SSD)
#   GSK_RENDERER=... GTK's renderer (default: GTK's choice)
#
# The container uses the network of the host to reach the node's RPC port. Its HOME, its
# settings and its cache are in /tmp of the container and are gone after the run. No
# public API is enabled.
set -eu
here="$(cd "$(dirname "$0")" && pwd)"
repo="$(dirname "$here")"
object="${1:-}"
out="${2:?usage: shot.sh <object-or-empty> <out.png>}"
conf="${CONF:-/Volumes/Extreme SSD/CarbideSilica-VRSC/VRSC.conf}"
image=carbide-silica-linux
mkdir -p "$(dirname "$out")"
outdir="$(cd "$(dirname "$out")" && pwd)"
docker build -q -t "$image" "$here" >/dev/null
docker run --rm --network host \
    -v "$repo":/src \
    -v carbide-linux-cargo:/usr/local/cargo/registry \
    -v "${CARBIDE_TARGET_VOLUME:-carbide-linux-target}":/target \
    -v "$conf":/conf/VRSC.conf:ro \
    -v "$outdir":/out \
    -e CARGO_TARGET_DIR=/target \
    -e WAIT="${WAIT:-60}" \
    -e KEYS="${KEYS:-}" \
    -e KEYWAIT="${KEYWAIT:-3}" \
    -e MOUSE="${MOUSE:-}" \
    -e GSK_RENDERER="${GSK_RENDERER:-}" \
    -w /src/linux \
    "$image" sh /src/linux/tools/shot-inside.sh "$object" "/out/$(basename "$out")"
