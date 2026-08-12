#!/usr/bin/env bash
set -euo pipefail

root="/tmp/plaque-forge-python"
repo="$(cd "$(dirname "$0")/.." && pwd)"
reinstall=false
verify_only=false

usage() {
  printf 'usage: %s [--reinstall | --verify]\n' "$0" >&2
}

[[ "$root" == /tmp/plaque-forge-python ]] || {
  printf 'refusing unexpected segmentation root: %s\n' "$root" >&2
  exit 1
}

while (( $# )); do
  case "$1" in
    --reinstall) reinstall=true ;;
    --verify) verify_only=true ;;
    -h|--help) usage; exit 0 ;;
    *) usage; exit 2 ;;
  esac
  shift
done

if [[ "$reinstall" == true && "$verify_only" == true ]]; then
  printf '%s\n' '--reinstall and --verify are mutually exclusive' >&2
  exit 2
fi

# Keep every Python/model/cache side effect in /tmp. This function is also used
# by --verify so verification never falls back to the real HOME/XDG caches.
configure_runtime_env() {
  export PLAQUE_FORGE_PYTHON_ROOT="$root"
  export HOME="$root/home"
  export XDG_CACHE_HOME="$root/cache/xdg"
  export XDG_CONFIG_HOME="$root/config"
  export XDG_DATA_HOME="$root/data"
  export UV_CACHE_DIR="$root/cache/uv"
  export UV_PYTHON_INSTALL_DIR="$root/python"
  export UV_PYTHON_BIN_DIR="$root/bin"
  export PIP_CACHE_DIR="$root/cache/pip"
  export HF_HOME="$root/cache/huggingface"
  export TORCH_HOME="$root/cache/torch"
  export MPLCONFIGDIR="$root/cache/matplotlib"
  export TRITON_CACHE_DIR="$root/cache/triton"
  export TORCHINDUCTOR_CACHE_DIR="$root/cache/torchinductor"
  export TORCH_EXTENSIONS_DIR="$root/cache/torch-extensions"
  export PYTHONPYCACHEPREFIX="$root/cache/pycache"
  export TMPDIR="$root/tmp"
}

verify_runtime() {
  local python="$root/venv/bin/python"
  [[ -x "$python" ]] || {
    printf 'Python runtime is not installed: %s\n' "$python" >&2
    return 1
  }
  configure_runtime_env
  mkdir -p "$TMPDIR"
  printf '[setup] verifying ML runtime (offline)\n' >&2
  HF_HUB_OFFLINE=1 TRANSFORMERS_OFFLINE=1 \
    "$python" "$repo/tools/segmentation_worker.py" --verify-runtime
}

if [[ "$verify_only" == true ]]; then
  [[ -f "$root/.complete" ]] || {
    printf 'segmentation runtime is incomplete: %s\n' "$root" >&2
    printf 'run %s --reinstall\n' "$0" >&2
    exit 1
  }
  verify_runtime
  exit 0
fi

if [[ "$reinstall" == false && -f "$root/.complete" && -x "$root/venv/bin/python" ]]; then
  printf 'already installed: %s\n' "$root"
  printf 'verify with: %s --verify\n' "$0"
  exit 0
fi

for command in uv git; do
  command -v "$command" >/dev/null || {
    printf 'required command not found: %s\n' "$command" >&2
    exit 1
  }
done

if [[ -e "$root" ]]; then
  if [[ "$reinstall" == false ]]; then
    printf 'incomplete Python environment: %s\nrerun with --reinstall to delete and replace it\n' "$root" >&2
    exit 1
  fi
  printf 'deleting Python environment for explicit reinstall: %s\n' "$root" >&2
  rm -rf -- "$root"
fi

mkdir -p "$root"/{bin,cache,config,data,home,python,src,tmp}
configure_runtime_env

uv python install 3.10
uv venv --python 3.10 --seed "$root/venv"
python="$root/venv/bin/python"
uv pip install --python "$python" torch==2.13.0 torchvision==0.28.0 \
  --index-url https://download.pytorch.org/whl/xpu
uv pip install --python "$python" -r "$repo/tools/segmentation-requirements.txt"

git clone https://github.com/facebookresearch/sam2.git "$root/src/sam2"
git -C "$root/src/sam2" checkout 2b90b9f5ceec907a1c18123530e92e794ad901a4
# SAM2's optional _C module is a CUDA extension. Plaque Forge supports Intel XPU,
# so deliberately omit it and provide backend-neutral small-mask cleanup in the worker.
SAM2_BUILD_CUDA=0 uv pip install --python "$python" --no-deps --no-build-isolation -e "$root/src/sam2"

git clone https://github.com/hkchengrex/Cutie.git "$root/src/Cutie"
git -C "$root/src/Cutie" checkout ec5cdd4cf16f75c73ad785a2f96fb97dbad4125a
uv pip install --python "$python" --no-deps -e "$root/src/Cutie"

git clone https://github.com/pq-yang/MatAnyone2.git "$root/src/MatAnyone2"
git -C "$root/src/MatAnyone2" checkout 0079197acd6d16a741f71558809c06c586c579e0
uv pip install --python "$python" --no-deps -e "$root/src/MatAnyone2"

printf '[setup] downloading Cutie model and backbone weights\n' >&2
"$python" "$root/src/Cutie/cutie/utils/download_models.py"
# Cutie still uses torchvision's historical ResNet URLs internally. Prefetch the
# exact files it requests so analyze_assets.sh never discovers these downloads.
"$python" - <<'PY'
from pathlib import Path
import torch

checkpoints = Path(torch.hub.get_dir()) / "checkpoints"
checkpoints.mkdir(parents=True, exist_ok=True)
for url in (
    "https://download.pytorch.org/models/resnet18-5c106cde.pth",
    "https://download.pytorch.org/models/resnet50-19c8e357.pth",
):
    destination = checkpoints / url.rsplit("/", 1)[-1]
    if destination.is_file() and destination.stat().st_size > 0:
        print(f"[setup] cached: {destination}")
        continue
    print(f"[setup] downloading: {url}")
    hash_prefix = destination.stem.rsplit("-", 1)[-1]
    torch.hub.download_url_to_file(
        url, str(destination), hash_prefix=hash_prefix, progress=True
    )
PY

printf '[setup] downloading Hugging Face model snapshots\n' >&2
"$python" - <<'PY'
from huggingface_hub import snapshot_download

for repo_id in (
    "facebook/sam2.1-hiera-large",
    "hustvl/vitmatte-small-composition-1k",
    "PeiqingYang/MatAnyone2",
):
    snapshot_download(repo_id=repo_id)
PY

# Do not mark the environment complete until imports, caches, the selected
# accelerator, Cutie, SAM2 and ViTMatte survive an offline smoke test.
verify_runtime
touch "$root/.complete"
printf 'installed and verified: %s\n' "$root"
