"""Instantiate negative consumers and require their intended diagnostic."""
from dsl_gate_evidence import fingerprint, completed
_gate_before = fingerprint('compile-fail')
from pathlib import Path
import tempfile
import json
import re
import subprocess
import sys

package = Path(__file__).resolve().parents[1]
zig = sys.argv[1] if len(sys.argv) > 1 else "zig"
cases = {
    "private_owned_evidence": "owned payload requires public ownership, clone and deinit evidence",
    "service_required_pure": "codec requires services and cannot provide pure codec evidence",
    "contradictory_exact_bounds": "invalid static JSON-number bounds",
    "missing_writer_evidence": "codec has no direct JSON writer evidence",
    "empty_alternatives": "alternative definition cannot be empty",
    "invalid_native_field": "native record requires every named field exactly once",
    "wrong_continuation_environment": "flatMap callback must accept u32 and the current environment as *const",
    "wrong_ref_modifier": "Ref.modify requires a function from the stored type to a struct with result and a value of the stored type",
    "native_missing_io": "Native Effect execution requires environment .io: std.Io",
    "missing_parallel_engine": "Effect execution requires environment .parallel_engine: effect.Executor",
    "reusable_owned_payload": "Reusable owned success requires a lazy sync/fromOutcome factory or a scoped resource",
    "weak_cache": "WeakCache requires GC ephemeron semantics",
    "owned_layer_service": "Layer services must be borrowed dependency records or handles",
    "wrong_success": "flatMap callback must accept u16",
    "wrong_error": "flatMap error type mismatch",
    "wrong_environment": "flatMap environment mismatch",
    "wrong_allocator": "Effect environment .allocator must be std.mem.Allocator",
    "wrong_release": "acquireRelease finalizer must accept",
    "missing_allocator": "acquireRelease requires environment .allocator",
    "unhandled_exit": "switch must handle all possibilities",
    "owned_wrong_error": "flatMapOwned error type mismatch",
    "owned_const_source": "cast discards const qualifier",
    "wrong_workflow_effect": "WorkflowBuilder accepts only its described durable steps",
    "incomplete_record_builder": "has no member named 'build'",
    "unpaired_rpc_operation": "protocol members must pair operations with handlers",
    "wrong_rpc_stream_handler": "cannot cast into return type 'effect.Stream",
    "forged_rpc_descriptor": "an RPC operation needs a stable name",
    "transaction_rejects_effect": "transaction composition accepts only a typed transaction description",
}
dsl_cases = json.loads((package / "tests/dsl_compile_cases.json").read_text())
cases.update({name: entry["diagnostic"] for name, entry in dsl_cases.items()})
def intended_diagnostic(stderr, diagnostic):
    messages = re.findall(r"^.*?: (?:error|note): (.*)$", stderr, re.MULTILINE)
    return any(diagnostic in message for message in messages)

# HAR-03: an unrelated syntax failure is a harness failure, even when the
# expected words appear in source. This is part of the existing negative gate.
with tempfile.TemporaryDirectory(prefix="effect-negative-harness-") as temporary:
    malformed = Path(temporary) / "unrelated.zig"
    malformed.write_text("// " + next(iter(cases.values())) + "\npub fn main( {\n")
    unrelated = subprocess.run([zig, "build-exe", "-fno-emit-bin", str(malformed)], cwd=package, capture_output=True, text=True)
    if unrelated.returncode == 0 or intended_diagnostic(unrelated.stderr, next(iter(cases.values()))):
        raise SystemExit("negative harness accepted an unrelated syntax failure")

for name, diagnostic in cases.items():
    checked = subprocess.run(
        [zig, "build-exe", "-fno-emit-bin", "--dep", "effect",
         f"-Mroot={package / 'tests' / 'compile_fail' / (name + '.zig')}",
         f"-Meffect={package / 'src' / 'root.zig'}"],
        cwd=package, capture_output=True, text=True,
    )
    if checked.returncode == 0 or not intended_diagnostic(checked.stderr, diagnostic):
        sys.exit(f"{name}: expected compiler rejection containing {diagnostic!r}\n{checked.stderr}")
    print(f"{name}: rejected for the intended reason")

for name, entry in dsl_cases.items():
    companion = package / entry["valid_companion"]
    checked = subprocess.run([zig, "build-exe", "-fno-emit-bin", "--dep", "effect", f"-Mroot={companion}", f"-Meffect={package / 'src/root.zig'}"], cwd=package, capture_output=True, text=True)
    if checked.returncode != 0:
        sys.exit(f"{name}: valid companion failed independently\n{checked.stderr}")
    print(f"{name}: valid companion compiled independently")

completed('compile-fail', 'shared', _gate_before)
