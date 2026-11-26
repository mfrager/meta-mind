#!/bin/sh
# subject.sh — count the records in a JSONL stream.
# BUG: an unreadable stream is counted as zero records and reported as success.
set -u
file=$1
count=$(grep -c . "$file" 2>/dev/null || true)
echo "records $count"
exit 0
