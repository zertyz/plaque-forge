"""Pinned, testable model-loading contracts for Plaque Forge segmentation.

Keep this module free of heavyweight imports at module load time.  The setup script
and the production worker both import it, while CI can exercise the pinning logic
without installing PyTorch, SAM2, or Hugging Face.
"""

MODEL_SPECS = {
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


def load_sam2_video_predictor(
    model_name,
    device,
    *,
    downloader=None,
    builder=None,
    pretrained_loader=None,
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
        if builder is None:
            from sam2.build_sam import build_sam2_video_predictor

            builder = build_sam2_video_predictor

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

    if pretrained_loader is None:
        from sam2.sam2_video_predictor import SAM2VideoPredictor

        pretrained_loader = SAM2VideoPredictor.from_pretrained
    return pretrained_loader(model_name, device=device, **model_revision(model_name))
