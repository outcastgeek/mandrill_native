use std::{collections::HashSet, sync::Arc};

use either::Either;
use futures_core::{future::BoxFuture, stream::BoxStream};
use futures_util::{FutureExt, StreamExt, TryStreamExt, stream};
use sqlx_core::{
    HashMap,
    column::Column,
    error::Error,
    executor::{Execute, Executor},
    ext::ustr::UStr,
    logger::QueryLogger,
    sql_str::SqlStr,
    statement::Statement,
};
use turso_parser::{lexer::Lexer, parser::Parser, token::TokenType};

use crate::{
    Turso, TursoArguments, TursoColumn, TursoConnection, TursoQueryResult, TursoRow,
    TursoStatement, TursoValue, column::collect_column_names, error::unsupported_sqlx,
};

impl TursoConnection {
    pub(crate) async fn fetch_many_sql(
        &mut self,
        sql: SqlStr,
        persistent: bool,
        arguments: Option<TursoArguments>,
    ) -> Result<BoxStream<'static, Result<Either<TursoQueryResult, TursoRow>, Error>>, Error> {
        self.clear_pending_rollback().await?;

        let logger = QueryLogger::new(sql.clone(), self.options().log_settings().clone());
        let commands = parse_sql_commands(sql.as_str())?;
        if commands.len() > 1 {
            if arguments.is_some() {
                return Err(unsupported_sqlx("Turso SQLx batch query arguments"));
            }

            let items = self.execute_batch_sql(commands, logger).await?;
            return Ok(stream::iter(items.into_iter().map(Ok)).boxed());
        }

        let mut statement = self.prepare_query_statement(sql, persistent).await?;
        let columns: Arc<[TursoColumn]> = turso_columns(&statement.columns()).into();
        let column_names = collect_column_names(&columns);
        let rows = match arguments {
            Some(arguments) => statement
                .query(arguments.into_turso_values())
                .await
                .map_err(map_turso_error)?,
            None => statement.query(()).await.map_err(map_turso_error)?,
        };
        let connection = self.raw().clone();

        Ok(stream::try_unfold(
            FetchState::Rows {
                rows,
                statement,
                connection,
                columns,
                column_names,
                pending_row: None,
                logger,
            },
            FetchState::next,
        )
        .boxed())
    }

    async fn execute_batch_sql(
        &mut self,
        commands: Vec<String>,
        mut logger: QueryLogger,
    ) -> Result<Vec<Either<TursoQueryResult, TursoRow>>, Error> {
        let batch = self.raw().batch(commands).await.map_err(map_turso_error)?;
        let mut items = Vec::new();

        for result in batch {
            let columns: Arc<[TursoColumn]> = turso_columns(result.columns()).into();
            let column_names = collect_column_names(&columns);

            for row in result.rows() {
                let mut values = Vec::with_capacity(columns.len());
                for (index, column) in columns.iter().enumerate() {
                    values.push(
                        TursoValue::from_turso(row.get_value(index).map_err(map_turso_error)?)
                            .with_type_info(column.type_info().clone()),
                    );
                }
                items.push(Either::Right(TursoRow::with_shared_columns(
                    columns.clone(),
                    column_names.clone(),
                    values,
                )));
                logger.increment_rows_returned();
            }

            logger.increase_rows_affected(result.rows_affected());
            items.push(Either::Left(TursoQueryResult::new(
                result.rows_affected(),
                result.last_insert_rowid().unwrap_or_default(),
            )));
        }

        Ok(items)
    }

    async fn prepare_sql(&mut self, sql: SqlStr) -> Result<TursoStatement, Error> {
        if let Some(statement) = self.cached_statement(sql.as_str()) {
            return Ok(statement);
        }

        let statement = self
            .raw()
            .prepare(sql.as_str())
            .await
            .map_err(map_turso_error)?;
        let parameters = Some(parameter_count(sql.as_str())?);
        let statement = TursoStatement::with_raw(
            sql.as_str().to_owned(),
            turso_columns(&statement.columns()),
            parameters,
            statement,
        );
        self.cache_statement(sql.as_str(), statement.clone());
        Ok(statement)
    }

    async fn prepare_query_statement(
        &mut self,
        sql: SqlStr,
        persistent: bool,
    ) -> Result<turso::Statement, Error> {
        if persistent && let Some(statement) = self.prepare_sql(sql.clone()).await?.raw() {
            return Ok(statement);
        }

        self.raw()
            .prepare(sql.as_str())
            .await
            .map_err(map_turso_error)
    }
}

enum FetchState {
    Rows {
        rows: turso::Rows,
        statement: turso::Statement,
        connection: turso::Connection,
        columns: Arc<[TursoColumn]>,
        column_names: Arc<HashMap<UStr, usize>>,
        pending_row: Option<turso::Row>,
        logger: QueryLogger,
    },
    QueryResult {
        result: TursoQueryResult,
        logger: QueryLogger,
    },
    Done,
}

impl FetchState {
    async fn next(self) -> Result<Option<(Either<TursoQueryResult, TursoRow>, Self)>, Error> {
        if let Self::QueryResult { result, logger } = self {
            drop(logger);
            return Ok(Some((Either::Left(result), Self::Done)));
        }

        let Self::Rows {
            mut rows,
            statement,
            connection,
            columns,
            column_names,
            pending_row,
            mut logger,
        } = self
        else {
            return Ok(None);
        };

        let row = match pending_row {
            Some(row) => Some(row),
            None => rows.next().await.map_err(map_turso_error)?,
        };

        let Some(row) = row else {
            let rows_affected = statement.n_change();
            logger.increase_rows_affected(rows_affected);
            return Ok(Some((
                Either::Left(TursoQueryResult::new(
                    rows_affected,
                    connection.last_insert_rowid(),
                )),
                Self::Done,
            )));
        };
        logger.increment_rows_returned();

        let mut values = Vec::with_capacity(columns.len());
        for (index, column) in columns.iter().enumerate() {
            values.push(
                TursoValue::from_turso(row.get_value(index).map_err(map_turso_error)?)
                    .with_type_info(column.type_info().clone()),
            );
        }

        let pending_row = rows.next().await.map_err(map_turso_error)?;
        let next_state = if pending_row.is_some() {
            Self::Rows {
                rows,
                statement,
                connection: connection.clone(),
                columns: columns.clone(),
                column_names: column_names.clone(),
                pending_row,
                logger,
            }
        } else {
            let rows_affected = statement.n_change();
            logger.increase_rows_affected(rows_affected);
            Self::QueryResult {
                result: TursoQueryResult::new(rows_affected, connection.last_insert_rowid()),
                logger,
            }
        };

        Ok(Some((
            Either::Right(TursoRow::with_shared_columns(
                columns.clone(),
                column_names.clone(),
                values,
            )),
            next_state,
        )))
    }
}

impl<'c> Executor<'c> for &'c mut TursoConnection {
    type Database = Turso;

    fn fetch_many<'e, 'q: 'e, E>(
        self,
        mut query: E,
    ) -> BoxStream<'e, Result<Either<TursoQueryResult, TursoRow>, Error>>
    where
        'c: 'e,
        E: 'q + Execute<'q, Self::Database>,
    {
        let arguments = query.take_arguments().map_err(Error::Encode);
        let persistent = query.persistent();
        let sql = query.sql();

        stream::once(async move { self.fetch_many_sql(sql, persistent, arguments?).await })
            .try_flatten()
            .boxed()
    }

    fn fetch_optional<'e, 'q: 'e, E>(
        self,
        query: E,
    ) -> BoxFuture<'e, Result<Option<TursoRow>, Error>>
    where
        'c: 'e,
        E: 'q + Execute<'q, Self::Database>,
    {
        async move {
            let mut stream = self.fetch_many(query);

            while let Some(result) = stream.try_next().await? {
                if let Either::Right(row) = result {
                    return Ok(Some(row));
                }
            }

            Ok(None)
        }
        .boxed()
    }

    fn prepare_with<'e>(
        self,
        sql: SqlStr,
        _parameters: &'e [<Self::Database as sqlx_core::database::Database>::TypeInfo],
    ) -> BoxFuture<'e, Result<TursoStatement, Error>>
    where
        'c: 'e,
    {
        async move { self.prepare_sql(sql).await }.boxed()
    }

    fn describe<'e>(
        self,
        sql: SqlStr,
    ) -> BoxFuture<'e, Result<sqlx_core::describe::Describe<Self::Database>, Error>>
    where
        'c: 'e,
    {
        async move {
            let statement = self.prepare_sql(sql).await?;
            let columns = statement.columns().to_vec();
            let nullable = vec![None; columns.len()];

            Ok(sqlx_core::describe::Describe {
                columns,
                parameters: statement.parameters().map(|parameters| match parameters {
                    either::Either::Left(types) => either::Either::Left(types.to_vec()),
                    either::Either::Right(count) => either::Either::Right(count),
                }),
                nullable,
            })
        }
        .boxed()
    }
}

fn parse_sql_commands(sql: &str) -> Result<Vec<String>, Error> {
    Parser::new(sql.as_bytes())
        .map(|command| {
            command
                .map(|command| command.to_string())
                .map_err(|error| Error::Protocol(error.to_string()))
        })
        .collect()
}

fn parameter_count(sql: &str) -> Result<usize, Error> {
    let mut last_id = 0_usize;
    let mut named = HashSet::new();

    for token in Lexer::new(sql.as_bytes()) {
        let token = token.map_err(|error| Error::Protocol(error.to_string()))?;
        if token.token_type != TokenType::TK_VARIABLE {
            continue;
        }

        let value = token.as_bytes();
        if value == b"?" {
            last_id += 1;
        } else if value.first() == Some(&b'?') {
            let digits = value
                .strip_prefix(b"?")
                .ok_or_else(|| Error::Protocol("invalid positional parameter".to_owned()))?;
            let index = std::str::from_utf8(digits)
                .map_err(Error::config)?
                .parse::<usize>()
                .map_err(Error::config)?;
            last_id = last_id.max(index);
        } else if named.insert(value.to_vec()) {
            last_id += 1;
        }
    }

    Ok(last_id)
}

fn turso_columns(columns: &[turso::Column]) -> Vec<TursoColumn> {
    columns
        .iter()
        .enumerate()
        .map(|(ordinal, column)| TursoColumn::from_turso(ordinal, column))
        .collect()
}

pub(crate) fn map_turso_error(error: turso::Error) -> Error {
    crate::error::TursoDatabaseError::from_turso(error)
}

pub(crate) fn map_turso_error_with_options(
    error: turso::Error,
    options: &crate::TursoConnectOptions,
) -> Error {
    crate::error::TursoDatabaseError::from_turso_with_redactions(error, options.secret_values())
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use either::Either;
    use futures_util::TryStreamExt;
    use sqlx_core::{
        column::Column,
        connection::{ConnectOptions, Connection},
        error::{Error, ErrorKind},
        executor::Executor,
        row::Row,
        sql_str::SqlStr,
        statement::Statement,
        value::ValueRef,
    };

    use crate::{TursoConnectOptions, TursoRow};

    static QUERY_LOGGER: CapturingLogger = CapturingLogger(Mutex::new(Vec::new()));

    struct CapturingLogger(Mutex<Vec<String>>);

    impl log::Log for CapturingLogger {
        fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
            metadata.target() == "sqlx::query"
        }

        fn log(&self, record: &log::Record<'_>) {
            if self.enabled(record.metadata()) {
                self.0
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .push(record.args().to_string());
            }
        }

        fn flush(&self) {}
    }

    #[tokio::test]
    async fn executes_literal_sql_and_fetches_rows() -> sqlx_core::Result<()> {
        let mut connection = TursoConnectOptions::new().connect().await?;

        (&mut connection)
            .execute("CREATE TABLE test (id INTEGER PRIMARY KEY, name TEXT)")
            .await?;
        let result = (&mut connection)
            .execute("INSERT INTO test (name) VALUES ('alice')")
            .await?;
        assert_eq!(result.rows_affected(), 1);

        let rows = (&mut connection)
            .fetch_all("SELECT id, name FROM test")
            .await?;
        assert_eq!(rows.len(), 1);
        let row = rows
            .first()
            .ok_or_else(|| Error::Protocol("query returned no row".to_owned()))?;
        let column = row
            .columns()
            .first()
            .ok_or_else(|| Error::Protocol("row returned no column".to_owned()))?;
        assert_eq!(column.name(), "id");
        assert!(!row.try_get_raw(0)?.is_null());

        Ok(())
    }

    #[tokio::test]
    async fn reports_last_insert_rowid_and_checked_u64_encoding() -> sqlx_core::Result<()> {
        let mut connection = TursoConnectOptions::new().connect().await?;
        (&mut connection)
            .execute("CREATE TABLE test (id INTEGER PRIMARY KEY, value INTEGER)")
            .await?;

        let result =
            sqlx_core::query::query::<crate::Turso>("INSERT INTO test (id, value) VALUES (?, ?)")
                .bind(41_u64)
                .bind(u64::try_from(i64::MAX).map_err(sqlx_core::Error::config)?)
                .execute(&mut connection)
                .await?;
        assert_eq!(result.rows_affected(), 1);
        assert_eq!(result.last_insert_rowid(), 41);

        let overflow = match sqlx_core::query::query::<crate::Turso>(
            "INSERT INTO test (id, value) VALUES (42, ?)",
        )
        .bind(u64::MAX)
        .execute(&mut connection)
        .await
        {
            Err(error) => error,
            Ok(_) => return Err(Error::Protocol("oversized u64 was encoded".to_owned())),
        };
        assert!(matches!(overflow, sqlx_core::Error::Encode(_)));
        let count: (i64,) = sqlx_core::query_as::query_as("SELECT COUNT(*) FROM test")
            .fetch_one(&mut connection)
            .await?;
        assert_eq!(count.0, 1);

        Ok(())
    }

    #[tokio::test]
    async fn decodes_integer_storage_as_real() -> sqlx_core::Result<()> {
        let mut connection = TursoConnectOptions::new().connect().await?;
        let row = (&mut connection).fetch_one("SELECT 7 AS value").await?;
        assert_eq!(row.try_get::<f64, _>("value")?, 7.0);
        Ok(())
    }

    #[tokio::test]
    async fn executes_multi_statement_batch() -> sqlx_core::Result<()> {
        let mut connection = TursoConnectOptions::new().connect().await?;

        (&mut connection)
            .execute(
                "CREATE TABLE test (id INTEGER PRIMARY KEY, name TEXT); \
                 INSERT INTO test (name) VALUES ('alice'); \
                 INSERT INTO test (name) VALUES ('bob')",
            )
            .await?;

        let rows = (&mut connection)
            .fetch_all("SELECT name FROM test ORDER BY id")
            .await?;
        assert_eq!(rows.len(), 2);

        Ok(())
    }

    #[tokio::test]
    async fn streams_every_batch_row_and_statement_result_in_order() -> sqlx_core::Result<()> {
        let mut connection = TursoConnectOptions::new().connect().await?;

        let items = (&mut connection)
            .fetch_many(
                "CREATE TABLE test (id INTEGER PRIMARY KEY, name TEXT); \
                 INSERT INTO test (name) VALUES ('alice'); \
                 SELECT id, name FROM test; \
                 INSERT INTO test (name) VALUES ('bob') RETURNING id, name",
            )
            .try_collect::<Vec<_>>()
            .await?;

        let result_count = items
            .iter()
            .filter(|item| matches!(item, Either::Left(_)))
            .count();
        let rows = items
            .iter()
            .filter_map(|item| match item {
                Either::Right(row) => Some(row),
                Either::Left(_) => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(result_count, 4);
        assert_eq!(rows.len(), 2);

        let mut rows = rows.into_iter();
        let selected = rows
            .next()
            .ok_or_else(|| Error::Protocol("batch SELECT returned no row".to_owned()))?;
        let returned = rows
            .next()
            .ok_or_else(|| Error::Protocol("batch RETURNING returned no row".to_owned()))?;
        assert_eq!(selected.try_get::<i64, _>("id")?, 1);
        assert_eq!(selected.try_get::<String, _>("name")?, "alice");
        assert_eq!(returned.try_get::<i64, _>("id")?, 2);
        assert_eq!(returned.try_get::<String, _>("name")?, "bob");

        Ok(())
    }

    #[tokio::test]
    async fn batch_stops_at_failure_and_preserves_prior_effects() -> sqlx_core::Result<()> {
        let mut connection = TursoConnectOptions::new().connect().await?;

        let error = match (&mut connection)
            .execute(
                "CREATE TABLE test (id INTEGER PRIMARY KEY, name TEXT); \
                 INSERT INTO test (name) VALUES ('alice'); \
                 INSERT INTO missing_table (name) VALUES ('failure'); \
                 INSERT INTO test (name) VALUES ('skipped')",
            )
            .await
        {
            Err(error) => error,
            Ok(_) => return Err(Error::Protocol("failing batch succeeded".to_owned())),
        };
        assert!(error.to_string().contains("statement 2"));

        let row = (&mut connection)
            .fetch_one("SELECT COUNT(*) AS count FROM test")
            .await?;
        assert_eq!(row.try_get::<i64, _>("count")?, 1);

        Ok(())
    }

    #[tokio::test]
    async fn optional_fetch_returns_none_for_no_rows() -> sqlx_core::Result<()> {
        let mut connection = TursoConnectOptions::new().connect().await?;

        let row = (&mut connection)
            .fetch_optional("SELECT 1 WHERE false")
            .await?;
        assert!(row.is_none());

        Ok(())
    }

    #[tokio::test]
    async fn semicolon_inside_string_stays_single_statement() -> sqlx_core::Result<()> {
        let mut connection = TursoConnectOptions::new().connect().await?;

        let rows = (&mut connection).fetch_all("SELECT ';'").await?;
        assert_eq!(rows.len(), 1);

        Ok(())
    }

    #[tokio::test]
    async fn binds_anonymous_and_numbered_arguments() -> sqlx_core::Result<()> {
        let mut connection = TursoConnectOptions::new().connect().await?;

        let row = sqlx_core::query::query::<crate::Turso>(
            "SELECT ? AS anonymous, ?2 AS numbered_question, $3 AS numbered_dollar",
        )
        .bind(11_i64)
        .bind(22_i64)
        .bind(33_i64)
        .fetch_one(&mut connection)
        .await?;

        assert_eq!(row.try_get::<i64, _>("anonymous")?, 11);
        assert_eq!(row.try_get::<i64, _>("numbered_question")?, 22);
        assert_eq!(row.try_get::<i64, _>("numbered_dollar")?, 33);

        Ok(())
    }

    #[tokio::test]
    async fn binds_named_and_repeated_placeholders_by_native_slot() -> sqlx_core::Result<()> {
        let mut connection = TursoConnectOptions::new().connect().await?;

        let row = sqlx_core::query::query::<crate::Turso>(
            "SELECT $name AS first, $name AS repeated, :other AS other",
        )
        .bind(11_i64)
        .bind(22_i64)
        .fetch_one(&mut connection)
        .await?;
        assert_eq!(row.try_get::<i64, _>("first")?, 11);
        assert_eq!(row.try_get::<i64, _>("repeated")?, 11);
        assert_eq!(row.try_get::<i64, _>("other")?, 22);

        Ok(())
    }

    #[tokio::test]
    async fn round_trips_basic_storage_classes() -> sqlx_core::Result<()> {
        let mut connection = TursoConnectOptions::new().connect().await?;

        let row = sqlx_core::query::query::<crate::Turso>(
            "SELECT ? AS int_value, ? AS bool_value, ? AS real_value, \
             ? AS text_value, ? AS blob_value, ? AS null_value",
        )
        .bind(42_i64)
        .bind(true)
        .bind(3.5_f64)
        .bind("hello")
        .bind(vec![1_u8, 2, 3])
        .bind(Option::<i64>::None)
        .fetch_one(&mut connection)
        .await?;

        assert_eq!(row.try_get::<u32, _>("int_value")?, 42);
        assert!(row.try_get::<bool, _>("bool_value")?);
        assert_eq!(row.try_get::<f64, _>("real_value")?, 3.5);
        assert_eq!(row.try_get::<String, _>("text_value")?, "hello");
        assert_eq!(row.try_get::<Vec<u8>, _>("blob_value")?, [1, 2, 3]);
        assert_eq!(row.try_get::<Option<i64>, _>("null_value")?, None);

        Ok(())
    }

    #[tokio::test]
    async fn checked_unsigned_decode_rejects_negative_values() -> sqlx_core::Result<()> {
        let mut connection = TursoConnectOptions::new().connect().await?;

        let row = (&mut connection).fetch_one("SELECT -1 AS value").await?;
        assert!(row.try_get::<u32, _>("value").is_err());

        Ok(())
    }

    #[tokio::test]
    async fn declared_type_affinity_drives_decode_compatibility() -> sqlx_core::Result<()> {
        let mut connection = TursoConnectOptions::new().connect().await?;

        (&mut connection)
            .execute(
                "CREATE TABLE test (\
                 id BIGINT, \
                 flag BOOLEAN, \
                 label VARCHAR(20), \
                 payload BLOB, \
                 score DOUBLE PRECISION\
                 )",
            )
            .await?;

        sqlx_core::query::query::<crate::Turso>(
            "INSERT INTO test (id, flag, label, payload, score) VALUES (?, ?, ?, ?, ?)",
        )
        .bind(7_i64)
        .bind(true)
        .bind("alice")
        .bind(vec![9_u8, 8, 7])
        .bind(4.25_f64)
        .execute(&mut connection)
        .await?;

        let row = (&mut connection)
            .fetch_one("SELECT id, flag, label, payload, score FROM test")
            .await?;

        assert_eq!(row.try_get::<i64, _>("id")?, 7);
        assert!(row.try_get::<bool, _>("flag")?);
        assert_eq!(row.try_get::<String, _>("label")?, "alice");
        assert_eq!(row.try_get::<Vec<u8>, _>("payload")?, [9, 8, 7]);
        assert_eq!(row.try_get::<f64, _>("score")?, 4.25);

        Ok(())
    }

    #[tokio::test]
    async fn describe_uses_native_parameter_slot_metadata() -> sqlx_core::Result<()> {
        let mut connection = TursoConnectOptions::new().connect().await?;

        let describe = (&mut connection)
            .describe(SqlStr::from_static("SELECT ? AS value"))
            .await?;

        assert_eq!(describe.parameters(), Some(either::Either::Right(1)));
        assert_eq!(describe.columns().len(), 1);

        let sparse = (&mut connection)
            .prepare(SqlStr::from_static(
                "SELECT '?' AS literal, ?3 AS sparse, :named AS named, :named AS repeated -- ?99",
            ))
            .await?;
        assert_eq!(sparse.parameters(), Some(either::Either::Right(4)));

        Ok(())
    }

    #[tokio::test]
    async fn row_and_statement_name_lookup_use_same_metadata() -> sqlx_core::Result<()> {
        let mut connection = TursoConnectOptions::new().connect().await?;

        (&mut connection)
            .execute("CREATE TABLE test (id INTEGER PRIMARY KEY, name TEXT NOT NULL)")
            .await?;
        (&mut connection)
            .execute("INSERT INTO test (id, name) VALUES (1, 'alice')")
            .await?;

        let sql = SqlStr::from_static("SELECT id, name FROM test WHERE id = 1");
        let statement = (&mut connection).prepare(sql.clone()).await?;
        assert_eq!(statement.column("name").ordinal(), 1);
        assert!(statement.try_column("missing").is_err());

        let row = (&mut connection).fetch_one(sql).await?;
        assert_eq!(row.try_get::<String, _>("name")?, "alice");
        assert!(row.try_get_raw("missing").is_err());

        Ok(())
    }

    #[tokio::test]
    async fn row_and_statement_indexing_returns_precise_errors() -> sqlx_core::Result<()> {
        let mut connection = TursoConnectOptions::new().connect().await?;
        let sql = SqlStr::from_static("SELECT 1 AS duplicate, 2 AS duplicate");
        let statement = (&mut connection).prepare(sql.clone()).await?;

        assert!(matches!(
            statement.try_column(2),
            Err(Error::ColumnIndexOutOfBounds { len: 2, index: 2 })
        ));
        assert_eq!(statement.try_column("duplicate")?.ordinal(), 1);

        let row = (&mut connection).fetch_one(sql).await?;
        assert!(matches!(
            row.try_get_raw(2),
            Err(Error::ColumnIndexOutOfBounds { len: 2, index: 2 })
        ));
        assert_eq!(row.try_get::<i64, _>("duplicate")?, 2);
        assert!(matches!(
            row.try_get_raw("missing"),
            Err(Error::ColumnNotFound(name)) if name == "missing"
        ));

        let empty_row = TursoRow::new(Vec::new(), Vec::new());
        assert!(matches!(
            empty_row.try_get_raw(0),
            Err(Error::ColumnIndexOutOfBounds { len: 0, index: 0 })
        ));

        Ok(())
    }

    #[tokio::test]
    async fn supports_builtin_regexp_operator() -> sqlx_core::Result<()> {
        let mut connection = TursoConnectOptions::new().connect().await?;

        let row = (&mut connection)
            .fetch_one("SELECT 'alphabet' REGEXP '^alpha' AS matched")
            .await?;
        assert_eq!(row.try_get::<i64, _>("matched")?, 1);

        Ok(())
    }

    #[tokio::test]
    async fn maps_turso_database_errors_to_sqlx_database_errors() -> sqlx_core::Result<()> {
        let mut connection = TursoConnectOptions::new().connect().await?;

        (&mut connection)
            .execute("CREATE TABLE test (id INTEGER PRIMARY KEY)")
            .await?;
        (&mut connection)
            .execute("INSERT INTO test (id) VALUES (1)")
            .await?;

        let error = match (&mut connection)
            .execute("INSERT INTO test (id) VALUES (1)")
            .await
        {
            Err(error) => error,
            Ok(_) => {
                return Err(Error::Protocol(
                    "duplicate primary key succeeded".to_owned(),
                ));
            }
        };

        let database_error = error
            .as_database_error()
            .ok_or_else(|| Error::Protocol("Turso error was not a database error".to_owned()))?;

        assert_eq!(database_error.code().as_deref(), Some("SQLITE_CONSTRAINT"));
        assert_eq!(database_error.kind(), ErrorKind::UniqueViolation);

        Ok(())
    }

    #[tokio::test]
    async fn classifies_all_sqlite_constraint_kinds() -> sqlx_core::Result<()> {
        let mut connection = TursoConnectOptions::new().connect().await?;

        (&mut connection)
            .execute(
                "PRAGMA foreign_keys = ON;
                 CREATE TABLE parent (id INTEGER PRIMARY KEY);
                 CREATE TABLE child (
                     id INTEGER PRIMARY KEY,
                     required TEXT NOT NULL,
                     positive INTEGER CHECK (positive > 0),
                     parent_id INTEGER REFERENCES parent(id),
                     UNIQUE(required)
                 );
                 INSERT INTO parent (id) VALUES (1);
                 INSERT INTO child (id, required, positive, parent_id)
                 VALUES (1, 'present', 1, 1)",
            )
            .await?;

        let cases = [
            (
                "INSERT INTO child (id, required, positive, parent_id) VALUES (2, NULL, 1, 1)",
                ErrorKind::NotNullViolation,
            ),
            (
                "INSERT INTO child (id, required, positive, parent_id) VALUES (2, 'check', 0, 1)",
                ErrorKind::CheckViolation,
            ),
            (
                "INSERT INTO child (id, required, positive, parent_id) VALUES (2, 'foreign', 1, 999)",
                ErrorKind::ForeignKeyViolation,
            ),
            (
                "INSERT INTO child (id, required, positive, parent_id) VALUES (2, 'present', 1, 1)",
                ErrorKind::UniqueViolation,
            ),
        ];

        for (sql, expected_kind) in cases {
            let error = (&mut connection).execute(sql).await.err().ok_or_else(|| {
                Error::Protocol(format!(
                    "constraint statement unexpectedly succeeded: {sql}"
                ))
            })?;
            let database_error = error.as_database_error().ok_or_else(|| {
                Error::Protocol(format!("constraint was not a database error: {error}"))
            })?;
            assert_eq!(database_error.code().as_deref(), Some("SQLITE_CONSTRAINT"));
            assert_eq!(database_error.kind(), expected_kind);
        }

        Ok(())
    }

    #[tokio::test]
    async fn logs_query_counts_latency_and_slow_classification() -> sqlx_core::Result<()> {
        log::set_logger(&QUERY_LOGGER).map_err(Error::config)?;
        log::set_max_level(log::LevelFilter::Trace);
        QUERY_LOGGER
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();

        let options = TursoConnectOptions::new()
            .log_statements(log::LevelFilter::Info)
            .log_slow_statements(log::LevelFilter::Warn, std::time::Duration::ZERO);
        let mut connection = options.connect().await?;

        let rows = (&mut connection)
            .fetch_all(
                "SELECT 1 AS sqlx_turso_logging_probe UNION ALL SELECT 2 AS sqlx_turso_logging_probe",
            )
            .await?;
        assert_eq!(rows.len(), 2);

        let records = QUERY_LOGGER
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let record = records
            .iter()
            .find(|record| record.contains("sqlx_turso_logging_probe"))
            .ok_or_else(|| Error::Protocol(format!("query log was not emitted: {records:?}")))?;
        assert!(record.contains("rows_returned=2"), "{record}");
        assert!(record.contains("rows_affected=0"), "{record}");
        assert!(record.contains("elapsed"), "{record}");
        assert!(record.contains("slow statement"), "{record}");

        Ok(())
    }

    #[cfg(feature = "offline")]
    #[tokio::test]
    async fn serializes_describe_metadata_for_offline_mode() -> sqlx_core::Result<()> {
        use sqlx_core::sql_str::SqlSafeStr;

        let mut connection = TursoConnectOptions::new().connect().await?;

        (&mut connection)
            .execute("CREATE TABLE test (id INTEGER PRIMARY KEY, name TEXT NOT NULL)")
            .await?;

        let describe = (&mut connection)
            .describe("SELECT id, name FROM test ORDER BY id".into_sql_str())
            .await?;
        let serialized = serde_json::to_string(&describe).map_err(sqlx_core::Error::config)?;
        let deserialized: sqlx_core::describe::Describe<crate::Turso> =
            serde_json::from_str(&serialized).map_err(sqlx_core::Error::config)?;

        assert_eq!(deserialized.columns().len(), 2);
        let id = deserialized
            .columns()
            .first()
            .ok_or_else(|| Error::Protocol("describe omitted id column".to_owned()))?;
        let name = deserialized
            .columns()
            .get(1)
            .ok_or_else(|| Error::Protocol("describe omitted name column".to_owned()))?;
        assert_eq!(id.name(), "id");
        assert_eq!(
            sqlx_core::type_info::TypeInfo::name(name.type_info()),
            "TEXT"
        );

        Ok(())
    }

    #[cfg(feature = "uuid")]
    #[tokio::test]
    async fn round_trips_uuid_values() -> sqlx_core::Result<()> {
        let mut connection = TursoConnectOptions::new().connect().await?;
        let id =
            uuid::Uuid::parse_str("67e55044-10b1-426f-9247-bb680e5fe0c8").map_err(Error::config)?;

        let row = sqlx_core::query::query::<crate::Turso>("SELECT ? AS id")
            .bind(id)
            .fetch_one(&mut connection)
            .await?;

        assert_eq!(row.try_get::<uuid::Uuid, _>("id")?, id);

        Ok(())
    }

    #[cfg(feature = "json")]
    #[tokio::test]
    async fn round_trips_json_values() -> sqlx_core::Result<()> {
        let mut connection = TursoConnectOptions::new().connect().await?;
        let value = serde_json::json!({"name": "alice", "count": 2});

        let row = sqlx_core::query::query::<crate::Turso>("SELECT ? AS value")
            .bind(sqlx_core::types::Json(value.clone()))
            .fetch_one(&mut connection)
            .await?;

        let decoded = row.try_get::<sqlx_core::types::Json<serde_json::Value>, _>("value")?;
        assert_eq!(decoded.0, value);

        Ok(())
    }

    #[cfg(feature = "chrono")]
    #[tokio::test]
    async fn round_trips_chrono_values() -> sqlx_core::Result<()> {
        use chrono::{NaiveDate, NaiveDateTime, NaiveTime};

        let mut connection = TursoConnectOptions::new().connect().await?;
        let date = NaiveDate::from_ymd_opt(2026, 5, 26)
            .ok_or_else(|| Error::Protocol("test date is invalid".to_owned()))?;
        let time = NaiveTime::from_hms_micro_opt(14, 30, 15, 123_000)
            .ok_or_else(|| Error::Protocol("test time is invalid".to_owned()))?;
        let datetime = NaiveDateTime::new(date, time);

        let row = sqlx_core::query::query::<crate::Turso>(
            "SELECT ? AS date_value, ? AS time_value, ? AS datetime_value",
        )
        .bind(date)
        .bind(time)
        .bind(datetime)
        .fetch_one(&mut connection)
        .await?;

        assert_eq!(row.try_get::<NaiveDate, _>("date_value")?, date);
        assert_eq!(row.try_get::<NaiveTime, _>("time_value")?, time);
        assert_eq!(row.try_get::<NaiveDateTime, _>("datetime_value")?, datetime);

        Ok(())
    }

    #[cfg(feature = "time")]
    #[tokio::test]
    async fn round_trips_time_values() -> sqlx_core::Result<()> {
        use time::{Date, Month, PrimitiveDateTime, Time};

        let mut connection = TursoConnectOptions::new().connect().await?;
        let date = Date::from_calendar_date(2026, Month::May, 26).map_err(Error::config)?;
        let time = Time::from_hms_micro(14, 30, 15, 123_000).map_err(Error::config)?;
        let datetime = PrimitiveDateTime::new(date, time);

        let row = sqlx_core::query::query::<crate::Turso>(
            "SELECT ? AS date_value, ? AS time_value, ? AS datetime_value",
        )
        .bind(date)
        .bind(time)
        .bind(datetime)
        .fetch_one(&mut connection)
        .await?;

        assert_eq!(row.try_get::<Date, _>("date_value")?, date);
        assert_eq!(row.try_get::<Time, _>("time_value")?, time);
        assert_eq!(
            row.try_get::<PrimitiveDateTime, _>("datetime_value")?,
            datetime
        );

        Ok(())
    }

    #[tokio::test]
    async fn dropped_stream_allows_connection_reuse() -> sqlx_core::Result<()> {
        let mut connection = TursoConnectOptions::new().connect().await?;

        (&mut connection)
            .execute(
                "CREATE TABLE test (id INTEGER PRIMARY KEY); \
                 INSERT INTO test (id) VALUES (1); \
                 INSERT INTO test (id) VALUES (2)",
            )
            .await?;

        let mut rows = (&mut connection).fetch("SELECT id FROM test ORDER BY id");
        assert!(rows.try_next().await?.is_some());
        drop(rows);

        let row = (&mut connection).fetch_one("SELECT 3").await?;
        assert!(!row.try_get_raw(0)?.is_null());

        Ok(())
    }

    #[tokio::test]
    async fn dropped_unpolled_stream_does_not_execute_sql() -> sqlx_core::Result<()> {
        let mut connection = TursoConnectOptions::new().connect().await?;

        let rows = (&mut connection).fetch("CREATE TABLE test (id INTEGER PRIMARY KEY)");
        drop(rows);

        (&mut connection)
            .execute("CREATE TABLE test (id INTEGER PRIMARY KEY)")
            .await?;

        Ok(())
    }

    #[tokio::test]
    async fn bounds_and_clears_statement_cache() -> sqlx_core::Result<()> {
        let mut connection = TursoConnectOptions::new()
            .statement_cache_capacity(2)
            .connect()
            .await?;

        (&mut connection)
            .prepare(SqlStr::from_static("SELECT 1"))
            .await?;
        (&mut connection)
            .prepare(SqlStr::from_static("SELECT 2"))
            .await?;
        assert_eq!(connection.cached_statements_size(), 2);

        (&mut connection)
            .prepare(SqlStr::from_static("SELECT 3"))
            .await?;
        assert_eq!(connection.cached_statements_size(), 2);

        connection.clear_cached_statements().await?;
        assert_eq!(connection.cached_statements_size(), 0);

        Ok(())
    }
}
