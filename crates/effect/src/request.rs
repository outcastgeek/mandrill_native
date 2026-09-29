//! Typed request identities and grouping metadata.

use crate::{KeyDomain, StableKey};
use std::marker::PhantomData;
use std::sync::Arc;

/// Stable identity for one logical request. Its bytes come from a validated
/// namespaced key, not a process hash or a generated source location.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RequestKey(Arc<[u8]>);

impl RequestKey {
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

/// Stable batch partition. Two requests may be resolved together only when
/// their group identities match under the same resolver/environment scope.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RequestGroup(Arc<[u8]>);

impl RequestGroup {
    #[must_use]
    /// Wrap a typed stable key while preserving its identity bytes.
    ///
    /// # Type parameters
    /// - `D`: Identity-domain type. Bounds: `KeyDomain`.
    ///
    /// # Parameters
    /// - `group` (`StableKey<D>`): The group supplied to this operation.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn from_stable<D: KeyDomain>(group: StableKey<D>) -> Self {
        Self(group.into_bytes())
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

/// One owned request tied to its success and expected-error types. Q moves
/// into the batch runner only for a newly admitted load; duplicate observers
/// share the pending result without requiring Q, A or E to implement Clone.
/// Within one RequestScope, equal keys must name semantically equivalent
/// requests and the same result contract; a key is not a payload hash.
///
/// # Type parameters
/// - `Q`: Request type.
/// - `A`: Application value type.
/// - `E`: Domain-error type.
pub struct Request<Q, A, E> {
    key: RequestKey,
    group: RequestGroup,
    payload: Q,
    result: PhantomData<fn() -> (A, E)>,
}

impl<Q, A, E> Request<Q, A, E> {
    #[must_use]
    /// # Type parameters
    /// - `A`: Application value type.
    /// - `E`: Domain-error type.
    /// - `Q`: Request type.
    ///
    /// # Parameters
    /// - `key` (`RequestKey`): Stable key used to locate or identify the selected value.
    /// - `group` (`RequestGroup`): The group supplied to this operation.
    /// - `payload` (`Q`): Application data moved into the message or stored record.
    ///
    /// # Returns
    /// Type: `Self`.
    pub const fn new(key: RequestKey, group: RequestGroup, payload: Q) -> Self {
        Self {
            key,
            group,
            payload,
            result: PhantomData,
        }
    }

    #[must_use]
    /// Borrow the stable key identifying this value.
    ///
    /// # Type parameters
    /// - `Q`: Request type.
    /// - `A`: Application value type.
    /// - `E`: Domain-error type.
    ///
    /// # Returns
    /// Type: `&RequestKey`.
    pub fn key(&self) -> &RequestKey {
        &self.key
    }

    #[must_use]
    /// Borrow the request group that batches compatible work.
    ///
    /// # Type parameters
    /// - `Q`: Request type.
    /// - `A`: Application value type.
    /// - `E`: Domain-error type.
    ///
    /// # Returns
    /// Type: `&RequestGroup`.
    pub fn group(&self) -> &RequestGroup {
        &self.group
    }

    #[must_use]
    /// Borrow the application payload without cloning it.
    ///
    /// # Type parameters
    /// - `A`: Application value type.
    /// - `E`: Domain-error type.
    /// - `Q`: Request type.
    ///
    /// # Returns
    /// Type: `&Q`.
    pub fn payload(&self) -> &Q {
        &self.payload
    }

    /// # Type parameters
    /// - `A`: Application value type.
    /// - `E`: Domain-error type.
    /// - `Q`: Request type.
    ///
    /// # Returns
    /// Type: `(RequestKey, RequestGroup, Q)`.
    pub(crate) fn into_parts(self) -> (RequestKey, RequestGroup, Q) {
        (self.key, self.group, self.payload)
    }
}
