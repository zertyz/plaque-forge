import os
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import segmentation_service


class ServiceIdentityTest(unittest.TestCase):
    def test_identity_changes_when_worker_source_changes(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            tools = root / "tools"
            tools.mkdir()
            for name in [
                "segmentation_service.py",
                "segmentation_worker.py",
                "segmentation_runtime.py",
                "segmentation-requirements.txt",
            ]:
                (tools / name).write_text(name, encoding="utf-8")
            before = segmentation_service.service_identity(root)
            (tools / "segmentation_worker.py").write_text("changed", encoding="utf-8")
            after = segmentation_service.service_identity(root)
            self.assertNotEqual(before, after)

    def test_identity_changes_when_runtime_manifest_changes(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            tools = root / "tools"
            tools.mkdir()
            for name in [
                "segmentation_service.py",
                "segmentation_worker.py",
                "segmentation_runtime.py",
                "segmentation-requirements.txt",
            ]:
                (tools / name).write_text(name, encoding="utf-8")
            runtime = root / "runtime"
            runtime.mkdir()
            manifest = runtime / "runtime-manifest.json"
            manifest.write_text('{"runtime": 1}\n', encoding="utf-8")
            with patch.dict(os.environ, {"PLAQUE_FORGE_PYTHON_ROOT": str(runtime)}):
                before = segmentation_service.service_identity(root)
                manifest.write_text('{"runtime": 2}\n', encoding="utf-8")
                after = segmentation_service.service_identity(root)
            self.assertNotEqual(before, after)


if __name__ == "__main__":
    unittest.main()
