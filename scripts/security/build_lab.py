"""Build identical vulnerable/fixed SQL controls from a preloaded immutable Python base."""
import argparse
import json
import re
import subprocess
from pathlib import Path


# No base downloads or build network access: the operator selects a loaded digest.
def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base", required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if not re.fullmatch(r"(?:[a-zA-Z0-9./:_-]+@)?sha256:[a-f0-9]{64}", args.base):
        raise ValueError("--base must be an immutable preloaded image ID or digest")
    context = Path(__file__).resolve().parent / "fixtures"
    images = {}
    for mode in ("vulnerable", "fixed"):
        result = subprocess.run(["docker", "build", "--quiet", "--pull=false", "--network=none",
                                 "--build-arg", "PYTHON_IMAGE=" + args.base,
                                 "--build-arg", "LAB_MODE=" + mode, str(context)],
                                check=True, capture_output=True, text=True, timeout=300)
        image = result.stdout.strip().splitlines()[-1]
        if not re.fullmatch(r"sha256:[a-f0-9]{64}", image):
            raise ValueError("Docker did not return an immutable image ID")
        images[mode] = image
    plan = {"version": 1, "cases": [{"id": "sql-injection", "seconds": 20,
            "vulnerable_image": images["vulnerable"], "fixed_image": images["fixed"],
            "probe_image": images["fixed"], "ready": ["python", "/lab/probe.py", "ready"],
            "probe": ["python", "/lab/probe.py", "probe"]}]}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(plan, indent=2) + "\n")


if __name__ == "__main__":
    main()
