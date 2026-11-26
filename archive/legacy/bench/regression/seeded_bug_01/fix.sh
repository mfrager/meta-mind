#!/bin/sh
# The fix: an unreadable stream is an error, and only a readable empty stream is zero.
set -eu
dir=$(dirname "$0")
cat > "$dir/subject.sh" <<'SUBJECT'
#!/bin/sh
# subject.sh — count the records in a JSONL stream.
# An absent or unreadable file is an error rather than a count of zero.
set -u
file=$1
if [ ! -r "$file" ]; then
  echo "cannot read $file" >&2
  exit 2
fi
count=$(grep -c . "$file" || true)
if [ "$count" -eq 0 ] && [ -s "$file" ]; then
  echo "no record parsed from a non-empty stream" >&2
  exit 3
fi
echo "records $count"
exit 0
SUBJECT
chmod +x "$dir/subject.sh"
