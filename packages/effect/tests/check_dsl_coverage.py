"""Validate acceptance registration, real test names, and build reachability.

Registration is evidence indexing, never a substitute for executing a test.
"""
from dsl_gate_evidence import fingerprint, completed
import argparse
import hashlib
import json
import re
from pathlib import Path

COUNTS = {"COMP": 10, "BASE": 4, "HAR": 8, "ROLE": 7, "INF": 14, "NEG": 38,
          "CAP": 12, "LIFE": 12, "LEAF": 7, "SELECT": 3, "ASYNC": 6, "PIPE": 8,
          "PROG": 8, "RES": 6, "COLL": 5, "FAC": 8, "DECL": 5, "CODEC": 7,
          "RPC": 6, "TX": 6, "WF": 9, "STREAM": 8, "SINK": 6, "ATOM": 9,
          "ENTITY": 6, "EVENT": 7, "PERF": 6, "AUD": 5, "EX": 8}
PACKAGE = Path(__file__).resolve().parents[1]


def execution_sources():
    return set(re.findall(r'"((?:proofs/|examples/dsl/)[^"\n]+\.zig)"', (PACKAGE / "tests/dsl_execution.zon").read_text()))

def validate(document, require_all=True, manifest=None, mode=None):
    expected = {f"{family}-{n:02}" for family, count in COUNTS.items() for n in range(1, count + 1)}
    cases = document["cases"]
    seen = [case["id"] for case in cases]
    errors = []
    if len(seen) != len(set(seen)):
        errors.append("duplicate acceptance ID")
    if set(seen) != expected:
        errors.append(f"catalog mismatch missing={sorted(expected - set(seen))} unknown={sorted(set(seen) - expected)}")
    manifest = execution_sources() if manifest is None else manifest
    negatives = json.loads((PACKAGE / "tests/dsl_compile_cases.json").read_text())
    for case in cases:
        if require_all and not case["evidence"]:
            errors.append(f'{case["id"]}: missing mandatory implementation/verification evidence')
        variants = case.get("variants", [])
        names = [variant.get("name") for variant in variants]
        if len(names) != len(set(names)) or any(not name for name in names):
            errors.append(f'{case["id"]}: variant names must be nonempty and unique')
        available = {(item["kind"], item["source"], item.get("name", item.get("diagnostic")))
                     for item in case["evidence"]}
        for variant in variants:
            if require_all and not variant.get("evidence"):
                errors.append(f'{case["id"]}/{variant["name"]}: variant verification remains incomplete')
            for reference in variant.get("evidence", []):
                key = (reference["kind"], reference["source"], reference.get("name", reference.get("diagnostic")))
                if key not in available:
                    errors.append(f'{case["id"]}/{variant["name"]}: variant references unregistered evidence')
        for evidence in case["evidence"]:
            path = PACKAGE / evidence["source"]
            if not evidence.get("work_packages") or not evidence.get("requirements"):
                errors.append(f'{case["id"]}: missing work-package/requirement mapping')
            if not path.is_file():
                errors.append(f'{case["id"]}: missing source {path}')
                continue
            source = path.read_text()
            kind = evidence["kind"]
            if kind == "test":
                if evidence["name"] not in re.findall(r'\btest\s+"([^"\n]+)"', source):
                    errors.append(f'{case["id"]}: nonexistent test name')
                if evidence["source"] not in manifest:
                    errors.append(f'{case["id"]}: proof absent from execution harness')
                if mode:
                    result_path = PACKAGE / "zig-out/dsl-results" / mode / (path.stem + ".json")
                    if not result_path.is_file():
                        errors.append(f'{case["id"]}: missing actual execution record')
                    else:
                        result = json.loads(result_path.read_text())
                        digest = hashlib.sha256()
                        for dependency in sorted((PACKAGE / "src").rglob("*.zig")) + sorted((PACKAGE / "proofs/support").rglob("*.zig")) + sorted((PACKAGE / "examples/dsl").rglob("*.zig")) + [path]:
                            digest.update(str(dependency.relative_to(PACKAGE)).encode())
                            digest.update(dependency.read_bytes())
                        if result.get("fingerprint") != digest.hexdigest() or evidence["name"] not in result.get("tests", []):
                            errors.append(f'{case["id"]}: stale or unexecuted test evidence')

            elif kind == "compile-fail":
                key = path.stem
                if key not in negatives or negatives[key]["diagnostic"] != evidence["diagnostic"]:
                    errors.append(f'{case["id"]}: negative case absent from diagnostic harness')
                companion = PACKAGE / evidence["valid_companion"]
                if not companion.is_file() or negatives.get(key, {}).get("valid_companion") != evidence["valid_companion"]:
                    errors.append(f'{case["id"]}: valid companion absent from execution harness')
            elif kind == "gate":
                gate = evidence["name"]
                if gate not in {"consumer", "codegen", "compile-fail", "harness", "docs", "performance"}:
                    errors.append(f'{case["id"]}: unknown existing gate')
                if mode:
                    selected_mode = evidence.get("execution_mode", mode)
                    record = PACKAGE / "zig-out/dsl-results" / selected_mode / ("gate-" + gate + ".json")
                    if not record.is_file():
                        errors.append(f'{case["id"]}: missing actual {gate} gate execution')
                    else:
                        observed = json.loads(record.read_text())
                        if observed.get("fingerprint") != fingerprint(gate) or observed.get("exit_code") != 0:
                            errors.append(f'{case["id"]}: stale or unsuccessful {gate} gate')
            elif kind == "record":
                if hashlib.sha256(path.read_bytes()).hexdigest() != evidence["sha256"]:
                    errors.append(f'{case["id"]}: historical evidence changed')
            else:
                errors.append(f'{case["id"]}: unsupported evidence kind {kind}')
    return errors


def self_test(document):
    import copy
    mutation = copy.deepcopy(document)
    mutation["cases"].append(copy.deepcopy(mutation["cases"][0]))
    assert any("duplicate" in e for e in validate(mutation, False))
    mutation = copy.deepcopy(document)
    target = next((c for c in mutation["cases"] if c["evidence"]), None)
    if target:
        target["evidence"][0]["name"] = "nonexistent-test-name"
        assert any("nonexistent" in e for e in validate(mutation, False))
        original_source = target["evidence"][0]["source"]
        removed = execution_sources() - {original_source}
        assert any("absent from execution harness" in e for e in validate(document, False, manifest=removed))
        target["evidence"][0]["source"] = "proofs/unregistered.zig"
        assert any("missing source" in e for e in validate(mutation, False))
    mutation = copy.deepcopy(document)
    target = next((c for c in mutation["cases"] if c.get("variants")), None)
    if target:
        target["variants"][0]["evidence"] = [{"kind": "test", "source": "proofs/unregistered.zig", "name": "not-a-test"}]
        assert any("variant references unregistered evidence" in e for e in validate(mutation, False))


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--registered-only", action="store_true", help="development check; never full acceptance")
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument("--execution", choices=["debug", "safe", "fast", "small"])
    args = parser.parse_args()
    document = json.loads((PACKAGE / "tests/dsl_acceptance.json").read_text())
    if args.self_test:
        before = fingerprint("harness")
        self_test(document)
        completed("harness", args.execution or "debug", before)
    errors = validate(document, not args.registered_only, mode=args.execution)
    if errors:
        raise SystemExit("\n".join(errors))
    print(f'{len(document["cases"])} catalog obligations indexed; registration validated (execution results remain separate)')
