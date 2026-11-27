#!/bin/sh
# Makes the Linux packages in Docker, into linux/dist/ (not in git).
#
#   ./package.sh              release build, .deb and .tar.gz, check of the desktop files
#   ./package.sh verify       installs the .deb in a clean Debian 13 container, then takes a
#                             picture of the Dashboard (dist/verify-deb.png)
#   ./package.sh verify-tar   installs the tarball into /opt/test in a clean container, then
#                             takes a picture of the Dashboard (dist/verify-tar.png)
#
# The packages are for the architecture of the Docker host (dpkg --print-architecture in the
# container). A verify run has no source tree: it proves that the installed files are
# complete. It uses the network of the host and the node configuration as shot.sh does.
#
# Environment for verify: OUT=<picture.png>, and WAIT, STEPS, CONF as in shot.sh.
set -eu
here="$(cd "$(dirname "$0")" && pwd)"
repo="$(dirname "$here")"
image=carbide-silica-linux
case "${1:-build}" in
    build)
        docker build -q -t "$image" "$here" >/dev/null
        exec docker run --rm \
            -v "$repo":/src \
            -v carbide-linux-cargo:/usr/local/cargo/registry \
            -v "${CARBIDE_TARGET_VOLUME:-carbide-linux-target}":/target \
            -e CARGO_TARGET_DIR=/target \
            -w /src/linux \
            "$image" sh /src/linux/tools/package-inside.sh
        ;;
    verify | verify-tar)
        mode=deb
        [ "$1" = verify-tar ] && mode=tar
        ls "$here"/dist/carbide-silica_*.deb >/dev/null 2>&1 || {
            echo "There is no package in $here/dist. Run ./package.sh first." >&2
            exit 1
        }
        out="${OUT:-$here/dist/verify-$mode.png}"
        mkdir -p "$(dirname "$out")"
        outdir="$(cd "$(dirname "$out")" && pwd)"
        conf="${CONF:-/Volumes/Extreme SSD/CarbideSilica-VRSC/VRSC.conf}"
        docker build -q -t carbide-silica-verify -f "$here/Dockerfile.verify" "$here" >/dev/null
        exec docker run --rm --network host \
            -v "$here/dist":/dist:ro \
            -v "$here/tools":/tools:ro \
            -v "$conf":/conf/VRSC.conf:ro \
            -v "$outdir":/out \
            -e WAIT="${WAIT:-60}" \
            -e STEPS="${STEPS:-}" \
            carbide-silica-verify sh /tools/verify-inside.sh "$mode" "/out/$(basename "$out")"
        ;;
    *)
        echo "usage: ./package.sh [verify | verify-tar]" >&2
        exit 1
        ;;
esac
