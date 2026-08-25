#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "$0")/common.sh"
cd "$PF_ROOT"
python3 tools/check_segmentation_capabilities.py --json output/segmentation-capability-coverage.json
