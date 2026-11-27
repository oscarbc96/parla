#!/bin/bash
# Reads `call.parent` and `call.helper` by hand.
#
# Builds the release bundle like `mise run bundle`, launches it and prints one
# timestamped `ps -o rss=,pid=,comm=` line every 5 s for the Parla parent and
# every process it forks (the call helper and the webview's WebContent,
# Networking and GPU children), until Ctrl-C. The script seeds nothing: the app
# runs on the owner's own data, no synthetic perf workspace.
#
# The 5 s loop is fixed on purpose: this is a manual measurement tool, not app
# code, so it may sleep where the app itself never polls.
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"

if [ "$(uname -s)" != Darwin ]; then
  echo "perf:call reads RSS with ps on macOS only" >&2
  exit 1
fi

if pgrep -x parla >/dev/null 2>&1; then
  echo "Quit the running Parla first (one instance per user)." >&2
  exit 1
fi

# The bundle task also writes target/release-assets/NOTICE, which the packager needs.
mise run bundle

app=target/release/Parla.app/Contents/MacOS/parla
"$app" &
app_pid=$!
trap 'kill "$app_pid" 2>/dev/null || true' EXIT INT TERM

# The parent and every descendant, parents before children.
tree() {
  local pid=$1 child
  printf '%s\n' "$pid"
  for child in $(pgrep -P "$pid" 2>/dev/null || true); do
    tree "$child"
  done
}

echo "Parla pid $app_pid: join the huddle, leave after 5 min, sample 10 s more."
echo "RSS every 5 s (Ctrl-C to stop):"
while kill -0 "$app_pid" 2>/dev/null; do
  stamp=$(date +%H:%M:%S)
  pids=$(tree "$app_pid" | paste -sd, -)
  # A process may exit between pgrep and ps: an empty or failed read is fine.
  rows=$(ps -o rss=,pid=,comm= -p "$pids" 2>/dev/null || true)
  while read -r rss pid comm; do
    [ -n "${pid:-}" ] || continue
    role=child
    if [ "$pid" = "$app_pid" ]; then role=parent; fi
    printf '%s %s rss_kb=%s pid=%s %s\n' "$stamp" "$role" "$rss" "$pid" "$comm"
  done <<< "$rows"
  sleep 5
done
