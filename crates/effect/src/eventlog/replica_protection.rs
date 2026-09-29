//! Replica session and payload-protection boundaries.

use crate::{Effect, KeyDomain, StableKey};
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
/// Stable identity used by eventlog replica protection operations.
pub struct ReplicaSessionId(Arc<[u8]>);

impl ReplicaSessionId {
    #[must_use]
    /// Wrap a typed stable key while preserving its identity bytes.
    ///
    /// # Type parameters
    /// - `D`: Identity-domain type. Bounds: `KeyDomain`.
    ///
    /// # Parameters
    /// - `key` (`StableKey<D>`): Stable key used to locate or identify the selected value.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn from_stable<D: KeyDomain>(key: StableKey<D>) -> Self {
        Self(key.into_bytes())
    }
    #[must_use]
    /// Borrow the stable encoded bytes without transferring ownership.
    ///
    /// # Returns
    /// Type: `&[u8]`.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// The scheme and nonce needed to interpret a protected replica payload.
pub struct ProtectionMetadata {
    /// Selected protection scheme identifier. Type: `Arc<str>`.
    pub scheme: Arc<str>,
    /// The stable identity of the key. Type: `Arc<[u8]>`.
    pub key_id: Arc<[u8]>,
    /// Nonce supplied to the protection algorithm. Type: `Arc<[u8]>`.
    pub nonce: Arc<[u8]>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// Replica bytes paired with their session and protection metadata.
pub struct ProtectedReplicaPayload {
    /// Stable identity of the replica transfer session. Type: `ReplicaSessionId`.
    pub session: ReplicaSessionId,
    /// Application or protocol metadata attached to this value. Type: `ProtectionMetadata`.
    pub metadata: ProtectionMetadata,
    /// Protected replica bytes transferred between peers. Type: `Arc<[u8]>`.
    pub ciphertext: Arc<[u8]>,
}

/// Optional session-bound payload protection. Implementations in R choose
/// authentication/encryption, key lookup and nonce policy. The EventLog core
/// neither assumes plaintext transport nor embeds a native crypto runtime.
/// `seal` and `open` must authenticate the session identity and protection
/// metadata together with the payload; a matching untrusted outer session
/// field alone does not establish that binding.
pub trait ReplicaProtection: Send + Sync + 'static {
    /// Provider-specific failure type returned by this capability.
    type Error: Send + 'static;
    /// # Parameters
    /// - `session` (`ReplicaSessionId`): Scoped connection or stream session retained until settlement.
    /// - `plaintext` (`Arc<[u8]>`): The plaintext supplied to this operation.
    ///
    /// # Returns
    /// Type: `Effect<ProtectedReplicaPayload, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn seal(
        &self,
        session: ReplicaSessionId,
        plaintext: Arc<[u8]>,
    ) -> Effect<ProtectedReplicaPayload, Self::Error>;
    /// # Parameters
    /// - `payload` (`ProtectedReplicaPayload`): Application data moved into the message or stored record.
    ///
    /// # Returns
    /// Type: `Effect<Arc<[u8]>, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn open(&self, payload: ProtectedReplicaPayload) -> Effect<Arc<[u8]>, Self::Error>;
}

/// Selects replica-session and payload-protection capabilities.
pub trait HasReplicaProtection {
    /// Replica protection selected by this environment.
    type Protection: ReplicaProtection;
    /// # Returns
    /// Type: `Arc<Self::Protection>`.
    fn replica_protection(&self) -> Arc<Self::Protection>;
}
