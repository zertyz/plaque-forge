#!/usr/bin/env bash
set -euo pipefail

root="${PLAQUE_FORGE_SAM31_ROOT:-/tmp/plaque-forge-sam31}"
repo="$(cd "$(dirname "$0")/.." && pwd)"
ref="${PLAQUE_FORGE_SAM31_REF:-main}"
reinstall=false
verify_only=false

usage() {
  cat >&2 <<'USAGE'
usage: ./scripts/setup_sam31.sh [--reinstall | --verify] [--ref GIT_REF]

Installs the optional SAM 3.1 CUDA runtime under /tmp/plaque-forge-sam31.
SAM 3.1 model access is gated by Meta/Hugging Face; authenticate with `hf auth login`
or provide HF_TOKEN before the initial install.

This runtime is intentionally separate from setup_segmentation.sh because the official
SAM 3.1 stack currently requires Python >=3.12 and a CUDA-compatible GPU.
USAGE
}

while (( $# )); do
  case "$1" in
    --reinstall) reinstall=true ;;
    --verify) verify_only=true ;;
    --ref)
      (( $# >= 2 )) || { usage; exit 2; }
      ref="$2"; shift ;;
    -h|--help) usage; exit 0 ;;
    *) usage; exit 2 ;;
  esac
  shift
done

[[ "$root" == /tmp/plaque-forge-sam31 || -n "${PLAQUE_FORGE_ALLOW_CUSTOM_SAM31_ROOT:-}" ]] || {
  printf 'refusing unexpected SAM 3.1 root: %s\n' "$root" >&2
  exit 1
}

configure_env() {
  export PLAQUE_FORGE_SAM31_ROOT="$root"
  export HF_HOME="$root/cache/huggingface"
  export TORCH_HOME="$root/cache/torch"
  export UV_CACHE_DIR="$root/cache/uv"
  export UV_PYTHON_INSTALL_DIR="$root/python"
  export UV_PYTHON_BIN_DIR="$root/bin"
  export PIP_CACHE_DIR="$root/cache/pip"
  export TMPDIR="$root/tmp"
  mkdir -p "$HF_HOME" "$TORCH_HOME" "$UV_CACHE_DIR" "$TMPDIR"
}

verify_runtime() {
  configure_env
  local python="$root/venv/bin/python"
  [[ -x "$python" ]] || { printf 'SAM 3.1 Python runtime missing\n' >&2; return 1; }
  HF_HUB_OFFLINE=1 TRANSFORMERS_OFFLINE=1 "$python" - <<'PY'
import json
import os
from pathlib import Path
import torch
from huggingface_hub import snapshot_download

root = Path(os.environ["PLAQUE_FORGE_SAM31_ROOT"])
if not torch.cuda.is_available():
    raise SystemExit("SAM 3.1 verification requires a CUDA device; official CPU/XPU fallback is not assumed")
path = Path(snapshot_download("facebook/sam3.1", local_files_only=True))
checkpoint = path / "sam3.1_multiplex.pt"
if not checkpoint.is_file():
    matches = sorted(path.glob("*.pt"))
    if len(matches) == 1:
        checkpoint = matches[0]
    else:
        raise SystemExit(f"cannot identify SAM 3.1 multiplex checkpoint in {path}")
manifest = root / "runtime-manifest.json"
if not manifest.is_file():
    raise SystemExit(f"runtime manifest missing: {manifest}")
from sam3.model_builder import build_sam3_multiplex_video_predictor
try:
    predictor = build_sam3_multiplex_video_predictor(
        checkpoint_path=str(checkpoint),
        use_fa3=False,
        compile=False,
    )
except Exception as error:
    raise SystemExit(
        "SAM 3.1 package/checkpoint smoke failed. The backend remains unavailable; "
        f"this may be an upstream public-code/checkpoint compatibility issue: {error}"
    ) from error
del predictor
torch.cuda.empty_cache()
print(f"[sam3.1] runtime OK: torch={torch.__version__}, device={torch.cuda.get_device_name(0)}, checkpoint={checkpoint}")
PY
}

write_manifest() {
  configure_env
  "$root/venv/bin/python" - "$root" "$repo" <<'PY'
import hashlib
import importlib.metadata
import json
import os
import subprocess
import sys
from pathlib import Path
from huggingface_hub import snapshot_download

root, repo = map(Path, sys.argv[1:])
snapshot = Path(snapshot_download("facebook/sam3.1", local_files_only=True)).resolve()
checkpoint = snapshot / "sam3.1_multiplex.pt"
if not checkpoint.is_file():
    matches = sorted(snapshot.glob("*.pt"))
    if len(matches) != 1:
        raise SystemExit(f"cannot identify SAM 3.1 checkpoint in {snapshot}")
    checkpoint = matches[0]
commit = subprocess.check_output(["git", "-C", str(root / "src" / "sam3"), "rev-parse", "HEAD"], text=True).strip()

def sha(path):
    d = hashlib.sha256()
    with path.open("rb") as f:
        for b in iter(lambda: f.read(1024 * 1024), b""):
            d.update(b)
    return d.hexdigest()

doc = {
    "schema_version": 1,
    "source_commit": commit,
    "model_repo": "facebook/sam3.1",
    "model_snapshot": snapshot.name,
    "checkpoint_relative": str(checkpoint.relative_to(root)),
    "checkpoint_sha256": sha(checkpoint),
    "python": ".".join(map(str, sys.version_info[:3])),
    "torch": importlib.metadata.version("torch"),
    "cuda": __import__("torch").version.cuda,
    "implementation_sha256": {
        "sam31_worker.py": sha(repo / "tools" / "sam31_worker.py"),
        "setup_sam31.sh": sha(repo / "scripts" / "setup_sam31.sh"),
    },
}
(root / "runtime-manifest.json").write_text(json.dumps(doc, indent=2, sort_keys=True) + "\n")
PY
}

configure_env
if [[ "$verify_only" == true ]]; then
  verify_runtime
  exit 0
fi
if [[ "$reinstall" == true ]]; then
  rm -rf -- "$root"
  configure_env
fi

command -v uv >/dev/null || {
  printf 'uv is required; install uv first\n' >&2
  exit 1
}
command -v git >/dev/null || { printf 'git is required\n' >&2; exit 1; }

if [[ ! -x "$root/venv/bin/python" ]]; then
  uv python install 3.12
  uv venv --python 3.12 "$root/venv"
fi

python="$root/venv/bin/python"
uv pip install --python "$python" --index-url https://download.pytorch.org/whl/cu128 \
  'torch==2.10.0' torchvision

if [[ ! -d "$root/src/sam3/.git" ]]; then
  mkdir -p "$root/src"
  git clone https://github.com/facebookresearch/sam3.git "$root/src/sam3"
fi
git -C "$root/src/sam3" fetch --tags origin
if [[ "$ref" == main ]]; then
  git -C "$root/src/sam3" checkout --detach origin/main
else
  git -C "$root/src/sam3" checkout --detach "$ref"
fi
uv pip install --python "$python" -e "$root/src/sam3"

printf '[sam3.1] downloading/confirming gated checkpoint snapshot\n' >&2
"$python" - <<'PY'
from huggingface_hub import snapshot_download
snapshot_download("facebook/sam3.1")
PY

write_manifest
verify_runtime
printf '[sam3.1] installed at %s\n' "$root" >&2
