#!/usr/bin/env python3
"""Compare two lossless Plaque Forge segmentation-mask sequences.

The comparison is streaming and exact in the 16-bit stored domain. It is intended for
CPU/XPU/CUDA drift checks and model bake-offs; it does not pretend that similarity to
another model is ground-truth visual quality.
"""

from __future__ import annotations

import argparse
import concurrent.futures
import json
import math
import os
from pathlib import Path

import numpy as np
from PIL import Image

try:
    import cv2
except ImportError:
    cv2 = None

MAX_U16 = 65535


def mask_paths(root: Path) -> list[Path]:
    candidates = [root / "masks", root]
    for directory in candidates:
        paths = sorted(directory.glob("*.png"))
        if paths:
            return paths
    raise FileNotFoundError(f"no PNG masks found under {root}")


def read_u16(path: Path) -> np.ndarray:
    if cv2 is not None:
        values = cv2.imread(str(path), cv2.IMREAD_UNCHANGED)
        if values is None:
            values = np.asarray(Image.open(path))
    else:
        values = np.asarray(Image.open(path))
    if values.ndim == 3:
        values = values[..., -1]
    if values.dtype == np.uint8:
        # Accept legacy masks without hiding that their stored precision is lower.
        values = values.astype(np.uint16) * 257
    elif values.dtype != np.uint16:
        values = np.round(np.asarray(values, dtype=np.float64).clip(0, MAX_U16)).astype(
            np.uint16
        )
    return values


def percentile_from_histogram(
    histogram: np.ndarray, quantile: float, total: int
) -> int:
    if total <= 0:
        return 0
    target = max(1, math.ceil(total * quantile))
    return int(
        np.searchsorted(np.cumsum(histogram, dtype=np.uint64), target, side="left")
    )


def _compare_pair(
    pair: tuple[Path, Path],
) -> tuple[np.ndarray, int, int, int, int, int, int, int, int, int]:
    left_path, right_path = pair
    a = read_u16(left_path)
    b = read_u16(right_path)
    if a.shape != b.shape:
        raise ValueError(
            f"shape mismatch at {left_path.name}/{right_path.name}: {a.shape} vs {b.shape}"
        )
    if cv2 is not None:
        diff = cv2.absdiff(a, b)
    else:
        diff = np.abs(a.astype(np.int32) - b.astype(np.int32)).astype(np.uint16)
    counts = np.bincount(diff.ravel(), minlength=MAX_U16 + 1).astype(np.uint64)
    pixels = diff.size
    diff_flat = diff.ravel()
    absolute_sum = int(diff_flat.sum(dtype=np.uint64))
    squared_sum = 0
    # Avoid a full-frame uint64 copy (4x the uint16 diff buffer). Eight MiB
    # chunks bound the temporary while retaining exact uint64 accumulation.
    chunk_elements = (8 * 1024 * 1024) // np.dtype(np.uint64).itemsize
    for start in range(0, diff_flat.size, chunk_elements):
        chunk = diff_flat[start : start + chunk_elements].astype(np.uint64)
        squared_sum += int(np.dot(chunk, chunk))
    maximum = int(diff.max(initial=0))

    a_binary = a >= 32768
    b_binary = b >= 32768
    inter = int(np.count_nonzero(a_binary & b_binary))
    uni = int(np.count_nonzero(a_binary | b_binary))
    dis = uni - inter
    soft_a = int(np.count_nonzero((a > 0) & (a < MAX_U16)))
    soft_b = int(np.count_nonzero((b > 0) & (b < MAX_U16)))

    return (
        counts,
        pixels,
        absolute_sum,
        squared_sum,
        maximum,
        inter,
        uni,
        dis,
        soft_a,
        soft_b,
    )


def compare(left: Path, right: Path) -> dict:
    left_paths = mask_paths(left)
    right_paths = mask_paths(right)
    if len(left_paths) != len(right_paths):
        raise ValueError(
            f"frame-count mismatch: {left} has {len(left_paths)}, {right} has {len(right_paths)}"
        )

    histogram = np.zeros(MAX_U16 + 1, dtype=np.uint64)
    pixels = 0
    absolute_sum = 0
    squared_sum = 0
    maximum = 0
    intersection = 0
    union = 0
    disagreement = 0
    soft_left = 0
    soft_right = 0

    # Bounded parallel: 2 workers regain ~3-4× for 200×4K without OOM.
    # Each worker holds at most one 65k histogram (512 KiB) + diff buffers,
    # so even 200 frames stay bounded. Sequential for tiny frame counts or
    # single-CPU containers.
    try:
        affinity = (
            len(os.sched_getaffinity(0)) if hasattr(os, "sched_getaffinity") else 0
        )
    except OSError:
        affinity = 0
    cpus = affinity if affinity else (os.cpu_count() or 1)
    workers = 1
    if len(left_paths) > 1 and cpus > 1:
        workers = min(2, len(left_paths), cpus)

    if workers <= 1:
        for pair in zip(left_paths, right_paths):
            (
                counts,
                p_count,
                abs_sum,
                sq_sum,
                max_val,
                inter,
                uni,
                dis,
                soft_a,
                soft_b,
            ) = _compare_pair(pair)
            histogram += counts
            pixels += p_count
            absolute_sum += abs_sum
            squared_sum += sq_sum
            maximum = max(maximum, max_val)
            intersection += inter
            union += uni
            disagreement += dis
            soft_left += soft_a
            soft_right += soft_b
    else:
        with concurrent.futures.ThreadPoolExecutor(max_workers=workers) as pool:
            futures = {
                pool.submit(_compare_pair, pair): pair
                for pair in zip(left_paths, right_paths)
            }
            for future in concurrent.futures.as_completed(futures):
                (
                    counts,
                    p_count,
                    abs_sum,
                    sq_sum,
                    max_val,
                    inter,
                    uni,
                    dis,
                    soft_a,
                    soft_b,
                ) = future.result()
                histogram += counts
                pixels += p_count
                absolute_sum += abs_sum
                squared_sum += sq_sum
                maximum = max(maximum, max_val)
                intersection += inter
                union += uni
                disagreement += dis
                soft_left += soft_a
                soft_right += soft_b

    mean_stored = absolute_sum / max(pixels, 1)
    rmse_stored = math.sqrt(squared_sum / max(pixels, 1))
    return {
        "format": "plaque-forge.segmentation-drift/1",
        "left": str(left),
        "right": str(right),
        "frames": len(left_paths),
        "pixels": pixels,
        "alpha": {
            "mean_absolute": mean_stored / MAX_U16,
            "rmse": rmse_stored / MAX_U16,
            "p95_absolute": percentile_from_histogram(histogram, 0.95, pixels)
            / MAX_U16,
            "p99_absolute": percentile_from_histogram(histogram, 0.99, pixels)
            / MAX_U16,
            "maximum_absolute": maximum / MAX_U16,
        },
        "binary_at_0_5": {
            "iou": intersection / union if union else 1.0,
            "disagreement_fraction": disagreement / max(pixels, 1),
        },
        "soft_edge_fraction": {
            "left": soft_left / max(pixels, 1),
            "right": soft_right / max(pixels, 1),
        },
    }


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("left", type=Path)
    parser.add_argument("right", type=Path)
    parser.add_argument("--json", type=Path)
    parser.add_argument("--max-mean-absolute", type=float)
    parser.add_argument("--min-iou", type=float)
    args = parser.parse_args()

    report = compare(args.left, args.right)
    text = json.dumps(report, indent=2, sort_keys=True) + "\n"
    if args.json:
        args.json.parent.mkdir(parents=True, exist_ok=True)
        args.json.write_text(text, encoding="utf-8")
    print(text, end="")

    failed = False
    if (
        args.max_mean_absolute is not None
        and report["alpha"]["mean_absolute"] > args.max_mean_absolute
    ):
        failed = True
    if args.min_iou is not None and report["binary_at_0_5"]["iou"] < args.min_iou:
        failed = True
    raise SystemExit(1 if failed else 0)


if __name__ == "__main__":
    main()
