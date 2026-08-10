#!/usr/bin/env python3
from __future__ import annotations

import json
import py_compile
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def strip_rust_noncode(source: str) -> str:
    out: list[str] = []
    i = 0
    block_depth = 0
    while i < len(source):
        if block_depth:
            if source.startswith("/*", i):
                block_depth += 1
                out.extend("  ")
                i += 2
            elif source.startswith("*/", i):
                block_depth -= 1
                out.extend("  ")
                i += 2
            else:
                out.append("\n" if source[i] == "\n" else " ")
                i += 1
            continue
        if source.startswith("//", i):
            end = source.find("\n", i)
            if end < 0:
                out.extend(" " * (len(source) - i))
                break
            out.extend(" " * (end - i))
            i = end
            continue
        if source.startswith("/*", i):
            block_depth = 1
            out.extend("  ")
            i += 2
            continue
        is_string = source[i] == '"'
        is_char = source[i] == "'" and i + 2 < len(source) and (
            source[i + 2] == "'" or source[i + 1] == "\\"
        )
        if is_string or is_char:
            quote = source[i]
            out.append(" ")
            i += 1
            escaped = False
            while i < len(source):
                char = source[i]
                out.append("\n" if char == "\n" else " ")
                i += 1
                if escaped:
                    escaped = False
                elif char == "\\":
                    escaped = True
                elif char == quote:
                    break
            continue
        out.append(source[i])
        i += 1
    if block_depth:
        raise ValueError("unterminated block comment")
    return "".join(out)


def check_delimiters(path: Path) -> None:
    source = strip_rust_noncode(path.read_text())
    pairs = {")": "(", "]": "[", "}": "{"}
    stack: list[tuple[str, int]] = []
    for index, char in enumerate(source):
        if char in "([{":
            stack.append((char, index))
        elif char in pairs:
            if not stack or stack[-1][0] != pairs[char]:
                raise ValueError(f"{path}: mismatched {char} at byte {index}")
            stack.pop()
    if stack:
        char, index = stack[-1]
        raise ValueError(f"{path}: unclosed {char} at byte {index}")


def main() -> None:
    with (ROOT / "Cargo.toml").open("rb") as handle:
        cargo = tomllib.load(handle)
    assert cargo["package"]["name"] == "plaque-forge"
    assert cargo["package"]["version"].startswith("0.3.")
    with (ROOT / "config/default.toml").open("rb") as handle:
        tomllib.load(handle)

    rust_files = sorted((ROOT / "src").rglob("*.rs")) + sorted((ROOT / "tests").rglob("*.rs"))
    if not rust_files:
        raise RuntimeError("no Rust files found")
    for path in rust_files:
        check_delimiters(path)
        text = path.read_text()
        for marker in ("todo!()", "unimplemented!()"):
            if marker in text:
                raise RuntimeError(f"placeholder {marker} in {path}")

    py_compile.compile(str(ROOT / "tools/reference_validate_m3.py"), doraise=True)
    required = [
        "src/analyze/extraction.rs",
        "src/analyze/occlusion.rs",
        "src/metadata.rs",
        "src/metadata_commands.rs",
        "src/render/typography.rs",
        "src/verify/mod.rs",
        "METADATA.md",
        "README.md",
        "VALIDATION.md",
    ]
    missing = [name for name in required if not (ROOT / name).is_file()]
    if missing:
        raise RuntimeError(f"missing required files: {missing}")

    print(json.dumps({"rust_files": len(rust_files), "status": "structurally valid"}, indent=2))


if __name__ == "__main__":
    try:
        main()
    except Exception as error:
        print(f"validation failed: {error}", file=sys.stderr)
        raise
