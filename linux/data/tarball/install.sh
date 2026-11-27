#!/bin/sh
# Installs Carbide Silica into a prefix. The default prefix is ~/.local.
#
#   ./install.sh [prefix]
#
# The script copies bin/ and share/ into the prefix. It writes the list of the files into
# <prefix>/share/carbide-silica/installed-files, for uninstall.sh. The desktop file gets the
# full path of the binary, so the desktop finds the app when <prefix>/bin is not in PATH.
set -eu
here="$(cd "$(dirname "$0")" && pwd)"
prefix="${1:-$HOME/.local}"
mkdir -p "$prefix"
prefix="$(cd "$prefix" && pwd)"
list="$prefix/share/carbide-silica/installed-files"
desktop="share/applications/org.carbide.CarbideSilica.desktop"

cd "$here"
mkdir -p "$prefix/share/carbide-silica"
: > "$list.new"
find bin share -type f | sort | while IFS= read -r file; do
    mkdir -p "$prefix/$(dirname "$file")"
    cp "$file" "$prefix/$file"
    printf '%s\n' "$file" >> "$list.new"
done
chmod 755 "$prefix/bin/carbide-silica" "$prefix/bin/carbide-silica-demo"
sed -e "s|^Exec=carbide-silica$|Exec=\"$prefix/bin/carbide-silica\"|" \
    -e "s|^Exec=carbide-silica-demo$|Exec=\"$prefix/bin/carbide-silica-demo\"|" \
    -e "s|^TryExec=.*|TryExec=$prefix/bin/carbide-silica|" \
    "$desktop" > "$prefix/$desktop"
printf '%s\n' "share/carbide-silica/installed-files" >> "$list.new"
mv "$list.new" "$list"

echo "Carbide Silica is installed in $prefix."
case ":$PATH:" in
    *":$prefix/bin:"*) echo "Type carbide-silica to start the app." ;;
    *) echo "Type $prefix/bin/carbide-silica to start the app, or add $prefix/bin to PATH." ;;
esac
echo "Run the synthetic demo with: $prefix/bin/carbide-silica-demo"
