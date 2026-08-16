#!/usr/bin/env python3
"""Isolated SAM 3.1 executor for Plaque Forge.

SAM 3.1 currently has a materially different Python/CUDA dependency contract from
Plaque Forge's normal XPU/CPU segmentation runtime.  This bridge keeps that stack
isolated while preserving the same lossless 16-bit mask boundary.
"""

import argparse
import json
import os
from pathlib import Path

try:
    import numpy as np
    from PIL import Image
except ImportError:
    np = None
    Image = None


def save_probability_png(path: Path, probability) -> None:
    encoded = np.round(np.asarray(probability, dtype=np.float32).clip(0, 1) * 65535).astype(
        np.uint16
    )
    Image.fromarray(encoded).save(path, format="PNG", compress_level=6)


def union_output_masks(outputs, shape):
    masks = np.asarray(outputs.get("out_binary_masks", []))
    if masks.size == 0:
        return np.zeros(shape, dtype=np.float32)
    if masks.ndim == 2:
        masks = masks[None, ...]
    return np.max(masks.astype(np.float32), axis=0).clip(0, 1)


def prompt_request(prompt, session_id, torch, width, height):
    request = {
        "type": "add_prompt",
        "session_id": session_id,
        "frame_index": int(prompt["frame"]),
    }
    concept = (prompt.get("concept") or "").strip()
    if concept:
        request["text"] = concept
        return request

    positive = list(prompt.get("positive_points") or [])
    negative = list(prompt.get("negative_points") or [])
    if not positive:
        raise ValueError(
            "SAM 3.1 point mode requires at least one positive point; "
            "use a text concept for open-vocabulary dense tracking"
        )
    points = positive + negative
    relative = [[float(x) / width, float(y) / height] for x, y in points]
    request["points"] = torch.tensor(relative, dtype=torch.float32)
    request["point_labels"] = torch.tensor(
        [1] * len(positive) + [0] * len(negative), dtype=torch.int32
    )
    # The current Plaque Forge bridge accepts exactly one authored prompt, so a
    # stable object id of 1 is sufficient for point-seeded tracking.
    request["obj_id"] = 1
    return request


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--request", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()

    request = json.loads(args.request.read_text(encoding="utf-8"))
    plan = request.get("plan") or {}
    if plan.get("semantic_backend") != "sam3.1":
        raise ValueError("SAM 3.1 bridge received a non-SAM3.1 plan")
    if plan.get("precision") != "bf16":
        raise ValueError("SAM 3.1 bridge currently accepts only the sealed BF16 policy")

    import torch

    if not torch.cuda.is_available():
        raise RuntimeError(
            "SAM 3.1 requires the isolated CUDA runtime; no supported CUDA device is available"
        )

    from sam3.model_builder import build_sam3_multiplex_video_predictor

    root = Path(os.environ.get("PLAQUE_FORGE_SAM31_ROOT", "/tmp/plaque-forge-sam31"))
    manifest = json.loads((root / "runtime-manifest.json").read_text(encoding="utf-8"))
    checkpoint = root / manifest["checkpoint_relative"]
    if not checkpoint.is_file():
        raise RuntimeError(f"SAM 3.1 checkpoint from runtime manifest is missing: {checkpoint}")
    import hashlib

    digest = hashlib.sha256()
    with checkpoint.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    if digest.hexdigest() != manifest.get("checkpoint_sha256"):
        raise RuntimeError("SAM 3.1 checkpoint SHA-256 no longer matches runtime manifest")
    predictor = build_sam3_multiplex_video_predictor(
        checkpoint_path=str(checkpoint),
        compile=bool(plan.get("compile", False)),
        use_fa3=False,
        # Keep the baseline install simple/reproducible; FA3 can be benchmarked later.
    )

    frame_count = int(request["source"]["frames"])
    height = int(request["source"]["height"])
    width = int(request["source"]["width"])
    probabilities = [np.zeros((height, width), dtype=np.float32) for _ in range(frame_count)]

    response = predictor.handle_request(
        {
            "type": "start_session",
            "resource_path": request["source"]["path"],
            "offload_video_to_cpu": True,
        }
    )
    session_id = response["session_id"]
    try:
        prompts = request["layer"]["prompts"]
        if len(prompts) != 1:
            raise ValueError("SAM 3.1 bridge expects exactly one authored prompt")
        for prompt in prompts:
            response = predictor.handle_request(
                prompt_request(prompt, session_id, torch, width, height)
            )
            frame = int(response["frame_index"])
            probabilities[frame] = np.maximum(
                probabilities[frame], union_output_masks(response["outputs"], (height, width))
            )

        for response in predictor.handle_stream_request(
            {
                "type": "propagate_in_video",
                "session_id": session_id,
            }
        ):
            frame = int(response["frame_index"])
            if 0 <= frame < frame_count:
                probabilities[frame] = np.maximum(
                    probabilities[frame],
                    union_output_masks(response["outputs"], (height, width)),
                )
    finally:
        predictor.handle_request(
            {"type": "close_session", "session_id": session_id, "run_gc_collect": True}
        )

    masks = args.output / "masks"
    masks.mkdir(parents=True, exist_ok=True)
    for frame, probability in enumerate(probabilities):
        save_probability_png(masks / f"{frame:06}.png", probability)

    (args.output / "result.json").write_text(
        json.dumps(
            {
                "version": "sam3.1-multiplex",
                "device": f"cuda:{torch.cuda.current_device()}",
                "frames": frame_count,
            },
            indent=2,
        )
        + "\n",
        encoding="utf-8",
    )


if __name__ == "__main__":
    main()
