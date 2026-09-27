use std::{io, path::Path};

use sqlx_core::error::Error;

#[cfg(feature = "sync")]
use crate::options::TursoRemoteEncryption;
use crate::{
    TursoDatabaseTarget, TursoExperimentalFeature, TursoSyncOptions, error::unsupported_sqlx,
    executor::map_turso_error_with_options, options::TursoConnectOptions,
};

pub(crate) struct TursoDriver;

pub(crate) struct TursoDriverConnection {
    pub(crate) raw: turso::Connection,
    #[cfg(feature = "sync")]
    pub(crate) sync: Option<turso::sync::Database>,
}

impl TursoDriver {
    pub(crate) async fn connect(
        options: &TursoConnectOptions,
    ) -> Result<TursoDriverConnection, Error> {
        if let Some(database) = options.native_database() {
            let connection = database
                .connect()
                .map_err(|error| map_turso_error_with_options(error, options))?;
            Self::apply_connection_options(options, &connection).await?;
            return Ok(TursoDriverConnection {
                raw: connection,
                #[cfg(feature = "sync")]
                sync: None,
            });
        }

        #[cfg(feature = "sync")]
        if let Some(database) = options.native_sync_database() {
            let connection = database
                .connect()
                .await
                .map_err(|error| map_turso_error_with_options(error, options))?;
            Self::apply_connection_options(options, &connection).await?;
            return Ok(TursoDriverConnection {
                raw: connection,
                sync: Some(database),
            });
        }

        if let Some(sync) = options.sync_options() {
            return Self::connect_sync(options, sync).await;
        }

        let database = Self::database(options).await?;
        let connection = database
            .connect()
            .map_err(|error| map_turso_error_with_options(error, options))?;
        Self::apply_connection_options(options, &connection).await?;
        Ok(TursoDriverConnection {
            raw: connection,
            #[cfg(feature = "sync")]
            sync: None,
        })
    }

    #[cfg(feature = "sync")]
    async fn connect_sync(
        options: &TursoConnectOptions,
        sync: &TursoSyncOptions,
    ) -> Result<TursoDriverConnection, Error> {
        if options.is_read_only() {
            return Err(unsupported_sqlx("read-only Turso sync connections"));
        }

        if options.get_immutable() {
            return Err(unsupported_sqlx("immutable Turso sync connections"));
        }

        let path = match options.target() {
            TursoDatabaseTarget::Memory { .. } => ":memory:".to_owned(),
            TursoDatabaseTarget::File(path) => {
                Self::validate_file_target(options, path)?;
                path.to_string_lossy().into_owned()
            }
        };

        let mut builder =
            turso::sync::Builder::new_remote(&path).with_remote_url(sync.remote_url());

        if options.get_custom_io().is_some() {
            return Err(unsupported_sqlx("custom IO Turso sync connections"));
        }

        if let Some(auth_token_fn) = sync.auth_token_fn() {
            builder = builder.with_auth_token_fn(move || auth_token_fn());
        } else if let Some(auth_token) = sync.auth_token() {
            builder = builder.with_auth_token(auth_token);
        }

        if let Some(client_name) = sync.client_name() {
            builder = builder.with_client_name(client_name);
        }

        if let Some(timeout) = sync.long_poll_timeout() {
            builder = builder.with_long_poll_timeout(timeout);
        }

        if let Some(partial) = sync.partial_sync_options() {
            builder = builder.with_partial_sync_opts_experimental(partial);
        }

        if let Some(encryption) = sync.remote_encryption() {
            builder = match encryption {
                TursoRemoteEncryption::KeyAndCipher { key, cipher } => {
                    builder.with_remote_encryption(key, *cipher)
                }
                TursoRemoteEncryption::Key(key) => builder.with_remote_encryption_key(key),
            };
        }

        if let Some(enabled) = sync.logical_mvcc_pull() {
            builder = builder.with_logical_mvcc_pull(enabled);
        }

        builder = builder
            .bootstrap_if_empty(sync.bootstrap_if_empty())
            .experimental_attach(
                options
                    .experimental_features()
                    .is_enabled(TursoExperimentalFeature::Attach),
            )
            .experimental_custom_types(
                options
                    .experimental_features()
                    .is_enabled(TursoExperimentalFeature::CustomTypes),
            )
            .experimental_generated_columns(
                options
                    .experimental_features()
                    .is_enabled(TursoExperimentalFeature::GeneratedColumns),
            )
            .experimental_index_method(
                options
                    .experimental_features()
                    .is_enabled(TursoExperimentalFeature::IndexMethod),
            )
            .experimental_materialized_views(
                options
                    .experimental_features()
                    .is_enabled(TursoExperimentalFeature::MaterializedViews),
            )
            .experimental_multiprocess_wal(
                options
                    .experimental_features()
                    .is_enabled(TursoExperimentalFeature::MultiprocessWal),
            )
            .experimental_vacuum(
                options
                    .experimental_features()
                    .is_enabled(TursoExperimentalFeature::Vacuum),
            )
            .experimental_without_rowid(
                options
                    .experimental_features()
                    .is_enabled(TursoExperimentalFeature::WithoutRowid),
            );

        let database = builder
            .build()
            .await
            .map_err(|error| map_turso_error_with_options(error, options))?;
        let connection = database
            .connect()
            .await
            .map_err(|error| map_turso_error_with_options(error, options))?;
        Self::apply_connection_options(options, &connection).await?;
        Ok(TursoDriverConnection {
            raw: connection,
            sync: Some(database),
        })
    }

    #[cfg(not(feature = "sync"))]
    async fn connect_sync(
        _options: &TursoConnectOptions,
        _sync: &TursoSyncOptions,
    ) -> Result<TursoDriverConnection, Error> {
        Err(unsupported_sqlx(
            "Turso sync connections require the `sync` feature",
        ))
    }

    async fn database(options: &TursoConnectOptions) -> Result<turso::Database, Error> {
        Self::validate_open_support(options)?;

        match options.target() {
            TursoDatabaseTarget::Memory { .. } => Self::memory_database(options).await,
            TursoDatabaseTarget::File(path) => {
                Self::validate_file_target(options, path)?;
                let path = path.to_string_lossy();
                Self::build_database(options, &path).await
            }
        }
    }

    fn validate_file_target(options: &TursoConnectOptions, path: &Path) -> Result<(), Error> {
        if options.get_create_if_missing() || path.exists() {
            return Ok(());
        }

        Err(Error::Io(io::Error::new(
            io::ErrorKind::NotFound,
            format!("database file {} does not exist", path.display()),
        )))
    }

    fn validate_open_support(options: &TursoConnectOptions) -> Result<(), Error> {
        if options.get_immutable() {
            return Err(unsupported_sqlx("immutable Turso connections"));
        }

        Ok(())
    }

    async fn memory_database(options: &TursoConnectOptions) -> Result<turso::Database, Error> {
        if !options.get_shared_cache() {
            return Self::build_database(options, ":memory:").await;
        }

        let mut database = options.memory_state().database.lock().await;

        if let Some(database) = database.as_ref() {
            return Ok(database.clone());
        }

        let created = Self::build_database(options, ":memory:").await?;
        *database = Some(created.clone());
        Ok(created)
    }

    async fn build_database(
        options: &TursoConnectOptions,
        path: &str,
    ) -> Result<turso::Database, Error> {
        let mut builder = turso::Builder::new_local(path);

        builder = builder.read_only(options.is_read_only());

        if let Some(io) = options.get_custom_io() {
            builder = builder.with_io_impl(io.to_turso());
        } else if let Some(vfs) = options.get_vfs() {
            builder = builder.with_io(vfs.to_owned());
        }

        if let Some(encryption) = options.encryption() {
            builder = builder
                .experimental_encryption(true)
                .with_encryption(encryption.to_turso());
        }

        let experimental = options.experimental_features();
        builder = builder
            .experimental_attach(experimental.is_enabled(TursoExperimentalFeature::Attach))
            .experimental_custom_types(
                experimental.is_enabled(TursoExperimentalFeature::CustomTypes),
            )
            .experimental_generated_columns(
                experimental.is_enabled(TursoExperimentalFeature::GeneratedColumns),
            )
            .experimental_index_method(
                experimental.is_enabled(TursoExperimentalFeature::IndexMethod),
            )
            .experimental_materialized_views(
                experimental.is_enabled(TursoExperimentalFeature::MaterializedViews),
            )
            .experimental_multiprocess_wal(
                experimental.is_enabled(TursoExperimentalFeature::MultiprocessWal),
            )
            .experimental_vacuum(experimental.is_enabled(TursoExperimentalFeature::Vacuum))
            .experimental_without_rowid(
                experimental.is_enabled(TursoExperimentalFeature::WithoutRowid),
            )
            .experimental_mvcc_passive_checkpoint(
                experimental.is_enabled(TursoExperimentalFeature::MvccPassiveCheckpoint),
            );

        builder
            .build()
            .await
            .map_err(|error| map_turso_error_with_options(error, options))
    }

    async fn apply_connection_options(
        options: &TursoConnectOptions,
        connection: &turso::Connection,
    ) -> Result<(), Error> {
        let busy_timeout = options.get_busy_timeout().as_millis();
        connection
            .pragma_update("busy_timeout", busy_timeout)
            .await
            .map_err(|error| map_turso_error_with_options(error, options))?;

        for (key, value) in options.pragmas() {
            let Some(value) = value else {
                continue;
            };

            connection
                .pragma_update(key, value)
                .await
                .map_err(|error| map_turso_error_with_options(error, options))?;
        }

        Ok(())
    }
}
