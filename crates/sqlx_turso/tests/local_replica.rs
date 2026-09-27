#![cfg(feature = "sync")]

use std::{
    fs, io,
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, SystemTime},
};

use sqlx_turso::{
    TursoConnectOptions, TursoConnection, TursoSyncOptions,
    sqlx::{ConnectOptions, Executor, Row},
    turso,
};

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> io::Result<Self> {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_err(io::Error::other)?
            .as_nanos();
        let path = PathBuf::from(format!(
            "/tmp/sqlx-turso/replicas/{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(&path)?;
        Ok(Self(path))
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct TursoDevServer {
    child: Child,
    port: u16,
}

impl TursoDevServer {
    fn start(database: &Path, port: u16) -> io::Result<Self> {
        let mut command = database_cli_command();
        let working_directory = database.parent().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "Turso test database must have a parent directory",
            )
        })?;
        let child = command
            .current_dir(working_directory)
            .arg(database)
            .args(["--sync-server", &format!("127.0.0.1:{port}")])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        let mut server = Self { child, port };
        server.wait_until_ready()?;
        Ok(server)
    }

    fn url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    fn wait_until_ready(&mut self) -> io::Result<()> {
        let address = ("127.0.0.1", self.port);
        for _ in 0..200 {
            if TcpStream::connect(address).is_ok() {
                return Ok(());
            }
            if let Some(status) = self.child.try_wait()? {
                return Err(io::Error::other(format!(
                    "turso dev exited before readiness with {status}"
                )));
            }
            thread::sleep(Duration::from_millis(25));
        }
        Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "turso dev did not accept connections within five seconds",
        ))
    }

    fn stop(&mut self) -> io::Result<()> {
        match self.child.kill() {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::InvalidInput => {}
            Err(error) => return Err(error),
        }
        let _ = self.child.wait()?;
        Ok(())
    }
}

fn database_cli_command() -> Command {
    if let Some(program) = std::env::var_os("TURSO_DATABASE_CLI") {
        return Command::new(program);
    }

    if Command::new("tursodb")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
    {
        return Command::new("tursodb");
    }

    let mut command = Command::new("npx");
    command.args(["--yes", "turso@0.6.0"]);
    command
}

impl Drop for TursoDevServer {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

fn available_port() -> io::Result<u16> {
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    Ok(listener.local_addr()?.port())
}

#[tokio::test]
async fn sqlx_and_native_replicas_round_trip_through_turso_cli() -> sqlx::Result<()> {
    tokio::time::timeout(Duration::from_secs(120), run_replica_round_trip())
        .await
        .map_err(|_| sqlx::Error::Protocol("local Turso replica test timed out".to_owned()))?
}

async fn run_replica_round_trip() -> sqlx::Result<()> {
    if rustls::crypto::CryptoProvider::get_default().is_none() {
        let _ = rustls::crypto::ring::default_provider().install_default();
    }

    let directory = TestDirectory::new()?;
    let remote_path = directory.0.join("remote.db");
    let first_path = directory.0.join("first-replica.db");
    let second_path = directory.0.join("second-replica.db");
    let port = available_port()?;
    let mut server = TursoDevServer::start(&remote_path, port)?;
    let remote_url = server.url();
    let auth_calls = Arc::new(AtomicUsize::new(0));
    let auth_token = "local-replica-secret-token";
    let auth_token_fn: turso::sync::AuthTokenFn = Arc::new({
        let auth_calls = auth_calls.clone();
        move || {
            auth_calls.fetch_add(1, Ordering::SeqCst);
            Box::pin(async move { Ok(auth_token.to_owned()) })
        }
    });

    let second_database = turso::sync::Builder::new_remote(&second_path.to_string_lossy())
        .with_remote_url(&remote_url)
        .build()
        .await
        .map_err(sqlx::Error::config)?;

    let mut first = TursoConnectOptions::new()
        .filename(&first_path)
        .create_if_missing(true)
        .with_sync_options(
            TursoSyncOptions::new(&remote_url)
                .with_auth_token_fn(auth_token_fn)
                .with_client_name("sqlx-turso-integration")
                .with_long_poll_timeout(Duration::from_millis(250)),
        )
        .connect()
        .await?;
    first
        .execute("CREATE TABLE messages (id INTEGER PRIMARY KEY, value TEXT NOT NULL)")
        .await?;
    first
        .execute("INSERT INTO messages (id, value) VALUES (1, 'from-sqlx')")
        .await?;
    first.sync_push().await?;
    let first_push_stats = first.sync_stats().await?;
    assert!(first_push_stats.last_push_unix_time.is_some());
    assert!(first_push_stats.network_sent_bytes > 0);
    assert!(auth_calls.load(Ordering::SeqCst) > 0);

    second_database.pull().await.map_err(sqlx::Error::config)?;
    let second_native = second_database
        .connect()
        .await
        .map_err(sqlx::Error::config)?;
    let mut statement = second_native
        .prepare("SELECT value FROM messages WHERE id = 1")
        .await
        .map_err(sqlx::Error::config)?;
    let row = statement.query_row(()).await.map_err(sqlx::Error::config)?;
    assert_eq!(
        row.get::<String>(0).map_err(sqlx::Error::config)?,
        "from-sqlx"
    );

    second_native
        .execute(
            "INSERT INTO messages (id, value) VALUES (2, 'from-native')",
            (),
        )
        .await
        .map_err(sqlx::Error::config)?;
    second_database.push().await.map_err(sqlx::Error::config)?;
    assert!(first.sync_pull().await?);
    let first_pull_stats = first.sync_stats().await?;
    assert!(first_pull_stats.last_pull_unix_time.is_some());
    assert!(first_pull_stats.network_received_bytes > 0);

    let row = first
        .fetch_one("SELECT value FROM messages WHERE id = 2")
        .await?;
    assert_eq!(row.try_get::<String, _>("value")?, "from-native");

    server.stop()?;
    first
        .execute("INSERT INTO messages (id, value) VALUES (3, 'offline-sqlx')")
        .await?;
    second_native
        .execute(
            "INSERT INTO messages (id, value) VALUES (4, 'offline-native')",
            (),
        )
        .await
        .map_err(sqlx::Error::config)?;

    server = TursoDevServer::start(&remote_path, port)?;
    first.sync_push().await?;
    second_database.pull().await.map_err(sqlx::Error::config)?;
    second_database.push().await.map_err(sqlx::Error::config)?;
    first.sync_pull().await?;
    first.sync_checkpoint().await?;
    let row = first
        .fetch_one("SELECT COUNT(*) AS count FROM messages")
        .await?;
    assert_eq!(row.try_get::<i64, _>("count")?, 4);

    let second_row = second_native
        .prepare("SELECT COUNT(*) FROM messages")
        .await
        .map_err(sqlx::Error::config)?
        .query_row(())
        .await
        .map_err(sqlx::Error::config)?;
    assert_eq!(second_row.get::<i64>(0).map_err(sqlx::Error::config)?, 4);

    let deferred_path = directory.0.join("deferred-replica.db");
    let mut deferred = TursoConnectOptions::new()
        .filename(&deferred_path)
        .create_if_missing(true)
        .with_sync_options(
            TursoSyncOptions::new(&remote_url)
                .with_bootstrap_if_empty(false)
                .with_client_name("sqlx-turso-deferred"),
        )
        .connect()
        .await?;
    assert!(
        deferred
            .fetch_one("SELECT COUNT(*) FROM messages")
            .await
            .is_err()
    );
    assert!(deferred.sync_pull().await?);
    let row = deferred
        .fetch_one("SELECT COUNT(*) AS count FROM messages")
        .await?;
    assert_eq!(row.try_get::<i64, _>("count")?, 4);

    let mut partial = TursoConnectOptions::new()
        .in_memory(true)
        .with_sync_options(
            TursoSyncOptions::new(&remote_url)
                .with_partial_sync_options(turso::sync::PartialSyncOpts {
                    bootstrap_strategy: Some(turso::sync::PartialBootstrapStrategy::Prefix {
                        length: 128 * 1024,
                    }),
                    segment_size: 4 * 1024,
                    prefetch: true,
                })
                .with_logical_mvcc_pull(false),
        )
        .connect()
        .await?;
    let row = partial
        .fetch_one("SELECT COUNT(*) AS count FROM messages")
        .await?;
    assert_eq!(row.try_get::<i64, _>("count")?, 4);
    assert!(partial.sync_stats().await?.network_received_bytes > 0);

    let mut query_partial = TursoConnectOptions::new()
        .in_memory(true)
        .with_sync_options(
            TursoSyncOptions::new(&remote_url).with_partial_sync_options(
                turso::sync::PartialSyncOpts {
                    bootstrap_strategy: Some(turso::sync::PartialBootstrapStrategy::Query {
                        query: "SELECT * FROM messages WHERE id <= 2".to_owned(),
                    }),
                    segment_size: 0,
                    prefetch: false,
                },
            ),
        )
        .connect()
        .await?;
    let row = query_partial
        .fetch_one("SELECT value FROM messages WHERE id = 1")
        .await?;
    assert_eq!(row.try_get::<String, _>("value")?, "from-sqlx");

    let (first_database, first_native) = first.into_native_sync_parts().await?;
    let native_stats = first_database.stats().await.map_err(sqlx::Error::config)?;
    assert!(native_stats.last_push_unix_time.is_some());
    let mut first = TursoConnection::from_native_sync_parts(first_database, first_native)?;
    let row = first
        .fetch_one("SELECT COUNT(*) AS count FROM messages")
        .await?;
    assert_eq!(row.try_get::<i64, _>("count")?, 4);

    drop(server);
    drop(first);
    drop(query_partial);
    drop(partial);
    drop(deferred);
    drop(second_native);
    drop(second_database);
    Ok(())
}
