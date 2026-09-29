#!/usr/bin/env bash
# Installs or removes a development build beside the release package; `make install-dev` and
# `make uninstall-dev` call this. It writes only diffz-dev and io.github.zzwong.Diffz.Dev files
# under PREFIX and DATADIR, never the release package's diffz or io.github.zzwong.Diffz files.
# On macOS it installs "Diffz Dev.app" into APPLICATIONS_DIR (default ~/Applications) instead of
# a desktop entry, and diffz-dev is a symlink to the executable inside it.
set -euo pipefail
cd "$(dirname "$0")/.."

usage() { echo 'Usage: PREFIX=DIR DATADIR=DIR DEV_STATE_DIR=DIR [DEV_BIN=FILE] scripts/install-dev.sh install|uninstall' >&2; exit 2; }
action="${1:-}"
case "$action" in install|uninstall) ;; *) usage ;; esac
for var in PREFIX DATADIR DEV_STATE_DIR; do
  value="${!var:-}"
  [[ "$value" == /* ]] || { echo "$var must be an absolute path, got '$value'" >&2; exit 2; }
  # The Exec key would need escaping for these, and Make cannot pass them reliably anyway.
  [[ "$value" != *[\"\`\$\\%]* && "$value" != *$'\n'* ]] ||
    { echo "$var must not contain quotes, backslashes, \$, % or newlines: '$value'" >&2; exit 2; }
done

app_id=io.github.zzwong.Diffz.Dev
bin="$PREFIX/bin/diffz-dev"
app="${APPLICATIONS_DIR:-$HOME/Applications}/Diffz Dev.app"
desktop="$DATADIR/applications/$app_id.desktop"
hicolor="$DATADIR/icons/hicolor"
icon="$hicolor/scalable/apps/$app_id.svg"

# Refresh caches only where one already exists; creating new ones in a user data directory
# would hide icons and handlers that other applications add later without refreshing them.
refresh_caches() {
  if command -v update-desktop-database >/dev/null && [[ -f "$DATADIR/applications/mimeinfo.cache" ]]; then
    update-desktop-database -q "$DATADIR/applications" || true
  fi
  if command -v gtk-update-icon-cache >/dev/null && [[ -f "$hicolor/icon-theme.cache" ]]; then
    gtk-update-icon-cache -q -t -f "$hicolor" || true
  fi
}

if [[ "$action" == uninstall ]]; then
  if [[ "$(uname -s)" == Darwin ]]; then
    files=("$bin")
    if [[ -e "$app" || -L "$app" ]]; then
      rm -rf -- "$app"
      echo "removed $app"
    fi
  else
    files=("$bin" "$desktop" "$icon")
  fi
  for file in "${files[@]}"; do
    if [[ -e "$file" || -L "$file" ]]; then
      rm -f -- "$file"
      echo "removed $file"
    fi
  done
  refresh_caches
  echo "Kept development state in $DEV_STATE_DIR; remove it with: rm -rf '$DEV_STATE_DIR'"
  exit 0
fi

src="${DEV_BIN:?DEV_BIN must name the diffz binary to install}"
[[ -x "$src" ]] || { echo "no executable diffz binary at $src" >&2; exit 1; }

if [[ "$(uname -s)" == Darwin ]]; then
  # The handoff socket lives in the state directory, and a socket path must fit in 104 bytes.
  (( ${#DEV_STATE_DIR} + 14 <= 104 )) ||
    { echo "DEV_STATE_DIR is too long for a handoff socket path: '$DEV_STATE_DIR'" >&2; exit 2; }
  work=target/install-dev
  mkdir -p "$work"
  stage="$work/Diffz Dev.app"
  rm -rf -- "$stage"
  python3 - "$src" "$stage" "$app_id" "$DEV_STATE_DIR" <<'PYTHON'
import pathlib, plistlib, shutil, subprocess, sys
src, stage, app_id, state = sys.argv[1:]
version = subprocess.check_output([src, "--version"], text=True).split()[-1]
contents = pathlib.Path(stage) / "Contents"
(contents / "MacOS").mkdir(parents=True)
shutil.copy2(src, contents / "MacOS" / "diffz")
resources = contents / "Resources"
resources.mkdir()
shutil.copy2("LICENSE", resources / "LICENSE")
shutil.copy2("THIRD_PARTY_NOTICES.md", resources / "THIRD_PARTY_NOTICES.md")
# Read at startup, so this bundle never shares the release app's handoff socket and lock.
(resources / "state-dir").write_text(state + "\n")
(contents / "Info.plist").write_bytes(plistlib.dumps({
    "CFBundleExecutable": "diffz",
    "CFBundleIdentifier": app_id,
    "CFBundleName": "Diffz Dev",
    "CFBundleDisplayName": "Diffz Dev",
    "CFBundlePackageType": "APPL",
    "CFBundleVersion": "1",
    "CFBundleShortVersionString": version,
    "LSMinimumSystemVersion": "15.0",
    "NSHighResolutionCapable": True,
}))
PYTHON
  codesign --force --deep --sign - "$stage"
  mkdir -p "$(dirname "$app")" "$(dirname "$bin")"
  rm -rf -- "$app"
  cp -R "$stage" "$app"
  ln -sfn "$app/Contents/MacOS/diffz" "$bin"
  printf 'installed %s\n' "$app" "$bin"
  echo "Diffz Dev.app and diffz-dev use state in $DEV_STATE_DIR"
  case ":$PATH:" in *":$PREFIX/bin:"*) ;; *) echo "note: $PREFIX/bin is not on PATH" ;; esac
  exit 0
fi

# Desktop Entry Exec quoting: plain arguments stay bare, others are wrapped in double quotes.
exec_arg() {
  if [[ "$1" =~ ^[A-Za-z0-9_./+,:@=-]+$ ]]; then printf '%s' "$1"; else printf '"%s"' "$1"; fi
}
exec_line="$(exec_arg "$bin") --state-dir $(exec_arg "$DEV_STATE_DIR") %f"

# Derive the entry from the release one so shared fields cannot drift. MimeType is dropped so
# the release package remains the handler for patch files.
work=target/install-dev
mkdir -p "$work"
generated="$work/$app_id.desktop"
awk -v exec_line="$exec_line" -v try_exec="$bin" -v icon="$app_id" '
  /^\[/ { group = $0; if (group != "[Desktop Entry]") { extra = group; exit } }
  group == "[Desktop Entry]" && /^Name=/ { print "Name=Diffz (dev)"; seen["Name"]++; next }
  group == "[Desktop Entry]" && /^Exec=/ { print "Exec=" exec_line; seen["Exec"]++; next }
  group == "[Desktop Entry]" && /^TryExec=/ { print "TryExec=" try_exec; seen["TryExec"]++; next }
  group == "[Desktop Entry]" && /^Icon=/ { print "Icon=" icon; seen["Icon"]++; next }
  group == "[Desktop Entry]" && /^MimeType=/ { next }
  { print }
  END {
    if (extra != "") { print "unexpected group " extra "; update scripts/install-dev.sh" > "/dev/stderr"; exit 1 }
    split("Name Exec TryExec Icon", keys, " ")
    for (i in keys) if (seen[keys[i]] != 1) {
      print "expected one " keys[i] "= key in the release entry; update scripts/install-dev.sh" > "/dev/stderr"; exit 1
    }
  }
' packaging/linux/io.github.zzwong.Diffz.desktop > "$generated"

if command -v desktop-file-validate >/dev/null; then
  desktop-file-validate "$generated"
else
  echo 'desktop-file-validate not found; skipped desktop entry validation' >&2
fi

install -Dm755 "$src" "$bin"
install -Dm644 "$generated" "$desktop"
install -Dm644 packaging/linux/icons/io.github.zzwong.Diffz.svg "$icon"
refresh_caches

printf 'installed %s\n' "$bin" "$desktop" "$icon"
echo "diffz-dev and its launcher use state in $DEV_STATE_DIR"
case ":$PATH:" in *":$PREFIX/bin:"*) ;; *) echo "note: $PREFIX/bin is not on PATH" ;; esac
