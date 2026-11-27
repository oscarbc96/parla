#!/usr/bin/env bash
# Report the size of the stripped release binary and write it to
# target/perf/size.json for `perf:check`. Usage: perf-size.sh <binary>
set -euo pipefail

bin="${1:?usage: perf-size.sh <binary>}"
[ -f "$bin" ] || { echo "perf-size: $bin not found; build with cargo build --release -p parla" >&2; exit 1; }

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
cp "$bin" "$tmp/parla"
strip "$tmp/parla" 2>/dev/null || true
bytes="$(wc -c < "$tmp/parla" | tr -d ' ')"
mb="$(awk -v b="$bytes" 'BEGIN { printf "%.2f", b / 1048576 }')"

out="${PERF_OUT:-target/perf}"
mkdir -p "$out"
commit="$(git rev-parse --short HEAD 2>/dev/null || echo unknown)"
os="$(uname -s | tr '[:upper:]' '[:lower:]')"
machine="$(uname -m)"
cat > "$out/size.json" <<JSON
{"scenario":"size","commit":"$commit","os":"$os","machine":"$machine","seconds":0.0,"metrics":{"bin_mb":$mb}}
JSON
echo "stripped release binary: $mb MB ($bytes bytes)"
