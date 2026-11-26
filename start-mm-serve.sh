#!/usr/bin/env bash
# Start the turn server (mm-serve) on 127.0.0.1:8899 with the core config and
# data root `data/core/`. Extra arguments are passed through, for example:
# ./start-mm-serve.sh --data-dir /tmp/mm-core-data
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"
exec cargo run -p mm-serve -- --config config/server.toml "$@"
