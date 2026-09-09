# Execution Environment & Hardware "HwEnv"

Plaque Forge relies on high-performance computer vision algorithms, hardware video encoders, and deep learning runtimes.


## 01) Supported Hardware & Compute Platforms

The primary production operating system is Linux (x86_64, Ubuntu 22.04 LTS / 24.04 LTS, Arch Linux, and CachyOS).
Supported compute accelerators:
1. **NVIDIA GPUs**: Ampere, Ada Lovelace, Hopper, or newer with CUDA 12.x and cuDNN 9.x.
2. **Intel XPUs**: Intel Arc and Data Center GPU Flex via Intel oneAPI and PyTorch XPU extension.
3. **CPU Fallback**: Multi-threaded x86_64 CPU execution using OpenMP and SIMD (AVX-512 / AVX2).


## 02) Numerical Drift & Floating-Point Precision

To prevent cross-platform floating-point drift from altering segmentation mask boundaries:
1. Homologation acceptance runs and golden baseline generations must execute with explicit precision (`--precision fp32`).
2. Non-deterministic hardware shortcuts (e.g. TensorFloat-32 / TF32 on CUDA) must be explicitly managed or disabled when generating authoritative mask boundaries.
3. Running on CPU vs GPU must be verified via [scripts/compare_segmentation_devices.sh](file:///root/opencode/plaque-forge/scripts/compare_segmentation_devices.sh) to ensure device drift remains within contract bounds.



# Python Machine Learning Runtime "PyEnv"

The machine learning subsystem relies on an external Python environment containing vision foundation models.


## 01) Dedicated Virtual Environment

The segmentation worker must execute inside an isolated Python virtual environment managed via [scripts/setup_segmentation.sh](file:///root/opencode/plaque-forge/scripts/setup_segmentation.sh). Key dependencies (`torch`, `torchvision`, `sam2`, `opencv-python-headless`, `pillow`, `huggingface-hub`) must be pinned to exact versions in the project virtualenv.


## 02) Offline Execution & Cache Isolation

Production runs, continuous integration, and homologation verification must operate completely offline. Foundation model weights (`sam2.1_hiera_small.pt`, `sam2.1_hiera_large.pt`) must be pre-cached in local storage. Spurious network access or unpinned downloads during test or render execution are strictly prohibited.


## 03) Environment Health Verification

Operators and automation must run [scripts/ml_status.sh](file:///root/opencode/plaque-forge/scripts/ml_status.sh) and [scripts/check_segmentation.sh](file:///root/opencode/plaque-forge/scripts/check_segmentation.sh) before launching production batches to verify GPU accessibility, PyTorch acceleration status, and model weight availability.



# Continuous Integration & Quality Gates "CiCd"

Quality gates are tiered to balance rapid feedback with exhaustive visual protection.


## 01) Pull Request Fast Sentinel Gate

Every pull request and merge commit must execute:
1. `cargo test --all-targets` (Rust unit and integration tests);
2. `scripts/check_analysis_cache.sh` (Analysis cache integrity for all 19 scenes);
3. `scripts/check_segmentation.sh` (Python segmentation worker test suite);
4. `scripts/check_homologated_assets.sh` (Continuous homologation sentinels: 5 behaviorally diverse assets).
Total execution time must remain under 5 minutes on standard CI runners.


## 02) Full Matrix Scheduled Homologation

A nightly and pre-release workflow running on a dedicated GPU worker must execute the full 19-scene matrix ([scripts/run_homologation_matrix.sh](file:///root/opencode/plaque-forge/scripts/run_homologation_matrix.sh)). Every single scene must pass its dual-witness contract before a release candidate is approved.


## 03) Automated Model Bake-off Gate

Before any vision model upgrade or default weight replacement is merged, CI must execute [scripts/bakeoff_segmentation_matrix.sh](file:///root/opencode/plaque-forge/scripts/bakeoff_segmentation_matrix.sh). The workflow compares candidate masks against golden references and fails if any existing homologated asset suffers boundary degradation.



# Asset Storage & Cache Lifecycle "AstSto"

Managing media files, video frames, and analysis caches requires operational discipline.


## 01) Analysis Cache Integrity

Pre-computed video trajectories and surface parameters in [assets/analysis/](file:///root/opencode/plaque-forge/assets/analysis) are committed to git to allow instant rendering and testing. Any change to tracking algorithms or scene definitions must re-validate all 19 caches using `plaque-forge check-analysis-cache`. Stale caches are rejected by CI.


## 02) Storage Cleanup & Artifact Retention

Intermediate render artifacts (`output/*.hevc.mkv`, diagnostic frames) can consume tens of gigabytes. Local and CI environments must run [scripts/cleanup_work.sh](file:///root/opencode/plaque-forge/scripts/cleanup_work.sh) to purge ephemeral render outputs while retaining structured JSON test reports and regression failure packets.



# Release Packaging & Distribution "RelPkg"

Releases deliver verified, self-contained binaries for production deployment.


## 01) Single Binary Artifact with Optional Media Bundling

Release binaries are built with Link-Time Optimization (`lto = "fat"`), strip enabled, and single codegen units (`codegen-units = 1`). Under `--features bundle-media`, all scenes, fonts, textures, and styles are embedded directly into the binary, producing a completely self-contained executable.


## 02) Release Verification Lifecycle

Releases follow semantic versioning (`vMAJOR.MINOR.PATCH`). Creating a release candidate tag (e.g. `v0.10.0-rc.1`) triggers the full 19-asset homologation suite. Only when all 19 contracts pass and human visual sign-off is recorded in [management/RELEASES.md](file:///root/opencode/plaque-forge/management/RELEASES.md) may the production release tag (`v0.10.0`) be cut.
