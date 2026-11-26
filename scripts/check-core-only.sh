#!/usr/bin/env bash
# Fail when the workspace grows beyond the six kept crates, a kept source
# imports a retired crate, or the kept frontend references a retired surface.
set -euo pipefail

actual=$(cargo metadata --no-deps --format-version 1 \
  | python3 -c 'import json,sys; print("\n".join(sorted(p["name"] for p in json.load(sys.stdin)["packages"])))')
expected=$(printf '%s\n' \
  mm-agentstream mm-cme mm-core mm-llm mm-prompt mm-serve | sort)

if [ "$actual" != "$expected" ]; then
  echo "unexpected workspace members:"
  diff <(echo "$expected") <(echo "$actual") || true
  exit 1
fi

archived='mm_agent\b|mm_being|mm_cli|mm_codex|mm_data|mm_decision|mm_epistemic|mm_eventlog|mm_firewall|mm_library|mm_log|mm_memory|mm_metaanalysis|mm_metacog|mm_mistakes|mm_pi|mm_process|mm_runtime|mm_search|mm_selfeng|mm_sm|mm_store_graph|mm_store_sqlite|mm_tools|mm_llm_agentstream'
if rg -n "($archived)" crates --glob '*.rs' --glob '!target'; then
  echo "a kept crate references an archived crate" >&2
  exit 1
fi

retired_frontend='llm_feed|llm_activity|/api/llm-activity|@/agentgui|@/components/(inspector|trees|mindmap|docs|files|kit|sessions|state)|@/lib/store/(llm-log|inspector|status)|@/lib/api/(llm-activity|inspector|trees|data|files|focus|state)|@/lib/wire/(chat|inspector|focus|llm_activity)'
if rg -n "($retired_frontend)" frontend/src --glob '*.{ts,tsx}'; then
  echo "a kept frontend source references a retired surface" >&2
  exit 1
fi

echo "core-only guard: ok (6 crates, no archived imports, no retired frontend surfaces)"
