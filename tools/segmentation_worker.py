#!/usr/bin/env python3
import argparse
import concurrent.futures
import gc
import hashlib
import fcntl
import importlib.metadata
import json
import math
import os
import resource
import shutil
import subprocess
import sys
import threading
import time
import tempfile
import warnings
from contextlib import nullcontext
from pathlib import Path

try:
    import cv2
except ImportError:
    cv2 = None

try:
    import numpy as np
except ImportError:
    np = None

try:
    from PIL import Image, ImageDraw
except ImportError:
    Image = None
    ImageDraw = None

from segmentation_runtime import (
    MODEL_REVISIONS,
    load_sam2_video_predictor,
    model_revision,
)

WORKER_PROTOCOL = "plaque-forge.segmentation-request/2"
RESULT_PROTOCOL = "plaque-forge.segmentation-result/2"
FRAME_CACHE_FORMAT = "plaque-forge.frame-cache/1"
STAGE_METRICS = []
# Process-local heavyweight model cache. It becomes useful when the wrapper routes
# requests through the persistent segmentation service. Keys include device and all
# arithmetic/compile choices that can change executable state.
MODEL_CACHE = {}
_WARP_MESHGRID_CACHE = {}
_DIS_FLOW_LOCAL = threading.local()


# Upstream dependencies emit several warnings that are expected in Plaque Forge's
# Intel-XPU/CPU runtime. Replace those dependency implementation details with our
# own explicit status messages instead of dumping traceback-like noise at users.
warnings.filterwarnings(
    "ignore",
    message=r"cannot import name '_C' from 'sam2'.*",
    category=UserWarning,
)
warnings.filterwarnings(
    "ignore",
    message=r"`torch\.cuda\.amp\.autocast\(args\.\.\.\)` is deprecated.*",
    category=FutureWarning,
)
warnings.filterwarnings(
    "ignore",
    message=r"`self\.size_divisibility` attribute is deprecated.*",
)


def runtime_log(event, **fields):
    root = os.environ.get("PLAQUE_FORGE_PYTHON_ROOT")
    if not root:
        return
    path = Path(root) / "worker-runs.jsonl"
    record = {
        "time": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "event": event,
        "pid": os.getpid(),
        **fields,
    }
    try:
        path.parent.mkdir(parents=True, exist_ok=True)
        with path.open("a", encoding="utf-8") as handle:
            handle.write(json.dumps(record, sort_keys=True) + "\n")
    except OSError:
        # Runtime observability must never make segmentation itself fail.
        pass


def package_version(*names):
    for name in names:
        try:
            return importlib.metadata.version(name)
        except importlib.metadata.PackageNotFoundError:
            pass
    return "unknown"


def file_sha256(path):
    digest = hashlib.sha256()
    with Path(path).open("rb") as handle:
        for block in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def usable_cpu_count():
    """Return CPUs this process can actually schedule on, not host CPU inventory."""
    try:
        affinity = os.sched_getaffinity(0)
        if affinity:
            return len(affinity)
    except (AttributeError, OSError):
        pass

    process_cpu_count = getattr(os, "process_cpu_count", None)
    if process_cpu_count is not None:
        try:
            count = process_cpu_count()
            if count:
                return max(1, int(count))
        except OSError:
            pass

    return max(1, int(os.cpu_count() or 1))


def parallel_worker_count(item_count, *, cpus_per_worker=1):
    """Bound outer parallelism by the CPUs available to this process."""
    if item_count <= 1:
        return 1
    cpus_per_worker = max(1, int(cpus_per_worker))
    return max(1, min(int(item_count), usable_cpu_count() // cpus_per_worker))


def opencv_parallel_worker_count(item_count):
    """Avoid multiplying Python workers by OpenCV's own native thread pool."""
    native_threads = 1
    if cv2 is not None and hasattr(cv2, "getNumThreads"):
        try:
            native_threads = max(1, int(cv2.getNumThreads()))
        except (TypeError, ValueError):
            native_threads = 1
    return parallel_worker_count(item_count, cpus_per_worker=native_threads)


def sam2_loader_worker_count(frame_count):
    """Keep parallel decode tensors below 25% of SAM2's destination tensor."""
    if frame_count <= 1:
        return 1
    memory_bound = max(1, int(frame_count) // 4)
    return min(parallel_worker_count(frame_count), memory_bound)


def xpu_available(torch):
    return hasattr(torch, "xpu") and torch.xpu.is_available()


def device_candidates(requested, allow_xpu=True):
    import torch

    if requested != "auto":
        return [requested]
    devices = []
    if allow_xpu and xpu_available(torch):
        devices.append("xpu")
    if torch.cuda.is_available():
        devices.append("cuda")
    devices.append("cpu")
    return devices


def release_device(device):
    import torch

    gc.collect()
    try:
        if device.startswith("xpu") and xpu_available(torch):
            torch.xpu.empty_cache()
        elif device.startswith("cuda") and torch.cuda.is_available():
            torch.cuda.empty_cache()
    except RuntimeError:
        pass


def backend_label_from_plan(plan):
    semantic = plan.get("semantic_backend")
    matte = plan.get("matte_refiner")
    if semantic == "sam2":
        return "sam2-vitmatte" if matte == "vitmatte" else "sam2"
    if semantic == "cutie":
        return "cutie-vitmatte" if matte == "vitmatte" else "cutie"
    if semantic == "sam2-cutie":
        return "sam2-cutie-vitmatte" if matte == "vitmatte" else "sam2-cutie"
    if semantic == "matanyone2":
        return "matanyone2"
    if semantic == "sam3.1":
        return "sam3.1-vitmatte" if matte == "vitmatte" else "sam3.1"
    raise ValueError(f"unsupported sealed semantic backend: {semantic!r}")


def requested_precision(request):
    precision = request.get("plan", {}).get("precision")
    if precision not in {"fp32", "bf16"}:
        raise ValueError(f"unsupported sealed precision policy: {precision!r}")
    return precision


def precision_context(torch, device, precision):
    if precision == "fp32":
        return nullcontext()
    if precision != "bf16":
        raise ValueError(f"unsupported precision: {precision}")
    device_type = device.split(":", 1)[0]
    if device_type not in {"cpu", "cuda", "xpu"}:
        raise RuntimeError(f"BF16 autocast is unsupported on device {device!r}")
    return torch.autocast(device_type=device_type, dtype=torch.bfloat16)


def accelerator_peak_mib(device):
    try:
        import torch

        kind = str(device).split(":", 1)[0]
        module = getattr(torch, kind, None)
        if kind not in {"cuda", "xpu"} or module is None:
            return None
        maximum = getattr(module, "max_memory_allocated", None)
        if maximum is None:
            return None
        return float(maximum()) / (1024.0 * 1024.0)
    except (ImportError, RuntimeError, TypeError):
        return None


def reset_accelerator_peak(device):
    try:
        import torch

        kind = str(device).split(":", 1)[0]
        module = getattr(torch, kind, None)
        reset = (
            getattr(module, "reset_peak_memory_stats", None)
            if module is not None
            else None
        )
        if kind in {"cuda", "xpu"} and reset is not None:
            reset()
    except (ImportError, RuntimeError, TypeError):
        pass


def process_peak_rss_mib():
    # Linux ru_maxrss is KiB. Plaque Forge's supported execution/CI platforms are Linux.
    return float(resource.getrusage(resource.RUSAGE_SELF).ru_maxrss) / 1024.0


def record_stage(stage, device, precision, seconds, *, cache_hit=False, note=None):
    record = {
        "stage": stage,
        "device": device,
        "precision": precision,
        "seconds": round(float(seconds), 6),
        "cache_hit": bool(cache_hit),
        "process_peak_rss_mib": round(process_peak_rss_mib(), 3),
    }
    accelerator = accelerator_peak_mib(device)
    if accelerator is not None:
        record["accelerator_peak_mib"] = round(accelerator, 3)
    if note:
        record["note"] = str(note)
    STAGE_METRICS.append(record)


def run_component(name, requested, operation, *, precision="fp32", allow_xpu=True):
    failures = []
    for device in device_candidates(requested, allow_xpu=allow_xpu):
        try:
            reset_accelerator_peak(device)
            started = time.monotonic()
            result = operation(device)
            elapsed = time.monotonic() - started
            release_device(device)
            record_stage(name, device, precision, elapsed)
            print(f"{name}: {device}/{precision}, {elapsed:.1f}s", file=sys.stderr)
            return result, device
        except RuntimeError as error:
            # A resident accelerator model may be the difference between a healthy
            # fallback and an OOM loop. Drop process-local models before trying the
            # next backend; disk/stage caches remain intact.
            clear_resident_models()
            release_device(device)
            failures.append(f"{device}: {error}")
            if requested != "auto" or device == "cpu":
                raise
            print(f"{name} fallback after {device} failure: {error}", file=sys.stderr)
    raise RuntimeError("; ".join(failures))


def cached_model(key, factory):
    if os.environ.get("PLAQUE_FORGE_MODEL_CACHE", "1") == "0":
        return factory(), False
    cached = MODEL_CACHE.get(key)
    if cached is not None:
        return cached, True
    maximum = max(1, int(os.environ.get("PLAQUE_FORGE_MODEL_CACHE_ENTRIES", "4")))
    while len(MODEL_CACHE) >= maximum:
        oldest = next(iter(MODEL_CACHE))
        MODEL_CACHE.pop(oldest, None)
        gc.collect()
    value = factory()
    MODEL_CACHE[key] = value
    return value, False


def clear_resident_models():
    """Drop process-local model references after a failed request."""
    MODEL_CACHE.clear()
    gc.collect()
    try:
        import torch

        if xpu_available(torch):
            torch.xpu.empty_cache()
        if torch.cuda.is_available():
            torch.cuda.empty_cache()
    except (ImportError, RuntimeError):
        pass


def private_runtime_root():
    root = Path(os.environ.get("PLAQUE_FORGE_PYTHON_ROOT", "/tmp/plaque-forge-python"))
    if root != Path("/tmp/plaque-forge-python") and not os.environ.get(
        "PLAQUE_FORGE_ALLOW_CUSTOM_RUNTIME_ROOT"
    ):
        raise RuntimeError(f"unexpected segmentation runtime root: {root}")
    return root


def frame_cache_key(request):
    identity = {
        "format": FRAME_CACHE_FORMAT,
        "source_sha256": request["source"]["sha256"],
        "width": request["source"]["width"],
        "height": request["source"]["height"],
        "frames": request["source"]["frames"],
        "decoder": "ffmpeg-noautorotate-png-rgba",
    }
    return hashlib.sha256(json.dumps(identity, sort_keys=True).encode()).hexdigest()


def cache_lock(path):
    path.parent.mkdir(parents=True, exist_ok=True)
    handle = path.open("a+")
    fcntl.flock(handle.fileno(), fcntl.LOCK_EX)
    return handle


def prune_frame_cache(root, maximum_entries=12):
    if not root.is_dir():
        return
    entries = [
        path
        for path in root.iterdir()
        if path.is_dir() and not path.name.startswith(".")
    ]
    entries.sort(key=lambda path: path.stat().st_mtime, reverse=True)
    for stale in entries[maximum_entries:]:
        shutil.rmtree(stale, ignore_errors=True)


def extract_frames_cached(request):
    cache_root = private_runtime_root() / "cache" / "decoded-frames"
    key = frame_cache_key(request)
    target = cache_root / key
    metadata = target / "cache.json"
    lock = cache_lock(cache_root / ".locks" / f"{key}.lock")
    try:
        if metadata.is_file():
            document = json.loads(metadata.read_text(encoding="utf-8"))
            frames = sorted(target.glob("*.png"))
            if (
                document.get("format") == FRAME_CACHE_FORMAT
                and len(frames) == request["source"]["frames"]
            ):
                record_stage("Decode frames", "cpu", "lossless", 0.0, cache_hit=True)
                os.utime(target, None)
                print(
                    f"Decode frames: cache hit ({len(frames)} lossless PNGs)",
                    file=sys.stderr,
                )
                return frames
        staging = cache_root / f".{key}.{os.getpid()}.incoming"
        shutil.rmtree(staging, ignore_errors=True)
        staging.mkdir(parents=True, exist_ok=True)
        started = time.monotonic()
        subprocess.run(
            [
                "ffmpeg",
                "-hide_banner",
                "-loglevel",
                "error",
                "-noautorotate",
                "-i",
                str(request["source"]["path"]),
                "-start_number",
                "0",
                "-c:v",
                "png",
                "-compression_level",
                "1",
                "-pix_fmt",
                "rgba",
                "-f",
                "image2",
                "-y",
                str(staging / "%06d.png"),
            ],
            check=True,
        )
        frames = sorted(staging.glob("*.png"))
        if len(frames) != request["source"]["frames"]:
            shutil.rmtree(staging, ignore_errors=True)
            raise RuntimeError(
                f"decoded {len(frames)} frames, expected {request['source']['frames']}"
            )
        (staging / "cache.json").write_text(
            json.dumps(
                {
                    "format": FRAME_CACHE_FORMAT,
                    "source_sha256": request["source"]["sha256"],
                    "frames": len(frames),
                },
                indent=2,
            )
            + "\n",
            encoding="utf-8",
        )
        shutil.rmtree(target, ignore_errors=True)
        staging.rename(target)
        elapsed = time.monotonic() - started
        record_stage("Decode frames", "cpu", "lossless", elapsed)
        print(f"Decode frames: cache miss, {elapsed:.1f}s", file=sys.stderr)
        prune_frame_cache(cache_root)
        return sorted(target.glob("*.png"))
    finally:
        fcntl.flock(lock.fileno(), fcntl.LOCK_UN)
        lock.close()


def require_lossless_frames(frames, expected, stage):
    """Fail coherently if the parent transaction or its PNG frame set vanished."""
    if len(frames) != expected:
        raise RuntimeError(
            f"{stage}: lossless frame set has {len(frames)} entries, expected {expected}"
        )
    missing = next((path for path in frames if not path.is_file()), None)
    if missing is not None:
        raise RuntimeError(
            f"{stage}: parent analysis transaction removed lossless frame {missing.name}"
        )


def prompt_shape(prompt, size):
    mask = Image.new("L", size, 0)
    draw = ImageDraw.Draw(mask)
    points = prompt.get("polygon") or prompt.get("quad")
    if points:
        draw.polygon([tuple(point) for point in points], fill=255)
    elif prompt.get("box_bounds"):
        x, y, width, height = prompt["box_bounds"]
        draw.rectangle((x, y, x + width, y + height), fill=255)
    else:
        raise ValueError("Cutie and MatAnyone2 require a box, polygon, or quad seed")
    return mask


def exact_seed_mask(request, frame, size):
    for seed in request["layer"].get("seed_masks", []):
        if seed["frame"] != frame:
            continue
        if file_sha256(seed["path"]) != seed["sha256"]:
            raise ValueError(
                f"seed mask changed after request creation on frame {frame}"
            )
        image = Image.open(seed["path"]).convert("L")
        if image.size != size:
            raise ValueError(f"seed mask dimensions differ on frame {frame}")
        return image
    return None


def probability_from_png(path):
    """Load 8- or 16-bit grayscale PNG without quantizing it through PIL L mode."""
    if cv2 is not None:
        encoded = cv2.imread(str(path), cv2.IMREAD_UNCHANGED)
        if encoded is None:
            encoded = np.asarray(Image.open(path))
    else:
        encoded = np.asarray(Image.open(path))
    if encoded.ndim == 3:
        encoded = encoded[..., 0]
    maximum = (
        65535.0 if encoded.dtype == np.uint16 or encoded.max(initial=0) > 255 else 255.0
    )
    return encoded.astype(np.float32) / maximum


def require_cv2_image(path, flags):
    """Decode through OpenCV and report the path instead of failing later in cvtColor."""
    if cv2 is None:
        raise RuntimeError("OpenCV image decoding requested but OpenCV is unavailable")
    image = cv2.imread(str(path), flags)
    if image is None:
        raise OSError(f"OpenCV failed to decode image: {path}")
    return image


def save_probability_png(path, probability, compression=6):
    encoded = np.round(
        np.asarray(probability, dtype=np.float32).clip(0, 1) * 65535
    ).astype(np.uint16)
    level = int(compression)
    if not 0 <= level <= 9:
        raise ValueError(f"PNG compression level must be 0..9, got {compression!r}")
    if cv2 is not None:
        written = cv2.imwrite(str(path), encoded, [cv2.IMWRITE_PNG_COMPRESSION, level])
        if not written:
            raise OSError(f"OpenCV failed to write PNG: {path}")
    else:
        Image.fromarray(encoded).save(path, format="PNG", compress_level=level)


def seed_mask(request, frame, size):
    exact = exact_seed_mask(request, frame, size)
    if exact is not None:
        return exact
    mask = Image.new("L", size, 0)
    for prompt in request["layer"]["prompts"]:
        if prompt["frame"] == frame:
            mask = Image.fromarray(
                np.maximum(
                    np.asarray(mask), np.asarray(prompt_shape(prompt, size))
                ).astype(np.uint8)
            )
    if not np.asarray(mask).any():
        raise ValueError(f"no area seed exists on frame {frame}")
    return mask


def sam2_native_postprocessing_available():
    try:
        import sam2._C  # noqa: F401

        return True
    except (ImportError, OSError):
        return False


def cleanup_small_mask_defects(probability, max_area=16):
    """Conservative backend-neutral replacement for SAM2's optional CUDA cleanup.

    Only tiny binary islands/holes are changed. The later Cutie/ViTMatte stages still
    own soft-edge refinement, so this intentionally avoids broad morphology.
    """
    probability = np.asarray(probability, dtype=np.float32).clip(0, 1)
    binary = (probability >= 0.5).astype(np.uint8)
    if not binary.any():
        return probability

    count, labels, stats, _ = cv2.connectedComponentsWithStats(binary, connectivity=8)
    if count <= 1:
        return probability

    areas = stats[:, cv2.CC_STAT_AREA]
    lut = np.ones(count, dtype=np.uint8)
    lut[0] = 0
    lut[1:][areas[1:] <= max_area] = 0
    cleaned = lut[labels]

    result = probability.copy()
    removed = (binary == 1) & (cleaned == 0)
    result[removed] = 0.0
    return result


def normalize_sam2_images(images, mean, std):
    """Apply SAM2 channel normalization exactly once."""
    images -= mean
    images /= std
    return images


def sam2_masks(request, frames, device):
    import torch
    from sam2 import sam2_video_predictor as predictor_module
    from sam2.utils import misc as sam2_misc

    precision = requested_precision(request)

    # Upstream SAM2 filters directory input by a JPEG-only suffix even though its PIL
    # decoder already supports PNG. Install a narrow eager loader so model input stays
    # lossless and the filename truthfully describes its bytes.
    if not hasattr(predictor_module, "_plaque_forge_original_load_video_frames"):
        predictor_module._plaque_forge_original_load_video_frames = (
            predictor_module.load_video_frames
        )

    def load_lossless_frames(
        video_path,
        image_size,
        offload_video_to_cpu,
        img_mean=(0.485, 0.456, 0.406),
        img_std=(0.229, 0.224, 0.225),
        async_loading_frames=False,
        compute_device=torch.device("cuda"),
    ):
        png_paths = (
            sorted(Path(video_path).glob("*.png"), key=lambda path: int(path.stem))
            if isinstance(video_path, str) and Path(video_path).is_dir()
            else []
        )
        if not png_paths:
            return predictor_module._plaque_forge_original_load_video_frames(
                video_path,
                image_size,
                offload_video_to_cpu,
                img_mean,
                img_std,
                async_loading_frames,
                compute_device,
            )
        if async_loading_frames:
            raise ValueError(
                "lossless PNG loading does not support asynchronous loading"
            )
        images = torch.zeros(
            len(png_paths), 3, image_size, image_size, dtype=torch.float32
        )
        video_height = video_width = 0

        def _load_single(index_path):
            idx, p = index_path
            tensor, h, w = sam2_misc._load_img_as_tensor(str(p), image_size)
            return idx, tensor, h, w

        max_workers = sam2_loader_worker_count(len(png_paths))
        if max_workers > 1:
            with concurrent.futures.ThreadPoolExecutor(max_workers=max_workers) as pool:
                path_iter = iter(enumerate(png_paths))
                pending = set()
                for _ in range(max_workers):
                    try:
                        pending.add(pool.submit(_load_single, next(path_iter)))
                    except StopIteration:
                        break
                while pending:
                    done, pending = concurrent.futures.wait(
                        pending, return_when=concurrent.futures.FIRST_COMPLETED
                    )
                    for future in done:
                        idx, tensor, h, w = future.result()
                        images[idx] = tensor
                        video_height = h
                        video_width = w
                        try:
                            pending.add(pool.submit(_load_single, next(path_iter)))
                        except StopIteration:
                            pass
        else:
            for index, path in enumerate(png_paths):
                images[index], video_height, video_width = (
                    sam2_misc._load_img_as_tensor(str(path), image_size)
                )
        mean = torch.tensor(img_mean, dtype=torch.float32)[:, None, None]
        std = torch.tensor(img_std, dtype=torch.float32)[:, None, None]
        if not offload_video_to_cpu:
            images = images.to(compute_device)
            mean = mean.to(compute_device)
            std = std.to(compute_device)
        normalize_sam2_images(images, mean, std)
        return images, video_height, video_width

    predictor_module.load_video_frames = load_lossless_frames
    native_postprocessing = sam2_native_postprocessing_available()
    if not native_postprocessing:
        print(
            "[ml] SAM2 CUDA _C post-processing unavailable; "
            "using backend-neutral small-mask cleanup (expected on XPU/CPU)",
            file=sys.stderr,
            flush=True,
        )
    compile_requested = bool(request.get("plan", {}).get("compile"))
    predictor_key = ("sam2", request["model"], device, precision, compile_requested)

    def build_predictor():
        predictor = load_sam2_video_predictor(request["model"], device)
        compile_label = "eager"
        if compile_requested:
            # Compilation is deliberately a preview-only policy because upstream notes
            # that compiled inference can introduce small numerical prediction variance.
            image_encoder = getattr(predictor, "image_encoder", None)
            if image_encoder is not None and hasattr(torch, "compile"):
                try:
                    predictor.image_encoder = torch.compile(
                        image_encoder,
                        mode="reduce-overhead",
                        fullgraph=False,
                    )
                    compile_label = "compiled-image-encoder"
                except Exception as error:
                    compile_label = "compile-unavailable"
                    print(
                        f"[ml] SAM2 compile unavailable; continuing eager: {error}",
                        file=sys.stderr,
                    )

        # SAM2 compresses temporal memory to BF16 before storing it. Precision is part
        # of the Rust-sealed plan, not a side effect of the selected device: FP32 plans
        # restore stored memory tensors to FP32 at the two upstream compression points.
        if precision == "fp32":
            original_single_frame = predictor._run_single_frame_inference
            original_memory_encoder = predictor._run_memory_encoder

            def run_single_frame_fp32(*args, **kwargs):
                compact, masks = original_single_frame(*args, **kwargs)
                memory = compact.get("maskmem_features")
                if memory is not None:
                    compact["maskmem_features"] = memory.float()
                return compact, masks

            def run_memory_encoder_fp32(*args, **kwargs):
                memory, positions = original_memory_encoder(*args, **kwargs)
                return memory.float(), positions

            predictor._run_single_frame_inference = run_single_frame_fp32
            predictor._run_memory_encoder = run_memory_encoder_fp32
        return predictor, compile_label

    cached, model_cache_hit = cached_model(predictor_key, build_predictor)
    predictor, compile_label = cached
    if model_cache_hit:
        print(
            f"[ml] SAM2 resident-model hit: {request['model']} on {device}/{precision}",
            file=sys.stderr,
        )
    state = predictor.init_state(
        video_path=str(frames[0].parent), offload_video_to_cpu=True
    )
    prompts = request["layer"]["prompts"]
    precision = requested_precision(request)
    active_start, active_end = request["layer"].get("active_frames") or [
        0,
        len(frames) - 1,
    ]
    probabilities = [None] * len(frames)
    first = min(prompt["frame"] for prompt in prompts)
    last = max(prompt["frame"] for prompt in prompts)
    objects = {}
    # Precision is part of the Rust-sealed plan. A device fallback must not silently
    # change arithmetic from BF16 to FP32 or vice versa.
    autocast = precision_context(torch, device, precision)
    with torch.inference_mode(), autocast:
        for prompt in prompts:
            object_name = prompt.get("object") or "default"
            object_id = objects.setdefault(object_name, len(objects) + 1)
            kwargs = {
                "inference_state": state,
                "frame_idx": prompt["frame"],
                "obj_id": object_id,
            }
            exact = exact_seed_mask(
                request, prompt["frame"], Image.open(frames[0]).size
            )
            polygon = prompt.get("polygon") or prompt.get("quad")
            if exact is not None:
                predictor.add_new_mask(
                    **kwargs,
                    mask=torch.from_numpy(np.asarray(exact, dtype=np.uint8) > 24).to(
                        device
                    ),
                )
            elif polygon:
                mask = prompt_shape(prompt, Image.open(frames[0]).size)
                predictor.add_new_mask(
                    **kwargs,
                    mask=torch.from_numpy(np.asarray(mask, dtype=np.uint8) > 0).to(
                        device
                    ),
                )
            else:
                positive = prompt.get("positive_points", [])
                negative = prompt.get("negative_points", [])
                points = positive + negative
                if points:
                    kwargs["points"] = np.asarray(points, dtype=np.float32)
                    kwargs["labels"] = np.asarray(
                        [1] * len(positive) + [0] * len(negative), dtype=np.int32
                    )
                if prompt.get("box_bounds"):
                    x, y, width, height = prompt["box_bounds"]
                    kwargs["box"] = np.asarray(
                        [x, y, x + width, y + height], dtype=np.float32
                    )
                predictor.add_new_points_or_box(**kwargs)

        passes = [
            (first, False, active_end - first),
            (last, True, last - active_start),
        ]
        for start, reverse, limit in passes:
            for frame, _, logits in predictor.propagate_in_video(
                state,
                start_frame_idx=start,
                max_frame_num_to_track=limit,
                reverse=reverse,
            ):
                probability = (
                    logits.sigmoid().amax(dim=0).squeeze().float().cpu().numpy()
                )
                probabilities[frame] = (
                    probability
                    if probabilities[frame] is None
                    else np.maximum(probabilities[frame], probability)
                )
    if any(
        probabilities[frame] is None for frame in range(active_start, active_end + 1)
    ):
        raise RuntimeError("SAM 2 did not produce every source frame")
    empty = np.zeros(
        (request["source"]["height"], request["source"]["width"]), dtype=np.float32
    )
    probabilities = [
        empty.copy() if probability is None else probability
        for probability in probabilities
    ]
    if not native_postprocessing:
        probabilities = [
            cleanup_small_mask_defects(probability) for probability in probabilities
        ]
    return probabilities, (
        f"sam2-{package_version('sam-2', 'sam2')}+{precision}+{compile_label}"
    )


def load_cutie(device):
    import torch
    import cutie
    from hydra import compose, initialize_config_dir
    from hydra.core.global_hydra import GlobalHydra
    from omegaconf import open_dict
    from cutie.model.cutie import CUTIE
    from cutie.inference.utils.args_utils import get_dataset_cfg
    from cutie.utils.download_models import download_models_if_needed

    config = Path(next(iter(cutie.__path__))) / "config"
    GlobalHydra.instance().clear()
    with initialize_config_dir(version_base="1.3.2", config_dir=str(config)):
        cfg = compose(config_name="eval_config")
    weights = Path(download_models_if_needed()) / "cutie-base-mega.pth"
    with open_dict(cfg):
        cfg.weights = str(weights)
    get_dataset_cfg(cfg)
    key = ("cutie", str(weights), device)

    def build_model():
        model = CUTIE(cfg).to(device).eval()
        model.load_weights(torch.load(weights, map_location="cpu", weights_only=False))
        return model

    model, hit = cached_model(key, build_model)
    if hit:
        print(f"[ml] Cutie resident-model hit: {device}", file=sys.stderr)
    return model


def apply_authored_sam2_prompt_corrections(request, guides, tracked, radius=4):
    """Restore local SAM2 detail without replacing Cutie's temporal track.

    SAM2 is authoritative only where the author explicitly corrected the object.
    Limiting its confidence to a small neighborhood of Cutie's current support keeps
    unrelated SAM2 objects and inter-prompt propagation failures out of the result.
    """
    prompt_frames = {prompt["frame"] for prompt in request["layer"]["prompts"]}
    corrected = []
    for frame, probability in enumerate(tracked):
        probability = np.asarray(probability, dtype=np.float32).clip(0, 1)
        if frame in prompt_frames:
            nearby = dilate_binary_disk(probability >= 0.03, radius)
            guide = np.asarray(guides[frame], dtype=np.float32).clip(0, 1)
            probability = np.maximum(probability, np.where(nearby, guide, 0.0))
        corrected.append(probability.astype(np.float32))
    return corrected


def dilate_binary_disk(mask, radius):
    mask = np.asarray(mask, dtype=bool)
    if radius <= 0:
        return mask.copy()
    if cv2 is not None:
        kernel = cv2.getStructuringElement(
            cv2.MORPH_ELLIPSE, (2 * radius + 1, 2 * radius + 1)
        )
        return cv2.dilate(mask.astype(np.uint8), kernel, iterations=1).astype(bool)

    # Lightweight contract-test environments intentionally omit OpenCV. Keep an
    # exact disk fallback so importing and testing the worker does not weaken the
    # correction boundary; production runtimes take the optimized branch above.
    height, width = mask.shape
    dilated = np.zeros_like(mask)
    for offset_y in range(-radius, radius + 1):
        for offset_x in range(-radius, radius + 1):
            if offset_x * offset_x + offset_y * offset_y > radius * radius:
                continue
            source_y = slice(max(0, -offset_y), min(height, height - offset_y))
            source_x = slice(max(0, -offset_x), min(width, width - offset_x))
            target_y = slice(max(0, offset_y), min(height, height + offset_y))
            target_x = slice(max(0, offset_x), min(width, width + offset_x))
            dilated[target_y, target_x] |= mask[source_y, source_x]
    return dilated


def cutie_masks(request, frames, device, guides=None):
    import torch
    from cutie.inference.inference_core import InferenceCore
    from torchvision.transforms.functional import to_tensor

    precision = requested_precision(request)
    prompts = request["layer"]["prompts"]
    active_start, active_end = request["layer"].get("active_frames") or [
        0,
        len(frames) - 1,
    ]
    if device == "cpu":
        torch.set_num_threads(min(8, usable_cpu_count()))
    seed = min(prompt["frame"] for prompt in prompts)
    prompt_frames = {prompt["frame"] for prompt in prompts}
    source_width, source_height = Image.open(frames[0]).size

    model = load_cutie(device)

    def propagate(indices):
        processor = InferenceCore(model, cfg=model.cfg)

        def _load_image(frame):
            if cv2 is not None:
                bgr = require_cv2_image(frames[frame], cv2.IMREAD_COLOR)
                rgb = cv2.cvtColor(bgr, cv2.COLOR_BGR2RGB)
                return (
                    torch.from_numpy(rgb.transpose((2, 0, 1)))
                    .float()
                    .div(255.0)
                    .to(device)
                )
            else:
                return (
                    to_tensor(Image.open(frames[frame]).convert("RGB"))
                    .to(device)
                    .float()
                )

        output = {}
        with torch.inference_mode(), precision_context(torch, device, precision):
            # Double-buffer: prefetch next frame's CPU decode (cv2.imread + to_tensor)
            # while the current frame's GPU-bound processor.step runs.
            with concurrent.futures.ThreadPoolExecutor(max_workers=1) as prefetch:
                indices = list(indices)
                # Prime prefetch for first frame
                next_future = (
                    prefetch.submit(_load_image, indices[0]) if indices else None
                )
                for position, frame in enumerate(indices):
                    # Get current frame's image (wait for prefetch)
                    image = (
                        next_future.result()
                        if next_future is not None
                        else _load_image(frame)
                    )
                    # Prefetch next frame immediately (overlaps with current processor.step)
                    if position + 1 < len(indices):
                        next_frame = indices[position + 1]
                        next_future = prefetch.submit(_load_image, next_frame)
                    else:
                        next_future = None

                    correction = position == 0 or frame in prompt_frames
                    if correction:
                        source = (
                            guides[frame] >= 0.5
                            if guides is not None
                            else np.asarray(
                                seed_mask(request, frame, (source_width, source_height))
                            )
                            > 0
                        )
                        mask = torch.from_numpy(source.astype(np.float32)).to(device)
                        probability = processor.step(image, mask, objects=[1])
                    else:
                        probability = processor.step(image)
                    probability = probability.squeeze()
                    if probability.ndim == 3 and probability.shape[0] > 1:
                        probability = probability[1:].amax(dim=0)
                    elif probability.ndim == 3:
                        probability = probability[0]
                    output[frame] = (
                        probability.float().clamp(0, 1).detach().cpu().numpy()
                    )
        return output

    masks = propagate(range(seed, active_end + 1))
    masks.update(propagate(range(seed, active_start - 1, -1)))
    empty = np.zeros((source_height, source_width), dtype=np.float32)
    probabilities = [
        masks.get(frame, empty).astype(np.float32) for frame in range(len(frames))
    ]
    correction_version = ""
    if guides is not None:
        probabilities = apply_authored_sam2_prompt_corrections(
            request, guides, probabilities
        )
        correction_version = "+local-sam2-prompt-corrections-r4"
    return probabilities, f"cutie-{package_version('cutie')}{correction_version}"


def optical_trimap_known_foreground(request, frame_index, probability):
    """Return sparse known-foreground seeds for an optical matte.

    Semantic membership is not optical opacity.  A translucent foreground such as a
    web may have very confident semantic support while most pixels inside that support
    are still transparent.  ViTMatte therefore receives only sparse certain-foreground
    seeds; the wider semantic region remains unknown trimap.
    """
    height, width = probability.shape
    known = np.zeros((height, width), dtype=np.uint8)

    # Exact authored positive points are the strongest supervision available.
    for prompt in request["layer"]["prompts"]:
        if prompt["frame"] != frame_index:
            continue
        for x, y in prompt.get("positive_points", []):
            x = int(round(np.clip(x, 0, width - 1)))
            y = int(round(np.clip(y, 0, height - 1)))
            cv2.circle(known, (x, y), 1, 1, thickness=-1, lineType=cv2.LINE_AA)

    # Between authored corrections, provide sparse semantic anchors rather than an
    # eroded solid body.  One local maximum per tile gives ViTMatte enough foreground
    # evidence without declaring transparent holes to be opaque.
    if not known.any():
        stride = 32
        for top in range(0, height, stride):
            for left in range(0, width, stride):
                tile = probability[
                    top : min(top + stride, height), left : min(left + stride, width)
                ]
                if tile.size == 0 or float(tile.max(initial=0.0)) < 0.92:
                    continue
                _, maximum, _, location = cv2.minMaxLoc(tile)
                if maximum < 0.92:
                    continue
                x = left + location[0]
                y = top + location[1]
                known[y, x] = 1
    return known


def refine_vitmatte(request, probabilities, frames, model_name, device):
    import torch
    from transformers import VitMatteForImageMatting, VitMatteImageProcessor

    precision = requested_precision(request)
    revision = model_revision(model_name)

    def build_vitmatte():
        processor = VitMatteImageProcessor.from_pretrained(model_name, **revision)
        model = (
            VitMatteForImageMatting.from_pretrained(model_name, **revision)
            .to(device)
            .eval()
        )
        return processor, model

    (processor, model), hit = cached_model(
        ("vitmatte", model_name, device), build_vitmatte
    )
    if hit:
        print(
            f"[ml] ViTMatte resident-model hit: {model_name} on {device}",
            file=sys.stderr,
        )
    output = []
    optical_foreground = (
        request["layer"].get("role") == "foreground"
        and request["layer"].get("matte_mode", "optical") == "optical"
    )
    kernel = np.ones((3, 3), np.uint8)

    def _prepare_inputs(frame_index, probability, frame_path):
        prob = np.asarray(probability, dtype=np.float32).clip(0, 1)
        if optical_foreground:
            foreground = optical_trimap_known_foreground(request, frame_index, prob)
        else:
            foreground = cv2.erode((prob >= 0.82).astype(np.uint8), kernel)
            if not foreground.any():
                foreground = (prob >= 0.60).astype(np.uint8)
        if optical_foreground:
            support = cv2.dilate((prob >= 0.30).astype(np.uint8), kernel, iterations=1)
        else:
            support = cv2.dilate((prob >= 0.35).astype(np.uint8), kernel, iterations=3)
        active = cv2.findNonZero(support)
        if active is None:
            return None
        trimap = np.zeros(prob.shape, dtype=np.uint8)
        trimap[support > 0] = 128
        trimap[foreground > 0] = 255
        x, y, width, height = cv2.boundingRect(active)
        margin = 32
        left = max(0, x - margin)
        top = max(0, y - margin)
        if cv2 is not None:
            bgr = require_cv2_image(frame_path, cv2.IMREAD_COLOR)
            img_h, img_w = bgr.shape[:2]
            right = min(img_w, x + width + margin)
            bottom = min(img_h, y + height + margin)
            crop_rgb = cv2.cvtColor(bgr[top:bottom, left:right], cv2.COLOR_BGR2RGB)
            crop = Image.fromarray(crop_rgb)
        else:
            image = Image.open(frame_path).convert("RGB")
            right = min(image.width, x + width + margin)
            bottom = min(image.height, y + height + margin)
            crop = image.crop((left, top, right, bottom))
        crop_trimap = Image.fromarray(trimap[top:bottom, left:right])
        inputs = processor(images=crop, trimaps=crop_trimap, return_tensors="pt")
        inputs = {key: value.to(device) for key, value in inputs.items()}
        return (prob, support, left, top, right, bottom, crop, inputs)

    # Double-buffer: prefetch next frame's CPU work (cv2.imread + trimap + processor)
    # while the current frame's GPU inference (model) runs. This overlaps
    # CPU-bound I/O/morphology with GPU-bound inference without changing
    # the model's batch semantics (still 1 frame per inference).
    with concurrent.futures.ThreadPoolExecutor(max_workers=1) as prefetch:
        next_future = None
        # Prime the prefetch with frame 0
        if probabilities:
            next_future = prefetch.submit(
                _prepare_inputs, 0, probabilities[0], frames[0]
            )

        for frame_index in range(len(probabilities)):
            # Get prepared inputs for current frame (wait for prefetch)
            prepared = next_future.result() if next_future is not None else None
            # Prefetch next frame immediately (overlaps with current GPU)
            if frame_index + 1 < len(probabilities):
                next_future = prefetch.submit(
                    _prepare_inputs,
                    frame_index + 1,
                    probabilities[frame_index + 1],
                    frames[frame_index + 1],
                )
            else:
                next_future = None

            if prepared is None:
                # No active support -> empty matte
                prob_shape = np.asarray(
                    probabilities[frame_index], dtype=np.float32
                ).shape
                output.append(np.zeros(prob_shape, dtype=np.float32))
                continue

            prob, support, left, top, right, bottom, crop, inputs = prepared
            with torch.inference_mode(), precision_context(torch, device, precision):
                alpha = model(**inputs).alphas[0, 0].float().clamp(0, 1).cpu().numpy()
            alpha = alpha[: crop.height, : crop.width]
            guard = cv2.GaussianBlur(support.astype(np.float32), (0, 0), 2.0).clip(0, 1)
            matte = np.zeros(prob.shape, dtype=np.float32)
            matte[top:bottom, left:right] = alpha
            output.append((matte * guard).astype(np.float32))
    return output


def dense_flow(source, target):
    # Half-resolution variational flow preserves narrow vines/webs far better than
    # the former quarter-resolution FAST preset while keeping this post-pass bounded.
    scale = 0.5
    size = (
        max(1, round(source.shape[1] * scale)),
        max(1, round(source.shape[0] * scale)),
    )
    source_small = cv2.resize(source, size, interpolation=cv2.INTER_AREA)
    target_small = cv2.resize(target, size, interpolation=cv2.INTER_AREA)
    estimator = getattr(_DIS_FLOW_LOCAL, "estimator", None)
    if estimator is None:
        estimator = cv2.DISOpticalFlow_create(cv2.DISOPTICAL_FLOW_PRESET_MEDIUM)
        _DIS_FLOW_LOCAL.estimator = estimator
    flow = estimator.calc(source_small, target_small, None)
    flow = cv2.resize(
        flow, (source.shape[1], source.shape[0]), interpolation=cv2.INTER_LINEAR
    )
    flow[..., 0] /= scale
    flow[..., 1] /= scale
    return flow


def warp_alpha(alpha, source_gray, target_gray, forward, backward):
    h, w = target_gray.shape[:2]
    key = (h, w)
    cached = _WARP_MESHGRID_CACHE.get(key)
    if cached is None:
        x, y = np.meshgrid(
            np.arange(w, dtype=np.float32), np.arange(h, dtype=np.float32)
        )
        _WARP_MESHGRID_CACHE[key] = (x, y)
    else:
        x, y = cached
    map_x = x + backward[..., 0]
    map_y = y + backward[..., 1]
    warped = cv2.remap(
        alpha, map_x, map_y, cv2.INTER_LINEAR, borderMode=cv2.BORDER_CONSTANT
    )
    warped_gray = cv2.remap(
        source_gray, map_x, map_y, cv2.INTER_LINEAR, borderMode=cv2.BORDER_REFLECT
    )
    forward_x = cv2.remap(
        forward[..., 0], map_x, map_y, cv2.INTER_LINEAR, borderMode=cv2.BORDER_CONSTANT
    )
    forward_y = cv2.remap(
        forward[..., 1], map_x, map_y, cv2.INTER_LINEAR, borderMode=cv2.BORDER_CONSTANT
    )
    consistency = np.hypot(forward_x + backward[..., 0], forward_y + backward[..., 1])
    appearance = np.abs(warped_gray.astype(np.float32) - target_gray.astype(np.float32))
    weight = np.exp(-np.square(consistency / 2.5)) * np.exp(-appearance / 24.0)
    return warped, weight.astype(np.float32)


def stabilize_alpha(
    probabilities, frames, blend_strength=0.32, propagation_headroom=None
):
    if len(probabilities) < 2:
        return probabilities
    original = [
        np.asarray(value, dtype=np.float32).clip(0, 1) for value in probabilities
    ]
    active = [frame for frame, value in enumerate(original) if np.any(value >= 0.01)]
    if not active:
        return original
    start = max(0, active[0] - 1)
    end = min(len(original) - 1, active[-1] + 1)
    segment = original[start : end + 1]
    active_frame_paths = frames[start : end + 1]

    decode_workers = parallel_worker_count(len(active_frame_paths))
    if decode_workers > 1:
        with concurrent.futures.ThreadPoolExecutor(max_workers=decode_workers) as pool:
            gray = list(
                pool.map(
                    lambda path: require_cv2_image(path, cv2.IMREAD_GRAYSCALE),
                    active_frame_paths,
                )
            )
    else:
        gray = [
            require_cv2_image(path, cv2.IMREAD_GRAYSCALE) for path in active_frame_paths
        ]

    def _calc_flow_pair(frame_idx):
        g0 = gray[frame_idx]
        g1 = gray[frame_idx + 1]
        return frame_idx, (dense_flow(g0, g1), dense_flow(g1, g0))

    num_pairs = len(gray) - 1
    flow_workers = opencv_parallel_worker_count(num_pairs)
    if flow_workers > 1 and num_pairs > 1:
        with concurrent.futures.ThreadPoolExecutor(max_workers=flow_workers) as pool:
            flows = [
                pair[1]
                for pair in sorted(
                    pool.map(_calc_flow_pair, range(num_pairs)),
                    key=lambda item: item[0],
                )
            ]
    else:
        flows = [
            (
                dense_flow(gray[frame], gray[frame + 1]),
                dense_flow(gray[frame + 1], gray[frame]),
            )
            for frame in range(num_pairs)
        ]
    forward = [segment[0]]
    for frame in range(1, len(segment)):
        flow, backward_flow = flows[frame - 1]
        warped, weight = warp_alpha(
            forward[-1], gray[frame - 1], gray[frame], flow, backward_flow
        )
        blend = blend_strength * weight
        forward.append(segment[frame] * (1 - blend) + warped * blend)
    backward = [None] * len(segment)
    backward[-1] = segment[-1]
    for frame in range(len(segment) - 2, -1, -1):
        forward_flow, flow = flows[frame]
        warped, weight = warp_alpha(
            backward[frame + 1], gray[frame + 1], gray[frame], flow, forward_flow
        )
        blend = blend_strength * weight
        backward[frame] = segment[frame] * (1 - blend) + warped * blend
    smoothed = []
    for current, before, after in zip(segment, forward, backward):
        value = (0.50 * current + 0.25 * before + 0.25 * after).clip(0, 1)
        if propagation_headroom is not None:
            # Temporal propagation may reinforce real thin strands but must not keep
            # a strong invisible occluder alive when the current-frame matte no longer
            # supports it.
            value = np.minimum(value, np.clip(current + propagation_headroom, 0, 1))
        smoothed.append(value.astype(np.float32))
    output = original.copy()
    output[start : end + 1] = smoothed
    return output


def matanyone2_masks(request, output, size, frames, device):
    import torch
    import matanyone2.model.matanyone2 as model_module
    from matanyone2 import InferenceCore, MatAnyone2

    precision = requested_precision(request)
    prompts = request["layer"]["prompts"]
    if min(prompt["frame"] for prompt in prompts) != 0:
        raise ValueError("MatAnyone2 requires a frame-0 area seed")
    seed = seed_mask(request, 0, size)
    seed_path = output / "seed.png"
    seed.save(seed_path)
    model_module.device = torch.device(device)

    def build_matanyone2():
        return (
            MatAnyone2.from_pretrained(
                request["model"], **model_revision(request["model"])
            )
            .to(device)
            .eval()
        )

    model, hit = cached_model(
        ("matanyone2", request["model"], device), build_matanyone2
    )
    if hit:
        print(f"[ml] MatAnyone2 resident-model hit: {device}", file=sys.stderr)
    processor = InferenceCore(model, device=device)
    work = output / "matanyone2"
    with torch.inference_mode(), precision_context(torch, device, precision):
        processor.process_video(
            input_path=request["source"]["path"],
            mask_path=str(seed_path),
            output_path=str(work),
            save_image=True,
            r_erode=0,
            r_dilate=0,
        )
    alpha_paths = sorted(work.glob("*/pha/*.png"))
    if len(alpha_paths) != frames:
        raise RuntimeError(
            f"MatAnyone2 produced {len(alpha_paths)} masks, expected {frames}"
        )
    # Preserve the model's native PNG precision. Converting through PIL "L" used to
    # quantize the matte to 8 bits before Plaque Forge re-encoded it as 16-bit PNG.
    return [probability_from_png(path) for path in alpha_paths], (
        f"matanyone2-{package_version('matanyone2')}+{precision}"
    )


def sam2_cache_key(request):
    identity = {
        "format": "plaque-forge.sam2-cache/2",
        "model": request["model"],
        "source_sha256": request["source"]["sha256"],
        "frames": request["source"]["frames"],
        "prompts": request["layer"]["prompts"],
        "seed_masks": [
            {"frame": seed["frame"], "sha256": seed["sha256"]}
            for seed in request["layer"].get("seed_masks", [])
        ],
        "plan_sha256": request["plan_sha256"],
        "requested_device": request.get("device", "auto"),
        "runtime_sha256": request.get("runtime_sha256"),
    }
    return hashlib.sha256(json.dumps(identity, sort_keys=True).encode()).hexdigest()


def prune_stage_cache(root, maximum_entries=20):
    if not root.is_dir():
        return
    entries = [
        path
        for path in root.iterdir()
        if path.is_dir() and not path.name.startswith(".")
    ]
    entries.sort(key=lambda path: path.stat().st_mtime, reverse=True)
    for stale in entries[maximum_entries:]:
        shutil.rmtree(stale, ignore_errors=True)


def cutie_cache_key(request, guided):
    identity = {
        "format": "plaque-forge.cutie-cache/2",
        "source_sha256": request["source"]["sha256"],
        "prompt_sha256": request["prompt_sha256"],
        "plan_sha256": request["plan_sha256"],
        "requested_device": request.get("device", "auto"),
        "runtime_sha256": request.get("runtime_sha256"),
        "precision": requested_precision(request),
        "guided_by_sam2": bool(guided),
    }
    return hashlib.sha256(json.dumps(identity, sort_keys=True).encode()).hexdigest()


def load_cutie_stage_cache(
    request, frames, requested_device, guided, *, allow_xpu=True
):
    key = cutie_cache_key(request, guided)
    target = private_runtime_root() / "cache" / "model-stages" / "cutie" / key
    metadata_path = target / "cache.json"
    mask_root = target / "masks"
    if not metadata_path.is_file():
        return None
    metadata = json.loads(metadata_path.read_text(encoding="utf-8"))
    paths = sorted(mask_root.glob("*.png"))
    candidate_devices = device_candidates(requested_device, allow_xpu=allow_xpu)
    if (
        metadata.get("key") != key
        or metadata.get("device") not in candidate_devices
        or metadata.get("precision") != requested_precision(request)
        or len(paths) != len(frames)
    ):
        return None
    probabilities = [probability_from_png(path) for path in paths]
    os.utime(target, None)
    record_stage(
        "Cutie",
        metadata["device"],
        metadata["precision"],
        0.0,
        cache_hit=True,
    )
    print("Cutie: persistent cache hit", file=sys.stderr)
    return probabilities, metadata["version"], metadata["device"]


def cached_cutie(request, frames, requested_device, guides=None, *, allow_xpu=True):
    key = cutie_cache_key(request, guides is not None)
    cache_root = private_runtime_root() / "cache" / "model-stages" / "cutie"
    target = cache_root / key
    lock = cache_lock(cache_root / ".locks" / f"{key}.lock")
    try:
        cached = load_cutie_stage_cache(
            request,
            frames,
            requested_device,
            guides is not None,
            allow_xpu=allow_xpu,
        )
        if cached is not None:
            return cached

        (probabilities, version), device = run_component(
            "Cutie",
            requested_device,
            lambda candidate: cutie_masks(request, frames, candidate, guides=guides),
            precision=requested_precision(request),
            allow_xpu=allow_xpu,
        )
        staging = cache_root / f".{key}.{os.getpid()}.incoming"
        shutil.rmtree(staging, ignore_errors=True)
        (staging / "masks").mkdir(parents=True, exist_ok=True)
        max_workers = parallel_worker_count(len(probabilities))
        if max_workers > 1:
            with concurrent.futures.ThreadPoolExecutor(max_workers=max_workers) as pool:
                list(
                    pool.map(
                        lambda item: save_probability_png(
                            staging / "masks" / f"{item[0]:06}.png",
                            item[1],
                            compression=1,
                        ),
                        enumerate(probabilities),
                    )
                )
        else:
            for frame, probability in enumerate(probabilities):
                save_probability_png(
                    staging / "masks" / f"{frame:06}.png", probability, compression=1
                )
        (staging / "cache.json").write_text(
            json.dumps(
                {
                    "format": "plaque-forge.cutie-cache/2",
                    "key": key,
                    "version": version,
                    "device": device,
                    "precision": requested_precision(request),
                },
                indent=2,
            )
            + "\n",
            encoding="utf-8",
        )
        shutil.rmtree(target, ignore_errors=True)
        staging.rename(target)
        prune_stage_cache(cache_root)
        return probabilities, version, device
    finally:
        fcntl.flock(lock.fileno(), fcntl.LOCK_UN)
        lock.close()


def cached_sam2(request, frames, requested_device):
    key = sam2_cache_key(request)
    cache_root = private_runtime_root() / "cache" / "model-stages" / "sam2"
    target = cache_root / key
    metadata_path = target / "cache.json"
    mask_root = target / "masks"
    lock = cache_lock(cache_root / ".locks" / f"{key}.lock")
    try:
        if metadata_path.is_file():
            metadata = json.loads(metadata_path.read_text(encoding="utf-8"))
            paths = sorted(mask_root.glob("*.png"))
            candidate_devices = device_candidates(requested_device, allow_xpu=True)
            if (
                metadata.get("key") == key
                and metadata.get("device") in candidate_devices
                and metadata.get("precision") == requested_precision(request)
                and len(paths) == len(frames)
            ):
                probabilities = [probability_from_png(path) for path in paths]
                os.utime(target, None)
                record_stage(
                    "SAM 2",
                    metadata["device"],
                    metadata["precision"],
                    0.0,
                    cache_hit=True,
                )
                print("SAM 2: persistent cache hit", file=sys.stderr)
                return probabilities, metadata["version"], metadata["device"]

        (probabilities, version), device = run_component(
            "SAM 2",
            requested_device,
            lambda candidate: sam2_masks(request, frames, candidate),
            precision=requested_precision(request),
        )
        staging = cache_root / f".{key}.{os.getpid()}.incoming"
        shutil.rmtree(staging, ignore_errors=True)
        (staging / "masks").mkdir(parents=True, exist_ok=True)
        max_workers = parallel_worker_count(len(probabilities))
        if max_workers > 1:
            with concurrent.futures.ThreadPoolExecutor(max_workers=max_workers) as pool:
                list(
                    pool.map(
                        lambda item: save_probability_png(
                            staging / "masks" / f"{item[0]:06}.png",
                            item[1],
                            compression=1,
                        ),
                        enumerate(probabilities),
                    )
                )
        else:
            for frame, probability in enumerate(probabilities):
                save_probability_png(
                    staging / "masks" / f"{frame:06}.png", probability, compression=1
                )
        (staging / "cache.json").write_text(
            json.dumps(
                {
                    "format": "plaque-forge.sam2-cache/2",
                    "key": key,
                    "version": version,
                    "device": device,
                    "precision": requested_precision(request),
                },
                indent=2,
            )
            + "\n",
            encoding="utf-8",
        )
        shutil.rmtree(target, ignore_errors=True)
        staging.rename(target)
        prune_stage_cache(cache_root)
        return probabilities, version, device
    finally:
        fcntl.flock(lock.fileno(), fcntl.LOCK_UN)
        lock.close()


def sam31_masks(request, output):
    """Run the optional SAM 3.1 backend in its isolated Python 3.12/CUDA runtime."""
    if requested_precision(request) != "bf16":
        raise RuntimeError(
            "SAM 3.1 integration currently requires the sealed BF16 policy"
        )
    root = Path(os.environ.get("PLAQUE_FORGE_SAM31_ROOT", "/tmp/plaque-forge-sam31"))
    python = root / "venv" / "bin" / "python"
    manifest = root / "runtime-manifest.json"
    bridge = Path(__file__).with_name("sam31_worker.py")
    if not python.is_file() or not manifest.is_file():
        raise RuntimeError(
            "SAM 3.1 runtime is not installed; run ./scripts/setup_sam31.sh after obtaining gated model access"
        )
    work = output / ".sam31"
    shutil.rmtree(work, ignore_errors=True)
    work.mkdir(parents=True, exist_ok=True)
    request_path = work / "request.json"
    request_path.write_text(json.dumps(request, indent=2) + "\n", encoding="utf-8")
    started = time.monotonic()
    subprocess.run(
        [
            str(python),
            str(bridge),
            "--request",
            str(request_path),
            "--output",
            str(work),
        ],
        check=True,
        env={
            **os.environ,
            "PLAQUE_FORGE_SAM31_ROOT": str(root),
            "HF_HOME": str(root / "cache" / "huggingface"),
            "HF_HUB_OFFLINE": "1",
            "TRANSFORMERS_OFFLINE": "1",
        },
    )
    elapsed = time.monotonic() - started
    metadata = json.loads((work / "result.json").read_text(encoding="utf-8"))
    paths = sorted((work / "masks").glob("*.png"))
    if len(paths) != request["source"]["frames"]:
        raise RuntimeError(
            f"SAM 3.1 bridge produced {len(paths)} masks, expected {request['source']['frames']}"
        )
    record_stage("SAM 3.1", metadata.get("device", "cuda"), "bf16", elapsed)
    return (
        [probability_from_png(path) for path in paths],
        metadata["version"],
        metadata.get("device", "cuda"),
    )


def model_masks(request, frames, requested_device, output):
    plan = request["plan"]
    semantic_backend = plan["semantic_backend"]
    matte_refiner = plan["matte_refiner"]
    precision = requested_precision(request)
    expected_frames = request["source"]["frames"]
    require_lossless_frames(frames, expected_frames, "model startup")

    if semantic_backend == "sam2":
        probabilities, version, device = cached_sam2(request, frames, requested_device)
        version += f"@{device}"
    elif semantic_backend == "cutie":
        probabilities, version, device = cached_cutie(
            request, frames, requested_device, allow_xpu=True
        )
        version += f"@{device}/{precision}"
    elif semantic_backend == "sam2-cutie":
        cached = load_cutie_stage_cache(
            request, frames, requested_device, guided=True, allow_xpu=False
        )
        if cached is None:
            sam2, sam2_version, sam2_device = cached_sam2(
                request, frames, requested_device
            )
            require_lossless_frames(frames, expected_frames, "Cutie startup")
            # Full-resolution Cutie temporal memory has historically destabilized
            # Level Zero on this workload. Device fallback may change; precision may not.
            cutie, cutie_version, cutie_device = cached_cutie(
                request, frames, requested_device, guides=sam2, allow_xpu=False
            )
            version = (
                f"{sam2_version}@{sam2_device}+{cutie_version}@{cutie_device}/{precision}"
                "+sam2-guided-cutie"
            )
        else:
            cutie, cutie_version, cutie_device = cached
            version = (
                f"sam2-guided-stage-cache+{cutie_version}@{cutie_device}/{precision}"
                "+sam2-guided-cutie"
            )
        cutie = constrain_to_authored_motion_envelope(request, cutie)
        # SAM2 supplies semantic corrections and Cutie owns continuous temporal
        # propagation. Rust independently accepts or rejects this sealed strategy;
        # Python must not silently switch back to an anchor-biased SAM2 sequence.
        probabilities = cutie
    elif semantic_backend == "sam3.1":
        probabilities, version, device = sam31_masks(request, output)
        version += f"@{device}/bf16"
    else:
        raise ValueError(f"unsupported sealed semantic backend: {semantic_backend}")

    probabilities = constrain_to_authored_motion_envelope(request, probabilities)
    role = request["layer"].get("role")
    matte_mode = request["layer"].get("matte_mode", "optical")

    if matte_refiner == "none":
        if role == "writing-surface":
            probabilities = [
                (np.asarray(probability, dtype=np.float32) >= 0.5).astype(np.float32)
                for probability in probabilities
            ]
            matte_version = "categorical-membership-p50"
        elif matte_mode == "opaque":
            # Opaque foreground output is semantic confidence rather than physical
            # alpha. Preserve it losslessly; the Rust compositor owns the scene's
            # support-to-solid calibration policy.
            probabilities = [
                np.asarray(probability, dtype=np.float32).clip(0, 1)
                for probability in probabilities
            ]
            matte_version = "semantic-confidence-u16"
        else:
            matte_version = "semantic-probability"
        print(
            f"ViTMatte: skipped by Rust plan (role={role}, matte={matte_mode})",
            file=sys.stderr,
        )
    elif matte_refiner == "vitmatte":
        require_lossless_frames(frames, expected_frames, "ViTMatte startup")
        probabilities, matte_device = run_component(
            "ViTMatte",
            requested_device,
            lambda candidate: refine_vitmatte(
                request,
                probabilities,
                frames,
                "hustvl/vitmatte-base-composition-1k",
                candidate,
            ),
            precision=precision,
        )
        matte_version = (
            f"vitmatte-{package_version('transformers')}@{matte_device}/{precision}"
        )
    elif matte_refiner == "native":
        # Native alpha is produced by specialist backends (currently MatAnyone2)
        # and therefore never reaches this function.
        raise RuntimeError(
            "native matte refinement reached a non-native semantic executor"
        )
    else:
        raise ValueError(f"unsupported sealed matte refiner: {matte_refiner}")

    probabilities = constrain_to_authored_motion_envelope(request, probabilities)
    probabilities = preserve_authored_prompt_evidence(request, probabilities)
    started = time.monotonic()
    if role == "foreground" and matte_mode == "optical" and matte_refiner != "none":
        probabilities = stabilize_alpha(
            probabilities,
            frames,
            blend_strength=0.20,
            propagation_headroom=0.08,
        )
    elapsed = time.monotonic() - started
    record_stage("Temporal stabilization", "cpu", "fp32", elapsed)
    probabilities = constrain_to_authored_motion_envelope(request, probabilities)
    probabilities = preserve_authored_prompt_evidence(request, probabilities)
    if request["layer"].get("active_frames"):
        active_start, active_end = request["layer"]["active_frames"]
        empty = np.zeros_like(probabilities[0])
        probabilities = [
            probability if active_start <= frame <= active_end else empty.copy()
            for frame, probability in enumerate(probabilities)
        ]
    print(f"Temporal stabilization: {elapsed:.1f}s", file=sys.stderr)
    return probabilities, f"{version}+{matte_version}"


def constrain_to_authored_motion_envelope(request, probabilities):
    """Reject remote foreground leakage using the authored moving-object boxes.

    SAM2 boxes ordinarily describe the complete target object.  When the same
    named foreground object has two or more boxed corrections, their interpolated
    envelope is therefore authoritative spatial evidence.  Automatic discovery
    deliberately gives independent seeds distinct object names, so it is not
    accidentally pinned to one seed location by this authored-object contract.
    """
    layer = request["layer"]
    if layer.get("role") != "foreground":
        return probabilities
    prompts = [prompt for prompt in layer["prompts"] if prompt.get("box_bounds")]
    names = {prompt.get("object") for prompt in prompts}
    if len(prompts) < 2 or len(names) != 1:
        return probabilities

    prompts.sort(key=lambda prompt: prompt["frame"])
    prompt_frames = np.array([p["frame"] for p in prompts], dtype=np.int32)
    prompt_boxes = np.array([p["box_bounds"] for p in prompts], dtype=np.float64)
    height, width = np.asarray(probabilities[0]).shape

    def box_at(frame):
        if frame <= prompt_frames[0]:
            return prompt_boxes[0]
        if frame >= prompt_frames[-1]:
            return prompt_boxes[-1]
        right = int(np.searchsorted(prompt_frames, frame, side="left"))
        left = right - 1
        f_left, f_right = prompt_frames[left], prompt_frames[right]
        span = max(1, f_right - f_left)
        weight = (frame - f_left) / span
        return prompt_boxes[left] * (1.0 - weight) + prompt_boxes[right] * weight

    constrained = []
    for frame, probability in enumerate(probabilities):
        probability = np.asarray(probability, dtype=np.float32).clip(0, 1)
        x, y, box_width, box_height = box_at(frame)
        margin = max(20.0, 0.10 * max(box_width, box_height))
        left = max(0, math.floor(x - margin))
        top = max(0, math.floor(y - margin))
        right = min(width, math.ceil(x + box_width + margin))
        bottom = min(height, math.ceil(y + box_height + margin))
        bounded = np.zeros_like(probability)
        bounded[top:bottom, left:right] = probability[top:bottom, left:right]
        constrained.append(bounded)
    return constrained


def preserve_authored_prompt_evidence(request, probabilities):
    """Keep explicit semantic supervision true through matting and smoothing.

    SAM2 consumes these points before propagation, but a later trimap crop or
    temporal blend can otherwise erase a thin vine/web exactly at its authored
    seed.  A tiny feathered guard is local enough not to invent an object, while
    negative points remain exact background constraints.
    """
    output = [
        np.asarray(value, dtype=np.float32).clip(0, 1).copy() for value in probabilities
    ]
    for prompt in request["layer"]["prompts"]:
        probability = output[prompt["frame"]]
        height, width = probability.shape
        for x, y in prompt.get("positive_points", []):
            x = int(round(np.clip(x, 0, width - 1)))
            y = int(round(np.clip(y, 0, height - 1)))
            cv2.circle(probability, (x, y), 2, 1.0, thickness=-1, lineType=cv2.LINE_AA)
        for x, y in prompt.get("negative_points", []):
            x = int(round(np.clip(x, 0, width - 1)))
            y = int(round(np.clip(y, 0, height - 1)))
            cv2.circle(probability, (x, y), 2, 0.0, thickness=-1, lineType=cv2.LINE_AA)
    return output


def verify_torch_device(device):
    import torch

    tensor = torch.arange(64, dtype=torch.float32, device=device).reshape(8, 8)
    value = (tensor @ tensor.T).mean()
    if not torch.isfinite(value).item():
        raise RuntimeError(f"non-finite PyTorch result on {device}")
    return True


def verify_sam2_device(device):
    with tempfile.TemporaryDirectory(prefix="plaque-forge-sam2-") as directory:
        directory = Path(directory)
        frames = []
        for frame in range(2):
            path = directory / f"{frame:06}.png"
            image = Image.new("RGB", (64, 64), (24, 32, 40))
            draw = ImageDraw.Draw(image)
            draw.rectangle((20 + frame, 20, 44 + frame, 44), fill=(210, 210, 210))
            image.save(path)
            frames.append(path)
        request = {
            "model": "facebook/sam2.1-hiera-large",
            "plan": {"precision": "fp32", "compile": False},
            "source": {"width": 64, "height": 64},
            "layer": {
                "active_frames": [0, 1],
                "prompts": [
                    {
                        "frame": 0,
                        "object": "smoke-a",
                        "positive_points": [[32.0, 32.0]],
                        "negative_points": [[4.0, 4.0]],
                    },
                    {
                        "frame": 1,
                        "object": "smoke-b",
                        "positive_points": [[36.0, 32.0]],
                        "negative_points": [[4.0, 4.0]],
                    },
                ],
            },
        }
        probabilities, _ = sam2_masks(request, frames, device)
        if len(probabilities) != 2 or not any(
            np.any(mask >= 0.5) for mask in probabilities
        ):
            raise RuntimeError("SAM2 smoke test produced no foreground mask")
    return True


def verify_cutie_device(device):
    import torch
    from cutie.inference.inference_core import InferenceCore

    model = load_cutie(device)
    processor = InferenceCore(model, cfg=model.cfg)
    image = torch.zeros((3, 128, 128), dtype=torch.float32, device=device)
    image[:, 32:96, 32:96] = 0.8
    mask = torch.zeros((128, 128), dtype=torch.float32, device=device)
    mask[40:88, 40:88] = 1.0
    with torch.inference_mode():
        probability = processor.step(image, mask, objects=[1])
    if not torch.isfinite(probability).all().item():
        raise RuntimeError("Cutie smoke test produced non-finite values")
    del processor, model, image, mask, probability
    return True


def verify_vitmatte_device(device):
    import torch
    from transformers import VitMatteForImageMatting, VitMatteImageProcessor

    model_name = "hustvl/vitmatte-base-composition-1k"
    revision = model_revision(model_name)
    processor = VitMatteImageProcessor.from_pretrained(
        model_name, local_files_only=True, **revision
    )
    model = (
        VitMatteForImageMatting.from_pretrained(
            model_name, local_files_only=True, **revision
        )
        .to(device)
        .eval()
    )
    image = Image.new("RGB", (64, 64), (90, 100, 110))
    trimap = Image.new("L", (64, 64), 0)
    draw = ImageDraw.Draw(trimap)
    draw.rectangle((12, 12, 52, 52), fill=128)
    draw.rectangle((24, 24, 40, 40), fill=255)
    inputs = processor(images=image, trimaps=trimap, return_tensors="pt")
    inputs = {key: value.to(device) for key, value in inputs.items()}
    with torch.inference_mode():
        alpha = model(**inputs).alphas
    if not torch.isfinite(alpha).all().item():
        raise RuntimeError("ViTMatte smoke test produced non-finite values")
    del model, inputs, alpha
    return True


def verify_runtime():
    import torch
    import sam2  # noqa: F401
    import cutie  # noqa: F401
    import matanyone2  # noqa: F401
    from huggingface_hub import snapshot_download

    print(f"[verify] Python: {sys.version.split()[0]}", file=sys.stderr)
    print(f"[verify] PyTorch: {torch.__version__}", file=sys.stderr)
    print(f"[verify] Intel XPU available: {xpu_available(torch)}", file=sys.stderr)

    # Verification runs offline: these calls prove setup cached every required snapshot.
    for repo_id, revision in MODEL_REVISIONS.items():
        snapshot_download(repo_id=repo_id, revision=revision, local_files_only=True)
        print(f"[verify] cached model: {repo_id}", file=sys.stderr)

    checkpoints = Path(torch.hub.get_dir()) / "checkpoints"
    for filename in ("resnet18-5c106cde.pth", "resnet50-19c8e357.pth"):
        path = checkpoints / filename
        if not path.is_file() or path.stat().st_size == 0:
            raise RuntimeError(f"missing Cutie backbone checkpoint: {path}")
        print(f"[verify] cached backbone: {filename}", file=sys.stderr)

    _, torch_device = run_component("PyTorch smoke", "auto", verify_torch_device)
    _, sam2_device = run_component("SAM 2 smoke", "auto", verify_sam2_device)
    # Cutie is deliberately allowed to try XPU. Unsupported operators trigger the
    # existing automatic fallback to CUDA/CPU rather than disabling XPU forever.
    _, cutie_device = run_component(
        "Cutie smoke", "auto", verify_cutie_device, allow_xpu=True
    )
    _, vitmatte_device = run_component("ViTMatte smoke", "auto", verify_vitmatte_device)

    native = (
        "available"
        if sam2_native_postprocessing_available()
        else "not built (expected on XPU/CPU)"
    )
    print(f"[verify] SAM2 native CUDA cleanup: {native}", file=sys.stderr)
    print(
        f"[verify] runtime OK: torch={torch_device}, sam2={sam2_device}, "
        f"cutie={cutie_device}, vitmatte={vitmatte_device}, MatAnyone2=import/cache-ok",
        file=sys.stderr,
        flush=True,
    )


def write_output(request, output, probabilities, version):
    mask_dir = output
    mask_dir.mkdir(parents=True, exist_ok=True)
    confidences = []
    coverages = []
    soft_edge_pixels = 0
    active_start, active_end = request["layer"].get("active_frames") or [
        0,
        len(probabilities) - 1,
    ]

    def _process_and_save_frame(item):
        frame, probability = item
        prob = np.asarray(probability, dtype=np.float32).clip(0, 1)
        if not active_start <= frame <= active_end:
            prob = np.zeros_like(prob)
        encoded = np.round(prob * 65535).astype(np.uint16)
        validation_alpha = np.round(encoded.astype(np.float32) * 255 / 65535).astype(
            np.uint8
        )
        support = prob[encoded > 0]
        conf = float(np.mean(2.0 * np.abs(support - 0.5))) if support.size else 0.0
        cov = (
            float(np.count_nonzero(validation_alpha > 8) / encoded.size)
            if active_start <= frame <= active_end
            else None
        )
        soft = int(np.count_nonzero((validation_alpha > 0) & (validation_alpha < 255)))
        save_probability_png(mask_dir / f"{frame:06}.png", prob)
        return frame, conf, cov, soft

    max_workers = parallel_worker_count(len(probabilities))
    if max_workers > 1:
        with concurrent.futures.ThreadPoolExecutor(max_workers=max_workers) as pool:
            results = list(pool.map(_process_and_save_frame, enumerate(probabilities)))
    else:
        results = [_process_and_save_frame(item) for item in enumerate(probabilities)]

    results.sort(key=lambda x: x[0])
    for frame, conf, cov, soft in results:
        confidences.append(conf)
        if cov is not None:
            coverages.append(cov)
        soft_edge_pixels += soft
    backend = request["backend"]
    model = request["model"]
    artifact = (
        'format = "plaque-forge.layer/1"\n'
        'kind = "alpha-sequence"\n'
        'coordinates = "source-pixels"\n'
        'pattern = "%06d.png"\n'
        f"first_frame = 0\nlast_frame = {len(probabilities) - 1}\n"
        f"affects_layout = {str(request['layer']['affects_layout']).lower()}\n\n"
        "[generator]\n"
        f"backend = {json.dumps(backend)}\n"
        f"model = {json.dumps(model)}\n"
        f"version = {json.dumps(version)}\n"
        f"requested_device = {json.dumps(request.get('device', 'auto'))}\n"
        f"source_sha256 = {json.dumps(request['source']['sha256'])}\n"
        f"prompt_sha256 = {json.dumps(request['prompt_sha256'])}\n"
        f"worker_sha256 = {json.dumps(request['worker_sha256'])}\n"
        + (
            f"runtime_sha256 = {json.dumps(request['runtime_sha256'])}\n"
            if request.get("runtime_sha256")
            else ""
        )
        + f"request_sha256 = {json.dumps(request['request_sha256'])}\n"
        + f"plan_sha256 = {json.dumps(request['plan_sha256'])}\n"
    )
    (output / "artifact.toml").write_text(artifact, encoding="utf-8")
    result = {
        "format": RESULT_PROTOCOL,
        "backend": backend,
        "model": model,
        "version": version,
        "plan_sha256": request["plan_sha256"],
        "precision": requested_precision(request),
        "frames": len(probabilities),
        "mean_confidence": float(np.mean(confidences)),
        "minimum_confidence": float(np.min(confidences)),
        "request_sha256": request["request_sha256"],
        "source_sha256": request["source"]["sha256"],
        "prompt_sha256": request["prompt_sha256"],
        "worker_sha256": request["worker_sha256"],
        "runtime_sha256": request.get("runtime_sha256"),
        "nonempty_frames": int(sum(coverage > 0 for coverage in coverages)),
        "mean_coverage": float(np.mean(coverages)),
        "maximum_coverage": float(np.max(coverages)),
        "soft_edge_pixels": soft_edge_pixels,
        "execution": list(STAGE_METRICS),
    }
    (output / "result.json").write_text(
        json.dumps(result, indent=2) + "\n", encoding="utf-8"
    )


def remove_owned_directory(root, path):
    root = root.resolve()
    path = path.resolve()
    if path == root or root not in path.parents:
        raise RuntimeError(f"refusing to delete path outside {root}: {path}")
    shutil.rmtree(path)


def process_request(request_path, output_path):
    """Execute one sealed Rust request in the current Python process.

    The persistent service calls this repeatedly. All per-request metrics are reset,
    while imported modules and explicitly cached model objects remain resident.
    """
    STAGE_METRICS.clear()
    request_path = Path(request_path)
    output_path = Path(output_path)
    request = json.loads(request_path.read_text(encoding="utf-8"))
    runtime_log(
        "started",
        backend=request.get("backend"),
        model=request.get("model"),
        requested_device=request.get("device", "auto"),
        profile=request.get("plan", {}).get("profile"),
        precision=request.get("plan", {}).get("precision"),
        source_sha256=request.get("source", {}).get("sha256"),
        source_name=Path(request.get("source", {}).get("path", "source")).name,
    )
    print(
        f"[ml] Python worker active: pid={os.getpid()}, backend={request.get('backend')}, "
        f"model={request.get('model')}, profile={request.get('plan', {}).get('profile')}, "
        f"precision={request.get('plan', {}).get('precision')}, "
        f"requested_device={request.get('device', 'auto')}",
        file=sys.stderr,
        flush=True,
    )
    if request.get("format") != WORKER_PROTOCOL:
        raise ValueError("unsupported worker protocol")
    sealed_plan = request.get("plan") or {}
    if request.get("backend") != backend_label_from_plan(sealed_plan):
        raise ValueError("top-level backend differs from Rust-sealed segmentation plan")
    if request.get("model") != sealed_plan.get("semantic_model"):
        raise ValueError("top-level model differs from Rust-sealed segmentation plan")
    if file_sha256(request["source"]["path"]) != request["source"]["sha256"]:
        raise ValueError("source changed after the segmentation request was created")
    output_path.mkdir(parents=True, exist_ok=True)
    frame_count = request["source"]["frames"]
    size = (request["source"]["width"], request["source"]["height"])
    semantic_backend = sealed_plan.get("semantic_backend")
    requested_device = request.get("device", "auto")
    precision = requested_precision(request)
    if semantic_backend == "matanyone2":
        (probabilities, version), device = run_component(
            "MatAnyone2",
            requested_device,
            lambda candidate: matanyone2_masks(
                request, output_path, size, frame_count, candidate
            ),
            precision=precision,
        )
        version += f"@{device}/{precision}"
    else:
        frames = extract_frames_cached(request)
        require_lossless_frames(frames, frame_count, "model startup")
        probabilities, version = model_masks(
            request, frames, requested_device, output_path
        )
    write_output(request, output_path, probabilities, version)
    runtime_log("completed", frames=len(probabilities), version=version)
    print(
        f"[ml] Python worker completed: pid={os.getpid()}, frames={len(probabilities)}, version={version}",
        file=sys.stderr,
        flush=True,
    )


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--request", type=Path)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--verify-runtime", action="store_true")
    args = parser.parse_args()
    if args.verify_runtime:
        if args.request is not None or args.output is not None:
            parser.error("--verify-runtime cannot be combined with --request/--output")
        verify_runtime()
        return
    if args.request is None or args.output is None:
        parser.error(
            "--request and --output are required unless --verify-runtime is used"
        )
    process_request(args.request, args.output)


if __name__ == "__main__":
    try:
        main()
    except Exception as error:
        runtime_log("failed", error_type=type(error).__name__)
        raise
