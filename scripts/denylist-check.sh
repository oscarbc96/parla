#!/bin/sh
# Pre-push confidentiality check: fails when a commit message or the diff of the
# pushed branch contains a term from a private list kept outside the repo.
#
#   scripts/denylist-check.sh <base-ref> <head-sha>
#
# The list is $PARLA_DENYLIST or ~/.config/parla/denylist.txt: one term per line,
# blank lines and lines starting with # ignored, matched case-insensitively as
# plain substrings. Output is counts only, never the matched text. A missing
# list skips the check with a notice.
set -eu

base=${1:?usage: denylist-check.sh <base-ref> <head-sha>}
head=${2:?usage: denylist-check.sh <base-ref> <head-sha>}
list=${PARLA_DENYLIST:-$HOME/.config/parla/denylist.txt}

if [ ! -f "$list" ]; then
  echo "denylist: no term list (set PARLA_DENYLIST or create ~/.config/parla/denylist.txt); skipped"
  exit 0
fi

terms=$(mktemp)
trap 'rm -f "$terms"' EXIT INT TERM
grep -v -e '^[[:space:]]*$' -e '^[[:space:]]*#' "$list" > "$terms" || true
if [ ! -s "$terms" ]; then
  echo "denylist: the term list is empty; skipped"
  exit 0
fi

msgs=$(git log --format=%B "$base..$head" | grep -c -i -F -f "$terms" || true)
names=$(git diff --name-only "$base...$head" | grep -c -i -F -f "$terms" || true)
diff=$(git diff -U0 "$base...$head" | grep '^+' | grep -c -i -F -f "$terms" || true)

if [ "$msgs" -eq 0 ] && [ "$names" -eq 0 ] && [ "$diff" -eq 0 ]; then
  echo "denylist: clean"
  exit 0
fi
echo "denylist: $msgs commit message line(s), $names path(s), $diff added line(s) match the private term list"
echo "denylist: find them with your own list (for example git log -p $base..$head and a case-insensitive fixed-string search); matched text is never printed here"
exit 1
