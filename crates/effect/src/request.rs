use crate::{KeyDomain, StableKey};
use std::marker::PhantomData;
use std::sync::Arc;

/// Stable identity for one logical request. Its bytes come from a validated
/// namespaced key, not a process hash or a generated source location.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RequestKey(Arc<[u8]>);

impl RequestKey {
    #[must_use]
    pub fn from_stable<D: KeyDomain>(key: StableKey<D>) -> Self {
        Self(key.into_bytes())
    }

    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

/// Stable batch partition. Two requests may be resolved together only when
/// their group identities match under the same resolver/environment scope.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RequestGroup(Arc<[u8]>);

impl RequestGroup {
    #[must_use]
    pub fn from_stable<D: KeyDomain>(group: StableKey<D>) -> Self {
        Self(group.into_bytes())
    }

    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

/// One owned request tied to its success and expected-error types. Q moves
/// into the batch runner only for a newly admitted load; duplicate observers
/// share the pending result without requiring Q, A or E to implement Clone.
/// Within one RequestScope, equal keys must name semantically equivalent
/// requests and the same result contract; a key is not a payload hash.
pub struct Request<Q, A, E> {
    key: RequestKey,
    group: RequestGroup,
    payload: Q,
    result: PhantomData<fn() -> (A, E)>,
}

impl<Q, A, E> Request<Q, A, E> {
    #[must_use]
    pub const fn new(key: RequestKey, group: RequestGroup, payload: Q) -> Self {
        Self {
            key,
            group,
            payload,
            result: PhantomData,
        }
    }

    #[must_use]
    pub fn key(&self) -> &RequestKey {
        &self.key
    }

    #[must_use]
    pub fn group(&self) -> &RequestGroup {
        &self.group
    }

    #[must_use]
    pub fn payload(&self) -> &Q {
        &self.payload
    }

    pub(crate) fn into_parts(self) -> (RequestKey, RequestGroup, Q) {
        (self.key, self.group, self.payload)
    }
}
