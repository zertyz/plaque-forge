#!/usr/bin/env python3
import argparse
import gc
import hashlib
import importlib.metadata
import json
import os
import shutil
import subprocess
import sys
import time
import tempfile
import warnings
from contextlib import nullcontext
from pathlib import Path

import cv2
import numpy as np
from PIL import Image, ImageDraw

from segmentation_runtime import (
    MODEL_REVISIONS,
    load_sam2_video_predictor,
    model_revision,
)


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


def run_component(name, requested, operation, allow_xpu=True):
    failures = []
    for device in device_candidates(requested, allow_xpu=allow_xpu):
        try:
            started = time.monotonic()
            result = operation(device)
            release_device(device)
            print(f"{name}: {device}, {time.monotonic() - started:.1f}s", file=sys.stderr)
            return result, device
        except RuntimeError as error:
            release_device(device)
            failures.append(f"{device}: {error}")
            if requested != "auto" or device == "cpu":
                raise
            print(f"{name} fallback after {device} failure: {error}", file=sys.stderr)
    raise RuntimeError("; ".join(failures))


def extract_frames(source, directory):
    directory.mkdir(parents=True, exist_ok=True)
    subprocess.run(
        [
            "ffmpeg",
            "-hide_banner",
            "-loglevel",
            "error",
            "-noautorotate",
            "-i",
            str(source),
            "-start_number",
            "0",
            "-c:v",
            "png",
            "-pix_fmt",
            "rgba",
            "-f",
            "image2",
            "-y",
            str(directory / "%06d.png"),
        ],
        check=True,
    )
    return sorted(directory.glob("*.png"))


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
            raise ValueError(f"seed mask changed after request creation on frame {frame}")
        image = Image.open(seed["path"]).convert("L")
        if image.size != size:
            raise ValueError(f"seed mask dimensions differ on frame {frame}")
        return image
    return None


def probability_from_png(path):
    """Load 8- or 16-bit grayscale PNG without quantizing it through PIL L mode."""
    encoded = np.asarray(Image.open(path))
    if encoded.ndim == 3:
        encoded = encoded[..., 0]
    maximum = 65535.0 if encoded.dtype == np.uint16 or encoded.max(initial=0) > 255 else 255.0
    return encoded.astype(np.float32) / maximum


def save_probability_png(path, probability):
    encoded = np.round(np.asarray(probability, dtype=np.float32).clip(0, 1) * 65535).astype(
        np.uint16
    )
    Image.fromarray(encoded).save(path, format="PNG", compress_level=6)


def seed_mask(request, frame, size):
    exact = exact_seed_mask(request, frame, size)
    if exact is not None:
        return exact
    mask = Image.new("L", size, 0)
    for prompt in request["layer"]["prompts"]:
        if prompt["frame"] == frame:
            mask = Image.fromarray(
                np.maximum(np.asarray(mask), np.asarray(prompt_shape(prompt, size))).astype(np.uint8)
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

    cleaned = binary.copy()
    count, labels, stats, _ = cv2.connectedComponentsWithStats(binary, connectivity=8)
    for component in range(1, count):
        if int(stats[component, cv2.CC_STAT_AREA]) <= max_area:
            cleaned[labels == component] = 0

    result = probability.copy()
    removed = (binary == 1) & (cleaned == 0)
    result[removed] = 0.0
    return result


def sam2_masks(request, frames, device):
    import torch
    from sam2 import sam2_video_predictor as predictor_module
    from sam2.utils import misc as sam2_misc

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
            raise ValueError("lossless PNG loading does not support asynchronous loading")
        images = torch.zeros(
            len(png_paths), 3, image_size, image_size, dtype=torch.float32
        )
        video_height = video_width = 0
        for index, path in enumerate(png_paths):
            images[index], video_height, video_width = sam2_misc._load_img_as_tensor(
                str(path), image_size
            )
        mean = torch.tensor(img_mean, dtype=torch.float32)[:, None, None]
        std = torch.tensor(img_std, dtype=torch.float32)[:, None, None]
        if not offload_video_to_cpu:
            images = images.to(compute_device)
            mean = mean.to(compute_device)
            std = std.to(compute_device)
        images -= mean
        images /= std
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
    predictor = load_sam2_video_predictor(request["model"], device)

    # SAM2 unconditionally compresses its temporal memory to BF16 before storing
    # it. That is appropriate under the documented accelerator autocast path, but
    # CPU inference otherwise feeds BF16 memory into FP32 projection weights and
    # fails in memory attention. Keep the CPU fallback genuinely full precision by
    # restoring stored memory tensors to FP32 at the two points where upstream
    # performs that compression. This is intentionally instance-local: accelerator
    # predictors retain upstream's efficient BF16 memory contract.
    if device.startswith("cpu"):
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

    state = predictor.init_state(video_path=str(frames[0].parent), offload_video_to_cpu=True)
    prompts = request["layer"]["prompts"]
    active_start, active_end = request["layer"].get("active_frames") or [0, len(frames) - 1]
    probabilities = [None] * len(frames)
    first = min(prompt["frame"] for prompt in prompts)
    last = max(prompt["frame"] for prompt in prompts)
    objects = {}
    # Upstream SAM2 stores accelerator video memory in BF16. Its documented CUDA
    # inference path uses BF16 autocast; the same contract is required on Intel XPU
    # once more than one object exercises memory attention. CPU stays full FP32 via
    # the instance-local storage adapters above.
    autocast = (
        torch.autocast(device_type=device.split(":", 1)[0], dtype=torch.bfloat16)
        if device.startswith(("xpu", "cuda"))
        else nullcontext()
    )
    with torch.inference_mode(), autocast:
        for prompt in prompts:
            object_name = prompt.get("object") or "default"
            object_id = objects.setdefault(object_name, len(objects) + 1)
            kwargs = {
                "inference_state": state,
                "frame_idx": prompt["frame"],
                "obj_id": object_id,
            }
            exact = exact_seed_mask(request, prompt["frame"], Image.open(frames[0]).size)
            polygon = prompt.get("polygon") or prompt.get("quad")
            if exact is not None:
                predictor.add_new_mask(
                    **kwargs,
                    mask=torch.from_numpy(np.asarray(exact, dtype=np.uint8) > 24).to(device),
                )
            elif polygon:
                mask = prompt_shape(prompt, Image.open(frames[0]).size)
                predictor.add_new_mask(
                    **kwargs,
                    mask=torch.from_numpy(np.asarray(mask, dtype=np.uint8) > 0).to(device),
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
                probability = logits.sigmoid().amax(dim=0).squeeze().float().cpu().numpy()
                probabilities[frame] = (
                    probability
                    if probabilities[frame] is None
                    else np.maximum(probabilities[frame], probability)
                )
    if any(probabilities[frame] is None for frame in range(active_start, active_end + 1)):
        raise RuntimeError("SAM 2 did not produce every source frame")
    empty = np.zeros((request["source"]["height"], request["source"]["width"]), dtype=np.float32)
    probabilities = [empty.copy() if probability is None else probability for probability in probabilities]
    if not native_postprocessing:
        probabilities = [cleanup_small_mask_defects(probability) for probability in probabilities]
    return probabilities, f"sam2-{package_version('sam-2', 'sam2')}"


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
    model = CUTIE(cfg).to(device).eval()
    model.load_weights(torch.load(weights, map_location="cpu", weights_only=False))
    return model


def cutie_masks(request, frames, device, guides=None):
    import torch
    from cutie.inference.inference_core import InferenceCore
    from torchvision.transforms.functional import to_tensor

    prompts = request["layer"]["prompts"]
    active_start, active_end = request["layer"].get("active_frames") or [0, len(frames) - 1]
    if device == "cpu":
        torch.set_num_threads(min(8, len(os.sched_getaffinity(0))))
    seed = min(prompt["frame"] for prompt in prompts)
    prompt_frames = {prompt["frame"] for prompt in prompts}
    source_width, source_height = Image.open(frames[0]).size

    model = load_cutie(device)

    def propagate(indices):
        processor = InferenceCore(model, cfg=model.cfg)
        output = {}
        with torch.inference_mode():
            for position, frame in enumerate(indices):
                image = to_tensor(Image.open(frames[frame]).convert("RGB")).to(device).float()
                correction = position == 0 or frame in prompt_frames
                if correction:
                    source = (
                        guides[frame] >= 0.5
                        if guides is not None
                        else np.asarray(seed_mask(request, frame, (source_width, source_height))) > 0
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
                output[frame] = probability.float().clamp(0, 1).detach().cpu().numpy()
        return output

    masks = propagate(range(seed, active_end + 1))
    masks.update(propagate(range(seed, active_start - 1, -1)))
    empty = np.zeros((source_height, source_width), dtype=np.float32)
    return [masks.get(frame, empty).astype(np.float32) for frame in range(len(frames))], (
        f"cutie-{package_version('cutie')}"
    )


def refine_vitmatte(probabilities, frames, model_name, device):
    import torch
    from transformers import VitMatteForImageMatting, VitMatteImageProcessor

    revision = model_revision(model_name)
    processor = VitMatteImageProcessor.from_pretrained(model_name, **revision)
    model = VitMatteForImageMatting.from_pretrained(model_name, **revision).to(device).eval()
    output = []
    for probability, frame_path in zip(probabilities, frames):
        probability = np.asarray(probability, dtype=np.float32).clip(0, 1)
        kernel = np.ones((3, 3), np.uint8)
        foreground = cv2.erode((probability >= 0.82).astype(np.uint8), kernel)
        if not foreground.any():
            foreground = (probability >= 0.60).astype(np.uint8)
        # SAM2 probabilities below roughly 0.35 are semantic uncertainty, not
        # evidence that a pixel may be opaque. The former 0.08 support threshold
        # often admitted most of a frame into ViTMatte's unknown trimap, allowing
        # dark background/actors to become false solid occluders. A modest dilation
        # around calibrated semantic support still leaves room for translucent
        # hair, vines, and web strands without filling their holes.
        support = cv2.dilate((probability >= 0.35).astype(np.uint8), kernel, iterations=3)
        trimap = np.zeros(probability.shape, dtype=np.uint8)
        trimap[support > 0] = 128
        trimap[foreground > 0] = 255
        image = Image.open(frame_path).convert("RGB")
        active = cv2.findNonZero(support)
        if active is None:
            output.append(np.zeros(probability.shape, dtype=np.float32))
            continue
        x, y, width, height = cv2.boundingRect(active)
        margin = 32
        left = max(0, x - margin)
        top = max(0, y - margin)
        right = min(image.width, x + width + margin)
        bottom = min(image.height, y + height + margin)
        crop = image.crop((left, top, right, bottom))
        crop_trimap = Image.fromarray(trimap[top:bottom, left:right])
        inputs = processor(images=crop, trimaps=crop_trimap, return_tensors="pt")
        inputs = {key: value.to(device) for key, value in inputs.items()}
        with torch.inference_mode():
            alpha = model(**inputs).alphas[0, 0].clamp(0, 1).cpu().numpy()
        alpha = alpha[: crop.height, : crop.width]
        guard = cv2.GaussianBlur(support.astype(np.float32), (0, 0), 2.0).clip(0, 1)
        matte = np.zeros(probability.shape, dtype=np.float32)
        matte[top:bottom, left:right] = alpha
        output.append((matte * guard).astype(np.float32))
    return output


def dense_flow(source, target):
    # Half-resolution variational flow preserves narrow vines/webs far better than
    # the former quarter-resolution FAST preset while keeping this post-pass bounded.
    scale = 0.5
    size = (max(1, round(source.shape[1] * scale)), max(1, round(source.shape[0] * scale)))
    source_small = cv2.resize(source, size, interpolation=cv2.INTER_AREA)
    target_small = cv2.resize(target, size, interpolation=cv2.INTER_AREA)
    estimator = cv2.DISOpticalFlow_create(cv2.DISOPTICAL_FLOW_PRESET_MEDIUM)
    flow = estimator.calc(source_small, target_small, None)
    flow = cv2.resize(flow, (source.shape[1], source.shape[0]), interpolation=cv2.INTER_LINEAR)
    flow[..., 0] /= scale
    flow[..., 1] /= scale
    return flow


def warp_alpha(alpha, source_gray, target_gray, forward, backward):
    y, x = np.mgrid[: target_gray.shape[0], : target_gray.shape[1]].astype(np.float32)
    map_x = x + backward[..., 0]
    map_y = y + backward[..., 1]
    warped = cv2.remap(alpha, map_x, map_y, cv2.INTER_LINEAR, borderMode=cv2.BORDER_CONSTANT)
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


def stabilize_alpha(probabilities, frames):
    if len(probabilities) < 2:
        return probabilities
    original = [np.asarray(value, dtype=np.float32).clip(0, 1) for value in probabilities]
    active = [frame for frame, value in enumerate(original) if np.any(value >= 0.01)]
    if not active:
        return original
    start = max(0, active[0] - 1)
    end = min(len(original) - 1, active[-1] + 1)
    segment = original[start : end + 1]
    gray = [cv2.imread(str(path), cv2.IMREAD_GRAYSCALE) for path in frames[start : end + 1]]
    flows = [
        (dense_flow(gray[frame], gray[frame + 1]), dense_flow(gray[frame + 1], gray[frame]))
        for frame in range(len(gray) - 1)
    ]
    forward = [segment[0]]
    for frame in range(1, len(segment)):
        flow, backward_flow = flows[frame - 1]
        warped, weight = warp_alpha(
            forward[-1], gray[frame - 1], gray[frame], flow, backward_flow
        )
        blend = 0.32 * weight
        forward.append(segment[frame] * (1 - blend) + warped * blend)
    backward = [None] * len(segment)
    backward[-1] = segment[-1]
    for frame in range(len(segment) - 2, -1, -1):
        forward_flow, flow = flows[frame]
        warped, weight = warp_alpha(
            backward[frame + 1], gray[frame + 1], gray[frame], flow, forward_flow
        )
        blend = 0.32 * weight
        backward[frame] = segment[frame] * (1 - blend) + warped * blend
    smoothed = [
        (0.50 * current + 0.25 * before + 0.25 * after).clip(0, 1).astype(np.float32)
        for current, before, after in zip(segment, forward, backward)
    ]
    output = original.copy()
    output[start : end + 1] = smoothed
    return output


def matanyone2_masks(request, output, size, frames, device):
    import torch
    import matanyone2.model.matanyone2 as model_module
    from matanyone2 import InferenceCore, MatAnyone2

    prompts = request["layer"]["prompts"]
    if min(prompt["frame"] for prompt in prompts) != 0:
        raise ValueError("MatAnyone2 requires a frame-0 area seed")
    seed = seed_mask(request, 0, size)
    seed_path = output / "seed.png"
    seed.save(seed_path)
    model_module.device = torch.device(device)
    model = MatAnyone2.from_pretrained(
        request["model"], **model_revision(request["model"])
    ).to(device).eval()
    processor = InferenceCore(model, device=device)
    work = output / "matanyone2"
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
        raise RuntimeError(f"MatAnyone2 produced {len(alpha_paths)} masks, expected {frames}")
    return [np.asarray(Image.open(path).convert("L"), dtype=np.float32) / 255 for path in alpha_paths], (
        f"matanyone2-{package_version('matanyone2')}"
    )


def sam2_cache_key(request):
    identity = {
        "model": request["model"],
        "source_sha256": request["source"]["sha256"],
        "frames": request["source"]["frames"],
        "prompts": request["layer"]["prompts"],
    }
    return hashlib.sha256(json.dumps(identity, sort_keys=True).encode()).hexdigest()


def cached_sam2(request, frames, requested_device, cache_root):
    metadata_path = cache_root / "sam2.json"
    mask_root = cache_root / "sam2"
    if metadata_path.is_file():
        metadata = json.loads(metadata_path.read_text(encoding="utf-8"))
        paths = sorted(mask_root.glob("*.png"))
        if metadata.get("key") == sam2_cache_key(request) and len(paths) == len(frames):
            probabilities = [
                probability_from_png(path) for path in paths
            ]
            print("SAM 2: checkpoint", file=sys.stderr)
            return probabilities, metadata["version"], metadata["device"]

    (probabilities, version), device = run_component(
        "SAM 2", requested_device, lambda candidate: sam2_masks(request, frames, candidate)
    )
    mask_root.mkdir(parents=True, exist_ok=True)
    for frame, probability in enumerate(probabilities):
        save_probability_png(mask_root / f"{frame:06}.png", probability)
    metadata_path.write_text(
        json.dumps(
            {"key": sam2_cache_key(request), "version": version, "device": device}, indent=2
        )
        + "\n",
        encoding="utf-8",
    )
    return probabilities, version, device


def model_masks(request, frames, requested_device, cache_root):
    backend = request["backend"]
    expected_frames = request["source"]["frames"]
    require_lossless_frames(frames, expected_frames, "model startup")
    if backend == "sam2-vitmatte":
        probabilities, version, device = cached_sam2(
            request, frames, requested_device, cache_root
        )
        version += f"@{device}"
    elif backend == "cutie-vitmatte":
        (probabilities, version), device = run_component(
            "Cutie",
            requested_device,
            lambda candidate: cutie_masks(request, frames, candidate),
            allow_xpu=True,
        )
        version += f"@{device}"
    elif backend == "sam2-cutie-vitmatte":
        sam2, sam2_version, sam2_device = cached_sam2(
            request, frames, requested_device, cache_root
        )
        require_lossless_frames(frames, expected_frames, "Cutie startup")
        (cutie, cutie_version), cutie_device = run_component(
            "Cutie",
            requested_device,
            # SAM2 turns sparse points/boxes into semantic per-frame guides.  Feeding
            # those guides into Cutie is materially better than asking Cutie to track
            # the coarse prompt boxes themselves: it preserves disconnected objects,
            # corrects every authored prompt frame, and still lets the independent
            # sequence scorer choose SAM2 when Cutie's temporal memory is worse.
            lambda candidate: cutie_masks(request, frames, candidate, guides=sam2),
            # Short XPU smoke tests pass, but full-resolution temporal memory can
            # terminate the Level Zero device. CPU runs the identical weights and
            # arithmetic contract without first wasting work on a doomed XPU pass.
            allow_xpu=False,
        )
        # Repeated boxes for one authored foreground object are a spatial contract,
        # not merely loose hints.  Constrain both independent candidates before
        # scoring so a temporally stable background leak cannot win selection.
        sam2 = constrain_to_authored_motion_envelope(request, sam2)
        cutie = constrain_to_authored_motion_envelope(request, cutie)
        probabilities, selection = select_sequence_candidate(request, frames, sam2, cutie)
        version = f"{sam2_version}@{sam2_device}+{cutie_version}@{cutie_device}"
        version += f"+selected-{selection}"
    else:
        raise ValueError(f"unsupported backend: {backend}")
    probabilities = constrain_to_authored_motion_envelope(request, probabilities)
    role = request["layer"].get("role")
    if role == "writing-surface":
        # A writing-surface mask represents categorical material membership, not
        # optical opacity. Feeding an opaque, low-contrast iron/cloud face through
        # an image-matting model can legitimately return a near-zero alpha even
        # though SAM2 identified the semantic object correctly. Preserve the
        # strongest selected semantic sequence for tracking/depth support; soft
        # alpha matting remains mandatory for foreground occluders below.
        probabilities = [
            (np.asarray(probability, dtype=np.float32) >= 0.5).astype(np.float32)
            for probability in probabilities
        ]
        matte_version = "categorical-material-support-p50"
        print("ViTMatte: skipped for categorical writing-surface membership", file=sys.stderr)
    else:
        require_lossless_frames(frames, expected_frames, "ViTMatte startup")
        probabilities, matte_device = run_component(
            "ViTMatte",
            requested_device,
            lambda candidate: refine_vitmatte(
                probabilities, frames, "hustvl/vitmatte-base-composition-1k", candidate
            ),
        )
        matte_version = f"vitmatte-{package_version('transformers')}@{matte_device}"
    probabilities = constrain_to_authored_motion_envelope(request, probabilities)
    probabilities = preserve_authored_prompt_evidence(request, probabilities)
    require_lossless_frames(frames, expected_frames, "temporal stabilization startup")
    started = time.monotonic()
    # Semantic material membership is already produced independently for every
    # frame by SAM2/Cutie. Alpha blending it across time would blur boundaries and
    # reintroduce non-membership. Optical alpha stabilization is for foreground
    # translucency only.
    if role != "writing-surface":
        probabilities = stabilize_alpha(probabilities, frames)
    probabilities = constrain_to_authored_motion_envelope(request, probabilities)
    probabilities = preserve_authored_prompt_evidence(request, probabilities)
    if request["layer"].get("active_frames"):
        active_start, active_end = request["layer"]["active_frames"]
        empty = np.zeros_like(probabilities[0])
        probabilities = [
            probability if active_start <= frame <= active_end else empty.copy()
            for frame, probability in enumerate(probabilities)
        ]
    print(f"Temporal stabilization: {time.monotonic() - started:.1f}s", file=sys.stderr)
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
    height, width = np.asarray(probabilities[0]).shape

    def box_at(frame):
        if frame <= prompts[0]["frame"]:
            return np.asarray(prompts[0]["box_bounds"], dtype=np.float64)
        if frame >= prompts[-1]["frame"]:
            return np.asarray(prompts[-1]["box_bounds"], dtype=np.float64)
        right = next(index for index, prompt in enumerate(prompts) if prompt["frame"] >= frame)
        left = right - 1
        before = prompts[left]
        after = prompts[right]
        span = max(1, after["frame"] - before["frame"])
        weight = (frame - before["frame"]) / span
        return (
            np.asarray(before["box_bounds"], dtype=np.float64) * (1.0 - weight)
            + np.asarray(after["box_bounds"], dtype=np.float64) * weight
        )

    constrained = []
    for frame, probability in enumerate(probabilities):
        probability = np.asarray(probability, dtype=np.float32).clip(0, 1)
        x, y, box_width, box_height = box_at(frame)
        margin = max(20.0, 0.10 * max(box_width, box_height))
        left = max(0, int(np.floor(x - margin)))
        top = max(0, int(np.floor(y - margin)))
        right = min(width, int(np.ceil(x + box_width + margin)))
        bottom = min(height, int(np.ceil(y + box_height + margin)))
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
    output = [np.asarray(value, dtype=np.float32).clip(0, 1).copy() for value in probabilities]
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


def sequence_candidate_score(request, frames, probabilities):
    """Score a complete candidate independently; never average incompatible masks."""
    active_start, active_end = request["layer"].get("active_frames") or [0, len(frames) - 1]
    prompt_score = 0.0
    prompt_count = 0
    for prompt in request["layer"]["prompts"]:
        probability = probabilities[prompt["frame"]]
        height, width = probability.shape
        for label, points in ((1.0, prompt.get("positive_points", [])), (0.0, prompt.get("negative_points", []))):
            for x, y in points:
                x = int(round(np.clip(x, 0, width - 1)))
                y = int(round(np.clip(y, 0, height - 1)))
                prompt_score += 1.0 - abs(float(probability[y, x]) - label)
                prompt_count += 1
        seed = exact_seed_mask(request, prompt["frame"], (width, height))
        if seed is not None:
            region = np.asarray(seed) > 24
            if region.any():
                prompt_score += float(np.mean(probability[region]))
                prompt_count += 1

    prompt_score = prompt_score / max(prompt_count, 1)
    temporal_scores = []
    gray = [cv2.imread(str(path), cv2.IMREAD_GRAYSCALE) for path in frames]
    stride = max(1, (active_end - active_start + 1) // 24)
    for frame in range(active_start, active_end, stride):
        target = min(active_end, frame + stride)
        if target == frame:
            continue
        forward = dense_flow(gray[frame], gray[target])
        backward = dense_flow(gray[target], gray[frame])
        warped, confidence = warp_alpha(
            probabilities[frame], gray[frame], gray[target], forward, backward
        )
        reliable = confidence > 0.35
        if reliable.any():
            temporal_scores.append(
                1.0 - float(np.mean(np.abs(warped[reliable] - probabilities[target][reliable])))
            )
    temporal_score = float(np.mean(temporal_scores)) if temporal_scores else 0.0
    areas = np.asarray(
        [np.mean(probabilities[frame] > 0.08) for frame in range(active_start, active_end + 1)]
    )
    area_stability = float(np.exp(-np.std(np.diff(areas)) / max(np.mean(areas), 1e-4)))
    return 0.55 * prompt_score + 0.35 * temporal_score + 0.10 * area_stability


def select_sequence_candidate(request, frames, sam2, cutie):
    candidates = {"sam2": sam2, "cutie": cutie}
    scores = {
        name: sequence_candidate_score(request, frames, values)
        for name, values in candidates.items()
    }
    selected = max(scores, key=scores.get)
    print(
        "Candidate selection: "
        + ", ".join(f"{name}={score:.4f}" for name, score in scores.items())
        + f", selected={selected}",
        file=sys.stderr,
    )
    return candidates[selected], selected


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
        if len(probabilities) != 2 or not any(np.any(mask >= 0.5) for mask in probabilities):
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
    model = VitMatteForImageMatting.from_pretrained(
        model_name, local_files_only=True, **revision
    ).to(device).eval()
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
    _, cutie_device = run_component("Cutie smoke", "auto", verify_cutie_device, allow_xpu=True)
    _, vitmatte_device = run_component("ViTMatte smoke", "auto", verify_vitmatte_device)

    native = "available" if sam2_native_postprocessing_available() else "not built (expected on XPU/CPU)"
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
    for frame, probability in enumerate(probabilities):
        probability = np.asarray(probability, dtype=np.float32).clip(0, 1)
        if not active_start <= frame <= active_end:
            probability = np.zeros_like(probability)
        encoded = np.round(probability * 65535).astype(np.uint16)
        validation_alpha = np.round(encoded.astype(np.float64) * 255 / 65535).astype(np.uint8)
        support = probability[encoded > 0]
        confidences.append(
            float(np.mean(2.0 * np.abs(support - 0.5))) if support.size else 0.0
        )
        if active_start <= frame <= active_end:
            coverages.append(float(np.count_nonzero(validation_alpha > 8) / encoded.size))
        soft_edge_pixels += int(
            np.count_nonzero((validation_alpha > 0) & (validation_alpha < 255))
        )
        save_probability_png(mask_dir / f"{frame:06}.png", probability)
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
    )
    (output / "artifact.toml").write_text(artifact, encoding="utf-8")
    result = {
        "format": "plaque-forge.segmentation-result/1",
        "backend": backend,
        "model": model,
        "version": version,
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
    }
    (output / "result.json").write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")


def remove_owned_directory(root, path):
    root = root.resolve()
    path = path.resolve()
    if path == root or root not in path.parents:
        raise RuntimeError(f"refusing to delete path outside {root}: {path}")
    shutil.rmtree(path)


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
        parser.error("--request and --output are required unless --verify-runtime is used")
    request = json.loads(args.request.read_text(encoding="utf-8"))
    runtime_log(
        "started",
        backend=request.get("backend"),
        model=request.get("model"),
        requested_device=request.get("device", "auto"),
        source_sha256=request.get("source", {}).get("sha256"),
        source_name=Path(request.get("source", {}).get("path", "source")).name,
    )
    print(
        f"[ml] Python worker active: pid={os.getpid()}, backend={request.get('backend')}, "
        f"model={request.get('model')}, requested_device={request.get('device', 'auto')}",
        file=sys.stderr,
        flush=True,
    )
    if request.get("format") != "plaque-forge.segmentation-request/1":
        raise ValueError("unsupported worker protocol")
    if file_sha256(request["source"]["path"]) != request["source"]["sha256"]:
        raise ValueError("source changed after the segmentation request was created")
    args.output.mkdir(parents=True, exist_ok=True)
    backend = request["backend"]
    frame_count = request["source"]["frames"]
    size = (request["source"]["width"], request["source"]["height"])
    frame_dir = args.output / "frames"
    frames = None
    if backend != "matanyone2":
        frames = extract_frames(request["source"]["path"], frame_dir)
        if len(frames) != frame_count:
            raise RuntimeError(f"decoded {len(frames)} frames, expected {frame_count}")

    requested_device = request.get("device", "auto")
    if backend == "matanyone2":
        (probabilities, version), device = run_component(
            "MatAnyone2",
            requested_device,
            lambda candidate: matanyone2_masks(
                request, args.output, size, frame_count, candidate
            ),
        )
        version += f"@{device}"
    else:
        probabilities, version = model_masks(
            request, frames, requested_device, args.output / ".worker-cache"
        )

    if frame_dir.exists():
        remove_owned_directory(args.output, frame_dir)
    write_output(request, args.output, probabilities, version)
    runtime_log("completed", frames=len(probabilities), version=version)
    print(
        f"[ml] Python worker completed: pid={os.getpid()}, frames={len(probabilities)}, version={version}",
        file=sys.stderr,
        flush=True,
    )
    cache_root = args.output / ".worker-cache"
    if cache_root.exists():
        remove_owned_directory(args.output, cache_root)


if __name__ == "__main__":
    try:
        main()
    except Exception as error:
        runtime_log("failed", error_type=type(error).__name__)
        raise
