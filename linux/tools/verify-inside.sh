#!/bin/sh
# The part of "package.sh verify" that runs in a clean Debian container. It has no source
# tree and no build tools. It installs a package from /dist, then takes a picture with
# shot-inside.sh.
#   verify-inside.sh deb <out.png>    apt-get install of the .deb
#   verify-inside.sh tar <out.png>    the libraries from the .deb, then the tarball in /opt/test
set -eu
mode="$1"
out="$2"
deb=$(ls /dist/carbide-silica_*.deb | head -1)
apt-get update -qq
# Only the Depends of the package, so a missing dependency shows here.
apt-get install -y -qq --no-install-recommends "$deb" >/tmp/apt.log 2>&1 || { tail -30 /tmp/apt.log; exit 1; }
case "$mode" in
    deb)
        binary=/usr/bin/carbide-silica
        dpkg -L carbide-silica | grep -v '/$' | head -40
        ;;
    tar)
        dpkg -r carbide-silica >/dev/null
        test ! -e /usr/bin/carbide-silica
        mkdir -p /tmp/unpack
        tar -C /tmp/unpack -xzf "$(ls /dist/carbide-silica-*-linux-*.tar.gz | head -1)"
        sh /tmp/unpack/carbide-silica-*/install.sh /opt/test
        binary=/opt/test/bin/carbide-silica
        find /opt/test -type f | sort | head -40
        grep '^Exec=' /opt/test/share/applications/org.carbide.CarbideSilica.desktop
        ;;
    *) echo "verify: deb or tar" >&2; exit 1 ;;
esac
test ! -e /src
echo "source tree: absent"
BINARY="$binary" exec sh /tools/shot-inside.sh "" "$out"
