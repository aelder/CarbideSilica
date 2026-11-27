#!/bin/sh
# The part of shot.sh that runs in the container. See shot.sh for the steps.
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
# More start-up arguments (ARGS), split at spaces.
# shellcheck disable=SC2086
[ -n "${ARGS:-}" ] && set -- "$@" $ARGS
# The app runs in its home folder, so a file dialog does not open in the source tree and a
# file saved during a run is gone with the container.
(cd "$HOME" && exec "$binary" "$@") >/tmp/app.log 2>&1 &
app=$!

# The window of the app that was shown last: the main window, or a dialog over it.
window=""
find_window() {
    timeout 10 xdotool search --sync --onlyvisible --name "Carbide Silica" >/dev/null 2>&1 || true
    window=$(xdotool search --onlyvisible --pid "$app" 2>/dev/null | tail -1 || true)
}

# Gives the last window of the app the keyboard. Xvfb has no window manager, so a window
# does not get the focus by itself, and without the focus the first key goes to the root
# window. The focus is set directly, and the first time a Shift press makes GTK take the
# focus before a real key comes.
focused=0
focus() {
    find_window
    [ -n "$window" ] || return 0
    # Without --sync: a window that cannot take the focus must not stop the run.
    xdotool windowfocus "$window" 2>/dev/null || true
    sleep 0.2
    if [ "$focused" = 0 ]; then
        sleep 0.3
        xdotool key --clearmodifiers shift
        sleep 0.3
        focused=1
    fi
}

# A picture of the whole display. A name without a folder goes to the folder of <out.png>.
picture() {
    name="$1"
    case "$name" in
        */*) echo "shot: a picture name has no folder: $name" >&2; return 1 ;;
    esac
    import -window root "$(dirname "$out")/$name"
    echo "picture: $name"
}

# One step of STEPS. See shot.sh.
step() {
    verb="$1"
    shift
    case "$verb" in
        wait | sleep) sleep "${1:-1}" ;;
        key)
            focus
            for key in "$@"; do xdotool key --clearmodifiers "$key"; sleep 0.4; done
            ;;
        type)
            focus
            xdotool type --delay 60 "$*"
            ;;
        move) xdotool mousemove "$1" "$2"; sleep 0.6 ;;
        click)
            xdotool mousemove "$1" "$2"
            sleep 0.3
            xdotool click "${3:-1}"
            focused=1
            sleep 0.6
            ;;
        dclick)
            xdotool mousemove "$1" "$2"
            sleep 0.3
            xdotool click --repeat 2 --delay 80 1
            focused=1
            sleep 0.6
            ;;
        drag)
            # drag x1 y1 x2 y2: press at the first point, move in steps, release at the second
            xdotool mousemove "$1" "$2"
            sleep 0.3
            xdotool mousedown 1
            for i in 1 2 3 4 5 6 7 8; do
                xdotool mousemove $(( $1 + ($3 - $1) * i / 8 )) $(( $2 + ($4 - $2) * i / 8 ))
                sleep 0.08
            done
            xdotool mouseup 1
            focused=1
            sleep 0.6
            ;;
        scroll)
            # scroll up|down|left|right [count] [x y]
            [ $# -ge 4 ] && xdotool mousemove "$3" "$4" && sleep 0.3
            case "$1" in
                up) button=4 ;;
                down) button=5 ;;
                left) button=6 ;;
                right) button=7 ;;
                *) echo "shot: scroll needs up, down, left or right" >&2; return 1 ;;
            esac
            xdotool click --repeat "${2:-1}" --delay 40 "$button"
            sleep 0.6
            ;;
        shot) picture "$1" ;;
        size)
            # size W H: the size of the last window of the app, such as a narrow window.
            find_window
            [ -n "$window" ] && xdotool windowsize "$window" "$1" "$2"
            sleep 1
            ;;
        clip)
            # clip name.txt: writes the text of the clipboard into the folder of <out.png>.
            xclip -o -selection clipboard > "$(dirname "$out")/$1" 2>/dev/null || true
            echo "clipboard: $1"
            ;;
        focus) focus ;;
        "") ;;
        *) echo "shot: unknown step: $verb" >&2; return 1 ;;
    esac
}

sleep "$WAIT"

# The older variables, as steps before STEPS.
if [ -n "${KEYS:-}" ]; then
    # shellcheck disable=SC2086
    step key $KEYS
    sleep "${KEYWAIT:-3}"
fi
if [ -n "${MOUSE:-}" ]; then
    # shellcheck disable=SC2086
    step move $MOUSE
fi

# STEPS: steps separated by ";" or new lines, done in order.
if [ -n "${STEPS:-}" ]; then
    printf '%s\n' "$STEPS" | tr ';' '\n' > /tmp/steps
    while IFS= read -r line; do
        # shellcheck disable=SC2086
        set -f; set -- $line; set +f
        [ $# -eq 0 ] && continue
        step "$@"
    done < /tmp/steps
fi

import -window root "$out"
echo "picture: $(basename "$out")"
kill "$app" 2>/dev/null || true
# The app's own messages, without anything that looks like a credential.
# A panic is named first, with the lines of this crate in its backtrace.
grep -viE 'rpcuser|rpcpassword|password' /tmp/app.log | grep -E "panicked|carbide_silica::" -A1 | head -30 || true
grep -viE 'rpcuser|rpcpassword|password' /tmp/app.log | tail -20 || true
