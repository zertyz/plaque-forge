#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
python3 tools/check_segmentation_capabilities.py --json output/segmentation-capability-coverage.json
