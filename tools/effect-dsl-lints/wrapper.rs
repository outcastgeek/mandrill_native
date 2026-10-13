//! Cargo-native wrapper retaining both compiler DSL enforcement and Clippy.
use std::env;
use std::ffi::{OsStr, OsString};
use std::fs::{self, File};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
mod coverage;

/// Compiler information requests stop before analysis; linking requests do not.
fn compiler_query(arguments: &[OsString]) -> bool {
    let mut query = false;
    let mut arguments = arguments.iter();
    while let Some(argument) = arguments.next() {
        let Some(argument) = argument.to_str() else { continue };
        if matches!(argument, "-vV" | "-V" | "--version") {
            query = true;
        }
        let request = if argument == "--print" {
            arguments.next().and_then(|value| value.to_str())
        } else {
            argument.strip_prefix("--print=")
        };
        if let Some(request) = request {
            let kind = request.split('=').next().unwrap_or(request);
            if matches!(kind, "native-static-libs" | "link-args" | "crate-root-lint-levels") {
                return false;
            }
            query |= matches!(kind,
                "all-target-specs-json" | "backend-has-mnemonic" | "backend-has-zstd"
                | "calling-conventions" | "cfg" | "check-cfg" | "code-models"
                | "crate-name" | "deployment-target" | "file-names" | "host-tuple"
                | "relocation-models" | "split-debuginfo" | "stack-protector-strategies"
                | "supported-crate-types" | "sysroot" | "target-cpus" | "target-features"
                | "target-libdir" | "target-list" | "target-spec-json"
                | "target-spec-json-schema" | "tls-models");
        }
    }
    query
}

/// Record lint policy inputs in rustc's dependency file so Cargo cannot reuse
/// a successful analysis after the compiler rules or kernel boundary change.
fn policy_dependencies(root: &Path, arguments: &[OsString]) -> io::Result<()> {
    let mut output_dir = None;
    let mut crate_name = None;
    let mut suffix = String::new();
    let mut arguments = arguments.iter();
    while let Some(argument) = arguments.next() {
        match argument.to_str() {
            Some("--out-dir") => output_dir = arguments.next().map(PathBuf::from),
            Some("--crate-name") => {
                crate_name = arguments
                    .next()
                    .map(|name| name.to_string_lossy().into_owned())
            }
            Some("-C") => {
                if let Some(option) = arguments.next().and_then(|option| option.to_str())
                    && let Some(value) = option.strip_prefix("extra-filename=")
                {
                    suffix = value.to_owned();
                }
            }
            _ => {}
        }
    }
    let (Some(directory), Some(name)) = (output_dir, crate_name) else {
        return Ok(());
    };
    let path = directory.join(format!("{name}{suffix}.d"));
    if !path.exists() {
        return Ok(());
    }
    let mut dependencies = String::new();
    for source in fs::read_dir(root.join("tools/effect-dsl-lints"))? {
        let source = source?.path();
        if source
            .extension()
            .is_some_and(|extension| extension == "rs")
        {
            dependencies.push(' ');
            dependencies.push_str(
                &source
                    .to_string_lossy()
                    .replace('\\', "\\\\")
                    .replace(' ', "\\ "),
            );
        }
    }
    let contents = fs::read_to_string(&path)?;
    if let Some((first, rest)) = contents.split_once('\n') {
        fs::write(path, format!("{first}{dependencies}\n{rest}"))?;
    }
    Ok(())
}

/// Ask the actual Cargo compiler for its version or sysroot.
fn output(compiler: &OsStr, arguments: &[&str]) -> io::Result<String> {
    let result = Command::new(compiler).args(arguments).output()?;
    if !result.status.success() {
        return Err(io::Error::other(
            String::from_utf8_lossy(&result.stderr).into_owned(),
        ));
    }
    String::from_utf8(result.stdout).map_err(io::Error::other)
}

/// Compile only the custom driver with rustc-dev's private compiler API.
fn driver(root: &Path, compiler: &OsStr, version: &str) -> io::Result<PathBuf> {
    let tool = root.join("tools/effect-dsl-lints");
    let mut hash = DefaultHasher::new();
    version.hash(&mut hash);
    for name in ["main.rs", "registry.rs", "kernel.rs"] {
        fs::read(tool.join(name))?.hash(&mut hash);
    }
    let cache = root
        .join("target/effect-dsl-lints")
        .join(format!("{:016x}", hash.finish()));
    fs::create_dir_all(&cache)?;
    let lock = File::options()
        .create(true)
        .write(true)
        .truncate(false)
        .open(cache.join("build.lock"))?;
    lock.lock()?;
    let executable = cache.join(format!("driver{}", env::consts::EXE_SUFFIX));
    if !executable.exists() {
        let status = Command::new(compiler)
            .arg("--edition=2024")
            .arg(tool.join("main.rs"))
            .arg("-o")
            .arg(&executable)
            .env("RUSTC_BOOTSTRAP", "1")
            .status()?;
        if !status.success() {
            return Err(io::Error::other(
                "custom lint driver failed to compile; install rustc-dev for Rust 1.99.0",
            ));
        }
    }
    Ok(executable)
}

/// Run the mandatory analysis and then the exact compiler/wrapper supplied by Cargo.
fn run() -> io::Result<i32> {
    let tool = Path::new(env!("CARGO_MANIFEST_DIR"));
    let root = tool
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| io::Error::other("lint tool has no workspace root"))?;
    let original: Vec<OsString> = env::args_os().skip(1).collect();
    if original
        .first()
        .is_some_and(|argument| argument == "--review-coverage")
    {
        coverage::print_snapshot(root)?;
        return Ok(0);
    }
    let original_compiler = original
        .first()
        .ok_or_else(|| io::Error::other("Cargo did not supply its compiler"))?;
    let manifest = env::var_os("CARGO_MANIFEST_DIR").map(PathBuf::from);
    let workspace = manifest.as_ref().is_some_and(|path| path.starts_with(root));
    let mut arguments = &original[1..];
    let mut compiler = original_compiler;
    if Path::new(compiler)
        .file_stem()
        .is_some_and(|name| name == "clippy-driver")
        && arguments
            .first()
            .is_some_and(|argument| !argument.to_string_lossy().starts_with('-'))
    {
        compiler = &arguments[0];
        arguments = &arguments[1..];
    }
    let query = compiler_query(arguments);
    if workspace && !query {
        coverage::verify(root)?;
        let version = output(compiler, &["-vV"])?;
        if !version.lines().any(|line| line == "release: 1.99.0") {
            return Err(io::Error::other(
                "DSL lint driver requires the workspace Rust 1.99.0 compiler",
            ));
        }
        let sysroot = PathBuf::from(output(compiler, &["--print", "sysroot"])?.trim());
        let library_env = if cfg!(target_os = "macos") {
            "DYLD_LIBRARY_PATH"
        } else {
            "LD_LIBRARY_PATH"
        };
        let mut libraries = vec![sysroot.join("lib")];
        if let Some(existing) = env::var_os(library_env) {
            libraries.extend(env::split_paths(&existing));
        }
        let library_path = env::join_paths(libraries).map_err(io::Error::other)?;
        // Rust's process API preserves Cargo's inherited jobserver descriptors.
        let status = Command::new(driver(root, compiler, &version)?)
            .args(arguments)
            .env(library_env, library_path)
            .status()?;
        if !status.success() {
            return Ok(status.code().unwrap_or(1));
        }
    }
    let status = Command::new(original_compiler)
        .args(&original[1..])
        .status()?;
    if workspace && !query && status.success() {
        policy_dependencies(root, arguments)?;
    }
    Ok(status.code().unwrap_or(1))
}

fn main() -> ExitCode {
    match run() {
        Ok(0) => ExitCode::SUCCESS,
        Ok(_) => ExitCode::FAILURE,
        Err(error) => {
            eprintln!("effect-dsl-lints: {error}");
            ExitCode::FAILURE
        }
    }
}
