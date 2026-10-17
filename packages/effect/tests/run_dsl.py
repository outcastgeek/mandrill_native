"""Run a registered public proof and record actual per-test execution."""
import hashlib
import json
from pathlib import Path
import re
import subprocess
import sys

package = Path(__file__).resolve().parents[1]
source, mode, binary = sys.argv[1:]
# The entire production source graph participates; a prior result cannot be
# reused after any implementation change merely because a proof is unchanged.
def fingerprint():
    digest = hashlib.sha256()
    for path in sorted((package / 'src').rglob('*.zig')) + sorted((package / 'proofs/support').rglob('*.zig')) + sorted((package / 'examples/dsl').rglob('*.zig')) + [package / source]:
        digest.update(str(path.relative_to(package)).encode())
        digest.update(path.read_bytes())
    return digest.hexdigest()

before = fingerprint()
try:
    checked = subprocess.run([binary], capture_output=True, text=True, timeout=120)
except subprocess.TimeoutExpired:
    raise SystemExit('DSL proof exceeded the test watchdog')
sys.stdout.write(checked.stdout)
sys.stderr.write(checked.stderr)
if checked.returncode:
    raise SystemExit(checked.returncode)
if fingerprint() != before:
    raise SystemExit('source changed while proof ran; execution provenance is invalid')
names = re.findall(r'^\d+/\d+\s+[^\n]*?\.test\.([^\n]*?)\.\.\.OK$', checked.stderr, re.MULTILINE)
# Zig may print progress prefixes and test output before OK. No unobserved name
# is fabricated; coverage refuses missing execution records.
if not names:
    raise SystemExit('proof produced no identifiable passing tests')
output = package / 'zig-out/dsl-results' / mode
output.mkdir(parents=True, exist_ok=True)
(output / (Path(source).stem + '.json')).write_text(json.dumps(dict(source=source, mode=mode, fingerprint=before, tests=names, exit_code=checked.returncode), indent=2) + '\n')
