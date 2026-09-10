#!/usr/bin/env bash
set -euo pipefail

root="/tmp/plaque-forge-python"
source "$(dirname "$0")/common.sh"
repo="$PF_ROOT"
reinstall=false
verify_only=false
requested_torch_profile="${PLAQUE_FORGE_TORCH_PROFILE:-}"

usage() {
  cat >&2 <<'USAGE'
usage: ./scripts/setup_segmentation.sh [--reinstall | --verify] [--torch-profile xpu|cpu]

  --reinstall           Delete and rebuild /tmp/plaque-forge-python.
  --verify              Verify the completed runtime without network access.
  --torch-profile NAME  Install PyTorch for xpu (default) or cpu.

An interrupted/failed setup with all pinned sources already present is repaired in
place when possible; --reinstall is only required for an incompatible partial tree.
USAGE
}

[[ "$root" == /tmp/plaque-forge-python ]] || {
  printf 'refusing unexpected segmentation root: %s\n' "$root" >&2
  exit 1
}

while (( $# )); do
  case "$1" in
    --reinstall) reinstall=true ;;
    --verify) verify_only=true ;;
    --torch-profile)
      (( $# >= 2 )) || { usage; exit 2; }
      requested_torch_profile="$2"
      shift
      ;;
    -h|--help) usage; exit 0 ;;
    *) usage; exit 2 ;;
  esac
  shift
done

if [[ "$reinstall" == true && "$verify_only" == true ]]; then
  printf '%s\n' '--reinstall and --verify are mutually exclusive' >&2
  exit 2
fi
if [[ -n "$requested_torch_profile" && "$requested_torch_profile" != xpu && "$requested_torch_profile" != cpu ]]; then
  printf 'invalid --torch-profile: %s (expected xpu or cpu)\n' "$requested_torch_profile" >&2
  exit 2
fi

installed_torch_profile() {
  if [[ -s "$root/.torch-profile" ]]; then
    cat "$root/.torch-profile"
  else
    # Runtimes created before the profile marker existed always installed XPU wheels.
    printf 'xpu\n'
  fi
}

if [[ -n "$requested_torch_profile" ]]; then
  torch_profile="$requested_torch_profile"
elif [[ -e "$root" ]]; then
  torch_profile="$(installed_torch_profile)"
else
  torch_profile="xpu"
fi

# Keep every Python/model/cache side effect in /tmp. This function is also used
# by --verify so verification never falls back to user XDG/model caches.
configure_runtime_env() {
  export PLAQUE_FORGE_PYTHON_ROOT="$root"
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

ensure_requested_profile_matches() {
  [[ -e "$root" ]] || return 0
  local installed
  installed="$(installed_torch_profile)"
  if [[ -n "$requested_torch_profile" && "$installed" != "$requested_torch_profile" ]]; then
    printf 'segmentation runtime uses torch profile %s, requested %s\n' \
      "$installed" "$requested_torch_profile" >&2
    printf 'run %s --reinstall --torch-profile %s\n' "$0" "$requested_torch_profile" >&2
    return 1
  fi
}

verify_runtime() {
  local python="$root/venv/bin/python"
  [[ -x "$python" ]] || {
    printf 'Python runtime is not installed: %s\n' "$python" >&2
    return 1
  }
  configure_runtime_env
  mkdir -p "$TMPDIR"
  printf '[setup] verifying ML runtime (offline, torch-profile=%s)\n' "$torch_profile" >&2
  HF_HUB_OFFLINE=1 TRANSFORMERS_OFFLINE=1 \
    "$python" "$repo/tools/segmentation_worker.py" --verify-runtime
}

write_runtime_manifest() {
  local python="$root/venv/bin/python"
  "$python" - "$repo" "$root" "$torch_profile" <<'PY'
import hashlib
import importlib.metadata
import json
import subprocess
import sys
from pathlib import Path

repo = Path(sys.argv[1])
root = Path(sys.argv[2])
torch_profile = sys.argv[3]
sys.path.insert(0, str(repo / "tools"))
from segmentation_runtime import MODEL_REVISIONS


def sha256(path):
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def commit(name):
    return subprocess.check_output(
        ["git", "-C", str(root / "src" / name), "rev-parse", "HEAD"], text=True
    ).strip()


packages = {}
for name in (
    "torch",
    "torchvision",
    "transformers",
    "opencv-python-headless",
    "pillow",
    "numpy",
    "cutie",
    "matanyone2",
):
    try:
        packages[name] = importlib.metadata.version(name)
    except importlib.metadata.PackageNotFoundError:
        packages[name] = "editable-source"

document = {
    "schema_version": 1,
    "python": ".".join(map(str, sys.version_info[:3])),
    "torch_profile": torch_profile,
    "packages": packages,
    "source_commits": {
        "sam2": commit("sam2"),
        "cutie": commit("Cutie"),
        "matanyone2": commit("MatAnyone2"),
    },
    "model_revisions": MODEL_REVISIONS,
    "implementation_sha256": {
        path.name: sha256(path)
        for path in (
            repo / "tools" / "segmentation-worker",
            repo / "tools" / "segmentation_worker.py",
            repo / "tools" / "segmentation_service.py",
            repo / "tools" / "segmentation_runtime.py",
            repo / "tools" / "segmentation-requirements.txt",
            repo / "scripts" / "setup_segmentation.sh",
        )
    },
}
(root / "runtime-manifest.json").write_text(
    json.dumps(document, indent=2, sort_keys=True) + "\n", encoding="utf-8"
)
PY
}

download_model_snapshots() {
  local python="$root/venv/bin/python"
  printf '[setup] ensuring pinned Hugging Face model snapshots\n' >&2
  "$python" - "$repo" <<'PY'
import sys
from pathlib import Path
from huggingface_hub import snapshot_download

repo = Path(sys.argv[1])
sys.path.insert(0, str(repo / "tools"))
from segmentation_runtime import MODEL_REVISIONS

for repo_id, revision in MODEL_REVISIONS.items():
    snapshot_download(repo_id=repo_id, revision=revision)
PY
}

model_snapshots_current() {
  local python="$root/venv/bin/python"
  configure_runtime_env
  HF_HUB_OFFLINE=1 "$python" - "$repo" <<'PY'
import sys
from pathlib import Path
from huggingface_hub import snapshot_download

repo = Path(sys.argv[1])
sys.path.insert(0, str(repo / "tools"))
from segmentation_runtime import MODEL_REVISIONS

for repo_id, revision in MODEL_REVISIONS.items():
    snapshot_download(repo_id=repo_id, revision=revision, local_files_only=True)
PY
}

source_checkouts_current() {
  command -v git >/dev/null || return 1
  [[ -d "$root/src/sam2/.git" && -d "$root/src/Cutie/.git" && -d "$root/src/MatAnyone2/.git" ]] || return 1
  [[ "$(git -C "$root/src/sam2" rev-parse HEAD 2>/dev/null)" == 2b90b9f5ceec907a1c18123530e92e794ad901a4 ]] || return 1
  [[ "$(git -C "$root/src/Cutie" rev-parse HEAD 2>/dev/null)" == ec5cdd4cf16f75c73ad785a2f96fb97dbad4125a ]] || return 1
  [[ "$(git -C "$root/src/MatAnyone2" rev-parse HEAD 2>/dev/null)" == 0079197acd6d16a741f71558809c06c586c579e0 ]] || return 1
}

complete_runtime() {
  printf '%s\n' "$torch_profile" > "$root/.torch-profile"
  write_runtime_manifest
  verify_runtime
  touch "$root/.complete"
}

if [[ "$reinstall" == false ]]; then
  ensure_requested_profile_matches
fi

if [[ "$verify_only" == true ]]; then
  [[ -f "$root/.complete" ]] || {
    printf 'segmentation runtime is incomplete: %s\n' "$root" >&2
    printf 'run %s to repair it in place, or %s --reinstall\n' "$0" "$0" >&2
    exit 1
  }
  source_checkouts_current || {
    printf 'segmentation source checkout pins changed; run %s --reinstall\n' "$0" >&2
    exit 1
  }
  model_snapshots_current || {
    printf 'one or more pinned model snapshots are absent; run %s without --verify to repair them\n' "$0" >&2
    exit 1
  }
  write_runtime_manifest
  verify_runtime
  exit 0
fi

if [[ "$reinstall" == false && -f "$root/.complete" && -x "$root/venv/bin/python" ]]; then
  source_checkouts_current || {
    printf 'segmentation source checkout pins changed; run %s --reinstall\n' "$0" >&2
    exit 1
  }
  # Completed runtimes are normally network-free. Fetch only when the exact new
  # pinned model revision is not already present in the private cache.
  if ! model_snapshots_current; then
    download_model_snapshots
  fi
  write_runtime_manifest
  verify_runtime
  printf 'installed runtime is current and verified: %s\n' "$root"
  exit 0
fi

if [[ "$reinstall" == false && -e "$root" ]]; then
  # setup may have been interrupted after gigabytes of models were downloaded.
  # Salvage a structurally complete pinned tree before asking the user to erase it.
  if [[ -x "$root/venv/bin/python" ]] && source_checkouts_current; then
    configure_runtime_env
    mkdir -p "$TMPDIR"
    printf '[setup] attempting in-place repair of incomplete runtime: %s\n' "$root" >&2
    printf '%s\n' "$torch_profile" > "$root/.torch-profile"
    if model_snapshots_current; then
      write_runtime_manifest
      if verify_runtime; then
        touch "$root/.complete"
        printf 'repaired and verified without downloads: %s\n' "$root"
        exit 0
      fi
    else
      printf '[setup] incomplete runtime is missing pinned model cache entries; repairing them\n' >&2
      download_model_snapshots
    fi
    complete_runtime
    printf 'repaired and verified: %s\n' "$root"
    exit 0
  fi
  printf 'incomplete Python environment cannot be repaired safely: %s\n' "$root" >&2
  printf 'rerun with --reinstall to delete and replace it\n' >&2
  exit 1
fi

for command in uv git; do
  command -v "$command" >/dev/null || {
    printf 'required command not found: %s\n' "$command" >&2
    exit 1
  }
done

if [[ -e "$root" ]]; then
  printf 'deleting Python environment for explicit reinstall: %s\n' "$root" >&2
  rm -rf -- "$root"
fi

mkdir -p "$root"/{bin,cache,config,data,python,src,tmp}
configure_runtime_env
printf '%s\n' "$torch_profile" > "$root/.torch-profile"

uv python install 3.10
uv venv --python 3.10 --seed "$root/venv"
python="$root/venv/bin/python"
case "$torch_profile" in
  xpu) torch_index='https://download.pytorch.org/whl/xpu' ;;
  cpu) torch_index='https://download.pytorch.org/whl/cpu' ;;
esac
uv pip install --python "$python" torch==2.13.0 torchvision==0.28.0 \
  --index-url "$torch_index"
uv pip install --python "$python" -r "$repo/tools/segmentation-requirements.txt"

git clone https://github.com/facebookresearch/sam2.git "$root/src/sam2"
git -C "$root/src/sam2" checkout 2b90b9f5ceec907a1c18123530e92e794ad901a4
[[ "$(git -C "$root/src/sam2" rev-parse HEAD)" == 2b90b9f5ceec907a1c18123530e92e794ad901a4 ]]
# SAM2's optional _C module is a CUDA extension. Plaque Forge supports Intel XPU
# and CPU, so deliberately omit it and use backend-neutral cleanup in the worker.
SAM2_BUILD_CUDA=0 uv pip install --python "$python" --no-deps --no-build-isolation -e "$root/src/sam2"

git clone https://github.com/hkchengrex/Cutie.git "$root/src/Cutie"
git -C "$root/src/Cutie" checkout ec5cdd4cf16f75c73ad785a2f96fb97dbad4125a
[[ "$(git -C "$root/src/Cutie" rev-parse HEAD)" == ec5cdd4cf16f75c73ad785a2f96fb97dbad4125a ]]
uv pip install --python "$python" --no-deps -e "$root/src/Cutie"

git clone https://github.com/pq-yang/MatAnyone2.git "$root/src/MatAnyone2"
git -C "$root/src/MatAnyone2" checkout 0079197acd6d16a741f71558809c06c586c579e0
[[ "$(git -C "$root/src/MatAnyone2" rev-parse HEAD)" == 0079197acd6d16a741f71558809c06c586c579e0 ]]
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

download_model_snapshots

# Do not mark the environment complete until imports, caches, the selected
# accelerator, Cutie, SAM2 and ViTMatte survive an offline smoke test.
complete_runtime
printf 'installed and verified: %s\n' "$root"
