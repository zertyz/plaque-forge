import tempfile
import unittest
from pathlib import Path

import numpy as np
from PIL import Image

from compare_segmentation_outputs import compare


class SegmentationDriftTests(unittest.TestCase):
    def write(self, root: Path, values):
        masks = root / "masks"
        masks.mkdir(parents=True)
        Image.fromarray(np.asarray(values, dtype=np.uint16)).save(masks / "000000.png")

    def test_identical_sequences_have_zero_drift(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            values = [[0, 32768], [65535, 12000]]
            self.write(root / "a", values)
            self.write(root / "b", values)
            report = compare(root / "a", root / "b")
            self.assertEqual(report["alpha"]["maximum_absolute"], 0.0)
            self.assertEqual(report["binary_at_0_5"]["iou"], 1.0)

    def test_binary_disagreement_and_soft_alpha_are_reported(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.write(root / "a", [[0, 65535], [32767, 32768]])
            self.write(root / "b", [[0, 0], [32767, 65535]])
            report = compare(root / "a", root / "b")
            self.assertGreater(report["alpha"]["mean_absolute"], 0.0)
            self.assertEqual(report["binary_at_0_5"]["disagreement_fraction"], 0.25)
            self.assertEqual(report["binary_at_0_5"]["iou"], 0.5)
            self.assertGreater(report["soft_edge_fraction"]["left"], 0.0)

    def test_check_acceptance_detects_iou_and_drift_regressions(self):
        from compare_segmentation_outputs import check_acceptance

        report = {
            "alpha": {"mean_absolute": 0.05, "p95_absolute": 0.12},
            "binary_at_0_5": {"iou": 0.94, "disagreement_fraction": 0.03},
        }
        # Passing thresholds
        failures = check_acceptance(
            report,
            min_iou=0.90,
            max_mean_absolute=0.08,
            max_disagreement_fraction=0.05,
            max_p95_absolute=0.15,
        )
        self.assertEqual(failures, [])

        # Failing thresholds
        failures = check_acceptance(
            report,
            min_iou=0.98,
            max_mean_absolute=0.02,
            max_disagreement_fraction=0.01,
            max_p95_absolute=0.10,
        )
        self.assertEqual(len(failures), 4)
        self.assertTrue(any("iou" in f for f in failures))
        self.assertTrue(any("mean_absolute" in f for f in failures))
        self.assertTrue(any("disagreement" in f for f in failures))
        self.assertTrue(any("p95" in f for f in failures))

    def test_compare_matches_by_common_filename_pairs(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            dir_a = root / "canonical"
            dir_b = root / "candidate" / "masks"
            dir_a.mkdir(parents=True)
            dir_b.mkdir(parents=True)

            Image.fromarray(np.full((10, 10), 65535, dtype=np.uint16)).save(
                dir_a / "000001.png"
            )
            Image.fromarray(np.full((10, 10), 65535, dtype=np.uint16)).save(
                dir_b / "000001.png"
            )

            report = compare(dir_a, root / "candidate")
            self.assertEqual(report["frames"], 1)
            self.assertEqual(report["binary_at_0_5"]["iou"], 1.0)


if __name__ == "__main__":
    unittest.main()
