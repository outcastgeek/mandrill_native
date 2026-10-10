//! Versioned security persistence schema owned by Effect Identity.
//!
//! Apply this source through `effect_sql`'s migration catalog and SQLx journal.
//! Construction of a store does not create tables or maintain another history.
//! Existing native Turso deployments require an explicit import migration.

/// Stable SQLx migration identity for the first Effect Identity schema, dated
/// 2026-10-09 with sequence 0001. It is independent of source locations.
pub const SCHEMA_MIGRATION_VERSION: i64 = 202610090001;

/// Version-one schema source. Account snapshots and indexes are private persistence
/// data; the record codec version never identifies a public HTTP view schema.
pub const SCHEMA_UP: &str = include_str!("../../migrations/202610090001_identity.sql");

/// Persistence codec version for the account/role/native security DTOs at the
/// pinned Identity revision. Unknown versions reject rather than reinterpret data.
pub const SECURITY_CODEC_VERSION: u16 = 1;

/// Maximum bounded page and per-owner claim/login/role/passkey collection count
/// required by identity_rs revision 7ba00864's store/user.rs and store/role.rs.
pub const NATIVE_COLLECTION_LIMIT: usize = 100;
/// One extra row used to reject overfull security collections or expose page continuation.
pub const NATIVE_COLLECTION_LOOKAHEAD: usize = NATIVE_COLLECTION_LIMIT + 1;

/// Recovery-hash capacity from the pinned native UserTwoFactorRecoveryCodeStore.
pub const NATIVE_RECOVERY_LIMIT: usize = 256;

/// Browser-context capacity permitted by the pinned native browser implementation.
/// Applications supply their own nonzero capacity at or below this contract bound.
pub const NATIVE_BROWSER_LIMIT: usize = 4096;

/// Maximum readable protection-index versions accepted by the native encrypted
/// personal-data store contract; this is a compatibility bound, not retention policy.
pub const NATIVE_PROTECTED_INDEX_LIMIT: usize = 128;

/// Embed the pinned per-owner count in static SQL without runtime query building.
macro_rules! collection_limit_sql {
    ($before:literal,$after:literal) => {
        concat!($before, "100", $after)
    };
}
/// Embed the native page/collection one-row lookahead in static SQL.
macro_rules! collection_lookahead_sql {
    ($before:literal,$after:literal) => {
        concat!($before, "101", $after)
    };
}
/// Embed the native recovery hash one-row lookahead in static SQL.
macro_rules! recovery_lookahead_sql {
    ($before:literal,$after:literal) => {
        concat!($before, "257", $after)
    };
}
pub(crate) use {collection_limit_sql, collection_lookahead_sql, recovery_lookahead_sql};

/// Private codec envelope shared by native security DTOs, never HTTP views.
#[derive(serde::Serialize, serde::Deserialize)]
struct SecurityEnvelope<T> {
    /// Format generation independent of native public model serialization.
    version: u16,
    /// Validated native security representation, borrowed during encoding.
    value: T,
}
/// Encode native security records by borrow under the explicitly versioned codec.
pub(crate) fn encode_security<T: serde::Serialize + ?Sized>(
    value: &T,
) -> Result<String, identity_rs::identity::StoreError> {
    serde_json::to_string(&SecurityEnvelope {
        version: SECURITY_CODEC_VERSION,
        value,
    })
    .map_err(|_| identity_rs::identity::StoreError::CorruptStoredData)
}
/// Decode a known codec generation through native validators; reject unknown formats.
pub(crate) fn decode_security<T: serde::de::DeserializeOwned>(
    payload: &str,
) -> Result<T, identity_rs::identity::StoreError> {
    let envelope: SecurityEnvelope<T> = serde_json::from_str(payload)
        .map_err(|_| identity_rs::identity::StoreError::CorruptStoredData)?;
    if envelope.version != SECURITY_CODEC_VERSION {
        return Err(identity_rs::identity::StoreError::CorruptStoredData);
    }
    Ok(envelope.value)
}
