#!/usr/bin/env bash
set -euo pipefail

root="/tmp/plaque-forge-python"
python="$root/venv/bin/python"
log="$root/worker-runs.jsonl"

if [[ ! -x "$python" ]]; then
  printf '[ml] runtime: NOT INSTALLED (%s)\n' "$root"
  printf 'Run ./scripts/setup_segmentation.sh to create the temporary ML runtime.\n'
  exit 1
fi

printf '[ml] runtime: %s\n' "$root"
printf '[ml] python: '
"$python" --version 2>&1
if command -v pgrep >/dev/null 2>&1; then
  running="$(pgrep -af 'segmentation_(worker|service).py' || true)"
  if [[ -n "$running" ]]; then
    printf '[ml] active worker/service process(es):\n%s\n' "$running"
  else
    printf '[ml] active worker/service processes: none\n'
  fi
fi

if [[ -f "$log" ]]; then
  printf '[ml] recent Python worker events (%s):\n' "$log"
  tail -n 20 "$log"
else
  printf '[ml] no Python worker run has been recorded since this /tmp runtime was created.\n'
fi
