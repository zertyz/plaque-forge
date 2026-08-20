#!/usr/bin/env python3
"""Small local transport that keeps Plaque Forge's heavyweight Python runtime resident.

The Rust contract remains a process invocation (`tools/segmentation-worker`). The wrapper
uses this service as an implementation detail, so tests and alternate workers do not need
to understand sockets or Python lifecycle.
"""

import argparse
import contextlib
import fcntl
import hashlib
import io
import json
import os
import socket
import struct
import subprocess
import sys
import time
import traceback
from pathlib import Path


def runtime_root():
    return Path(os.environ.get("PLAQUE_FORGE_PYTHON_ROOT", "/tmp/plaque-forge-python"))


def service_identity(repo=None):
    repo = Path(repo or Path(__file__).resolve().parent.parent)
    digest = hashlib.sha256()
    for relative in [
        "tools/segmentation_service.py",
        "tools/segmentation_worker.py",
        "tools/segmentation_runtime.py",
        "tools/segmentation-requirements.txt",
    ]:
        path = repo / relative
        digest.update(relative.encode())
        digest.update(b"\0")
        digest.update(path.read_bytes())
        digest.update(b"\0")
    # A source-identical service must still restart after the Python environment is
    # rebuilt. Otherwise already-imported torch/model packages could execute under an
    # old runtime while Rust seals provenance for the new runtime manifest.
    manifest = runtime_root() / "runtime-manifest.json"
    if manifest.is_file():
        digest.update(b"runtime-manifest.json\0")
        digest.update(manifest.read_bytes())
        digest.update(b"\0")
    return digest.hexdigest()


def service_paths(identity):
    run = runtime_root() / "run"
    run.mkdir(parents=True, exist_ok=True)
    short = identity[:20]
    return (
        run / f"segmentation-{short}.sock",
        run / "segmentation-start.lock",
        run / f"segmentation-{short}.log",
    )


def service_process_alive(pid):
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        return False
    except PermissionError:
        return True
    return True


def connected_service_pid(connection):
    if not hasattr(socket, "SO_PEERCRED"):
        return None
    try:
        credentials = connection.getsockopt(
            socket.SOL_SOCKET, socket.SO_PEERCRED, struct.calcsize("3i")
        )
    except OSError:
        return None
    return struct.unpack("3i", credentials)[0]


def request_server(socket_path, payload, timeout=24 * 60 * 60):
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as connection:
        deadline = time.monotonic() + timeout
        connection.settimeout(min(timeout, 2))
        connection.connect(str(socket_path))
        peer_pid = connected_service_pid(connection)
        connection.sendall(json.dumps(payload).encode("utf-8") + b"\n")
        data = bytearray()
        while True:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise TimeoutError("persistent segmentation service timed out")
            connection.settimeout(min(remaining, 2))
            try:
                chunk = connection.recv(65536)
            except socket.timeout:
                if peer_pid is not None and not service_process_alive(peer_pid):
                    raise RuntimeError(
                        f"persistent segmentation service process {peer_pid} exited without a response"
                    )
                continue
            if not chunk:
                break
            data.extend(chunk)
            if b"\n" in data:
                break
    if not data:
        raise RuntimeError("persistent segmentation service returned no response")
    return json.loads(bytes(data).split(b"\n", 1)[0].decode("utf-8"))


def server_ready(socket_path, identity):
    if not socket_path.exists():
        return False
    try:
        response = request_server(socket_path, {"op": "ping"}, timeout=2)
        return response.get("ok") is True and response.get("identity") == identity
    except (OSError, RuntimeError, json.JSONDecodeError):
        return False


def ensure_server(repo, identity):
    socket_path, lock_path, log_path = service_paths(identity)
    if server_ready(socket_path, identity):
        return socket_path
    with lock_path.open("a+") as lock:
        fcntl.flock(lock.fileno(), fcntl.LOCK_EX)
        if server_ready(socket_path, identity):
            return socket_path
        try:
            socket_path.unlink()
        except FileNotFoundError:
            pass
        log = log_path.open("ab", buffering=0)
        subprocess.Popen(
            [sys.executable, str(Path(__file__).resolve()), "serve", "--identity", identity],
            cwd=str(repo),
            stdin=subprocess.DEVNULL,
            stdout=log,
            stderr=subprocess.STDOUT,
            start_new_session=True,
            close_fds=True,
        )
        deadline = time.monotonic() + 120
        while time.monotonic() < deadline:
            if server_ready(socket_path, identity):
                return socket_path
            time.sleep(0.1)
        raise RuntimeError(
            f"persistent segmentation service did not start; inspect {log_path}"
        )


def client(args):
    repo = Path(__file__).resolve().parent.parent
    identity = service_identity(repo)
    socket_path = ensure_server(repo, identity)
    response = request_server(
        socket_path,
        {
            "op": "process",
            "identity": identity,
            "request": str(args.request.resolve()),
            "output": str(args.output.resolve()),
        },
    )
    if response.get("stdout"):
        sys.stdout.write(response["stdout"])
        sys.stdout.flush()
    if response.get("stderr"):
        sys.stderr.write(response["stderr"])
        sys.stderr.flush()
    if not response.get("ok"):
        error = response.get("error", "persistent worker failed")
        trace = response.get("traceback")
        if trace:
            sys.stderr.write(trace)
        raise RuntimeError(error)
    print(
        f"[ml] persistent Python service: pid={response.get('pid')}, identity={identity[:12]}",
        file=sys.stderr,
    )


def serve(args):
    identity = args.identity
    expected = service_identity()
    if identity != expected:
        raise RuntimeError("service identity does not match checked-out worker sources")
    socket_path, _, log_path = service_paths(identity)
    try:
        socket_path.unlink()
    except FileNotFoundError:
        pass
    idle_seconds = int(os.environ.get("PLAQUE_FORGE_SERVICE_IDLE_SECONDS", "1200"))
    import segmentation_worker as worker_module

    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as listener:
        listener.bind(str(socket_path))
        os.chmod(socket_path, 0o600)
        listener.listen(4)
        listener.settimeout(max(1, idle_seconds))
        print(
            f"[ml-service] ready pid={os.getpid()} identity={identity[:12]} socket={socket_path}",
            flush=True,
        )
        while True:
            try:
                connection, _ = listener.accept()
            except socket.timeout:
                print(f"[ml-service] idle timeout after {idle_seconds}s", flush=True)
                break
            with connection:
                raw = bytearray()
                while True:
                    chunk = connection.recv(65536)
                    if not chunk:
                        break
                    raw.extend(chunk)
                    if b"\n" in raw:
                        break
                try:
                    message = json.loads(bytes(raw).split(b"\n", 1)[0].decode("utf-8"))
                    if message.get("op") == "ping":
                        response = {"ok": True, "identity": identity, "pid": os.getpid()}
                    elif message.get("op") == "process":
                        if message.get("identity") != identity:
                            raise RuntimeError("client/service source identity mismatch")
                        stdout = io.StringIO()
                        stderr = io.StringIO()
                        try:
                            with contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
                                worker_module.process_request(Path(message["request"]), Path(message["output"]))
                            response = {
                                "ok": True,
                                "identity": identity,
                                "pid": os.getpid(),
                                "stdout": stdout.getvalue(),
                                "stderr": stderr.getvalue(),
                            }
                        except Exception as error:
                            worker_module.clear_resident_models()
                            response = {
                                "ok": False,
                                "identity": identity,
                                "pid": os.getpid(),
                                "stdout": stdout.getvalue(),
                                "stderr": stderr.getvalue(),
                                "error": f"{type(error).__name__}: {error}",
                                "traceback": traceback.format_exc(),
                            }
                    else:
                        raise ValueError("unsupported persistent-worker operation")
                except Exception as error:
                    response = {
                        "ok": False,
                        "identity": identity,
                        "pid": os.getpid(),
                        "error": f"{type(error).__name__}: {error}",
                        "traceback": traceback.format_exc(),
                    }
                connection.sendall(json.dumps(response).encode("utf-8") + b"\n")
    try:
        socket_path.unlink()
    except FileNotFoundError:
        pass
    print(f"[ml-service] stopped; log={log_path}", flush=True)


def main():
    parser = argparse.ArgumentParser()
    sub = parser.add_subparsers(dest="command", required=True)
    client_parser = sub.add_parser("client")
    client_parser.add_argument("--request", type=Path, required=True)
    client_parser.add_argument("--output", type=Path, required=True)
    server_parser = sub.add_parser("serve")
    server_parser.add_argument("--identity", required=True)
    args = parser.parse_args()
    if args.command == "client":
        client(args)
    else:
        serve(args)


if __name__ == "__main__":
    main()
