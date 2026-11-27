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
#   WAIT=60          seconds to wait for data before the first step
#   STEPS="..."      steps done in order after WAIT, separated by ";" or new lines (below)
#   KEYS="j j"       keys to press (xdotool names) before STEPS
#   KEYWAIT=3        seconds to wait after KEYS
#   MOUSE="x y"      a place for the pointer before STEPS, to show a hover state
#   ARGS="-rpcport=1" more start-up arguments for the app, separated by spaces
#   STANDARD_CONF=1  put the configuration at ~/.komodo/VRSC and give no -conf argument
#   CONF=<file>      the node configuration (default: the VRSC.conf of the Extreme SSD)
#   GSK_RENDERER=... GTK's renderer (default: GTK's choice)
#   GEOIP=<folder>   an existing location data folder, mounted read-only where the app looks
#
# Steps (coordinates are pixels of the 1281x1084 display):
#   wait 5              wait 5 seconds
#   key Tab ctrl+k      press keys, with xdotool's key names
#   type vETH@          type a text
#   move 400 300        move the pointer
#   click 400 300 [b]   move the pointer and click (button b, 1 by default)
#   dclick 400 300      double-click
#   drag 400 300 600 300  press, move and release button 1
#   scroll down 5 [x y] turn the wheel 5 steps (up, down, left, right), at x y if given
#   shot name.png       take a picture now, into the folder of <out.png>
#   size 980 700        resize the last window of the app
#   clip name.txt       write the text of the clipboard into the folder of <out.png>
#   icon name.png       write the largest window icon of the app into that folder, and
#                       print the WM_CLASS of the window
# The picture <out.png> is taken after the last step. For example:
#   STEPS="key Tab; wait 2; shot json.png; scroll down 10 640 600; wait 1" \
#     ./shot.sh 4338679 /tmp/block/end.png
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
# GEOIP=<folder>: an existing location data folder, mounted read-only where the app looks.
set --
[ -n "${GEOIP:-}" ] && set -- -v "$GEOIP":/tmp/home/cache/verus-carbide/geoip:ro
docker run --rm --network host "$@" \
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
    -e STEPS="${STEPS:-}" \
    -e ARGS="${ARGS:-}" \
    -e STANDARD_CONF="${STANDARD_CONF:-}" \
    -e GSK_RENDERER="${GSK_RENDERER:-}" \
    -w /src/linux \
    "$image" sh /src/linux/tools/shot-inside.sh "$object" "/out/$(basename "$out")"
