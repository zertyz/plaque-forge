#!/usr/bin/env python3
import os
import unittest
from unittest.mock import patch

import segmentation_worker as worker


class ResidentModelCacheTests(unittest.TestCase):
    def setUp(self):
        worker.MODEL_CACHE.clear()

    def tearDown(self):
        worker.MODEL_CACHE.clear()

    def test_reuses_model_for_identical_key(self):
        builds = []

        def factory():
            builds.append(object())
            return builds[-1]

        first, first_hit = worker.cached_model(("unit", "same"), factory)
        second, second_hit = worker.cached_model(("unit", "same"), factory)

        self.assertFalse(first_hit)
        self.assertTrue(second_hit)
        self.assertIs(first, second)
        self.assertEqual(len(builds), 1)

    def test_cache_is_bounded_and_evicts_oldest_entry(self):
        with patch.dict(os.environ, {"PLAQUE_FORGE_MODEL_CACHE_ENTRIES": "1"}):
            first, _ = worker.cached_model(("unit", "first"), object)
            second, second_hit = worker.cached_model(("unit", "second"), object)
            rebuilt, rebuilt_hit = worker.cached_model(("unit", "first"), object)

        self.assertFalse(second_hit)
        self.assertFalse(rebuilt_hit)
        self.assertIsNot(first, rebuilt)
        self.assertIsNot(second, rebuilt)
        self.assertEqual(len(worker.MODEL_CACHE), 1)

    def test_cache_can_be_disabled_without_changing_execution_semantics(self):
        builds = []

        def factory():
            builds.append(object())
            return builds[-1]

        with patch.dict(os.environ, {"PLAQUE_FORGE_MODEL_CACHE": "0"}):
            first, first_hit = worker.cached_model(("unit", "disabled"), factory)
            second, second_hit = worker.cached_model(("unit", "disabled"), factory)

        self.assertFalse(first_hit)
        self.assertFalse(second_hit)
        self.assertIsNot(first, second)
        self.assertEqual(len(builds), 2)
        self.assertEqual(worker.MODEL_CACHE, {})


if __name__ == "__main__":
    unittest.main()
