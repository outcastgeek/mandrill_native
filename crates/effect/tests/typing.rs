use std::path::PathBuf;
use std::process::Command;

struct FixtureDirectory(PathBuf);
impl Drop for FixtureDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn external_compiler_contracts() -> Result<(), Box<dyn std::error::Error>> {
    let directory = FixtureDirectory(
        std::env::temp_dir().join(format!("mandrill-effect-typing-{}", std::process::id())),
    );
    std::fs::create_dir_all(directory.0.join("src"))?;
    let manifest = format!(
        "[package]\nname = \"effect-typing-consumer\"\nversion = \"0.0.0\"\nedition = \"2024\"\n[workspace]\n[dependencies]\nmandrill_effect = {{ package = \"effect\", path = {:?} }}\n",
        env!("CARGO_MANIFEST_DIR")
    );
    std::fs::write(directory.0.join("Cargo.toml"), manifest)?;
    let cases = [
        ("services", include_str!("ui/services.rs"), None),
        ("macro_loops", include_str!("ui/macro_loops.rs"), None),
        ("no_clock", include_str!("ui/no_clock.rs"), Some("HasClock")),
        (
            "no_random",
            include_str!("ui/no_random.rs"),
            Some("HasRandom"),
        ),
        (
            "shared_value_borrow",
            include_str!("ui/shared_value_borrow.rs"),
            Some("lifetime may not live long enough"),
        ),
        ("renamed", include_str!("ui/renamed.rs"), None),
        ("defect_kind", include_str!("ui/defect_kind.rs"), None),
        ("const_defect", include_str!("ui/const_defect.rs"), None),
        ("fiber_shared", include_str!("ui/fiber_shared.rs"), None),
        (
            "no_task_spawner",
            include_str!("ui/no_task_spawner.rs"),
            Some("HasTaskSpawner"),
        ),
        (
            "shared_exit_borrow",
            include_str!("ui/shared_exit_borrow.rs"),
            Some("lifetime may not live long enough"),
        ),
        ("cancellation", include_str!("ui/cancellation.rs"), None),
        ("resource_macro", include_str!("ui/resource_macro.rs"), None),
        ("branches", include_str!("ui/branches.rs"), None),
        ("parallel", include_str!("ui/parallel.rs"), None),
        ("jobs", include_str!("ui/jobs.rs"), None),
        (
            "no_cpu_executor",
            include_str!("ui/no_cpu_executor.rs"),
            Some("HasCpuExecutor"),
        ),
        (
            "no_blocking_executor",
            include_str!("ui/no_blocking_executor.rs"),
            Some("HasBlockingExecutor"),
        ),
        (
            "job_borrow",
            include_str!("ui/job_borrow.rs"),
            Some("E0521"),
        ),
        (
            "run_future_ownership",
            include_str!("ui/run_future_ownership.rs"),
            None,
        ),
        ("queue", include_str!("ui/queue.rs"), None),
        (
            "queue_capacity",
            include_str!("ui/queue_capacity.rs"),
            Some("NonZero"),
        ),
        (
            "queue_negative_capacity",
            include_str!("ui/queue_negative_capacity.rs"),
            Some("E0600"),
        ),
        (
            "queue_clone",
            include_str!("ui/queue_clone.rs"),
            Some("Clone"),
        ),
        (
            "synchronization",
            include_str!("ui/synchronization.rs"),
            None,
        ),
        (
            "reference_borrow",
            include_str!("ui/reference_borrow.rs"),
            Some("lifetime may not live long enough"),
        ),
        (
            "reference_get_non_clone",
            include_str!("ui/reference_get_non_clone.rs"),
            Some("Clone"),
        ),
        (
            "no_parallel_provider",
            include_str!("ui/no_parallel_provider.rs"),
            Some("HasTaskSpawner"),
        ),
        (
            "zero_parallel_limit",
            include_str!("ui/zero_parallel_limit.rs"),
            Some("NonZero"),
        ),
        (
            "parallel_borrow",
            include_str!("ui/parallel_borrow.rs"),
            Some("E0521"),
        ),
        ("branch_return", include_str!("ui/branch_return.rs"), None),
        (
            "branch_mutation",
            include_str!("ui/branch_mutation.rs"),
            None,
        ),
        (
            "resource_borrow",
            include_str!("ui/resource_borrow.rs"),
            Some("lifetime may not live long enough"),
        ),
        (
            "no_cancellation",
            include_str!("ui/no_cancellation.rs"),
            Some("HasCancellation"),
        ),
        (
            "projection",
            include_str!("ui/projection.rs"),
            Some("E0308"),
        ),
        ("let_else", include_str!("ui/let_else.rs"), None),
        ("errors", include_str!("ui/errors.rs"), Some("E0308")),
        (
            "environment",
            include_str!("ui/environment.rs"),
            Some("E0308"),
        ),
        ("success", include_str!("ui/success.rs"), Some("E0308")),
        ("private", include_str!("ui/private.rs"), Some("E0603")),
        (
            "constructor",
            include_str!("ui/constructor.rs"),
            Some("E0624"),
        ),
        ("nested", include_str!("ui/nested.rs"), None),
        ("result", include_str!("ui/result.rs"), Some("Bindable")),
        ("refutable", include_str!("ui/refutable.rs"), Some("E0005")),
    ];
    for (name, source, expected) in cases {
        std::fs::write(directory.0.join("src/main.rs"), source)?;
        let output = Command::new(env!("CARGO"))
            .args(["check", "--quiet", "--offline", "--manifest-path"])
            .arg(directory.0.join("Cargo.toml"))
            .env("CARGO_TARGET_DIR", directory.0.join("target"))
            .output()?;
        let diagnostics = String::from_utf8(output.stderr)?;
        match expected {
            Some(reason) => assert!(
                !output.status.success() && diagnostics.contains(reason),
                "{name}: expected {reason}; got {diagnostics}"
            ),
            None => assert!(output.status.success(), "{name}: {diagnostics}"),
        }
    }
    Ok(())
}
