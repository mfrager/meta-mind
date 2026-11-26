#!/usr/bin/env bash
# Start the web console on 0.0.0.0:8900. It proxies /api to mm-serve on
# 127.0.0.1:8899. Start ./start-mm-serve.sh first.
#
# The console is one workspace: session picker, transcript, composer, and the
# Agentstream dock with the full prompt/tools viewer.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/frontend"
exec npm run dev -- "$@"
