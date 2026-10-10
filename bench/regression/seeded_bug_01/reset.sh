#!/bin/sh
# Restore the bug: a counter that cannot tell "the stream was empty" from "I could not
# read the stream", and reports success either way.
set -eu
dir=$(dirname "$0")
cat > "$dir/subject.sh" <<'SUBJECT'
#!/bin/sh
# subject.sh — count the records in a JSONL stream.
# BUG: an unreadable stream is counted as zero records and reported as success.
set -u
file=$1
count=$(grep -c . "$file" 2>/dev/null || true)
echo "records $count"
exit 0
SUBJECT
chmod +x "$dir/subject.sh"
