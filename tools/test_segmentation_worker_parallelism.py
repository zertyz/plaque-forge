#!/usr/bin/env python3
import unittest
from pathlib import Path
from unittest import mock

import segmentation_worker as worker


class SegmentationWorkerParallelismTests(unittest.TestCase):
    def test_usable_cpu_count_prefers_process_affinity(self):
        with (
            mock.patch.object(
                worker.os, "sched_getaffinity", return_value={2, 4, 6}, create=True
            ),
            mock.patch.object(worker.os, "cpu_count", return_value=128),
        ):
            self.assertEqual(worker.usable_cpu_count(), 3)

    def test_opencv_outer_workers_discount_native_threads(self):
        fake_cv2 = mock.Mock()
        fake_cv2.getNumThreads.return_value = 4
        with (
            mock.patch.object(worker, "cv2", fake_cv2),
            mock.patch.object(worker, "usable_cpu_count", return_value=16),
        ):
            self.assertEqual(worker.opencv_parallel_worker_count(20), 4)

    def test_sam2_loader_bounds_inflight_tensor_count(self):
        with mock.patch.object(worker, "usable_cpu_count", return_value=128):
            self.assertEqual(worker.sam2_loader_worker_count(100), 25)
            self.assertEqual(worker.sam2_loader_worker_count(3), 1)

    @unittest.skipIf(worker.np is None, "NumPy unavailable")
    def test_motion_envelope_uses_true_ceil(self):
        request = {
            "layer": {
                "role": "foreground",
                "prompts": [
                    {"frame": 0, "object": "subject", "box_bounds": [0, 0, 30.00005, 10]},
                    {"frame": 1, "object": "subject", "box_bounds": [0, 0, 30.00005, 10]},
                ],
            }
        }
        probabilities = [
            worker.np.zeros((64, 64), dtype=worker.np.float32) for _ in range(2)
        ]
        probabilities[0][0, 50] = 1.0
        constrained = worker.constrain_to_authored_motion_envelope(request, probabilities)
        self.assertEqual(float(constrained[0][0, 50]), 1.0)

    @unittest.skipIf(worker.np is None, "NumPy unavailable")
    def test_failed_opencv_write_is_not_silent(self):
        fake_cv2 = mock.Mock()
        fake_cv2.IMWRITE_PNG_COMPRESSION = 16
        fake_cv2.imwrite.return_value = False
        probability = worker.np.zeros((2, 2), dtype=worker.np.float32)
        with mock.patch.object(worker, "cv2", fake_cv2):
            with self.assertRaises(OSError):
                worker.save_probability_png(Path("unwritable.png"), probability)

    def test_failed_required_opencv_decode_names_path(self):
        fake_cv2 = mock.Mock()
        fake_cv2.imread.return_value = None
        with mock.patch.object(worker, "cv2", fake_cv2):
            with self.assertRaisesRegex(OSError, "missing.png"):
                worker.require_cv2_image(Path("missing.png"), 0)


if __name__ == "__main__":
    unittest.main()
