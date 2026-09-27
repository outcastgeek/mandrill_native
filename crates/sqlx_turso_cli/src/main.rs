use std::{
    collections::BTreeMap,
    env,
    ffi::OsString,
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    process::{Command, ExitCode},
};

use clap::{Args, CommandFactory, Parser, Subcommand};
use serde::Deserialize;

fn main() -> ExitCode {
    match CommandLine::try_parse() {
        Ok(command) => match run(command) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                let _ = writeln!(io::stderr(), "error: {error}");
                ExitCode::FAILURE
            }
        },
        Err(error) => {
            let exit_code = if error.use_stderr() {
                ExitCode::FAILURE
            } else {
                ExitCode::SUCCESS
            };

            let _ = error.print();
            exit_code
        }
    }
}

fn run(command: CommandLine) -> Result<(), String> {
    match command.command {
        Some(CommandLineCommand::Prepare(prepare)) => prepare.run(),
        None => {
            let mut command = CommandLine::command();
            command
                .print_help()
                .map_err(|error| format!("failed to print help: {error}"))?;
            io::stdout()
                .write_all(b"\n")
                .map_err(|error| format!("failed to print trailing newline: {error}"))?;
            Ok(())
        }
    }
}

#[derive(Debug, Eq, Parser, PartialEq)]
#[command(version, about)]
struct CommandLine {
    #[command(subcommand)]
    command: Option<CommandLineCommand>,
}

#[derive(Debug, Eq, PartialEq, Subcommand)]
enum CommandLineCommand {
    Prepare(Prepare),
}

#[derive(Args, Debug, Eq, PartialEq)]
struct Prepare {
    #[arg(short = 'D', long)]
    database_url: Option<String>,

    #[arg(long, default_value = ".sqlx")]
    offline_dir: PathBuf,

    #[arg(long)]
    check: bool,

    #[arg(last = true, value_name = "CARGO CHECK ARGS")]
    cargo_args: Vec<String>,
}

impl Prepare {
    fn run(self) -> Result<(), String> {
        let cargo = env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
        self.run_with_program(cargo)
    }

    fn run_with_program(self, cargo: impl Into<String>) -> Result<(), String> {
        let cargo = cargo.into();
        let workspace = PrepareWorkspace::create(&self.offline_dir)?;
        self.force_macro_recompile(&cargo)?;
        let mut command = self
            .command_spec(cargo, workspace.staging_dir())
            .into_command();

        let status = command
            .status()
            .map_err(|error| format!("failed to run cargo check: {error}"))?;

        if !status.success() {
            workspace.cleanup();
            return Err(format!("cargo check exited with {status}"));
        }

        if self.check {
            let matches = workspace.matches_existing().map_err(|error| {
                format!(
                    "failed to compare prepared metadata in {}: {error}",
                    self.offline_dir.display()
                )
            })?;
            workspace.cleanup();

            if matches {
                Ok(())
            } else {
                Err(format!(
                    "prepared metadata in {} is out of date; run prepare without --check",
                    self.offline_dir.display()
                ))
            }
        } else {
            workspace.commit().map_err(|error| {
                format!(
                    "failed to replace {} with prepared metadata: {error}",
                    self.offline_dir.display()
                )
            })
        }
    }

    fn command_spec(&self, cargo: impl Into<String>, offline_dir: &Path) -> CommandSpec {
        let mut args = vec!["check".to_owned()];
        if self.cargo_args.is_empty() {
            args.push("--all-targets".to_owned());
        } else {
            args.extend(self.cargo_args.iter().cloned());
        }

        let mut envs = vec![
            (
                "SQLX_OFFLINE_DIR".to_owned(),
                offline_dir.as_os_str().to_owned(),
            ),
            ("SQLX_OFFLINE".to_owned(), OsString::from("false")),
        ];
        if let Some(database_url) = &self.database_url {
            envs.push(("DATABASE_URL".to_owned(), OsString::from(database_url)));
        }

        CommandSpec {
            program: cargo.into(),
            args,
            envs,
        }
    }

    fn force_macro_recompile(&self, cargo: &str) -> Result<(), String> {
        let mut packages = packages_to_clean(&self.cargo_args);
        if packages.is_empty() {
            packages = workspace_packages(cargo)?;
        }
        let mut command = Command::new(cargo);
        command.arg("clean");
        for package in packages {
            command.args(["-p", &package]);
        }

        let status = command
            .status()
            .map_err(|error| format!("failed to prepare macro recompilation: {error}"))?;
        if status.success() {
            Ok(())
        } else {
            Err(format!("cargo clean exited with {status}"))
        }
    }
}

#[derive(Deserialize)]
struct CargoMetadata {
    packages: Vec<CargoPackage>,
    workspace_members: Vec<String>,
}

#[derive(Deserialize)]
struct CargoPackage {
    id: String,
    name: String,
}

fn workspace_packages(cargo: &str) -> Result<Vec<String>, String> {
    let output = Command::new(cargo)
        .args(["metadata", "--format-version=1", "--no-deps"])
        .output()
        .map_err(|error| format!("failed to inspect Cargo workspace packages: {error}"))?;
    if !output.status.success() {
        return Err(format!("cargo metadata exited with {}", output.status));
    }

    let metadata = std::str::from_utf8(&output.stdout)
        .map_err(|error| format!("cargo metadata returned invalid UTF-8: {error}"))?;
    workspace_packages_from_metadata(metadata)
}

fn workspace_packages_from_metadata(metadata: &str) -> Result<Vec<String>, String> {
    let metadata: CargoMetadata = serde_json::from_str(metadata)
        .map_err(|error| format!("failed to parse cargo metadata: {error}"))?;
    let mut packages = metadata
        .packages
        .into_iter()
        .filter(|package| metadata.workspace_members.contains(&package.id))
        .map(|package| package.name)
        .collect::<Vec<_>>();
    packages.sort_unstable();
    packages.dedup();
    if packages.is_empty() {
        return Err("cargo metadata returned no workspace packages".to_owned());
    }
    Ok(packages)
}

fn packages_to_clean(cargo_args: &[String]) -> Vec<String> {
    let mut packages = Vec::new();
    let mut arguments = cargo_args.iter();

    while let Some(argument) = arguments.next() {
        if argument == "-p" || argument == "--package" {
            if let Some(package) = arguments.next() {
                packages.push(package.clone());
            }
        } else if let Some(package) = argument.strip_prefix("--package=") {
            packages.push(package.to_owned());
        }
    }

    packages
}

#[derive(Debug, Eq, PartialEq)]
struct CommandSpec {
    program: String,
    args: Vec<String>,
    envs: Vec<(String, OsString)>,
}

impl CommandSpec {
    fn into_command(self) -> Command {
        let mut command = Command::new(self.program);
        command.args(self.args);
        for (key, value) in self.envs {
            command.env(key, value);
        }

        command
    }
}

#[derive(Debug)]
struct PrepareWorkspace {
    final_dir: PathBuf,
    staging_dir: PathBuf,
    backup_dir: PathBuf,
}

impl PrepareWorkspace {
    fn create(final_dir: &Path) -> Result<Self, String> {
        validate_offline_dir(final_dir)?;

        if final_dir.exists() && !final_dir.is_dir() {
            return Err(format!("{} is not a directory", final_dir.display()));
        }

        let parent = final_dir.parent().unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "failed to create parent directory {}: {error}",
                parent.display()
            )
        })?;

        let staging_dir = unique_sibling(final_dir, "tmp");
        fs::create_dir(&staging_dir).map_err(|error| {
            format!(
                "failed to create staging directory {}: {error}",
                staging_dir.display()
            )
        })?;

        Ok(Self {
            final_dir: final_dir.to_path_buf(),
            staging_dir,
            backup_dir: unique_sibling(final_dir, "old"),
        })
    }

    fn staging_dir(&self) -> &Path {
        &self.staging_dir
    }

    fn matches_existing(&self) -> io::Result<bool> {
        if !self.final_dir.is_dir() {
            return Ok(false);
        }

        Ok(read_directory_files(&self.final_dir)? == read_directory_files(&self.staging_dir)?)
    }

    fn commit(self) -> io::Result<()> {
        if self.backup_dir.exists() {
            fs::remove_dir_all(&self.backup_dir)?;
        }

        let had_existing = self.final_dir.exists();
        if had_existing {
            fs::rename(&self.final_dir, &self.backup_dir)?;
        }

        match fs::rename(&self.staging_dir, &self.final_dir) {
            Ok(()) => {
                if had_existing {
                    fs::remove_dir_all(&self.backup_dir)?;
                }
                Ok(())
            }
            Err(error) => {
                if had_existing {
                    let _ = fs::rename(&self.backup_dir, &self.final_dir);
                }

                Err(error)
            }
        }
    }

    fn cleanup(&self) {
        let _ = fs::remove_dir_all(&self.staging_dir);
    }
}

fn read_directory_files(directory: &Path) -> io::Result<BTreeMap<OsString, Vec<u8>>> {
    let mut files = BTreeMap::new();

    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if !file_type.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("{} contains a non-file entry", entry.path().display()),
            ));
        }
        files.insert(entry.file_name(), fs::read(entry.path())?);
    }

    Ok(files)
}

fn validate_offline_dir(path: &Path) -> Result<(), String> {
    if path.as_os_str().is_empty() {
        return Err("offline directory cannot be empty".to_owned());
    }

    if path == Path::new(".") {
        return Err("offline directory cannot be the current directory".to_owned());
    }

    if path.parent().is_none() && path.has_root() {
        return Err("offline directory cannot be the filesystem root".to_owned());
    }

    Ok(())
}

fn unique_sibling(path: &Path, suffix: &str) -> PathBuf {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let name = path
        .file_name()
        .unwrap_or_else(|| std::ffi::OsStr::new("sqlx"))
        .to_string_lossy();

    let pid = std::process::id();

    let mut attempt = 0_u64;
    loop {
        let candidate = parent.join(format!(".{name}.{suffix}-{pid}-{attempt}"));
        if !candidate.exists() {
            return candidate;
        }
        attempt = attempt.saturating_add(1);
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, process};

    use super::{
        CommandSpec, Prepare, PrepareWorkspace, packages_to_clean, workspace_packages_from_metadata,
    };

    #[test]
    fn command_spec_uses_default_check_args_and_staging_dir() {
        let prepare = Prepare {
            database_url: Some("turso::memory:".to_owned()),
            offline_dir: ".sqlx".into(),
            check: false,
            cargo_args: Vec::new(),
        };

        let spec = prepare.command_spec("cargo-test", "target/offline-staging".as_ref());

        assert_eq!(
            spec,
            CommandSpec {
                program: "cargo-test".to_owned(),
                args: vec!["check".to_owned(), "--all-targets".to_owned()],
                envs: vec![
                    (
                        "SQLX_OFFLINE_DIR".to_owned(),
                        "target/offline-staging".into()
                    ),
                    ("SQLX_OFFLINE".to_owned(), "false".into()),
                    ("DATABASE_URL".to_owned(), "turso::memory:".into()),
                ],
            }
        );
    }

    #[test]
    fn package_selection_matches_cargo_package_arguments() {
        let arguments = [
            "-p".to_owned(),
            "first".to_owned(),
            "--package=second".to_owned(),
            "--package".to_owned(),
            "third".to_owned(),
            "--all-targets".to_owned(),
        ];

        assert_eq!(packages_to_clean(&arguments), ["first", "second", "third"]);
    }

    #[test]
    fn workspace_package_selection_uses_only_workspace_members() -> Result<(), String> {
        let metadata = r#"{
            "packages": [
                {"id": "path+file:///workspace#consumer@0.1.0", "name": "consumer"},
                {"id": "registry+https://example.invalid#index@1.0.0", "name": "dependency"}
            ],
            "workspace_members": ["path+file:///workspace#consumer@0.1.0"]
        }"#;

        assert_eq!(workspace_packages_from_metadata(metadata)?, ["consumer"]);
        Ok(())
    }

    #[test]
    fn workspace_keeps_existing_dir_until_commit() -> Result<(), String> {
        let root = std::env::temp_dir().join(format!(
            "sqlx-turso-cli-{}-{}",
            process::id(),
            "workspace-keeps-existing"
        ));
        let final_dir = root.join(".sqlx");
        let old_file = final_dir.join("old.json");
        let new_file = "new.json";

        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&final_dir).map_err(|error| error.to_string())?;
        fs::write(&old_file, b"old").map_err(|error| error.to_string())?;

        let workspace = PrepareWorkspace::create(&final_dir)?;
        fs::write(workspace.staging_dir().join(new_file), b"new")
            .map_err(|error| error.to_string())?;

        assert!(old_file.exists());
        workspace.commit().map_err(|error| error.to_string())?;

        assert!(!old_file.exists());
        assert!(final_dir.join(new_file).exists());

        let _ = fs::remove_dir_all(&root);
        Ok(())
    }

    #[test]
    fn workspace_cleanup_preserves_existing_dir() -> Result<(), String> {
        let root = std::env::temp_dir().join(format!(
            "sqlx-turso-cli-{}-{}",
            process::id(),
            "workspace-cleanup"
        ));
        let final_dir = root.join(".sqlx");
        let old_file = final_dir.join("old.json");

        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&final_dir).map_err(|error| error.to_string())?;
        fs::write(&old_file, b"old").map_err(|error| error.to_string())?;

        let workspace = PrepareWorkspace::create(&final_dir)?;
        let staging_dir = workspace.staging_dir().to_path_buf();
        workspace.cleanup();

        assert!(old_file.exists());
        assert!(!staging_dir.exists());

        let _ = fs::remove_dir_all(&root);
        Ok(())
    }

    #[test]
    fn workspace_check_compares_exact_metadata_contents() -> Result<(), String> {
        let root = std::env::temp_dir().join(format!(
            "sqlx-turso-cli-{}-{}",
            process::id(),
            "workspace-check"
        ));
        let final_dir = root.join(".sqlx");

        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&final_dir).map_err(|error| error.to_string())?;
        fs::write(final_dir.join("query.json"), b"current").map_err(|error| error.to_string())?;

        let workspace = PrepareWorkspace::create(&final_dir)?;
        fs::write(workspace.staging_dir().join("query.json"), b"current")
            .map_err(|error| error.to_string())?;
        assert!(
            workspace
                .matches_existing()
                .map_err(|error| error.to_string())?
        );

        fs::write(workspace.staging_dir().join("query.json"), b"stale")
            .map_err(|error| error.to_string())?;
        assert!(
            !workspace
                .matches_existing()
                .map_err(|error| error.to_string())?
        );
        workspace.cleanup();

        let _ = fs::remove_dir_all(&root);
        Ok(())
    }
}
