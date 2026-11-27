#!/bin/sh
# Removes Carbide Silica from a prefix. The default prefix is ~/.local.
#
#   ./uninstall.sh [prefix]
#
# The script removes only the files that install.sh wrote into that prefix. It does not
# remove the settings (~/.config/carbide-silica) or the cache (~/.cache/verus-carbide).
set -eu
prefix="${1:-$HOME/.local}"
list="$prefix/share/carbide-silica/installed-files"
if [ ! -f "$list" ]; then
    echo "Carbide Silica is not installed in $prefix: $list does not exist." >&2
    exit 1
fi
while IFS= read -r file; do
    case "$file" in
        "" | /* | *..*) continue ;;
    esac
    [ "$file" = share/carbide-silica/installed-files ] && continue
    rm -f "$prefix/$file"
done < "$list"
rm -f "$list"
for folder in fonts sounds demo ""; do
    rmdir "$prefix/share/carbide-silica/$folder" 2>/dev/null || true
done
rmdir "$prefix/share/doc/carbide-silica" 2>/dev/null || true
echo "Carbide Silica is removed from $prefix."
