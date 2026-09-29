"""Run the plan's configured campaigns and reject incomplete evidence.

The Rust campaign executables own semantics and emit per-campaign counts. This
script supplies reproducible workloads, captures commands and refuses partial
or timed-out results. It is orchestration, not an execution runtime.
"""

import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import time


ROOT = Path(__file__).resolve().parents[3]
MINIMUMS = {
    "schema": {"seeds": 1000},
    "encoded_input": {"mutations": 100000},
    "persistence": {"seeds": 1000, "transitions": 100},
    "rpc": {"seeds": 1000, "transitions": 100},
    "reactivity": {"seeds": 1000, "transitions": 100},
    "delivery": {"seeds": 1000, "transitions": 100},
    "workflow": {"seeds": 1000, "transitions": 100},
    "eventlog": {"seeds": 1000, "transitions": 100},
    "transaction": {"seeds": 1000, "transitions": 100},
    "native_contention": {"operations_per_feature": 10000},
    "schema_first_use": {"callers": 2, "rounds": 2},
    "bridges": {"seeds": 1000, "transitions": 100},
    "lifecycle_retention": {"cycles": 2},
}
FORMATS = {"schema", "rpc", "stored_outcome", "queue", "hydration", "event", "replica", "shared"}
FAMILIES = {"schema", "persistence", "rpc", "reactivity", "cluster", "workflow", "eventlog", "transaction"}
BRIDGES = {"INT01", "INT02", "INT03", "INT04", "INT05"}

# Each target runs once per profile. Persistence and RPC have multiple
# independent models; their reports are combined only after every part passes.
CAMPAIGN_TESTS = (
    ("schema", "feature_campaigns", "schema_generation_configured_seeds_and_boundary_corpus"),
    ("encoded_input", "encoded_input_campaign", "bounded_mutations_visit_eight_real_decode_boundaries"),
    ("persistence_record", "persistence_record_campaign", "configured_record_histories_match_expiry_and_batch_model"),
    ("persistence_queue", "persistence_queue_campaign", "configured_queue_histories_match_independent_state_model"),
    ("persistence_rate", "persistence_rate_campaign", "configured_rate_histories_match_independent_models"),
    ("rpc_unary", "rpc_cluster_dispatch", "configured_rpc_unary_histories_preserve_correlation_and_handler_gate"),
    ("rpc_stream", "rpc_cluster_dispatch", "configured_rpc_stream_histories_preserve_acknowledgement_and_scope"),
    ("reactivity", "reactivity_campaign", "configured_dynamic_graph_histories_match_independent_model"),
    ("delivery", "delivery_campaign", "configured_delivery_histories_match_acceptance_and_fencing_model"),
    ("workflow", "workflow_campaign", "configured_workflow_histories_match_replay_and_transition_model"),
    ("eventlog", "eventlog_campaign", "configured_journal_histories_match_independent_model"),
    ("transaction", "transaction_campaign", "configured_transaction_histories_match_independent_state_model"),
    ("lifecycle_retention", "lifecycle_retention_campaign", "configured_lifecycle_cycles_release_all_eight_feature_families"),
    ("native_contention", "native_contention_campaign", "configured_native_contention_preserves_eight_feature_invariants"),
    ("bridges", "bridges_campaign", "configured_bridge_histories_preserve_authority_after_faults"),
)
REQUIRED_REPORTS = {
    "schema", "encoded_input", "persistence_record", "persistence_queue",
    "persistence_rate", "rpc_unary", "rpc_stream", "reactivity", "delivery",
    "workflow", "eventlog", "transaction", "native_contention", "bridges",
    "lifecycle_retention",
}


def load_config(path: Path) -> dict:
    config = json.loads(path.read_text())
    if type(config) is not dict:
        raise ValueError("campaign config must be a JSON object")
    if type(config.get("seed")) is not int or config["seed"] < 0:
        raise ValueError("seed must be a nonnegative integer")
    if config.get("profiles") != ["debug", "release"]:
        raise ValueError("both debug and release profiles are required")
    if config.get("target") != "aarch64-apple-darwin":
        raise ValueError("this host campaign requires the declared arm64 macOS target")
    budgets = config.get("budgets")
    if type(budgets) is not dict or any(
        type(budgets.get(key)) is not int or budgets[key] <= 0
        for key in ("bytes", "depth", "width", "steps", "seconds")
    ):
        raise ValueError("positive byte/depth/width/step/time budgets are required")
    if budgets["bytes"] < 2:
        raise ValueError("byte budget must permit an exact-limit JSON string corpus")
    campaigns = config.get("campaigns")
    if type(campaigns) is not dict or set(campaigns) != set(MINIMUMS):
        raise ValueError("all thirteen named campaigns are required, without substitutions")
    for name, minima in MINIMUMS.items():
        item = campaigns[name]
        if type(item) is not dict or any(
            type(item.get(key)) is not int or item[key] < minimum
            for key, minimum in minima.items()
        ):
            raise ValueError(f"{name} does not meet its required workload")
    queue = campaigns["persistence"].get("queue")
    if type(queue) is not dict or any(
        type(queue.get(key)) is not int or queue[key] <= 0
        for key in ("maximum_attempts", "lease_millis", "retention_millis")
    ) or queue["maximum_attempts"] < 2:
        raise ValueError("persistence queue model needs explicit retry, lease and retention policies")
    delivery = campaigns["delivery"]
    if any(type(delivery.get(key)) is not int or delivery[key] <= 0
           for key in ("claim_lease_millis", "retention_millis", "maximum_attempts")):
        raise ValueError("delivery model needs explicit lease, retention and attempt policies")
    records = campaigns["persistence"].get("records")
    if type(records) is not dict or any(
        type(records.get(key)) is not int or records[key] <= 0
        for key in ("keys", "ttl_millis")
    ) or records["keys"] < 2:
        raise ValueError("persistence record model needs explicit multi-key and expiry policies")
    rate = campaigns["persistence"].get("rate_limit")
    rate_keys = ("capacity", "refill", "period_millis", "adaptive_minimum",
        "adaptive_initial", "adaptive_maximum", "adaptive_increase", "adaptive_decrease",
        "simultaneous_callers")
    if type(rate) is not dict or any(
        type(rate.get(key)) is not int or rate[key] <= 0 for key in rate_keys
    ) or not (rate["adaptive_minimum"] <= rate["adaptive_initial"]
              <= rate["adaptive_maximum"]) or rate["simultaneous_callers"] <= rate["capacity"]:
        raise ValueError("persistence rate model needs explicit valid policy parameters")
    transaction = campaigns["transaction"]
    if any(type(transaction.get(key)) is not int or transaction[key] <= 0
           for key in ("initial_balance", "max_transfer")):
        raise ValueError("transaction model needs explicit positive balance and transfer bounds")
    if (transaction["max_transfer"] > sys.maxsize
            or transaction["initial_balance"] * 2 + transaction["transitions"] > sys.maxsize):
        raise ValueError("transaction model values exceed the fixture's signed value range")
    eventlog = campaigns["eventlog"]
    if any(type(eventlog.get(key)) is not int or eventlog[key] <= 0
           for key in ("identities", "page_limit")):
        raise ValueError("event journal model needs explicit identity and page policies")
    retention = campaigns["lifecycle_retention"]
    if (type(retention.get("warmup_cycles")) is not int
            or not 0 < retention["warmup_cycles"] < retention["cycles"]):
        raise ValueError("retention measurement needs explicit warmup cycles")
    contention = campaigns["native_contention"]
    if type(contention.get("max_conflicts")) is not int or contention["max_conflicts"] <= 0:
        raise ValueError("native transaction contention needs an explicit conflict bound")
    bridges = campaigns["bridges"]
    if any(type(bridges.get(key)) is not int or bridges[key] <= 0
           for key in ("task_workers", "cpu_workers", "queued_jobs")):
        raise ValueError("bridge workers and admission must be explicitly bounded")
    histories = ("persistence", "rpc", "reactivity", "delivery", "workflow",
        "eventlog", "transaction", "bridges")
    if budgets["steps"] < max(
        campaigns[name]["seeds"] * campaigns[name]["transitions"]
        for name in histories
    ):
        raise ValueError("step budget cannot complete a required history")
    if set(campaigns["encoded_input"].get("formats", [])) != FORMATS:
        raise ValueError("encoded-input campaign must cover all eight format boundaries")
    levels = campaigns["native_contention"].get("levels")
    if levels != [1, 2, 8, 32]:
        raise ValueError("native contention must request levels 1, 2, 8 and 32")
    artifacts = config.get("artifacts")
    if not isinstance(artifacts, str) or not artifacts.strip():
        raise ValueError("artifact directory is required")
    artifact_path = (ROOT / artifacts).resolve()
    if not artifact_path.is_relative_to(ROOT / "scratch" / "reports"):
        raise ValueError("artifacts must stay under scratch/reports")
    return config


def record_run(
    command: list[str], config_path: Path, output: Path, timeout: int,
    expected_campaign: str | None = None,
) -> dict | None:
    report = output.with_name(output.stem.replace("-command", "-completed") + ".json")
    fingerprint = source_fingerprint(command, config_path)
    if output.exists() and (expected_campaign is None or report.exists()):
        previous = json.loads(output.read_text())
        if (previous.get("command") == command
                and previous.get("fingerprint") == fingerprint
                and previous.get("exit_code") == 0):
            if expected_campaign is None:
                return None
            evidence = json.loads(report.read_text())
            if expected_campaign.startswith("consumer_"):
                verify_consumer_report(evidence, expected_campaign)
            else:
                verify_campaign_report(evidence, expected_campaign, config_path)
            return evidence
    if report.exists():
        report.unlink()
    start = time.monotonic()
    try:
        completed = subprocess.run(
            command, cwd=ROOT, text=True, capture_output=True,
            timeout=timeout, env=campaign_environment(config_path, report),
        )
    except subprocess.TimeoutExpired as error:
        output.write_text(json.dumps({"command": command, "timed_out": True,
            "elapsed_seconds": time.monotonic() - start}, indent=2))
        raise RuntimeError(f"campaign timed out: {command}") from error
    output.write_text(json.dumps({
        "command": command, "exit_code": completed.returncode,
        "fingerprint": fingerprint,
        "elapsed_seconds": time.monotonic() - start,
        "stdout": completed.stdout, "stderr": completed.stderr,
    }, indent=2))
    if completed.returncode != 0:
        raise RuntimeError(f"campaign command failed: {command}; see {output}")
    if expected_campaign is not None:
        if not report.exists():
            raise RuntimeError(f"campaign did not write completed counts: {report}")
        evidence = json.loads(report.read_text())
        if expected_campaign.startswith("consumer_"):
            verify_consumer_report(evidence, expected_campaign)
        else:
            verify_campaign_report(evidence, expected_campaign, config_path)
        return evidence
    return None


def source_fingerprint(command: list[str], config_path: Path) -> str:
    """Invalidate one completed command only when its own inputs change."""
    inputs = [config_path, ROOT / "Cargo.toml", ROOT / "Cargo.lock",
        ROOT / "crates/effect/Cargo.toml", ROOT / "crates/effect_macros/Cargo.toml"]
    inputs.extend((ROOT / "crates/effect/src").rglob("*.rs"))
    inputs.extend((ROOT / "crates/effect_macros/src").rglob("*.rs"))
    if "--test" in command:
        target = command[command.index("--test") + 1]
        inputs.append(ROOT / "crates/effect/tests" / f"{target}.rs")
        inputs.extend((ROOT / "crates/effect/tests/support").rglob("*.rs"))
    if "--manifest-path" in command:
        manifest = ROOT / command[command.index("--manifest-path") + 1]
        inputs.extend(manifest.parent.rglob("*.rs"))
        inputs.extend(manifest.parent.rglob("*.toml"))
        inputs.extend(manifest.parent.rglob("Cargo.lock"))
        inputs.extend((ROOT / "crates/effect/tests/fixtures/shared").rglob("*.rs"))
    digest = hashlib.sha256()
    digest.update(subprocess.run(
        ["rustc", "--version", "--verbose"], check=True, capture_output=True,
    ).stdout)
    digest.update(subprocess.run(
        ["cargo", "--version"], check=True, capture_output=True,
    ).stdout)
    for path in sorted(set(inputs)):
        digest.update(str(path.relative_to(ROOT)).encode())
        digest.update(path.read_bytes())
    return digest.hexdigest()


def verify_campaign_report(report: dict, expected: str, config_path: Path) -> None:
    config = load_config(config_path)
    if type(report) is not dict or report.get("campaign") != expected:
        raise ValueError(f"wrong campaign report: expected {expected}")
    completed = report.get("completed")
    if type(completed) is not dict:
        raise ValueError(f"{expected} lacks completed counts")
    base = expected if expected in MINIMUMS else expected.split("_", 1)[0]
    required = config["campaigns"][base]
    for key in MINIMUMS[base]:
        actual = completed.get(key)
        if type(actual) is not int or actual < required[key]:
            raise ValueError(f"{expected} did not complete configured {key}")
    for key in ("operations", "coverage", "decisions", "feedbacks"):
        values = completed.get(key)
        if values is not None and (not isinstance(values, list)
                or not values or any(type(value) is not int or value <= 0 for value in values)):
            raise ValueError(f"{expected} lacks nonzero {key} coverage")
    if expected == "schema":
        for key in ("callers", "rounds"):
            if completed.get(key, 0) < config["campaigns"]["schema_first_use"][key]:
                raise ValueError(f"schema first-use {key} workload is incomplete")
    if expected == "encoded_input":
        formats = completed.get("formats")
        if type(formats) is not dict or set(formats) != FORMATS or any(
            type(count) is not int or count <= 0 for count in formats.values()
        ):
            raise ValueError("encoded-input format coverage is incomplete")
    if expected == "lifecycle_retention":
        if (completed.get("feature_families") != len(FAMILIES)
                or completed.get("warmup_cycles") != required["warmup_cycles"]
                or completed.get("maximum_retained_after_warmup") != 0
                or type(completed.get("released_payloads")) is not int
                or completed["released_payloads"] < required["cycles"]):
            raise ValueError("lifecycle retention coverage or disposal is incomplete")
    if expected == "native_contention":
        levels = completed.get("levels")
        if (completed.get("feature_families") != len(FAMILIES)
                or type(levels) is not list
                or any(type(level) is not dict for level in levels)
                or [level.get("workers") for level in levels] != required["levels"]):
            raise ValueError("native contention levels or feature families are incomplete")
        for level in levels:
            overlaps = level.get("overlapped_operations")
            if (level.get("per_feature_operations", 0) < required["operations_per_feature"]
                    or type(overlaps) is not list or len(overlaps) != len(FAMILIES)
                    or any(type(count) is not int or count < 0 for count in overlaps)
                    or (level["workers"] > 1 and any(count == 0 for count in overlaps))):
                raise ValueError("native contention lacks executed overlap or work")
    if expected == "bridges":
        bridges = completed.get("bridges")
        if (type(bridges) is not dict or set(bridges) != BRIDGES
                or any(type(count) is not int or count <= 0 for count in bridges.values())
                or sum(bridges.values()) != required["seeds"] * required["transitions"]):
            raise ValueError("bridge history coverage is incomplete")


def verify_consumer_report(report: dict, expected: str) -> None:
    if type(report) is not dict or report.get("campaign") != expected:
        raise ValueError(f"wrong consumer report: expected {expected}")
    families = report.get("families")
    if type(families) is not dict or set(families) != FAMILIES or any(
        type(count) is not int or count <= 0 for count in families.values()
    ):
        raise ValueError(f"{expected} lacks executed feature-family coverage")
    bridges = report.get("bridges")
    if type(bridges) is not dict or set(bridges) != BRIDGES or any(
        type(count) is not int or count <= 0 for count in bridges.values()
    ):
        raise ValueError(f"{expected} lacks executed bridge coverage")


def campaign_environment(config_path: Path, report: Path) -> dict[str, str]:
    import os
    environment = os.environ.copy()
    environment["EFFECT_CAMPAIGN_CONFIG"] = str(config_path)
    environment["EFFECT_CAMPAIGN_REPORT"] = str(report)
    return environment


def verify_report(report: dict, config_path: Path) -> None:
    config = load_config(config_path)
    if type(report) is not dict or report.get("seed") != config["seed"]:
        raise ValueError("campaign report has wrong seed")
    completed = report.get("completed")
    if type(completed) is not dict or set(completed) != set(MINIMUMS):
        raise ValueError("campaign report is missing completed campaign counts")
    for name, minima in MINIMUMS.items():
        actual = completed[name]
        if type(actual) is not dict or any(
            type(actual.get(key)) is not int
            or actual[key] < config["campaigns"][name][key]
            or actual[key] < minimum
            for key, minimum in minima.items()
        ):
            raise ValueError(f"{name} campaign did not complete its configured workload")
    formats = report.get("formats")
    if type(formats) is not dict or set(formats) != FORMATS or any(
        type(value) is not int or value <= 0 for value in formats.values()
    ):
        raise ValueError("report lacks nonzero coverage for every encoded format")
    families = report.get("families")
    if type(families) is not dict or set(families) != FAMILIES or any(
        type(value) is not int or value <= 0 for value in families.values()
    ):
        raise ValueError("report lacks one or more feature families")
    bridges = report.get("bridges")
    if type(bridges) is not dict or set(bridges) != BRIDGES or any(
        type(value) is not int or value <= 0 for value in bridges.values()
    ):
        raise ValueError("report lacks one or more required bridges")
    if report.get("failed_invariants"):
        raise ValueError("campaign reports failed invariants")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--campaign-config", type=Path, required=True)
    parser.add_argument("--validate-only", action="store_true")
    arguments = parser.parse_args()
    config_path = arguments.campaign_config.resolve()
    config = load_config(config_path)
    if arguments.validate_only:
        print("Campaign configuration meets the plan's minimum workloads")
        return 0
    missing = REQUIRED_REPORTS - {name for name, _, _ in CAMPAIGN_TESTS}
    if missing:
        raise RuntimeError(
            "required campaign implementations are missing: " + ", ".join(sorted(missing))
        )
    artifacts = (ROOT / config["artifacts"]).resolve()
    artifacts.mkdir(parents=True, exist_ok=True)
    timeout = config["budgets"]["seconds"]
    consumers = {
        "independent": ("crates/effect/tests/fixtures/capability_consumer/Cargo.toml",
            "effect-capability-consumer"),
        "mixed": ("crates/effect/tests/fixtures/mixed_provider_consumer/Cargo.toml",
            "effect-mixed-provider-consumer"),
    }
    for profile in config["profiles"]:
        reports = {}
        for campaign, target, test in CAMPAIGN_TESTS:
            command = ["cargo", "test", "-p", "effect", "--test", target, "--locked"]
            if profile == "release":
                command.append("--release")
            command.extend([test, "--", "--exact"])
            reports[campaign] = record_run(command, config_path,
                artifacts / f"{campaign}-{profile}-command.json", timeout,
                expected_campaign=campaign)
        for name, (manifest, binary_name) in consumers.items():
            if profile == "debug":
                command = ["cargo", "run", "--locked", "--manifest-path", manifest,
                    "--", "--campaign-config", str(config_path)]
            else:
                target_dir = (ROOT / "target" / "effect-campaign").resolve()
                build = ["cargo", "build", "--release", "--locked",
                    "--target", config["target"], "--target-dir", str(target_dir),
                    "--manifest-path", manifest]
                record_run(build, config_path,
                    artifacts / f"{name}-release-build-command.json", timeout)
                binary = target_dir / config["target"] / "release" / binary_name
                inspected = subprocess.run(["file", str(binary)], text=True,
                    capture_output=True, check=True)
                if "Mach-O" not in inspected.stdout or "arm64" not in inspected.stdout:
                    raise RuntimeError(f"wrong native artifact: {inspected.stdout}")
                (artifacts / f"{name}-release-artifact.txt").write_text(inspected.stdout)
                command = [str(binary), "--campaign-config", str(config_path)]
            reports[f"consumer_{name}"] = record_run(command, config_path,
                artifacts / f"{name}-{profile}-command.json", timeout,
                expected_campaign=f"consumer_{name}")
        counts = {name: reports[name]["completed"] for name in MINIMUMS
            if name in reports}
        schema = reports["schema"]["completed"]
        counts["schema_first_use"] = {
            "callers": schema["callers"], "rounds": schema["rounds"]}
        for family, members in {
            "persistence": ("persistence_record", "persistence_queue", "persistence_rate"),
            "rpc": ("rpc_unary", "rpc_stream"),
        }.items():
            counts[family] = {key: min(reports[member]["completed"][key]
                for member in members) for key in MINIMUMS[family]}
        summary = {
            "seed": config["seed"], "completed": counts,
            "formats": reports["encoded_input"]["completed"]["formats"],
            "families": reports["consumer_independent"]["families"],
            "bridges": reports["consumer_independent"]["bridges"],
            "failed_invariants": [],
        }
        verify_report(summary, config_path)
        (artifacts / f"{profile}-completed.json").write_text(json.dumps(summary, indent=2))
    print(f"All configured campaign profiles completed; evidence: {artifacts}")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (ValueError, RuntimeError, OSError, subprocess.SubprocessError) as error:
        print(f"campaign verification failed: {error}", file=sys.stderr)
        sys.exit(1)
