#!/usr/bin/env python3
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import numpy as np
from PIL import Image

import segmentation_worker as worker


class SegmentationQualityContractTests(unittest.TestCase):
    def test_sam2_lossless_input_is_normalized_exactly_once(self):
        images = np.asarray([[[[0.80]], [[0.60]], [[0.40]]]], dtype=np.float32)
        mean = np.asarray([0.50, 0.25, 0.10], dtype=np.float32)[None, :, None, None]
        std = np.asarray([0.10, 0.25, 0.20], dtype=np.float32)[None, :, None, None]

        normalized = worker.normalize_sam2_images(images.copy(), mean, std)

        np.testing.assert_allclose(
            normalized,
            np.asarray([[[[3.0]], [[1.4]], [[1.5]]]], dtype=np.float32),
            rtol=0,
            atol=1.0e-6,
        )

    def test_sam2_cutie_uses_the_temporal_track_and_keeps_soft_opaque_confidence(self):
        sam2 = [
            np.asarray([[1.0, 0.0], [0.0, 0.0]], dtype=np.float32),
            np.asarray([[0.0, 0.0], [0.0, 0.0]], dtype=np.float32),
        ]
        cutie = [
            np.asarray([[0.80, 0.10], [0.0, 0.0]], dtype=np.float32),
            np.asarray([[0.35, 0.12], [0.0, 0.0]], dtype=np.float32),
        ]
        request = {
            "plan": {
                "semantic_backend": "sam2-cutie",
                "matte_refiner": "none",
                "precision": "fp32",
            },
            "source": {"frames": 2, "width": 2, "height": 2},
            "layer": {
                "role": "foreground",
                "matte_mode": "opaque",
                "prompts": [],
            },
        }

        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            frames = []
            for frame in range(2):
                path = root / f"{frame:06}.png"
                Image.new("RGB", (2, 2), (32, 48, 64)).save(path)
                frames.append(path)
            with (
                patch.object(worker, "load_cutie_stage_cache", return_value=None),
                patch.object(
                    worker,
                    "cached_sam2",
                    return_value=(sam2, "sam2-test", "cpu"),
                ),
                patch.object(
                    worker,
                    "cached_cutie",
                    return_value=(cutie, "cutie-test", "cpu"),
                ),
            ):
                probabilities, version = worker.model_masks(
                    request, frames, "cpu", root
                )

        np.testing.assert_allclose(probabilities, cutie, rtol=0, atol=1.0e-6)
        self.assertIn("sam2-guided-cutie", version)
        self.assertIn("semantic-confidence", version)
        self.assertNotIn("selected-sam2", version)
        self.assertNotIn("categorical-membership-p50", version)

    def test_exact_guided_cutie_cache_avoids_recomputing_its_sam2_guide(self):
        cutie = [
            np.asarray([[0.80, 0.10], [0.0, 0.0]], dtype=np.float32),
            np.asarray([[0.35, 0.12], [0.0, 0.0]], dtype=np.float32),
        ]
        request = {
            "plan": {
                "semantic_backend": "sam2-cutie",
                "matte_refiner": "none",
                "precision": "fp32",
            },
            "source": {"frames": 2, "width": 2, "height": 2},
            "layer": {
                "role": "foreground",
                "matte_mode": "opaque",
                "prompts": [],
            },
        }

        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            frames = []
            for frame in range(2):
                path = root / f"{frame:06}.png"
                Image.new("RGB", (2, 2), (32, 48, 64)).save(path)
                frames.append(path)
            with (
                patch.object(
                    worker,
                    "load_cutie_stage_cache",
                    return_value=(cutie, "cutie-test", "cpu"),
                ),
                patch.object(
                    worker,
                    "cached_sam2",
                    side_effect=AssertionError("SAM2 must not run for an exact Cutie hit"),
                ),
                patch.object(
                    worker,
                    "cached_cutie",
                    side_effect=AssertionError("Cutie must not be recomputed"),
                ),
            ):
                probabilities, version = worker.model_masks(
                    request, frames, "cpu", root
                )

        np.testing.assert_allclose(probabilities, cutie, rtol=0, atol=1.0e-6)
        self.assertIn("guided-stage-cache", version)

    def test_sam2_prompt_correction_is_local_and_does_not_touch_interprompt_frames(self):
        cutie = [np.zeros((11, 11), dtype=np.float32) for _ in range(2)]
        sam2 = [np.zeros((11, 11), dtype=np.float32) for _ in range(2)]
        cutie[0][5, 5] = 0.9
        cutie[1][5, 6] = 0.8
        sam2[0][5, 7] = 0.7
        sam2[0][0, 0] = 1.0
        sam2[1][5, 8] = 1.0
        request = {"layer": {"prompts": [{"frame": 0}]}}

        corrected = worker.apply_authored_sam2_prompt_corrections(
            request, sam2, cutie, radius=2
        )

        self.assertEqual(corrected[0][5, 7], 0.7)
        self.assertEqual(corrected[0][0, 0], 0.0)
        np.testing.assert_array_equal(corrected[1], cutie[1])


if __name__ == "__main__":
    unittest.main()
