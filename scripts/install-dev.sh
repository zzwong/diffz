#!/usr/bin/env bash
# Installs or removes a development build beside the release package; `make install-dev` and
# `make uninstall-dev` call this. It writes only diffz-dev and io.github.zzwong.Diffz.Dev files
# under PREFIX and DATADIR, never the release package's diffz or io.github.zzwong.Diffz files.
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
  for file in "$bin" "$desktop" "$icon"; do
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
