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


if __name__ == "__main__":
    unittest.main()
