# Segmentation strategy

Plaque Forge treats **model choice**, **execution device**, and **numeric precision** as separate contracts. Rust owns the strategy; Python executes the sealed plan and records the exact plan hash in generated-layer provenance.

## Profiles

| Profile | General semantic path | Precision default | Compilation | Purpose |
|---|---|---|---|---|
| `preview` | SAM 2.1 Small → Large on evidence failure | BF16 | image encoder may compile | fast visual iteration |
| `balanced` | SAM 2.1 Large → SAM 2.1 Large + Cutie on evidence failure | BF16 | off | normal local analysis |
| `canonical` | SAM 2.1 Large + Cutie | FP32 | off | reproducible acceptance/bake-off baseline |

A request that does not specify a profile uses `canonical`; `--precision auto`
therefore resolves to FP32. `preview` and `balanced` must be selected explicitly
when iteration speed is more important than the final-quality contract.

A caller may override model/backend/precision explicitly. Device selection (`cpu`, `xpu`, `cuda`, `auto`) never changes the sealed precision policy. If execution falls back to another device, the same requested precision remains in force or the worker fails explicitly.

## Semantic planning

The planner uses scene semantics that are actually known, rather than the current sample-video theme:

- writing surfaces and opaque foregrounds need categorical membership, not optical alpha; ViTMatte is skipped;
- `sam2-cutie` has one deterministic meaning: SAM2 supplies semantic seed masks and Cutie supplies the authoritative full temporal track; the worker does not choose between their incompatible intermediate sequences;
- on an explicitly authored prompt frame, SAM2 may restore fine seed detail only within four pixels of Cutie's object support; this correction never substitutes anchor-only SAM2 masks between prompts;
- opaque foreground confidence remains lossless 16-bit evidence until Rust applies the declared matte support/solid thresholds; the worker does not collapse it with an image-wide percentile;
- optical foregrounds use ViTMatte after the semantic tracker;
- `subject = "human"` may select MatAnyone2 in canonical mode when a frame-zero area seed exists;
- unspecified subjects remain generic and never accidentally select the human specialist;
- SAM 3.1 is experimental and explicit-only because its official runtime currently requires CUDA and gated model access.

This keeps future people, vehicles, animals, foliage, smoke, fabric, machinery, water, UI elements, fantasy objects, and unknown content on the same generic planning path unless the author supplies a genuinely useful specialist hint.

## Quality-neutral performance work

The worker keeps decoded lossless source frames in a content-addressed cache keyed by source identity, and keeps reusable SAM2/Cutie stage masks in a separate content-addressed cache keyed by source, prompt, plan, runtime, requested device, and numeric precision. An exact guided-stage cache hit is resolved before SAM2 inference, so reusing a valid temporal result cannot needlessly recompute its seed model. Cache reuse never changes the requested model or arithmetic contract.

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

Run `./scripts/bakeoff_segmentation_matrix.sh` to execute the same bake-off over every currently represented prompted ML capability in `assets/homologation/segmentation-capabilities.toml`. Each run writes `summary.json` and `summary.md` with stage timing, devices, peak process/accelerator memory, cache hits, and numerical comparisons. The report is evidence for policy tuning, never automatic ground truth.

### SAM 3.1

SAM 3.1 lives in a separate `/tmp/plaque-forge-sam31` Python 3.12/CUDA runtime so its dependencies cannot destabilize the XPU/CPU SAM2 stack. Install it only when CUDA hardware and gated model access are available:

```bash
./scripts/setup_sam31.sh
./scripts/setup_sam31.sh --verify
```

The setup performs a real predictor-construction smoke test. If the current public SAM 3.1 code/checkpoint pair is incompatible, setup fails and the backend remains unavailable instead of publishing misleading provenance.

## Adaptive policy and independent evidence

`assets/segmentation/policy.toml` is the versioned acceptance policy for automatic candidate escalation. Rust checks prompt survival, negative-prompt rejection, active-frame occupancy, catastrophic frame coverage, and whether a foreground mask merely copied most of its prompt rectangle in the stored 16-bit mask domain. For a repeatedly boxed foreground object, it also measures every frame between prompts: minimum and fifth-percentile area relative to the interpolated prompt-frame area, plus fifth-percentile adjacent-frame IoU. This rejects both rectangular/blob false positives and anchor-only masks that look correct at prompts but blink away between them. Python never decides whether a cheaper candidate is sufficient.

Each prompted layer records `strategy-selection.json` beside its artifact with the candidates attempted, plan hashes, independent evidence, rejection reasons, and selected plan. Explicit `--backend` requests disable adaptive substitution and execute exactly one plan. Canonical currently remains on SAM2+Cutie until measured bake-offs and homologated renders justify a cheaper candidate.

For an authored opaque source-pixel foreground, the semantic sequence establishes object identity while lossless source-versus-canonical evidence may recover fine material that semantic downsampling omitted. Recovery is bounded to the semantic neighborhood, requires a sharp transition in both the source and residual, and is evaluated independently on each frame. Diffuse cast shadows therefore remain illumination instead of becoming opaque holes in the title, and neighboring-frame residuals cannot grow a restoration halo. Automatic depth discovery remains a separate mode.

When authored and automatic foregrounds coexist, the analyzer keeps three private channels until final fusion:

- component-filtered automatic discovery, with known authored foreground removed before it can become an ML prompt;
- frame-local, lossless photometric material used to gate the automatic semantic track;
- frame-exact authored detail, recalibrated with its declared opaque matte policy after projective resampling.

Only automatic discovery receives temporal detail recovery. The automatic semantic track may identify a porous object such as a web, but it cannot fill pixels where the lossless material channel observes a gap. Authored detail is max-unioned only after that fusion, so a known actor remains continuously in front without teaching the automatic model to rediscover it. The private channel directories are removed before publishing the reusable analysis; only the final `occluder` sequence and ML provenance remain.

## Persistent worker service

Normal `tools/segmentation-worker` calls route through a single-request-at-a-time Unix-domain service. Python imports, accelerator initialization, SAM2/Cutie/ViTMatte/MatAnyone2 model objects, and compiled image encoders may remain resident between requests. A backend that clears Hydra's global configuration cannot poison the next request: SAM2 verifies and reinitializes its configuration state before predictor construction. The service also rejects a stale socket whose recorded owner process is no longer alive. The socket name is derived from the worker/runtime source identity **and the installed runtime manifest**, so either a code change or a rebuilt Python environment automatically starts a fresh service. Set `PLAQUE_FORGE_PERSISTENT_WORKER=0` to force the old one-process-per-request behavior; set `PLAQUE_FORGE_MODEL_CACHE=0` to keep the service but disable resident model objects. The service exits after an idle timeout (20 minutes by default).

`result.json` and `strategy-selection.json` are retained beside generated masks in the final analysis package because they are small, provenance-bound, and supply the worker timing/cache record plus Rust's independent acceptance decision.

## ML capability coverage

`assets/homologation/segmentation-capabilities.toml` tracks ML-specific behaviors separately from the final-render capability matrix. Run `./scripts/check_segmentation_capabilities.sh`. Empty representatives and missing final homologation contracts are explicit review debt, not generated goldens.
