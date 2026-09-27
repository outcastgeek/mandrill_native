"""Run the standard Zig test binary with an external hang/stack-risk watchdog.

No custom test runner: Zig still reports assertions, errors and allocator leaks.
The explicit timeout is test policy, never an Effect execution timeout.
"""
import subprocess
import sys

try:
    result = subprocess.run(sys.argv[2:], timeout=float(sys.argv[1]))
except subprocess.TimeoutExpired:
    sys.exit("Zig test subprocess exceeded its test watchdog")
sys.exit(result.returncode)
