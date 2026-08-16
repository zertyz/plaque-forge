# Segmentation strategy

Plaque Forge treats **model choice**, **execution device**, and **numeric precision** as separate contracts. Rust owns the strategy; Python executes the sealed plan and records the exact plan hash in generated-layer provenance.

## Profiles

| Profile | General semantic path | Precision default | Compilation | Purpose |
|---|---|---|---|---|
| `preview` | SAM 2.1 Small | BF16 | image encoder may compile | fast visual iteration |
| `balanced` | SAM 2.1 Large + Cutie | BF16 | off | normal local analysis |
| `canonical` | SAM 2.1 Large + Cutie | FP32 | off | reproducible acceptance/bake-off baseline |

A caller may override model/backend/precision explicitly. Device selection (`cpu`, `xpu`, `cuda`, `auto`) never changes the sealed precision policy. If execution falls back to another device, the same requested precision remains in force or the worker fails explicitly.

## Semantic planning

The planner uses scene semantics that are actually known, rather than the current sample-video theme:

- writing surfaces and opaque foregrounds need categorical membership, not optical alpha; ViTMatte is skipped;
- optical foregrounds use ViTMatte after the semantic tracker;
- `subject = "human"` may select MatAnyone2 in canonical mode when a frame-zero area seed exists;
- unspecified subjects remain generic and never accidentally select the human specialist;
- SAM 3.1 is experimental and explicit-only because its official runtime currently requires CUDA and gated model access.

This keeps future people, vehicles, animals, foliage, smoke, fabric, machinery, water, UI elements, fantasy objects, and unknown content on the same generic planning path unless the author supplies a genuinely useful specialist hint.

## Quality-neutral performance work

The worker keeps decoded lossless source frames in a content-addressed cache keyed by source identity, and keeps reusable SAM2/Cutie stage masks in a separate content-addressed cache keyed by source, prompt, plan, runtime, requested device, and numeric precision. Cache reuse never changes the requested model or arithmetic contract.

The preview profile deliberately trades some model accuracy for speed by using SAM 2.1 Small; that trade is explicit and must not be confused with device acceleration.

## Numeric drift

Use:

```bash
./scripts/compare_segmentation_devices.sh ASSET_STEM LAYER_ID
```

to run the **same sealed plan and precision** on two devices and compare the stored 16-bit masks. The report includes alpha MAE/RMSE/p95/p99, binary IoU at 0.5, and soft-edge fractions. This measures implementation/backend drift; neither output is assumed to be visual ground truth.

## Backend bake-off

Use:

```bash
./scripts/bakeoff_segmentation_backends.sh \
  --backends "sam2 sam2-cutie sam2-cutie-vitmatte" \
  ASSET_STEM LAYER_ID
```

The first backend is only a numerical comparison baseline. Promote a strategy only after render/homologation evidence is reviewed.

### SAM 3.1

SAM 3.1 lives in a separate `/tmp/plaque-forge-sam31` Python 3.12/CUDA runtime so its dependencies cannot destabilize the XPU/CPU SAM2 stack. Install it only when CUDA hardware and gated model access are available:

```bash
./scripts/setup_sam31.sh
./scripts/setup_sam31.sh --verify
```

The setup performs a real predictor-construction smoke test. If the current public SAM 3.1 code/checkpoint pair is incompatible, setup fails and the backend remains unavailable instead of publishing misleading provenance.
