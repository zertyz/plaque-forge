#!/usr/bin/env python3
import argparse
import json
import tomllib
from pathlib import Path

FORMAT = "plaque-forge.segmentation-capabilities/1"
REPORT_FORMAT = "plaque-forge.segmentation-capability-coverage/1"


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--json", type=Path)
    parser.add_argument(
        "matrix",
        type=Path,
        nargs="?",
        default=Path("assets/homologation/segmentation-capabilities.toml"),
    )
    args = parser.parse_args()

    document = tomllib.loads(args.matrix.read_text(encoding="utf-8"))
    if document.get("format") != FORMAT:
        raise SystemExit("unsupported segmentation capability matrix")
    capabilities = document.get("capabilities", [])
    identifiers = [capability.get("id") for capability in capabilities]
    if not capabilities or len(identifiers) != len(set(identifiers)) or any(not value for value in identifiers):
        raise SystemExit("invalid or duplicate segmentation capability id")

    represented = 0
    homologated = 0
    for capability in capabilities:
        asset = str(capability.get("representative_asset", "")).strip()
        contract = str(capability.get("final_homologation", "")).strip()
        if asset:
            scene = Path("assets/scenes") / asset / "scene.toml"
            source = Path("assets") / f"{asset}.mp4"
            if not scene.is_file() or not source.is_file():
                raise SystemExit(
                    f"segmentation capability {capability['id']!r} references missing representative asset {asset!r}"
                )
            represented += 1
        if contract:
            if not asset:
                raise SystemExit(
                    f"segmentation capability {capability['id']!r} has homologation without a representative asset"
                )
            path = Path("assets/homologation") / contract
            if not path.is_file():
                raise SystemExit(
                    f"segmentation capability {capability['id']!r} references missing homologation contract {contract!r}"
                )
            homologated += 1

    minimum_represented = int(document.get("minimum_represented", 0))
    minimum_homologated = int(document.get("minimum_final_homologated", 0))
    if represented < minimum_represented:
        raise SystemExit(
            f"segmentation capability coverage regressed: represented={represented}, required>={minimum_represented}"
        )
    if homologated < minimum_homologated:
        raise SystemExit(
            f"segmentation homologation coverage regressed: final_homologated={homologated}, required>={minimum_homologated}"
        )

    report = {
        "format": REPORT_FORMAT,
        "capabilities": len(capabilities),
        "represented": represented,
        "final_homologated": homologated,
        "minimum_represented": minimum_represented,
        "minimum_final_homologated": minimum_homologated,
        "complete": homologated == len(capabilities),
    }
    text = json.dumps(report, indent=2) + "\n"
    print(text, end="")
    if args.json:
        args.json.parent.mkdir(parents=True, exist_ok=True)
        args.json.write_text(text, encoding="utf-8")


if __name__ == "__main__":
    main()
