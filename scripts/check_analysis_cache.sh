#!/usr/bin/env bash
# Read-only CI gate: fail while any bundled asset's analysis cache is stale,
# invalid, or missing, and state exactly how to regenerate it.
set -euo pipefail

source "$(dirname "$0")/common.sh"
cd "$PF_ROOT"

pf_build_release
exec target/release/plaque-forge check-analysis-cache --assets-dir assets
