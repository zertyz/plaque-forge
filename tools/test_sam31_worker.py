import unittest

import numpy as np

from sam31_worker import prompt_request


class FakeTorch:
    float32 = np.float32
    int32 = np.int32

    @staticmethod
    def tensor(values, dtype):
        return np.asarray(values, dtype=dtype)


class Sam31BridgeContractTests(unittest.TestCase):
    def test_text_concept_is_forwarded_without_geometry(self):
        request = prompt_request(
            {"frame": 4, "concept": "person in red", "positive_points": [], "negative_points": []},
            "session", FakeTorch, 1280, 720,
        )
        self.assertEqual(request["text"], "person in red")
        self.assertEqual(request["frame_index"], 4)
        self.assertNotIn("points", request)

    def test_point_prompt_is_normalized_and_preserves_labels(self):
        request = prompt_request(
            {
                "frame": 2,
                "concept": None,
                "positive_points": [[640.0, 360.0]],
                "negative_points": [[128.0, 72.0]],
            },
            "session", FakeTorch, 1280, 720,
        )
        np.testing.assert_allclose(request["points"], [[0.5, 0.5], [0.1, 0.1]])
        self.assertEqual(request["point_labels"].tolist(), [1, 0])
        self.assertEqual(request["obj_id"], 1)

    def test_box_only_prompt_is_rejected_until_video_geometry_support_is_stable(self):
        with self.assertRaisesRegex(ValueError, "at least one positive point"):
            prompt_request(
                {
                    "frame": 0,
                    "concept": None,
                    "positive_points": [],
                    "negative_points": [],
                    "box_bounds": [0.0, 0.0, 10.0, 10.0],
                },
                "session", FakeTorch, 1280, 720,
            )


if __name__ == "__main__":
    unittest.main()
