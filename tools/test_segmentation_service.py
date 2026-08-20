import os
import socket
import struct
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

    def test_request_reports_a_service_that_dies_without_responding(self):
        class DeadServiceConnection:
            def __enter__(self):
                return self

            def __exit__(self, *_args):
                return False

            def settimeout(self, _timeout):
                pass

            def connect(self, _path):
                pass

            def sendall(self, _payload):
                pass

            def getsockopt(self, _level, _option, _size):
                return struct.pack("3i", 424_242, os.getuid(), os.getgid())

            def recv(self, _size):
                raise socket.timeout

        with (
            patch.object(
                segmentation_service.socket,
                "socket",
                return_value=DeadServiceConnection(),
            ),
            patch.object(
                segmentation_service,
                "service_process_alive",
                return_value=False,
                create=True,
            ),
        ):
            with self.assertRaisesRegex(RuntimeError, "exited without a response"):
                segmentation_service.request_server(
                    Path("unused.sock"), {"op": "process"}, timeout=10
                )

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
