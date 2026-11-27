#!/bin/sh
# Runs one perf scenario on the release build: perf-measure.sh <scenario> [--seconds <n>].
# On macOS a window that another app covers draws no frames, and the OS
# refuses to raise an app that the app itself started from a background
# process. So this script, started from the user's terminal, keeps asking
# System Events to put the scenario's process in front while it runs.
set -u

raise_loop() {
  # The window shows after the process starts and other apps take the focus
  # back, so ask every second; the call does nothing once it is in front.
  while :; do
    for p in $(pgrep -x parla); do
      case $(ps -o args= -p "$p") in *--perf-scenario*) ;; *) continue ;; esac
      osascript \
        -e 'tell application "System Events"' \
        -e "set pr to first process whose unix id = $p" \
        -e 'if not (frontmost of pr) then set frontmost of pr to true' \
        -e 'end tell' >/dev/null 2>&1
    done
    sleep 1
  done
}

# Idle runs measure what a covered window costs: leave them alone.
case ${1:-} in
  idle*|soak|start*|warm) raise=0 ;;
  *) raise=1 ;;
esac

if [ "$(uname -s)" = Darwin ] && [ "$raise" = 1 ]; then
  raise_loop &
  loop=$!
  trap 'kill "$loop" 2>/dev/null' EXIT INT TERM
fi
cargo run --release -p parla --features fixture -- --perf-scenario "$@"
