"""Instantiate negative consumers and require their intended diagnostic."""
from pathlib import Path
import subprocess
import sys

package = Path(__file__).resolve().parents[1]
zig = sys.argv[1] if len(sys.argv) > 1 else "zig"
cases = {
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
for name, diagnostic in cases.items():
    checked = subprocess.run(
        [zig, "build-exe", "-fno-emit-bin", "--dep", "effect",
         f"-Mroot={package / 'tests' / 'compile_fail' / (name + '.zig')}",
         f"-Meffect={package / 'src' / 'root.zig'}"],
        cwd=package, capture_output=True, text=True,
    )
    if checked.returncode == 0 or diagnostic not in checked.stderr:
        sys.exit(f"{name}: expected compiler rejection containing {diagnostic!r}\n{checked.stderr}")
    print(f"{name}: rejected for the intended reason")
