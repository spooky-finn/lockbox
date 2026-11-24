#!/usr/bin/env bash
set -euo pipefail

if [[ $(uname -s) != Darwin || $# -gt 1 ]]; then
  printf 'Usage on macOS: %s [applications-directory]\n' "$0" >&2
  exit 1
fi

applications=$(cd -- "${1:-/Applications}" && pwd -P)
cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."
root=$(pwd -P)
app="$applications/Lockbox.app"
stage="$applications/.lockbox-install"
backup="$stage/previous/Lockbox.app"

if ! mkdir -- "$stage"; then
  printf 'Could not prepare %s. Check permissions or an existing installation in progress.\n' "$stage" >&2
  exit 1
fi

cleanup() {
  local status=$?
  if [[ -d "$backup" && ! -e "$app" && ! -L "$app" ]]; then
    if ! mv -- "$backup" "$app"; then
      printf 'Could not restore the previous app from %s\n' "$backup" >&2
      return 1
    fi
  fi
  rm -rf -- "$stage"
  return "$status"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

package=$(cargo pkgid --locked -p lockbox)
version=${package##*#}
target=$(rustc -vV | sed -n 's/^host: //p')
cargo build --locked --release --target-dir "$root/target" --bin lockbox --bin lockbox-browser-host
RELEASE_VERSION="$version" RELEASE_TARGET="$target" RELEASE_OUTPUT_DIR="$stage" \
  bash .github/scripts/package-native.sh

ditto -x -k "$stage/lockbox-$version-$target.zip" "$stage"
new_app="$stage/Lockbox.app"
xattr -dr com.apple.quarantine "$new_app"
codesign --verify --deep --strict "$new_app"

if ps -ax -o comm= | grep -Fx -- "$app/Contents/MacOS/lockbox" >/dev/null; then
  printf 'Quit Lockbox before replacing %s.\n' "$app" >&2
  exit 1
fi

if [[ -L "$app" || ( -e "$app" && ! -d "$app" ) ]]; then
  printf 'Expected an app bundle directory at %s.\n' "$app" >&2
  exit 1
fi
if [[ -d "$app" ]]; then
  mkdir -- "$stage/previous"
  mv -- "$app" "$backup"
fi
mv -- "$new_app" "$app"
printf 'Installed %s\n' "$app"
