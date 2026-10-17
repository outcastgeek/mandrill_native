"""Provenance for the existing non-Zig acceptance gates, in the DSL result tree."""
import hashlib
import json
import sys
from pathlib import Path
PACKAGE = Path(__file__).resolve().parents[1]

def fingerprint(name):
    paths = set((PACKAGE / 'src').rglob('*.zig'))
    if name == 'compile-fail':
        paths.update((PACKAGE / 'tests/compile_fail').glob('*.zig'))
        paths.update((PACKAGE / 'tests/compile_pass').glob('*.zig'))
        paths.update([PACKAGE / 'tests/check_compile_fail.py', PACKAGE / 'tests/dsl_compile_cases.json'])
    elif name == 'consumer':
        paths.update(p for p in (PACKAGE / 'tests/capabilities/consumer').rglob('*') if p.is_file() and '.zig-cache' not in p.parts and 'zig-out' not in p.parts)
        paths.update([PACKAGE / 'tests/stage_consumer.py', PACKAGE / 'build.zig.zon'])
    elif name == 'codegen':
        paths.update([PACKAGE / 'proofs/codegen.zig', PACKAGE / 'tests/check_codegen.py'])
    elif name == 'performance':
        paths.update((PACKAGE / 'proofs/support').glob('*.zig'))
        paths.update([PACKAGE / 'proofs/dsl-performance.zig', PACKAGE / 'tests/check_dsl_performance.py'])
    elif name == 'harness':
        paths.update([PACKAGE / 'tests/check_dsl_coverage.py', PACKAGE / 'tests/dsl_acceptance.json', PACKAGE / 'tests/dsl_compile_cases.json', PACKAGE / 'tests/dsl_execution.zon'])
    digest = hashlib.sha256()
    for path in sorted(paths):
        digest.update(str(path.relative_to(PACKAGE)).encode())
        digest.update(path.read_bytes())
    return digest.hexdigest()

def completed(name, mode, before, details=None):
    if before != fingerprint(name):
        raise SystemExit('source changed during acceptance gate; provenance is invalid')
    output = PACKAGE / 'zig-out/dsl-results' / mode
    output.mkdir(parents=True, exist_ok=True)
    (output / ('gate-' + name + '.json')).write_text(json.dumps(dict(gate=name, mode=mode, fingerprint=before, exit_code=0, details=details), indent=2) + '\n')

if __name__ == '__main__':
    name, mode, artifact = sys.argv[1:]
    document = Path(artifact) / 'index.html' if Path(artifact).is_dir() else Path(artifact)
    if name != 'docs' or not document.is_file():
        raise SystemExit('API documentation output is missing')
    completed(name, mode, fingerprint(name), dict(artifact=str(Path(artifact).resolve())))
