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

    def test_sam2_prompt_correction_honors_layer_scoped_radius(self):
        cutie = [np.zeros((11, 11), dtype=np.float32)]
        sam2 = [np.zeros((11, 11), dtype=np.float32)]
        cutie[0][5, 5] = 0.9
        sam2[0][5, 8] = 0.8
        # Distance between (5, 5) and (5, 8) is 3
        request_custom = {
            "layer": {"prompts": [{"frame": 0}], "prompt_correction_radius": 2}
        }
        corrected_custom = worker.apply_authored_sam2_prompt_corrections(
            request_custom, sam2, cutie
        )
        self.assertEqual(corrected_custom[0][5, 8], 0.0)

        request_default = {"layer": {"prompts": [{"frame": 0}]}}
        corrected_default = worker.apply_authored_sam2_prompt_corrections(
            request_default, sam2, cutie
        )
        self.assertEqual(corrected_default[0][5, 8], 0.8)

    def test_smooth_temporal_boundaries_suppresses_high_frequency_boundary_chatter(self):
        # Thin feature (width 2) with a sudden single-frame dip on a boundary pixel
        f0 = np.zeros((10, 10), dtype=np.float32)
        f1 = np.zeros((10, 10), dtype=np.float32)
        f2 = np.zeros((10, 10), dtype=np.float32)
        f0[4:6, 4] = 0.9
        f1[4:6, 4] = 0.9
        f2[4:6, 4] = 0.9
        # Boundary pixel (4, 5) chatters: high at frame 0 and 2, dips at frame 1
        f0[4, 5] = 0.85
        f1[4, 5] = 0.15
        f2[4, 5] = 0.85

        smoothed = worker.smooth_temporal_boundaries(
            [f0, f1, f2], method="ema", blend_strength=0.40
        )

        # The chatter at (4, 5) in frame 1 should be significantly smoothed/raised
        self.assertGreater(smoothed[1][4, 5], 0.40)
        # Adjacent solid feature should remain high
        self.assertGreaterEqual(smoothed[1][4, 4], 0.80)

    def test_smooth_temporal_boundaries_prevents_motion_ghosting_in_empty_background(self):
        # Fast moving object: in frame 0 at [1, 1], in frame 1 at [8, 8]
        f0 = np.zeros((10, 10), dtype=np.float32)
        f1 = np.zeros((10, 10), dtype=np.float32)
        f0[1:3, 1:3] = 1.0
        f1[8:10, 8:10] = 1.0

        smoothed = worker.smooth_temporal_boundaries(
            [f0, f1], method="ema", blend_strength=0.25
        )

        # Frame 1 at [1, 1] (where object was in frame 0) must NOT have ghosting
        self.assertEqual(smoothed[1][1, 1], 0.0)
        # Frame 0 at [8, 8] (where object will be in frame 1) must NOT have ghosting
        self.assertEqual(smoothed[0][8, 8], 0.0)

    def test_smooth_temporal_boundaries_preserves_solid_interior(self):
        # Large solid object
        f0 = np.ones((10, 10), dtype=np.float32)
        f1 = np.ones((10, 10), dtype=np.float32)
        f2 = np.ones((10, 10), dtype=np.float32)

        smoothed = worker.smooth_temporal_boundaries(
            [f0, f1, f2], method="ema", blend_strength=0.25
        )

        # Solid interior remains 1.0
        np.testing.assert_allclose(smoothed[1], 1.0, rtol=0, atol=1.0e-5)

    def test_model_masks_applies_temporal_smoothing_for_opaque_layer_when_requested(self):
        cutie = [
            np.asarray([[0.9, 0.85], [0.0, 0.0]], dtype=np.float32),
            np.asarray([[0.9, 0.15], [0.0, 0.0]], dtype=np.float32),
            np.asarray([[0.9, 0.85], [0.0, 0.0]], dtype=np.float32),
        ]
        request = {
            "plan": {
                "semantic_backend": "cutie",
                "matte_refiner": "none",
                "precision": "fp32",
            },
            "source": {"frames": 3, "width": 2, "height": 2},
            "layer": {
                "role": "foreground",
                "matte_mode": "opaque",
                "temporal_smoothing": True,
                "temporal_smoothing_strength": 0.35,
                "prompts": [],
            },
        }
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            frames = []
            for frame in range(3):
                path = root / f"{frame:06}.png"
                Image.new("RGB", (2, 2), (32, 48, 64)).save(path)
                frames.append(path)
            with patch.object(worker, "cached_cutie", return_value=(cutie, "cutie-test", "cpu")):
                probabilities, version = worker.model_masks(request, frames, "cpu", root)

        # Boundary pixel at (0, 1) in frame 1 should be smoothed
        self.assertGreater(probabilities[1][0, 1], 0.35)

    def test_smooth_temporal_boundaries_optical_flow_stabilization(self):
        f0 = np.zeros((32, 32), dtype=np.float32)
        f1 = np.zeros((32, 32), dtype=np.float32)
        f2 = np.zeros((32, 32), dtype=np.float32)
        f0[14:18, 14:18] = 0.9
        f1[14:18, 14:18] = 0.9
        f2[14:18, 14:18] = 0.9
        # Chatter pixel on perimeter
        f0[14, 18] = 0.85
        f1[14, 18] = 0.15
        f2[14, 18] = 0.85

        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            frame_paths = []
            for frame in range(3):
                path = root / f"{frame:06}.png"
                Image.new("RGB", (32, 32), (64, 64, 64)).save(path)
                frame_paths.append(path)

            smoothed = worker.smooth_temporal_boundaries(
                [f0, f1, f2],
                frames=frame_paths,
                method="optical-flow",
                blend_strength=0.30,
            )

        self.assertGreater(smoothed[1][14, 18], 0.30)
        self.assertEqual(smoothed[1][0, 0], 0.0)


class DummyCudaMatmul:
    allow_tf32 = True


class DummyCudnn:
    allow_tf32 = True
    deterministic = False
    benchmark = True


class DummyBackends:
    def __init__(self):
        self.cuda = type("DummyCuda", (), {"matmul": DummyCudaMatmul()})()
        self.cudnn = DummyCudnn()


class DummyTorch:
    bfloat16 = "bfloat16"
    float32 = "float32"

    def __init__(self):
        self.deterministic = False
        self.warn_only = False
        self.backends = DummyBackends()

    def use_deterministic_algorithms(self, mode, warn_only=False):
        self.deterministic = mode
        self.warn_only = warn_only

    def are_deterministic_algorithms_enabled(self):
        return self.deterministic

    def autocast(self, device_type="cpu", dtype=None):
        from contextlib import nullcontext
        return nullcontext()


class SegmentationDeterminismContractTests(unittest.TestCase):
    def test_configure_determinism_fp32_enforces_strict_flags(self):
        dummy_torch = DummyTorch()
        status = worker.configure_determinism(dummy_torch, "fp32")

        self.assertTrue(dummy_torch.deterministic)
        self.assertTrue(dummy_torch.warn_only)
        self.assertFalse(dummy_torch.backends.cuda.matmul.allow_tf32)
        self.assertFalse(dummy_torch.backends.cudnn.allow_tf32)
        self.assertTrue(dummy_torch.backends.cudnn.deterministic)
        self.assertFalse(dummy_torch.backends.cudnn.benchmark)
        self.assertEqual(status.get("deterministic_algorithms"), True)
        self.assertEqual(status.get("cuda_matmul_allow_tf32"), False)
        self.assertEqual(status.get("cudnn_allow_tf32"), False)
        self.assertEqual(status.get("cudnn_deterministic"), True)
        self.assertEqual(status.get("cudnn_benchmark"), False)

        reported = worker.get_determinism_status(dummy_torch)
        self.assertTrue(reported.get("deterministic_algorithms"))
        self.assertFalse(reported.get("cuda_matmul_allow_tf32"))
        self.assertFalse(reported.get("cudnn_allow_tf32"))
        self.assertTrue(reported.get("cudnn_deterministic"))
        self.assertFalse(reported.get("cudnn_benchmark"))

    def test_configure_determinism_bf16_relaxes_flags(self):
        dummy_torch = DummyTorch()
        # First configure fp32
        worker.configure_determinism(dummy_torch, "fp32")
        # Then switch to bf16
        status = worker.configure_determinism(dummy_torch, "bf16")

        self.assertFalse(dummy_torch.deterministic)
        self.assertTrue(dummy_torch.backends.cuda.matmul.allow_tf32)
        self.assertTrue(dummy_torch.backends.cudnn.allow_tf32)
        self.assertFalse(dummy_torch.backends.cudnn.deterministic)
        self.assertTrue(dummy_torch.backends.cudnn.benchmark)
        self.assertEqual(status.get("deterministic_algorithms"), False)
        self.assertEqual(status.get("cuda_matmul_allow_tf32"), True)
        self.assertEqual(status.get("cudnn_allow_tf32"), True)
        self.assertEqual(status.get("cudnn_deterministic"), False)
        self.assertEqual(status.get("cudnn_benchmark"), True)

    def test_precision_context_triggers_determinism_configuration(self):
        dummy_torch = DummyTorch()
        worker.precision_context(dummy_torch, "cpu", "fp32")
        self.assertTrue(dummy_torch.deterministic)
        self.assertFalse(dummy_torch.backends.cuda.matmul.allow_tf32)

        worker.precision_context(dummy_torch, "cpu", "bf16")
        self.assertFalse(dummy_torch.deterministic)
        self.assertTrue(dummy_torch.backends.cuda.matmul.allow_tf32)

    def test_configure_determinism_handles_none_and_partial_torch_gracefully(self):
        self.assertEqual(worker.configure_determinism(None, "fp32"), {})
        self.assertEqual(worker.get_determinism_status(None), {})

        minimal_torch = object()
        self.assertEqual(worker.configure_determinism(minimal_torch, "fp32"), {})
        self.assertEqual(worker.get_determinism_status(minimal_torch), {})

    def test_fp32_determinism_satisfies_dungeon_spider_contract_thresholds(self):
        from compare_segmentation_outputs import check_acceptance

        # Contract bounds from assets/homologation/16_9_dungeon_spider_iron_plaque/contract.toml:
        # maximum_mean_absolute_error = 12.0, maximum_p95_absolute_error = 25.0
        simulated_fp32_drift_report = {
            "alpha": {"mean_absolute": 0.0001, "p95_absolute": 0.0005},
            "binary_at_0_5": {"iou": 1.0, "disagreement_fraction": 0.0},
        }
        failures = check_acceptance(
            simulated_fp32_drift_report,
            min_iou=0.99,
            max_mean_absolute=12.0 / 255.0,
            max_disagreement_fraction=0.01,
            max_p95_absolute=25.0 / 255.0,
        )
        self.assertEqual(failures, [])


if __name__ == "__main__":
    unittest.main()


