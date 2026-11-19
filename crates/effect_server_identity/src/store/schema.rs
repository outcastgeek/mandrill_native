//! Versioned security persistence schema owned by Effect Identity.
//!
//! Apply this source through `effect_sql`'s migration catalog and SQLx journal.
//! Construction of a store does not create tables or maintain another history.
//! Existing native Turso deployments require an explicit import migration.

use effect::memory::{
    Buffer, BufferLease, Bytes, MemoryError, PreparedMemory, RegionId, SharedSlot, SlotPool,
};
use identity_rs::identity::StoreError;
use std::alloc::Layout;
use std::io;
use std::num::NonZeroUsize;

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

struct SecurityLane {
    records: SlotPool<'static, Buffer<'static>>,
}

/// Startup-backed native store lanes for versioned security record bytes.
pub struct PreparedSecurityRecords {
    lanes: SlotPool<'static, SecurityLane>,
}

impl PreparedSecurityRecords {
    /// Region demand for independently retained lanes and their record buffers.
    pub fn storage_bytes(
        invocations: NonZeroUsize,
        records: NonZeroUsize,
        bytes: Bytes,
    ) -> Result<Bytes, MemoryError> {
        let record = Layout::array::<u8>(bytes.get()).map_err(|_| MemoryError::InvalidLayout)?;
        let lane = SlotPool::<Buffer<'static>>::storage_bytes(records.get())?
            .checked_add(Bytes::new(record.size()).checked_mul(records.get())?)?;
        SlotPool::<SecurityLane>::storage_bytes(invocations.get())?
            .checked_add(lane.checked_mul(invocations.get())?)
    }

    /// Prepare actual encoded record bytes and borrowed decode staging.
    pub fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        invocations: NonZeroUsize,
        records: NonZeroUsize,
        bytes: Bytes,
    ) -> Result<Self, MemoryError> {
        Self::storage_bytes(invocations, records, bytes)?;
        Ok(Self {
            lanes: memory.slots(region, invocations.get(), |_| {
                Ok(SecurityLane {
                    records: memory.buffers(region, records.get(), bytes)?,
                })
            })?,
        })
    }

    pub(super) fn reserve(&self) -> Result<SecurityScope, MemoryError> {
        Ok(SecurityScope(Some(self.lanes.try_checkout()?.share())))
    }
}

#[derive(Default)]
pub(super) struct SecurityScope(Option<SharedSlot<'static, SecurityLane>>);

impl Clone for SecurityScope {
    #[expect(
        clippy::expect_used,
        reason = "existing native store clones must retain their exact admitted security lane"
    )]
    fn clone(&self) -> Self {
        self.try_clone()
            .expect("identity security lane reference count")
    }
}

impl SecurityScope {
    pub(super) fn is_prepared(&self) -> bool {
        self.0.is_some()
    }

    pub(super) fn try_clone(&self) -> Result<Self, MemoryError> {
        Ok(Self(
            self.0.as_ref().map(SharedSlot::try_clone).transpose()?,
        ))
    }

    fn buffer(&self) -> Result<Option<SecurityBuffer>, MemoryError> {
        self.0
            .as_ref()
            .map(|lane| {
                let bytes = lane.records.try_checkout()?;
                Ok(SecurityBuffer {
                    bytes,
                    _lane: lane.try_clone()?,
                })
            })
            .transpose()
    }

    pub(super) fn encode<T: serde::Serialize + ?Sized>(
        &self,
        value: &T,
    ) -> Result<SecurityText, SecurityFailure> {
        let Some(mut buffer) = self.buffer()? else {
            return encode_security(value)
                .map(SecurityText::Ordinary)
                .map_err(SecurityFailure::Native);
        };
        let mut writer = SecurityWriter {
            bytes: &mut buffer.bytes,
            refusal: None,
        };
        let result = serde_json::to_writer(
            &mut writer,
            &SecurityEnvelope {
                version: SECURITY_CODEC_VERSION,
                value,
            },
        );
        if let Some(error) = writer.refusal {
            return Err(error.into());
        }
        result.map_err(|_| SecurityFailure::Native(StoreError::CorruptStoredData))?;
        Ok(SecurityText::Prepared(buffer))
    }

    pub(super) fn decode<T: serde::de::DeserializeOwned>(
        &self,
        payload: &str,
    ) -> Result<T, SecurityFailure> {
        let Some(mut buffer) = self.buffer()? else {
            return decode_security(payload).map_err(SecurityFailure::Native);
        };
        buffer.bytes.extend_from_slice(payload.as_bytes())?;
        decode_envelope(serde_json::from_slice(buffer.bytes.as_slice()))
            .map_err(SecurityFailure::Native)
    }
}

pub(super) enum SecurityFailure {
    Native(StoreError),
    Memory(MemoryError),
}

impl From<MemoryError> for SecurityFailure {
    fn from(error: MemoryError) -> Self {
        Self::Memory(error)
    }
}

pub(super) enum SecurityText {
    Ordinary(String),
    Prepared(SecurityBuffer),
}

pub(super) struct SecurityBuffer {
    bytes: BufferLease<'static>,
    _lane: SharedSlot<'static, SecurityLane>,
}

impl Drop for SecurityBuffer {
    fn drop(&mut self) {
        self.bytes.clear_zeroed();
    }
}

impl SecurityBuffer {
    pub(super) fn text(&self) -> Result<&str, std::str::Utf8Error> {
        std::str::from_utf8(self.bytes.as_slice())
    }
}

struct SecurityWriter<'a> {
    bytes: &'a mut Buffer<'static>,
    refusal: Option<MemoryError>,
}

impl io::Write for SecurityWriter<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if let Err(error) = self.bytes.extend_from_slice(bytes) {
            self.refusal = Some(error);
            return Err(io::ErrorKind::WriteZero.into());
        }
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
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
    decode_envelope(serde_json::from_str(payload))
}

fn decode_envelope<T>(result: serde_json::Result<SecurityEnvelope<T>>) -> Result<T, StoreError> {
    let envelope = result.map_err(|_| StoreError::CorruptStoredData)?;
    if envelope.version != SECURITY_CODEC_VERSION {
        return Err(identity_rs::identity::StoreError::CorruptStoredData);
    }
    Ok(envelope.value)
}
