#!/usr/bin/env bash
# Offline Flows A–D against a fresh data directory (design/planning/ui/10 §8, §12).
# No language-model provider is called. The script boots mm-serve, checks the
# kernel's own sentences, and stops the process.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

if [[ "${1:-}" != "--offline" ]]; then
  echo "usage: ./scripts/ui_flows.sh --offline" >&2
  exit 2
fi

DATA="$(mktemp -d)"
PORT="${MM_UI_PORT:-8899}"
BASE="http://127.0.0.1:${PORT}"
PID=""

cleanup() {
  if [[ -n "$PID" ]] && kill -0 "$PID" 2>/dev/null; then
    kill "$PID" 2>/dev/null || true
    wait "$PID" 2>/dev/null || true
  fi
  rm -rf "$DATA"
}
trap cleanup EXIT

if ss -ltn | grep -Eq ":${PORT}([^0-9]|$)"; then
  echo "ui_flows: port ${PORT} is already in use" >&2
  exit 1
fi

cargo run -q -p mm-serve -- --data-dir "$DATA" --port "$PORT" >/tmp/mm-ui-flows.log 2>&1 &
PID=$!

ready=""
for _ in $(seq 1 240); do
  if curl -fsS "$BASE/api/health" >/dev/null 2>&1; then
    ready=1
    break
  fi
  if ! kill -0 "$PID" 2>/dev/null; then
    echo "ui_flows: mm-serve exited" >&2
    cat /tmp/mm-ui-flows.log >&2 || true
    exit 1
  fi
  sleep 0.5
done
if [[ -z "$ready" ]]; then
  echo "ui_flows: health did not become ready" >&2
  exit 1
fi

need() {
  local name="$1"
  local body="$2"
  local needle="$3"
  if ! grep -qF -- "$needle" <<<"$body"; then
    echo "ui_flows: ${name} missing: ${needle}" >&2
    echo "$body" >&2
    exit 1
  fi
}

health="$(curl -fsS "$BASE/api/health")"
need health "$health" '"state":"ok"'
if ! grep -q '"name":"graph","state":"ok"' <<<"$health" && ! grep -q 'RDF' <<<"$health"; then
  echo "ui_flows: health graph is neither open nor explained" >&2
  echo "$health" >&2
  exit 1
fi
if ! grep -q 'no language-model provider is configured' <<<"$health" && ! grep -q 'model=' <<<"$health"; then
  echo "ui_flows: health llm names neither a missing provider nor a model" >&2
  echo "$health" >&2
  exit 1
fi

runs="$(curl -fsS "$BASE/api/runs")"
need runs "$runs" "no run is recorded"

provenance="$(curl -fsS "$BASE/api/trees/provenance")"
need provenance "$provenance" "No events are recorded."

changesets="$(curl -fsS "$BASE/api/trees/changesets")"
need changesets "$changesets" "No change sets are recorded."

chat() {
  curl -fsS --max-time 10 -X POST "$BASE/api/chat" \
    -H 'content-type: application/json' \
    -d "$1"
}

need budget "$(chat '{"query":"budget show"}')" "meta_analysis"
need budget "$(chat '{"query":"budget show"}')" "Open the budgets tree"
need decide "$(chat '{"query":"decide whether to continue"}')" "no decision core is bound on this route"
need being "$(chat '{"query":"being goal add"}')" "the being store is opened with the RDF graph, and this route does not open that store"
need promote "$(chat '{"query":"promote"}')" "use the change-set flow"
need sandbox "$(chat '{"query":"sandbox run"}')" "no sandbox run was recorded for this request"
need audit "$(chat '{"query":"audit production-tree"}')" "no production audit was recorded for this data directory"
need stage "$(chat '{"query":"why is stage 6 slow?"}')" "No stage timings are recorded for this run."
need graph "$(chat '{"query":"graph"}')" "RDF"
need changes "$(chat '{"query":"changeset show"}')" "No change sets are recorded."

load_code="$(curl -s -o /tmp/mm-ui-load.json -w '%{http_code}' -X POST "$BASE/api/modules/01jmodule0000000000000000/load")"
if [[ "$load_code" != "409" ]]; then
  echo "ui_flows: module load status ${load_code}" >&2
  exit 1
fi
need "module load" "$(cat /tmp/mm-ui-load.json)" "use the change-set flow"

cs_code="$(curl -s -o /tmp/mm-ui-cs.json -w '%{http_code}' -X POST "$BASE/api/changesets" \
  -H 'content-type: application/json' \
  -d '{"docId":"01jdocprose000000000000000","reason":"keep the note a draft","summary":"note","path":"docs/console-note.md","content":"A note."}')"
if [[ "$cs_code" == "409" || "$cs_code" -ge 400 ]]; then
  echo "ui_flows: changeset create status ${cs_code}" >&2
  cat /tmp/mm-ui-cs.json >&2
  exit 1
fi

focus="$(curl -fsS "$BASE/api/focus/01jdocrun00000000000000000")"
need focus "$focus" "No stage timings are recorded for this run."

echo "ui_flows: offline flows ok"
