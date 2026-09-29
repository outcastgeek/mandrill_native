use crate::{Effect, KeyDomain, StableKey};
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ReplicaSessionId(Arc<[u8]>);

impl ReplicaSessionId {
    #[must_use]
    pub fn from_stable<D: KeyDomain>(key: StableKey<D>) -> Self {
        Self(key.into_bytes())
    }
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProtectionMetadata {
    pub scheme: Arc<str>,
    pub key_id: Arc<[u8]>,
    pub nonce: Arc<[u8]>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProtectedReplicaPayload {
    pub session: ReplicaSessionId,
    pub metadata: ProtectionMetadata,
    pub ciphertext: Arc<[u8]>,
}

/// Optional session-bound payload protection. Implementations in R choose
/// authentication/encryption, key lookup and nonce policy. The EventLog core
/// neither assumes plaintext transport nor embeds a native crypto runtime.
/// `seal` and `open` must authenticate the session identity and protection
/// metadata together with the payload; a matching untrusted outer session
/// field alone does not establish that binding.
pub trait ReplicaProtection: Send + Sync + 'static {
    type Error: Send + 'static;
    fn seal(
        &self,
        session: ReplicaSessionId,
        plaintext: Arc<[u8]>,
    ) -> Effect<ProtectedReplicaPayload, Self::Error>;
    fn open(&self, payload: ProtectedReplicaPayload) -> Effect<Arc<[u8]>, Self::Error>;
}

pub trait HasReplicaProtection {
    type Protection: ReplicaProtection;
    fn replica_protection(&self) -> Arc<Self::Protection>;
}
