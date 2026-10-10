//! Native ticket and key-ring adapters over selected SQL records.
//!
//! Protection, parsing and ticket verification remain native Identity algorithms.
//! Async key persistence precedes publication; a request retains an immutable
//! generation and the synchronous getter never queries SQL or borrows a lock guard.

use super::{
    SqlxIdentityStore,
    records::{RecordCategory, RecordCodec, RecordError, RecordPolicy, SqlRecordAdapter},
};
use identity_rs::identity::{
    ticket::{AuthenticationTicket, AuthenticationTicketFormat, TicketError},
    tokens::data_protection::{
        DataProtectionError, DataProtectionKeyRing, DataProtectionKeyRingSource,
    },
};
use std::{
    num::NonZeroU16,
    sync::{Arc, RwLock},
};
use time::OffsetDateTime;
use uuid::Uuid;

/// Private component record codec v1, independent of public account-view schemas.
const COMPONENT_CODEC_VERSION: u16 = 1;
/// The compile-time nonzero v1 protocol identity; no operational limit is implied.
fn component_version() -> NonZeroU16 {
    NonZeroU16::MIN
}

/// Validated native key XML. Debug deliberately exposes no key/document bytes.
pub struct KeyDocument {
    /// Original native document retained for exact durable reload.
    xml: String,
}
impl KeyDocument {
    /// Validate using the native parser before a document can be persisted/published.
    pub fn parse(xml: String) -> Result<Self, DataProtectionError> {
        DataProtectionKeyRing::parse_xml(&xml)?;
        Ok(Self { xml })
    }
    /// Borrow native key XML for an explicitly selected key-document backend.
    #[must_use]
    pub fn xml(&self) -> &str {
        &self.xml
    }
}
impl std::fmt::Debug for KeyDocument {
    /// Keys and ciphertext are never formatted, including in structured diagnostics.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("KeyDocument(redacted)")
    }
}
/// Native XML validation codec; limits belong to the selected record policy.
pub struct KeyDocumentCodec;
impl RecordCodec<KeyDocument> for KeyDocumentCodec {
    /// Stable private key-document storage version.
    fn version(&self) -> NonZeroU16 {
        component_version()
    }
    /// Persist the exact native XML bytes; this ownership conversion is required by SQLx.
    fn encode(&self, value: &KeyDocument) -> Result<Vec<u8>, RecordError> {
        Ok(value.xml.as_bytes().to_vec())
    }
    /// Reject unknown versions and validate native XML before creating a source generation.
    fn decode(&self, version: NonZeroU16, bytes: &[u8]) -> Result<KeyDocument, RecordError> {
        if version.get() != COMPONENT_CODEC_VERSION {
            return Err(RecordError::Corrupt);
        }
        let xml = String::from_utf8(bytes.to_vec()).map_err(|_| RecordError::Corrupt)?;
        KeyDocument::parse(xml).map_err(|_| RecordError::Corrupt)
    }
}

/// Immutable request-retained native key generation, implementing the synchronous
/// borrowed source contract without a global source, SQL query or dropped lock guard.
#[derive(Clone)]
pub struct KeyRingSnapshot {
    /// Retained native generation; old requests keep it alive until cleanup ends.
    ring: Arc<DataProtectionKeyRing>,
    /// Acknowledged database revision identifying publication, never issuance authority.
    revision: Uuid,
}
impl KeyRingSnapshot {
    /// Inspect the acknowledged storage generation without exposing key bytes.
    #[must_use]
    pub fn revision(&self) -> Uuid {
        self.revision
    }
}
impl DataProtectionKeyRingSource for KeyRingSnapshot {
    /// Borrow this snapshot's immutable native generation synchronously.
    fn key_ring(&self) -> Result<&DataProtectionKeyRing, DataProtectionError> {
        Ok(&self.ring)
    }
}
/// Durable key-document repository with acknowledged-only generation publication.
/// Retention of readable keys within a document is owned by explicit caller policy;
/// this adapter never invents rotation/retention durations or generates cryptography.
pub struct KeyRepository {
    /// Selected native XML SQL adapter, fenced to its account-store configuration.
    records: SqlRecordAdapter<KeyDocument, KeyDocumentCodec>,
    /// Explicit application key-domain owner namespace.
    owner: String,
    /// Stable key-document identity selected by configuration.
    id: Uuid,
    /// Latest acknowledged generation. Requests obtain an Arc-retained copy first.
    current: RwLock<Option<KeyRingSnapshot>>,
    /// Serialize asynchronous publication ordering for this repository generation.
    publication: tokio::sync::Mutex<()>,
}
impl KeyRepository {
    /// Construct inert SQL/key publication configuration; acquisition is performed by reload.
    #[must_use]
    pub fn new(backend: SqlxIdentityStore, owner: String, id: Uuid, policy: RecordPolicy) -> Self {
        Self {
            records: SqlRecordAdapter::new(backend, RecordCategory::Key, policy, KeyDocumentCodec),
            owner,
            id,
            current: RwLock::new(None),
            publication: tokio::sync::Mutex::new(()),
        }
    }
    /// Borrow the underlying selected key-record adapter for definition configuration checks.
    pub fn records(&self) -> &SqlRecordAdapter<KeyDocument, KeyDocumentCodec> {
        &self.records
    }
    /// Retain the currently published generation. Missing acquisition is typed;
    /// no native default key ring is silently created.
    pub fn snapshot(&self) -> Result<KeyRingSnapshot, DataProtectionError> {
        self.current
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
            .ok_or(DataProtectionError::NoDefaultKey)
    }
    /// Load acknowledged XML asynchronously, validate its native readable generation,
    /// then atomically publish it. Failure leaves the last good generation retained.
    pub async fn reload(&self, now: OffsetDateTime) -> Result<KeyRingSnapshot, RecordError> {
        let _publication = self.publication.lock().await;
        let record = self
            .records
            .find(&self.owner, self.id, now)
            .await?
            .ok_or(RecordError::Conflict)?;
        let ring = DataProtectionKeyRing::parse_xml(record.value.xml())
            .map_err(|_| RecordError::Corrupt)?;
        let snapshot = KeyRingSnapshot {
            ring: Arc::new(ring),
            revision: record.revision,
        };
        *self
            .current
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(snapshot.clone());
        Ok(snapshot)
    }
    /// Create/conditionally replace a validated caller-supplied key document. Native
    /// XML is parsed before I/O; only acknowledged writes publish the new generation.
    pub async fn persist(
        &self,
        expected: Option<Uuid>,
        document: KeyDocument,
    ) -> Result<KeyRingSnapshot, RecordError> {
        let _publication = self.publication.lock().await;
        let ring =
            DataProtectionKeyRing::parse_xml(document.xml()).map_err(|_| RecordError::Corrupt)?;
        let revision = match expected {
            Some(expected) => {
                self.records
                    .replace(&self.owner, self.id, expected, None, document)
                    .await?
            }
            None => {
                self.records
                    .create(&self.owner, self.id, None, document)
                    .await?
            }
        };
        let snapshot = KeyRingSnapshot {
            ring: Arc::new(ring),
            revision,
        };
        *self
            .current
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(snapshot.clone());
        Ok(snapshot)
    }
}

/// Opaque native protected ticket bytes. They are not arbitrary JSON identity proofs.
pub struct StoredTicket {
    /// Native AuthenticationTicketFormat output; secrets are excluded from Debug.
    protected: String,
}
impl std::fmt::Debug for StoredTicket {
    /// Secret ticket contents never enter logs through generic Debug formatting.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("StoredTicket(redacted)")
    }
}
/// Persist exactly the selected native protected ticket format, not a competing cookie codec.
pub struct StoredTicketCodec;
impl RecordCodec<StoredTicket> for StoredTicketCodec {
    /// Version one stores the native protected UTF-8 output bytes unchanged.
    fn version(&self) -> NonZeroU16 {
        component_version()
    }
    /// Encode the native ticket bytes into the provider's required owned binding.
    fn encode(&self, value: &StoredTicket) -> Result<Vec<u8>, RecordError> {
        Ok(value.protected.as_bytes().to_vec())
    }
    /// Reject malformed UTF-8/version; cryptographic validation remains the selected native format's job.
    fn decode(&self, version: NonZeroU16, bytes: &[u8]) -> Result<StoredTicket, RecordError> {
        if version.get() != COMPONENT_CODEC_VERSION {
            return Err(RecordError::Corrupt);
        }
        let protected = String::from_utf8(bytes.to_vec()).map_err(|_| RecordError::Corrupt)?;
        Ok(StoredTicket { protected })
    }
}
/// Stored session persistence with explicit native protection and revocation semantics.
/// Native cookie/stamp mode is a distinct definition selection and makes no stored
/// revocation promise. Current-user/security-version eligibility uses a trusted validator.
pub struct SessionRepository<T> {
    /// Same selected SQL configuration as account/native managers.
    records: SqlRecordAdapter<StoredTicket, StoredTicketCodec>,
    /// Selected native format retained once, never reacquired from defaults.
    format: Arc<T>,
}
/// Session failures keep native protection/verification errors distinct from SQL outcomes.
#[derive(Debug)]
pub enum SessionError {
    /// Native algorithm/key/format availability or verification failure.
    Ticket(TicketError),
    /// Durable session acknowledgment/codec failure including commit uncertainty.
    Storage(RecordError),
}
impl std::fmt::Display for SessionError {
    /// Keep ticket contents and database details out of public failure messages.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Ticket(_) => "identity ticket service failed",
            Self::Storage(_) => "identity session persistence failed",
        })
    }
}
impl std::error::Error for SessionError {
    /// Preserve selected native/provider diagnostic sources for trusted consumers.
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Ticket(error) => Some(error),
            Self::Storage(error) => Some(error),
        }
    }
}
impl<T: AuthenticationTicketFormat + 'static> SessionRepository<T> {
    /// Select native ticket protection and actual SQL-backed session state without I/O.
    pub fn new(backend: SqlxIdentityStore, policy: RecordPolicy, format: T) -> Self {
        Self {
            records: SqlRecordAdapter::new(
                backend,
                RecordCategory::Session,
                policy,
                StoredTicketCodec,
            ),
            format: Arc::new(format),
        }
    }
    /// Borrow the selected adapter for a definition's configuration fence.
    #[must_use]
    pub fn records(&self) -> &SqlRecordAdapter<StoredTicket, StoredTicketCodec> {
        &self.records
    }
    /// Protect with the native serializer/algorithm, then acknowledge stored-session acceptance.
    /// Caller supplies expiry deliberately; protected native-ticket expiry is checked again on load.
    pub async fn store(
        &self,
        owner: &str,
        id: Uuid,
        expires_at: OffsetDateTime,
        ticket: &AuthenticationTicket,
    ) -> Result<Uuid, SessionError> {
        let protected = self
            .format
            .protect(ticket)
            .await
            .map_err(SessionError::Ticket)?;
        self.records
            .create(owner, id, Some(expires_at), StoredTicket { protected })
            .await
            .map_err(SessionError::Storage)
    }
    /// Load current unrevoked storage, then authenticate/decode/check native ticket expiry.
    /// This alone is not current-user eligibility; the trusted authentication workflow revalidates it.
    pub async fn load(
        &self,
        owner: &str,
        id: Uuid,
        now: OffsetDateTime,
    ) -> Result<Option<AuthenticationTicket>, SessionError> {
        let Some(record) = self
            .records
            .find(owner, id, now)
            .await
            .map_err(SessionError::Storage)?
        else {
            return Ok(None);
        };
        self.format
            .unprotect(&record.value.protected)
            .await
            .map_err(SessionError::Ticket)
    }
    /// Revoke exactly the stored revision and await acknowledgment before reporting logout.
    pub async fn revoke(
        &self,
        owner: &str,
        id: Uuid,
        expected: Uuid,
    ) -> Result<bool, SessionError> {
        self.records
            .remove(owner, id, expected)
            .await
            .map_err(SessionError::Storage)
    }
}
