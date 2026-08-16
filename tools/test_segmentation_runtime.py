#!/usr/bin/env python3
import unittest

from segmentation_runtime import MODEL_REVISIONS, load_sam2_video_predictor, model_revision


class SegmentationRuntimeContractTests(unittest.TestCase):
    def test_known_sam2_model_uses_pinned_offline_checkpoint(self):
        calls = {}

        def download(**kwargs):
            calls["download"] = kwargs
            return "/cache/sam2.1_hiera_large.pt"

        def build(config, checkpoint, **kwargs):
            calls["build"] = (config, checkpoint, kwargs)
            return "predictor"

        predictor = load_sam2_video_predictor(
            "facebook/sam2.1-hiera-large",
            "cpu",
            downloader=download,
            builder=build,
        )

        self.assertEqual(predictor, "predictor")
        self.assertEqual(
            calls["download"],
            {
                "repo_id": "facebook/sam2.1-hiera-large",
                "filename": "sam2.1_hiera_large.pt",
                "revision": "665f8e2ad61cf5f53d65644ff27c8ee525124610",
                "local_files_only": True,
            },
        )
        self.assertEqual(
            calls["build"],
            (
                "configs/sam2.1/sam2.1_hiera_l.yaml",
                "/cache/sam2.1_hiera_large.pt",
                {"device": "cpu"},
            ),
        )

    def test_preview_sam2_model_is_also_pinned_and_offline(self):
        calls = {}

        def download(**kwargs):
            calls["download"] = kwargs
            return "/cache/sam2.1_hiera_small.pt"

        def build(config, checkpoint, **kwargs):
            calls["build"] = (config, checkpoint, kwargs)
            return "preview-predictor"

        predictor = load_sam2_video_predictor(
            "facebook/sam2.1-hiera-small",
            "xpu",
            downloader=download,
            builder=build,
        )

        self.assertEqual(predictor, "preview-predictor")
        self.assertEqual(
            calls["download"],
            {
                "repo_id": "facebook/sam2.1-hiera-small",
                "filename": "sam2.1_hiera_small.pt",
                "revision": "6c381d9c16faed5e8a7c4a2cd99918bdca8316e4",
                "local_files_only": True,
            },
        )
        self.assertEqual(
            calls["build"],
            (
                "configs/sam2.1/sam2.1_hiera_s.yaml",
                "/cache/sam2.1_hiera_small.pt",
                {"device": "xpu"},
            ),
        )

    def test_unknown_model_preserves_upstream_loading_contract(self):
        calls = {}

        def pretrained(model_name, **kwargs):
            calls["pretrained"] = (model_name, kwargs)
            return "experimental"

        predictor = load_sam2_video_predictor(
            "example/experimental-sam2",
            "cpu",
            pretrained_loader=pretrained,
        )

        self.assertEqual(predictor, "experimental")
        self.assertEqual(
            calls["pretrained"],
            ("example/experimental-sam2", {"device": "cpu"}),
        )

    def test_all_production_model_revisions_are_full_commit_hashes(self):
        self.assertGreaterEqual(len(MODEL_REVISIONS), 3)
        for model, revision in MODEL_REVISIONS.items():
            with self.subTest(model=model):
                self.assertEqual(len(revision), 40)
                int(revision, 16)
                self.assertEqual(model_revision(model), {"revision": revision})


if __name__ == "__main__":
    unittest.main()
