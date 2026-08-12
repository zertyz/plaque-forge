#!/usr/bin/env bash
set -euo pipefail

root="/tmp/plaque-forge-python"
repo="$(cd "$(dirname "$0")/.." && pwd)"
reinstall=false

[[ "$root" == /tmp/plaque-forge-python ]] || {
  printf 'refusing unexpected segmentation root: %s\n' "$root" >&2
  exit 1
}
for command in uv git; do
  command -v "$command" >/dev/null || {
    printf 'required command not found: %s\n' "$command" >&2
    exit 1
  }
done

if (( $# > 1 )) || { (( $# == 1 )) && [[ "$1" != --reinstall ]]; }; then
  printf 'usage: %s [--reinstall]\n' "$0" >&2
  exit 2
fi
if [[ "${1:-}" == --reinstall ]]; then
  reinstall=true
fi

if [[ "$reinstall" == false && -f "$root/.complete" && -x "$root/venv/bin/python" ]]; then
  printf 'already installed: %s\n' "$root"
  exit 0
fi

if [[ -e "$root" ]]; then
  if [[ "$reinstall" == false ]]; then
    printf 'incomplete Python environment: %s\nrerun with --reinstall to delete and replace it\n' "$root" >&2
    exit 1
  fi
  printf 'deleting Python environment for explicit reinstall: %s\n' "$root" >&2
  rm -rf -- "$root"
fi
mkdir -p "$root"/{bin,cache,config,data,home,python,src,tmp}
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

uv python install 3.10
uv venv --python 3.10 --seed "$root/venv"
python="$root/venv/bin/python"
uv pip install --python "$python" torch==2.13.0 torchvision==0.28.0 \
  --index-url https://download.pytorch.org/whl/xpu
uv pip install --python "$python" -r "$repo/tools/segmentation-requirements.txt"

git clone https://github.com/facebookresearch/sam2.git "$root/src/sam2"
git -C "$root/src/sam2" checkout 2b90b9f5ceec907a1c18123530e92e794ad901a4
SAM2_BUILD_CUDA=0 uv pip install --python "$python" --no-deps --no-build-isolation -e "$root/src/sam2"

git clone https://github.com/hkchengrex/Cutie.git "$root/src/Cutie"
git -C "$root/src/Cutie" checkout ec5cdd4cf16f75c73ad785a2f96fb97dbad4125a
uv pip install --python "$python" --no-deps -e "$root/src/Cutie"

git clone https://github.com/pq-yang/MatAnyone2.git "$root/src/MatAnyone2"
git -C "$root/src/MatAnyone2" checkout 0079197acd6d16a741f71558809c06c586c579e0
uv pip install --python "$python" --no-deps -e "$root/src/MatAnyone2"

"$python" "$root/src/Cutie/cutie/utils/download_models.py"
"$python" -c 'from huggingface_hub import snapshot_download as d; [d(repo_id=x) for x in ("facebook/sam2.1-hiera-large", "hustvl/vitmatte-small-composition-1k", "PeiqingYang/MatAnyone2")]'
touch "$root/.complete"
printf 'installed: %s\n' "$root"
