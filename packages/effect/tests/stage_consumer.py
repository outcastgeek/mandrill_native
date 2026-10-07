"""Stage the exported package below an independent consumer package root."""
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

zig, mode, source = sys.argv[1:]
source = Path(source).resolve()
with tempfile.TemporaryDirectory(prefix="effect-consumer-") as directory:
    staged = Path(directory)
    fixture = source / "tests/capabilities/consumer"
    for name in ("build.zig", "build.zig.zon", "src"):
        origin, target = fixture / name, staged / name
        if origin.is_dir():
            shutil.copytree(origin, target)
        else:
            shutil.copy2(origin, target)
    exported = staged / "effect"
    exported.mkdir()
    # Matches build.zig.zon's exported paths; cache/output trees are excluded.
    for name in ("build.zig", "build.zig.zon", "src", "examples", "proofs", "tests", "docs", "README.md"):
        origin, target = source / name, exported / name
        if origin.is_dir():
            shutil.copytree(origin, target, ignore=shutil.ignore_patterns(".zig-cache", "zig-out", "__pycache__"))
        else:
            shutil.copy2(origin, target)
    subprocess.run([zig, "build", "run", f"-Doptimize={mode}"], cwd=staged, check=True)
