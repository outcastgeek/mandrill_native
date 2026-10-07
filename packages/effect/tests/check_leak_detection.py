"""Prove that allocator leaks fail the native Zig test runner and build target."""
from pathlib import Path
import re
import subprocess
import sys

package = Path(__file__).resolve().parents[1]
zig = sys.argv[1] if len(sys.argv) > 1 else "zig"
cases = (
    ("leak detector accepts complete teardown", False),
    ("leak detector rejects unreleased allocation", True),
)

for mode in ("debug", "safe"):
    for name, should_leak in cases:
        executed = subprocess.run(
            [zig, "build", "--build-file", str(package / "tests" / "leak_detection_build.zig"),
             "--cache-dir", str(package / ".zig-cache" / "leak-check"),
             f"-Doptimize={mode}", f"-Dleak={'true' if should_leak else 'false'}",
             "--summary", "all"],
            cwd=package, capture_output=True, text=True,
        )
        output = executed.stdout + executed.stderr
        # The selected test must pass its own assertions. Compilation errors,
        # traps, launch errors, and assertion failures cannot count as evidence.
        leak_line = r"^error: 'leak_detection\.test\." + re.escape(name) + r"' leaked 1 allocations:$"
        detected_leak = re.search(leak_line, output, re.MULTILINE) is not None
        unexpected = re.sub(leak_line, "", output, flags=re.MULTILINE)
        if ("fixture assertions completed" not in output or "1 pass (1 total)" not in output
                or re.search(r"(?:error:|panic:|[1-9][0-9]* (?:fail|crash|timeout) \()", unexpected)):
            sys.exit(f"{mode}: fixture did not finish its assertions\n{output}")
        if should_leak:
            if executed.returncode == 0 or not detected_leak:
                sys.exit(f"{mode}: leaked allocation did not fail the build\n{output}")
            print(f"{mode}: leaked allocation fails the test build")
        elif executed.returncode != 0 or "leaked" in output:
            sys.exit(f"{mode}: complete teardown was rejected\n{output}")
        else:
            print(f"{mode}: complete teardown passes the test build")
