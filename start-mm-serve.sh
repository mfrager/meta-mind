#!/usr/bin/env bash
# Start the kernel HTTP daemon (mm-serve) on 127.0.0.1:8899.
# Extra arguments are passed through, for example:
# ./start-mm-serve.sh --data-dir /tmp/mm-console-data
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"
exec cargo run -p mm-serve -- "$@"
