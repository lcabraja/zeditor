#!/usr/bin/env bash
set -euo pipefail

# Alfred passes the keyword argument as $1 (for example: {query}).
input="${1-}"

if command -v zeditor >/dev/null 2>&1; then
  if [[ -n "$input" ]]; then
    exec zeditor "$input"
  fi
  exec zeditor
fi

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
local_bin="$repo_root/target/release/zeditor"
app_bin="/Applications/Zeditor.app/Contents/MacOS/zeditor"
user_app_bin="$HOME/Applications/Zeditor.app/Contents/MacOS/zeditor"

if [[ -x "$local_bin" ]]; then
  if [[ -n "$input" ]]; then
    exec "$local_bin" "$input"
  fi
  exec "$local_bin"
fi

for candidate in "$user_app_bin" "$app_bin"; do
  if [[ -x "$candidate" ]]; then
    if [[ -n "$input" ]]; then
      exec "$candidate" "$input"
    fi
    exec "$candidate"
  fi
done

echo "zeditor not found in PATH, target/release, ~/Applications, or /Applications" >&2
exit 1
