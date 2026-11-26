#!/bin/sh
# Builds the Linux app in Docker. The cargo registry and the target folder are named Docker
# volumes, so a rebuild compiles only what changed and nothing is written to this Mac.
#   ./build.sh            debug build
#   ./build.sh test       unit tests
#   ./build.sh <cargo args>
set -eu
here="$(cd "$(dirname "$0")" && pwd)"
repo="$(dirname "$here")"
image=carbide-silica-linux
docker build -q -t "$image" "$here" >/dev/null
case "${1:-build}" in
    build) set -- build ;;
    test) set -- test ;;
esac
exec docker run --rm \
    -v "$repo":/src \
    -v carbide-linux-cargo:/usr/local/cargo/registry \
    -v "${CARBIDE_TARGET_VOLUME:-carbide-linux-target}":/target \
    -e CARGO_TARGET_DIR=/target \
    -w /src/linux \
    "$image" cargo "$@"
