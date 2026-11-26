#!/bin/sh
# The part of shot.sh that runs in the container. See shot.sh.
set -eu
object="$1"
out="$2"
cargo build --quiet 2>&1 | grep -E '^error' -A5 || true
binary=/target/debug/carbide-silica
test -x "$binary"
export HOME=/tmp/home
export XDG_CONFIG_HOME=/tmp/home/config XDG_CACHE_HOME=/tmp/home/cache XDG_DATA_HOME=/tmp/home/data
mkdir -p "$XDG_CONFIG_HOME" "$XDG_CACHE_HOME" "$XDG_DATA_HOME"
export CARBIDE_SILICA_SETTINGS=/tmp/home/settings.json
export GDK_BACKEND=x11
[ -n "${GSK_RENDERER:-}" ] || unset GSK_RENDERER
# Containers on the host network share abstract X sockets, so each run takes its own display
# number and does not listen on the shared socket.
display=$(( 300 + $(od -An -N2 -tu2 /dev/urandom) % 600 ))
Xvfb ":$display" -screen 0 1281x1084x24 -nolisten tcp -nolisten local >/tmp/xvfb.log 2>&1 &
export DISPLAY=":$display"
sleep 1
set -- "-conf=/conf/VRSC.conf"
[ -n "$object" ] && set -- "$@" "-open=$object"
"$binary" "$@" >/tmp/app.log 2>&1 &
app=$!
sleep "$WAIT"
# KEYS: keys to press before the picture, for xdotool (for example "ctrl+k" or "j j Return").
if [ -n "${KEYS:-}" ]; then
    xdotool search --name "Carbide Silica" windowactivate --sync 2>/dev/null || true
    for key in $KEYS; do xdotool key "$key"; sleep 0.4; done
    sleep "${KEYWAIT:-3}"
fi
# MOUSE: "x y" moves the pointer there before the picture, to show a hover state.
if [ -n "${MOUSE:-}" ]; then
    xdotool mousemove $MOUSE
    sleep 1
fi
import -window root "$out"
kill "$app" 2>/dev/null || true
# The app's own messages, without anything that looks like a credential.
grep -viE 'rpcuser|rpcpassword|password' /tmp/app.log | tail -20 || true
echo "picture: $out"
