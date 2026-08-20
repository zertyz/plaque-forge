"""Pinned, testable model-loading contracts for Plaque Forge segmentation.

Keep this module free of heavyweight imports at module load time.  The setup script
and the production worker both import it, while CI can exercise the pinning logic
without installing PyTorch, SAM2, or Hugging Face.
"""

MODEL_SPECS = {
    "facebook/sam2.1-hiera-small": {
        "revision": "6c381d9c16faed5e8a7c4a2cd99918bdca8316e4",
        "sam2_config": "configs/sam2.1/sam2.1_hiera_s.yaml",
        "sam2_checkpoint": "sam2.1_hiera_small.pt",
    },
    "facebook/sam2.1-hiera-large": {
        "revision": "665f8e2ad61cf5f53d65644ff27c8ee525124610",
        "sam2_config": "configs/sam2.1/sam2.1_hiera_l.yaml",
        "sam2_checkpoint": "sam2.1_hiera_large.pt",
    },
    "hustvl/vitmatte-base-composition-1k": {
        "revision": "bf486d01a7d9e3dbcc8400f7942835caf0eaf76e",
    },
    "PeiqingYang/MatAnyone2": {
        "revision": "40c894a6f68d1f55c86ab0de838d89dc61587930",
    },
}

MODEL_REVISIONS = {
    model: specification["revision"] for model, specification in MODEL_SPECS.items()
}


def model_revision(model_name):
    revision = MODEL_REVISIONS.get(model_name)
    return {"revision": revision} if revision else {}


def _ensure_sam2_hydra_initialized(hydra_ready=None, hydra_initializer=None):
    """Restore SAM2's Hydra search path after another backend clears it."""
    if hydra_ready is None:
        from hydra.core.global_hydra import GlobalHydra

        hydra_ready = GlobalHydra.instance().is_initialized
    if hydra_ready():
        return
    if hydra_initializer is None:
        from hydra import initialize_config_module

        hydra_initializer = initialize_config_module
    hydra_initializer(config_module="sam2", version_base="1.2")


def load_sam2_video_predictor(
    model_name,
    device,
    *,
    downloader=None,
    builder=None,
    pretrained_loader=None,
    hydra_ready=None,
    hydra_initializer=None,
):
    """Load a pinned SAM2 predictor without consulting an unpinned Hub ref.

    Upstream SAM2's ``from_pretrained`` currently downloads its checkpoint through
    an internal helper that does not forward ``revision`` to ``hf_hub_download``.
    That defeats an offline cache populated at an explicit commit.  Known Plaque
    Forge models therefore resolve the pinned checkpoint ourselves and feed it to
    SAM2's local builder.  Unknown models retain upstream behavior for explicit
    experimentation.
    """

    specification = MODEL_SPECS.get(model_name)
    if specification is not None and {
        "revision",
        "sam2_config",
        "sam2_checkpoint",
    }.issubset(specification):
        if downloader is None:
            from huggingface_hub import hf_hub_download

            downloader = hf_hub_download
        production_builder = builder is None
        if production_builder:
            from sam2.build_sam import build_sam2_video_predictor

            builder = build_sam2_video_predictor

        if production_builder or hydra_ready is not None or hydra_initializer is not None:
            _ensure_sam2_hydra_initialized(hydra_ready, hydra_initializer)

        checkpoint_path = downloader(
            repo_id=model_name,
            filename=specification["sam2_checkpoint"],
            revision=specification["revision"],
            local_files_only=True,
        )
        return builder(
            specification["sam2_config"],
            checkpoint_path,
            device=device,
        )

    production_loader = pretrained_loader is None
    if production_loader:
        from sam2.sam2_video_predictor import SAM2VideoPredictor

        pretrained_loader = SAM2VideoPredictor.from_pretrained
    if production_loader or hydra_ready is not None or hydra_initializer is not None:
        _ensure_sam2_hydra_initialized(hydra_ready, hydra_initializer)
    return pretrained_loader(model_name, device=device, **model_revision(model_name))
