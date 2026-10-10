#!/bin/sh
# The test. It asserts the *fixed* behaviour on two inputs: an unreadable stream must be
# an error (the bug reported success), and a readable stream with records must count
# them (so the fix is not "make everything fail").
set -eu
dir=$(dirname "$0")
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

# 1. A stream that is not there must not be reported as success.
if "$dir/subject.sh" "$work/absent.jsonl" >/dev/null 2>&1; then
  echo "an unreadable stream was reported as success" >&2
  exit 1
fi

# 2. A readable stream with two records counts two.
printf '{"id":1}\n{"id":2}\n' > "$work/two.jsonl"
if ! out=$("$dir/subject.sh" "$work/two.jsonl"); then
  echo "a readable stream was refused" >&2
  exit 1
fi
case "$out" in
  "records 2") ;;
  *) echo "expected 'records 2', got '$out'" >&2; exit 1 ;;
esac
