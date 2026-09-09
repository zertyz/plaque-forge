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
    left_by_name = {p.name: p for p in left_paths}
    right_by_name = {p.name: p for p in right_paths}
    common_names = sorted(set(left_by_name.keys()) & set(right_by_name.keys()))
    if common_names:
        pairs = [(left_by_name[name], right_by_name[name]) for name in common_names]
    elif len(left_paths) == len(right_paths):
        pairs = list(zip(left_paths, right_paths))
    else:
        raise ValueError(
            f"frame-count mismatch and no common filenames: {left} has {len(left_paths)}, {right} has {len(right_paths)}"
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
    if len(pairs) > 1 and cpus > 1:
        workers = min(2, len(pairs), cpus)

    if workers <= 1:
        for pair in pairs:
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
                for pair in pairs
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
        "frames": len(pairs),
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


def check_acceptance(
    report: dict,
    min_iou: float | None = None,
    max_mean_absolute: float | None = None,
    max_disagreement_fraction: float | None = None,
    max_p95_absolute: float | None = None,
) -> list[str]:
    failures = []
    if min_iou is not None:
        iou = float(report.get("binary_at_0_5", {}).get("iou", 0.0))
        if iou < min_iou:
            failures.append(f"binary_at_0_5.iou {iou:.4f} < threshold {min_iou:.4f}")
    if max_mean_absolute is not None:
        mae = float(report.get("alpha", {}).get("mean_absolute", 0.0))
        if mae > max_mean_absolute:
            failures.append(f"alpha.mean_absolute {mae:.4f} > threshold {max_mean_absolute:.4f}")
    if max_disagreement_fraction is not None:
        dis = float(report.get("binary_at_0_5", {}).get("disagreement_fraction", 0.0))
        if dis > max_disagreement_fraction:
            failures.append(f"binary_at_0_5.disagreement_fraction {dis:.4f} > threshold {max_disagreement_fraction:.4f}")
    if max_p95_absolute is not None:
        p95 = float(report.get("alpha", {}).get("p95_absolute", 0.0))
        if p95 > max_p95_absolute:
            failures.append(f"alpha.p95_absolute {p95:.4f} > threshold {max_p95_absolute:.4f}")
    return failures


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("left", type=Path)
    parser.add_argument("right", type=Path)
    parser.add_argument("--json", type=Path)
    parser.add_argument("--max-mean-absolute", type=float)
    parser.add_argument("--min-iou", type=float)
    parser.add_argument("--max-disagreement-fraction", type=float)
    parser.add_argument("--max-p95-absolute", type=float)
    parser.add_argument("--report-failures", action="store_true")
    args = parser.parse_args()

    report = compare(args.left, args.right)
    text = json.dumps(report, indent=2, sort_keys=True) + "\n"
    if args.json:
        args.json.parent.mkdir(parents=True, exist_ok=True)
        args.json.write_text(text, encoding="utf-8")
    print(text, end="")

    failures = check_acceptance(
        report,
        min_iou=args.min_iou,
        max_mean_absolute=args.max_mean_absolute,
        max_disagreement_fraction=args.max_disagreement_fraction,
        max_p95_absolute=args.max_p95_absolute,
    )
    if failures and args.report_failures:
        import sys
        for failure in failures:
            print(f"[REGRESSION] {failure}", file=sys.stderr)
    raise SystemExit(1 if failures else 0)


if __name__ == "__main__":
    main()
