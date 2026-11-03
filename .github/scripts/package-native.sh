#!/usr/bin/env bash
set -euo pipefail

version=${RELEASE_VERSION:?RELEASE_VERSION is required}
target=${RELEASE_TARGET:?RELEASE_TARGET is required}
host=$(rustc -vV | sed -n 's/^host: //p')
if [[ "$host" != "$target" ]]; then
  printf 'Expected native target %s, got %s\n' "$target" "$host" >&2
  exit 1
fi

root=$(pwd)
output="$root/target/release-assets"
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
mkdir -p "$output"

case "$target" in
  *-apple-darwin)
    app="$stage/Lockbox.app"
    mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
    install -m 755 target/release/lockbox target/release/lockbox-browser-host "$app/Contents/MacOS/"
    install -m 644 assets/LICENSE-simple-icons.md assets/DISCLAIMER-simple-icons.md \
      "$app/Contents/Resources/"
    python3 - "$app/Contents/Info.plist" "$version" <<'PY'
import plistlib
import sys

with open(sys.argv[1], "wb") as output:
    plistlib.dump({
        "CFBundleIdentifier": "app.lockbox.desktop",
        "CFBundleName": "Lockbox",
        "CFBundleExecutable": "lockbox",
        "CFBundlePackageType": "APPL",
        "CFBundleVersion": sys.argv[2],
        "CFBundleShortVersionString": sys.argv[2],
        "LSMinimumSystemVersion": "11.0",
        "NSHighResolutionCapable": True,
        "CFBundleURLTypes": [{
            "CFBundleURLName": "app.lockbox.browser",
            "CFBundleURLSchemes": ["lockbox"],
            "CFBundleTypeRole": "Viewer",
        }],
    }, output)
PY
    plutil -lint "$app/Contents/Info.plist"
    # Ad-hoc signatures need no Apple credentials. Distribution is not notarized.
    codesign --force --sign - "$app/Contents/MacOS/lockbox-browser-host"
    codesign --force --sign - "$app"
    codesign --verify --deep --strict "$app"
    ditto -c -k --sequesterRsrc --keepParent "$app" "$output/lockbox-$version-$target.zip"
    ;;
  x86_64-unknown-linux-gnu)
    package="$stage/deb-root"
    mkdir -p "$package/DEBIAN" "$package/usr/bin" \
      "$package/usr/share/applications" "$package/usr/share/icons/hicolor/scalable/apps" \
      "$package/usr/share/doc/lockbox" "$stage/debian"
    install -m 755 target/release/lockbox target/release/lockbox-browser-host "$package/usr/bin/"
    strip --strip-unneeded "$package/usr/bin/lockbox" "$package/usr/bin/lockbox-browser-host"
    install -m 644 assets/lockbox-mark.svg "$package/usr/share/icons/hicolor/scalable/apps/lockbox.svg"
    install -m 644 assets/LICENSE-simple-icons.md assets/DISCLAIMER-simple-icons.md \
      "$package/usr/share/doc/lockbox/"
    cat > "$package/usr/share/applications/lockbox.desktop" <<'EOF'
[Desktop Entry]
Version=1.0
Type=Application
Name=Lockbox
Comment=Local-first password manager
Exec=lockbox %u
Icon=lockbox
Terminal=false
Categories=Utility;Security;
MimeType=x-scheme-handler/lockbox;
StartupWMClass=lockbox
EOF
    cat > "$stage/debian/control" <<'EOF'
Source: lockbox
Section: utils
Priority: optional
Maintainer: Lockbox maintainers <lockbox@users.noreply.github.com>

Package: lockbox
Architecture: amd64
Description: Local-first password manager
EOF
    depends=$(cd "$stage" && dpkg-shlibdeps -O \
      -e"$package/usr/bin/lockbox" -e"$package/usr/bin/lockbox-browser-host" | sed -n 's/^shlibs:Depends=//p')
    test -n "$depends"
    cat > "$package/DEBIAN/control" <<EOF
Package: lockbox
Version: $version
Section: utils
Priority: optional
Architecture: amd64
Maintainer: Lockbox maintainers <lockbox@users.noreply.github.com>
Depends: $depends, xdg-utils
Installed-Size: $(du -sk "$package/usr" | cut -f1)
Description: Local-first password manager
 Local encrypted vaults and experimental shared-vault synchronization.
EOF
    dpkg-deb --root-owner-group --build "$package" "$output/lockbox_${version}_amd64.deb"
    portable="$stage/lockbox-$version-$target"
    mkdir -p "$portable"
    cp "$package/usr/bin/lockbox" "$package/usr/bin/lockbox-browser-host" \
      "$package/usr/share/doc/lockbox/"* "$portable/"
    cat > "$portable/README.txt" <<EOF
Lockbox $version — Linux x86_64; Ubuntu 22.04 / Linux Mint 21.1 or newer.
Runtime dependencies: $depends, xdg-utils
Run ./lockbox after installing these dependencies.
The .deb installs dependencies, the application menu entry, and lockbox:// handler:
  sudo apt install ./lockbox_${version}_amd64.deb
Native browser host: /usr/bin/lockbox-browser-host after installing the .deb.
Browser host registration remains a separate setup step; see ext/README.md.
EOF
    tar -C "$stage" -czf "$output/lockbox-$version-$target.tar.gz" "$(basename "$portable")"
    ;;
  *)
    printf 'Unsupported release target: %s\n' "$target" >&2
    exit 1
    ;;
esac
