use std::path::PathBuf;

use sqlx_core::{
    error::Error,
    pool::PoolOptions,
    testing::{FixtureSnapshot, TestArgs, TestContext, TestSupport},
};

use crate::{Turso, TursoConnectOptions, TursoConnection, lifecycle::known_database_files};

const TEST_DATABASE_ROOT: &str = "/tmp/sqlx-turso/test-dbs";

impl TestSupport for Turso {
    async fn test_context(args: &TestArgs) -> Result<TestContext<Self>, Error> {
        let path = test_database_path(args.test_path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        remove_database_files(&path).await?;

        Ok(TestContext {
            connect_opts: TursoConnectOptions::new()
                .filename(&path)
                .create_if_missing(true),
            pool_opts: PoolOptions::new().max_connections(16),
            db_name: path.to_string_lossy().into_owned(),
        })
    }

    async fn cleanup_test(db_name: &str) -> Result<(), Error> {
        remove_database_files(&PathBuf::from(db_name)).await
    }

    async fn cleanup_test_dbs() -> Result<Option<usize>, Error> {
        match std::fs::remove_dir_all(test_database_root()) {
            Ok(()) => Ok(None),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(Error::Io(error)),
        }
    }

    async fn snapshot(_conn: &mut TursoConnection) -> Result<FixtureSnapshot<Self>, Error> {
        Err(Error::Protocol(
            "SQLx fixture snapshot construction is not exposed by sqlx-core 0.9.0".to_owned(),
        ))
    }

    fn db_name(args: &TestArgs) -> String {
        test_database_path(args.test_path)
            .to_string_lossy()
            .into_owned()
    }
}

fn test_database_path(test_path: &str) -> PathBuf {
    let mut path = test_database_root();
    for segment in test_path.split("::") {
        path.push(segment);
    }
    path.set_extension("db");
    path
}

fn test_database_root() -> PathBuf {
    PathBuf::from(TEST_DATABASE_ROOT).join(std::process::id().to_string())
}

async fn remove_database_files(path: &std::path::Path) -> Result<(), Error> {
    for file in known_database_files(path) {
        match std::fs::remove_file(file) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(Error::Io(error)),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use sqlx_core::testing::TestSupport;

    use super::{TEST_DATABASE_ROOT, Turso, test_database_path, test_database_root};

    #[test]
    fn test_database_paths_are_scoped_to_the_current_process() {
        let root = test_database_root();
        assert_eq!(
            root,
            std::path::Path::new(TEST_DATABASE_ROOT).join(std::process::id().to_string())
        );
        assert!(test_database_path("module::test").starts_with(root));
    }

    #[tokio::test]
    async fn bulk_cleanup_preserves_other_process_directories() -> sqlx_core::Result<()> {
        let current_root = test_database_root();
        let sibling_root = std::path::Path::new(TEST_DATABASE_ROOT)
            .join(format!("other-process-{}", std::process::id()));
        tokio::fs::create_dir_all(&current_root).await?;
        tokio::fs::create_dir_all(&sibling_root).await?;
        tokio::fs::write(current_root.join("owned.db"), b"owned").await?;
        tokio::fs::write(sibling_root.join("unrelated.db"), b"unrelated").await?;

        <Turso as TestSupport>::cleanup_test_dbs().await?;

        assert!(!current_root.exists());
        assert!(sibling_root.join("unrelated.db").exists());
        tokio::fs::remove_dir_all(sibling_root).await?;
        Ok(())
    }
}
