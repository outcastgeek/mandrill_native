//! Optional safe diagnostics over the existing SQLx/Turso capability.
//!
//! No database, replica worker or sampler is acquired here. Native SQL sessions own
//! leases and cleanup. Only fields supported by this workspace's native sync provider
//! are projected; revision strings, URLs, credentials and failure details are excluded.

use crate::{effect_map, effect_map_error};
use effect::Effect;
use effect::effect;
use effect_sql::sql;
use effect_sql::{SqlClient, SqlError};

/// Immutable supported native sync counters and explicit Unix-second timestamps.
/// Missing timestamps remain absent; unsupported replica-attempt metadata is not invented.
#[derive(Clone, Copy, Debug)]
#[must_use]
pub struct SyncObservation {
    /// Native signed change-data-capture count, retaining provider sentinel values.
    pub cdc_operations: i64,
    /// Native main write-ahead-log size in bytes.
    pub main_wal_bytes: u64,
    /// Native revert write-ahead-log size in bytes.
    pub revert_wal_bytes: u64,
    /// Native total network bytes sent.
    pub network_sent_bytes: usize,
    /// Native total network bytes received.
    pub network_received_bytes: usize,
    /// Native last pull acknowledgment in Unix seconds, when reported.
    pub last_pull_unix_seconds: Option<i64>,
    /// Native last push acknowledgment in Unix seconds, when reported.
    pub last_push_unix_seconds: Option<i64>,
}
/// Execute the named native SQL readiness query through the supplied existing
/// client. Backend/decode/cleanup failures retain their typed SQL outcome channels.
pub fn readiness<R: Send + Sync + 'static>(client: &SqlClient) -> Effect<bool, SqlError, R> {
    effect_map! {
    program: client
    .scalar::<_, i64, R>(|| sql! { text: "SELECT 1", }), map: |value| value == 1, }
}
/// Borrow sync statistics under one existing SQL session and release its native
/// lease before returning a safe finite projection. Non-sync configurations return
/// the actual provider's typed unsupported error rather than fabricated zero stats.
pub fn sync<R: Send + Sync + 'static>(client: &SqlClient) -> Effect<SyncObservation, SqlError, R> {
    client.with_session(|session| {
        session.native(|connection| {
            Box::pin(async move {
                let stats = connection.sync_stats().await.map_err(SqlError::Provider)?;
                Ok(SyncObservation {
                    cdc_operations: stats.cdc_operations,
                    main_wal_bytes: stats.main_wal_size,
                    revert_wal_bytes: stats.revert_wal_size,
                    network_sent_bytes: stats.network_sent_bytes,
                    network_received_bytes: stats.network_received_bytes,
                    last_pull_unix_seconds: stats.last_pull_unix_time,
                    last_push_unix_seconds: stats.last_push_unix_time,
                })
            })
        })
    })
}
/// Collect a selected native observation and invoke application-owned Effect
/// presentation. Labels/locale/errors remain application policy; no request is retained.
pub fn sync_panel<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static>(
    client: &SqlClient,
    inject: impl Fn(SqlError) -> E + Send + Sync + 'static,
    present: impl Fn(SyncObservation) -> Effect<A, E, R> + Send + Sync + 'static,
) -> Effect<A, E, R> {
    effect! {
    bind;
    program: effect_map_error! { program: sync(client), map: inject, },
    body: present,
    }
}
