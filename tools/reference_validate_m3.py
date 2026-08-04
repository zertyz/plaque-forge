#!/usr/bin/env python3
"""Executable reference for Plaque Forge 0.3 adaptive-keyframe tracking.

This mirrors the Rust architecture closely enough to validate the algorithmic
choice in environments where the Rust/OpenCV crate cannot be compiled.
"""
from __future__ import annotations

import argparse
import json
import math
import tomllib
from pathlib import Path

import cv2
import numpy as np


def plaque_corners(rect):
    x, y, w, h = rect
    return np.array([[x, y], [x + w, y], [x + w, y + h], [x, y + h]], np.float64)


def mask_for_transform(shape, rect, H, margin=30):
    h, w = shape
    pts = cv2.perspectiveTransform(plaque_corners(rect).astype(np.float32)[None], H.astype(np.float64))[0]
    x0 = max(0, int(np.floor(pts[:, 0].min())) - margin)
    y0 = max(0, int(np.floor(pts[:, 1].min())) - margin)
    x1 = min(w - 1, int(np.ceil(pts[:, 0].max())) + margin)
    y1 = min(h - 1, int(np.ceil(pts[:, 1].max())) + margin)
    mask = np.full((h, w), 255, np.uint8)
    cv2.rectangle(mask, (x0, y0), (x1, y1), 0, -1)
    return mask


def features(sift, gray, mask=None):
    kp, des = sift.detectAndCompute(gray, mask)
    return kp or [], des


def estimate(kp_a, des_a, kp_b, des_b):
    if des_a is None or des_b is None or len(kp_a) < 12 or len(kp_b) < 12:
        return None
    pairs = cv2.BFMatcher(cv2.NORM_L2).knnMatch(des_a, des_b, k=2)
    good = [a for pair in pairs if len(pair) == 2 for a, b in [pair] if a.distance < 0.72 * b.distance]
    if len(good) < 8:
        return None
    src = np.float64([kp_a[m.queryIdx].pt for m in good])
    dst = np.float64([kp_b[m.trainIdx].pt for m in good])
    candidates = []
    A, inliers = cv2.estimateAffinePartial2D(src, dst, method=cv2.RANSAC, ransacReprojThreshold=3.0,
                                              maxIters=4000, confidence=0.995, refineIters=20)
    if A is not None:
        H = np.vstack([A, [0, 0, 1]])
        candidates.append(score(H, src, dst, inliers, 0.0))
    A, inliers = cv2.estimateAffine2D(src, dst, method=cv2.RANSAC, ransacReprojThreshold=3.0,
                                      maxIters=4000, confidence=0.995, refineIters=20)
    if A is not None:
        H = np.vstack([A, [0, 0, 1]])
        candidates.append(score(H, src, dst, inliers, 0.15))
    H, inliers = cv2.findHomography(src, dst, cv2.RANSAC, 3.0, maxIters=4000, confidence=0.995)
    if H is not None:
        candidates.append(score(H, src, dst, inliers, 0.35))
    return min(candidates, key=lambda item: item['objective']) if candidates else None


def score(H, src, dst, inliers, complexity):
    projected = cv2.perspectiveTransform(src.astype(np.float32)[None], H.astype(np.float64))[0]
    errors = np.linalg.norm(projected - dst, axis=1)
    median = float(np.median(errors))
    ratio = float(np.mean(inliers.ravel() > 0)) if inliers is not None else 0.0
    return {'H': H.astype(np.float64), 'error': median, 'inliers': ratio,
            'objective': median + complexity + (1.0 - ratio) * 2.0}


def choose(local, direct):
    if local is None:
        return direct
    if direct is None:
        return local
    ls = local['error'] + (1.0 - local['inliers']) * 3.0
    ds = direct['error'] + (1.0 - direct['inliers']) * 3.0
    # Prefer a globally anchored estimate whenever it is credible and not
    # materially worse. Adaptive references are the fallback for large appearance changes.
    direct_credible = direct['inliers'] >= 0.20 and direct['error'] <= 5.0
    return direct if direct_credible and ds <= ls + 0.75 else local


def regularize(Hs, rect, confidence, inertia, looped=True, reference=0):
    source = plaque_corners(rect).astype(np.float32)
    raw = np.array([cv2.perspectiveTransform(source[None], H)[0] for H in Hs], np.float64)
    smooth = raw.copy()
    n = len(raw)
    for _ in range(6):
        prev = smooth.copy()
        for i in range(n):
            if i == reference:
                smooth[i] = raw[i]
                continue
            l = (i - 1) % n if looped else max(0, i - 1)
            r = (i + 1) % n if looped else min(n - 1, i + 1)
            w = np.clip(inertia * (0.58 - 0.30 * confidence[i]), 0.0, 0.48)
            smooth[i] = raw[i] * (1.0 - w) + (prev[l] + prev[r]) * 0.5 * w
    out = []
    for q in smooth:
        out.append(cv2.getPerspectiveTransform(source.astype(np.float32), q.astype(np.float32)).astype(np.float64))
    return out, smooth


def eval_fourier(coeff, t, order):
    value = coeff[0]
    for k in range(1, order + 1):
        value += coeff[2 * k - 1] * math.cos(2 * math.pi * k * t)
        value += coeff[2 * k] * math.sin(2 * math.pi * k * t)
    return value


def ground_truth(path, frames):
    with open(path, 'rb') as f:
        data = tomllib.load(f)
    order = data['order']
    mats = []
    for i in range(frames):
        t = i / frames
        mats.append(np.array([
            [eval_fourier(data['m00'], t, order), eval_fourier(data['m01'], t, order), eval_fourier(data['tx'], t, order)],
            [eval_fourier(data['m10'], t, order), eval_fourier(data['m11'], t, order), eval_fourier(data['ty'], t, order)],
            [0, 0, 1],
        ], np.float64))
    inv0 = np.linalg.inv(mats[0])
    return [m @ inv0 for m in mats]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--video', required=True)
    ap.add_argument('--truth')
    ap.add_argument('--rect', default='130,160,458,268')
    ap.add_argument('--anchor-interval', type=int, default=24)
    ap.add_argument('--inertia', type=float, default=0.35)
    ap.add_argument('--output', required=True)
    ap.add_argument('--analysis-scale', type=float, default=0.5)
    args = ap.parse_args()
    rect_full = tuple(map(float, args.rect.split(',')))
    rect = tuple(value * args.analysis_scale for value in rect_full)
    out = Path(args.output); out.mkdir(parents=True, exist_ok=True)

    cap = cv2.VideoCapture(args.video)
    frames = []
    while True:
        ok, frame = cap.read()
        if not ok:
            break
        if args.analysis_scale != 1.0:
            frame = cv2.resize(frame, None, fx=args.analysis_scale, fy=args.analysis_scale, interpolation=cv2.INTER_AREA)
        frames.append(frame)
    cap.release()
    gray = [cv2.cvtColor(f, cv2.COLOR_BGR2GRAY) for f in frames]
    sift = cv2.SIFT_create(nfeatures=1000, nOctaveLayers=3, contrastThreshold=0.025,
                            edgeThreshold=12, sigma=1.6)
    root_mask = mask_for_transform(gray[0].shape, rect, np.eye(3))
    root_kp, root_des = features(sift, gray[0], root_mask)
    Hs = [np.eye(3)]
    conf = [1.0]
    anchor_i = 0; anchor_H = np.eye(3); anchor_kp, anchor_des = root_kp, root_des
    records = [{'frame': 0, 'source': 'root', 'inliers': 1.0, 'error': 0.0}]
    for i in range(1, len(frames)):
        kp, des = features(sift, gray[i], None)
        local = estimate(anchor_kp, anchor_des, kp, des)
        if local is not None:
            local = dict(local); local['H'] = local['H'] @ anchor_H; local['source'] = 'adaptive'
        direct = estimate(root_kp, root_des, kp, des)
        if direct is not None:
            direct = dict(direct); direct['source'] = 'root'
        best = choose(local, direct)
        if best is None:
            best = {'H': anchor_H.copy(), 'inliers': 0.0, 'error': 24.0, 'source': 'fallback'}
        Hs.append(best['H'])
        c = np.clip(best['inliers'] * math.exp(-min(best['error'], 20.0) / 5.0), 0, 1)
        conf.append(c)
        records.append({'frame': i, 'source': best['source'], 'inliers': best['inliers'], 'error': best['error']})
        if best['inliers'] >= 0.22 and best['error'] <= 5.0 and i - anchor_i >= args.anchor_interval:
            anchor_i = i; anchor_H = best['H']
            mask = mask_for_transform(gray[i].shape, rect, anchor_H)
            anchor_kp, anchor_des = features(sift, gray[i], mask)
    Hs, quads = regularize(Hs, rect, np.asarray(conf), args.inertia, looped=True)

    result = {'frames': len(frames), 'median_inliers': float(np.median([r['inliers'] for r in records])),
              'median_error': float(np.median([r['error'] for r in records])),
              'anchor_interval': args.anchor_interval, 'inertia': args.inertia}
    if args.truth:
        truth_full = ground_truth(args.truth, len(frames))
        S = np.diag([args.analysis_scale, args.analysis_scale, 1.0])
        Sinv = np.diag([1.0 / args.analysis_scale, 1.0 / args.analysis_scale, 1.0])
        truth = [S @ T @ Sinv for T in truth_full]
        src = plaque_corners(rect).astype(np.float32)
        errors = []
        for H, T in zip(Hs, truth):
            a = cv2.perspectiveTransform(src[None], H)[0]
            b = cv2.perspectiveTransform(src[None], T)[0]
            errors.append(float(np.linalg.norm(a - b, axis=1).mean()))
        result.update({'median_corner_error': float(np.median(errors)),
                       'p90_corner_error': float(np.percentile(errors, 90)),
                       'p95_corner_error': float(np.percentile(errors, 95)),
                       'max_corner_error': float(np.max(errors))})
    (out / 'metrics.json').write_text(json.dumps(result, indent=2))
    (out / 'tracking.json').write_text(json.dumps(records, indent=2))

    picks = np.linspace(0, len(frames) - 1, 12).round().astype(int)
    tiles = []
    for i in picks:
        image = frames[i].copy()
        q = quads[i].round().astype(np.int32)
        cv2.polylines(image, [q], True, (0, 255, 255), 2, cv2.LINE_AA)
        cv2.putText(image, f'{i}: {records[i]["source"]}', (12, 34), cv2.FONT_HERSHEY_SIMPLEX,
                    0.8, (255, 255, 255), 2, cv2.LINE_AA)
        tile = cv2.resize(image, (180, 320), interpolation=cv2.INTER_AREA)
        tiles.append(tile)
    sheet = np.vstack([np.hstack(tiles[i:i+3]) for i in range(0, len(tiles), 3)])
    cv2.imwrite(str(out / 'tracking-contact-sheet.jpg'), sheet)
    print(json.dumps(result, indent=2))


if __name__ == '__main__':
    main()
