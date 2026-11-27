#!/bin/sh
# The part of package.sh that runs in the build container. See package.sh.
set -eu
cd /src/linux
app_id=org.carbide.CarbideSilica
version=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -1)
arch=$(dpkg --print-architecture)
machine=$(uname -m)
dist=/src/linux/dist
work=/target/package

echo "Check the desktop file and the metainfo file."
desktop-file-validate "data/$app_id.desktop"
appstreamcli validate --no-net --pedantic "data/$app_id.metainfo.xml"

echo "Build the release binary."
cargo build --release --quiet
binary=/target/release/carbide-silica
test -x "$binary"

# Copies the installed files into a prefix: bin/, share/.
stage() {
    prefix="$1"
    share="$prefix/share"
    install -D -m 755 "$binary" "$prefix/bin/carbide-silica"
    install -d "$share/carbide-silica/fonts" "$share/carbide-silica/sounds"
    install -m 644 resources/fonts/* "$share/carbide-silica/fonts/"
    install -m 644 resources/sounds/* "$share/carbide-silica/sounds/"
    install -D -m 644 "data/$app_id.desktop" "$share/applications/$app_id.desktop"
    install -D -m 644 "data/$app_id.metainfo.xml" "$share/metainfo/$app_id.metainfo.xml"
    svg="data/icons/hicolor/scalable/apps/$app_id.svg"
    install -D -m 644 "$svg" "$share/icons/hicolor/scalable/apps/$app_id.svg"
    for size in 16 24 32 48 64 128 256 512; do
        folder="$share/icons/hicolor/${size}x${size}/apps"
        install -d "$folder"
        rsvg-convert -w "$size" -h "$size" "$svg" -o "$folder/$app_id.png"
    done
    install -D -m 644 data/copyright "$share/doc/carbide-silica/copyright"
    install -m 644 ../LICENSE "$share/doc/carbide-silica/LICENSE"
}

rm -rf "$work"
mkdir -p "$work" "$dist"

echo "Make the Debian package."
root="$work/deb"
stage "$root/usr"
# dpkg-shlibdeps reads the libraries that the binary links and finds their packages.
mkdir -p "$work/shlibs/debian"
printf 'Source: carbide-silica\n\nPackage: carbide-silica\nArchitecture: any\n' > "$work/shlibs/debian/control"
depends=$(cd "$work/shlibs" && dpkg-shlibdeps -O "$root/usr/bin/carbide-silica" 2>/dev/null \
    | sed -n 's/^shlibs:Depends=//p')
test -n "$depends"
# The HTTP client of the bridge loads the CA certificates of the system when it starts, also
# for a node on 127.0.0.1. Without them it does not connect. dpkg-shlibdeps cannot see this.
depends="$depends, ca-certificates"
size=$(du -sk "$root/usr" | cut -f1)
mkdir -p "$root/DEBIAN"
cat > "$root/DEBIAN/control" <<CONTROL
Package: carbide-silica
Version: $version
Architecture: $arch
Maintainer: aelder <https://github.com/aelder/CarbideSilica/issues>
Installed-Size: $size
Depends: $depends
Recommends: fonts-noto-color-emoji, fonts-noto-cjk
Section: net
Priority: optional
Homepage: https://github.com/aelder/CarbideSilica
Description: Explorer for Verus and PBaaS chains
 Carbide Silica shows the data of a Verus node as charts and diagrams. Each
 page also gives the exact figures. The app is read-only. It does not read a
 wallet, and it cannot create, sign or send a transaction.
CONTROL
(cd "$root" && find usr -type f | sort | xargs md5sum > DEBIAN/md5sums)
deb="$dist/carbide-silica_${version}_${arch}.deb"
rm -f "$dist"/carbide-silica_*.deb
dpkg-deb --root-owner-group -Zxz --build "$root" "$deb" >/dev/null

echo "Make the tarball."
name="carbide-silica-$version-linux-$machine"
tree="$work/tar/$name"
stage "$tree"
install -m 755 data/tarball/install.sh data/tarball/uninstall.sh "$tree/"
rm -f "$dist"/carbide-silica-*-linux-*.tar.gz
tar -C "$work/tar" --owner=0 --group=0 -czf "$dist/$name.tar.gz" "$name"

echo
echo "Depends: $depends"
ls -l "$dist"
