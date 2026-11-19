//! Stable namespaced identities and deterministic key encoding.

use std::marker::PhantomData;
use std::sync::Arc;

mod prepared;
pub use prepared::{PreparedKeyError, PreparedKeyPool, PreparedStableKey};

/// A consumer-defined, stable domain tag. Renaming it changes the encoded key
/// space and therefore requires an explicit migration policy.
pub trait KeyDomain: 'static {
    /// Stable tag identifying this key domain.
    const TAG: &'static str;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// A violation of stable-key identity construction rules.
pub enum KeyError {
    /// The required domain is empty.
    EmptyDomain,
    /// The required namespace is empty.
    EmptyNamespace,
    /// Computing length exceeded its supported range.
    LengthOverflow,
    /// The allocation operation failed.
    AllocationFailed,
    /// Encoded input ended before the complete value was available.
    Truncated,
    /// The version is not supported.
    UnsupportedVersion,
    /// The supplied domain does not match the required or recorded value.
    WrongDomain,
    /// The utf8 failed validation.
    InvalidUtf8,
    /// The segment failed validation.
    InvalidSegment,
    /// Encoded input contains bytes after the expected value.
    TrailingBytes,
}

impl std::fmt::Display for KeyError {
    ///
    /// # Parameters
    /// - `formatter` (`&mut std::fmt::Formatter<'_>`): Destination formatter used by the Rust formatting trait.
    ///
    /// # Returns
    /// Type: `std::fmt::Result`.
    /// Returns `std::fmt::Result`.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(self, formatter)
    }
}

impl std::error::Error for KeyError {}

/// A namespace is a validated consumer value, never a process hash or a
/// generated macro/source-location identifier.
///
/// # Type parameters
/// - `D`: Identity-domain type. Bound or type: `KeyDomain`.
pub struct KeyNamespace<D: KeyDomain> {
    name: Arc<str>,
    domain: PhantomData<fn() -> D>,
}

impl<D: KeyDomain> Clone for KeyNamespace<D> {
    ///
    /// # Returns
    /// Type: `Self`.
    fn clone(&self) -> Self {
        Self {
            name: Arc::clone(&self.name),
            domain: PhantomData,
        }
    }
}

impl<D: KeyDomain> KeyNamespace<D> {
    /// Construct a `KeyNamespace` from its required inputs and explicit policies.
    ///
    /// # Type parameters
    /// - `D`: Identity-domain type. Bound: `KeyDomain`.
    ///
    /// # Parameters
    /// - `name` (`impl Into<Arc<str>>`): Stable name used to register or locate this definition.
    ///
    /// # Returns
    /// Type: `Result<Self, KeyError>`.
    pub fn new(name: impl Into<Arc<str>>) -> Result<Self, KeyError> {
        if D::TAG.trim().is_empty() {
            return Err(KeyError::EmptyDomain);
        }
        let name = name.into();
        if name.trim().is_empty() {
            return Err(KeyError::EmptyNamespace);
        }
        Ok(Self {
            name,
            domain: PhantomData,
        })
    }

    #[must_use]
    /// Borrow the stable name used to identify this definition.
    ///
    /// # Type parameters
    /// - `D`: Identity-domain type. Bound: `KeyDomain`.
    ///
    /// # Returns
    /// Type: `&str`.
    pub fn name(&self) -> &str {
        &self.name
    }
}

/// Explicitly typed, owned logical key components. Text is UTF-8; binary is
/// not reinterpreted as text; numbers use network-order unsigned 64-bit data.
pub enum KeySegment {
    /// The message contains UTF-8 text. Contains `Arc<str>`.
    Text(Arc<str>),
    /// The message contains binary data. Contains `Arc<[u8]>`.
    Binary(Arc<[u8]>),
    /// The encoded integer is nonnegative. Contains `u64`.
    Unsigned(u64),
}

/// A logical key component borrowed for immediate encoding.
#[derive(Clone, Copy, Debug)]
pub enum KeySegmentRef<'a> {
    /// UTF-8 text.
    Text(&'a str),
    /// Uninterpreted bytes.
    Binary(&'a [u8]),
    /// An unsigned value encoded as one network-order word.
    Unsigned(u64),
}

impl<'a> From<&'a KeySegment> for KeySegmentRef<'a> {
    fn from(segment: &'a KeySegment) -> Self {
        match segment {
            KeySegment::Text(text) => Self::Text(text),
            KeySegment::Binary(bytes) => Self::Binary(bytes),
            KeySegment::Unsigned(value) => Self::Unsigned(*value),
        }
    }
}

/// Consumer-owned logical identity components. Domain-specific IDs implement
/// this trait once and can then be used by persistence, RPC and workflow
/// builders without concatenation or process-local hashing.
pub trait KeyParts {
    /// # Returns
    /// Type: `Result<Vec<KeySegment>, KeyError>`.
    fn key_segments(&self) -> Result<Vec<KeySegment>, KeyError>;
}

impl KeyParts for str {
    ///
    /// # Returns
    /// Type: `Result<Vec<KeySegment>, KeyError>`.
    fn key_segments(&self) -> Result<Vec<KeySegment>, KeyError> {
        Ok(vec![KeySegment::Text(Arc::from(self))])
    }
}

impl KeyParts for String {
    ///
    /// # Returns
    /// Type: `Result<Vec<KeySegment>, KeyError>`.
    fn key_segments(&self) -> Result<Vec<KeySegment>, KeyError> {
        self.as_str().key_segments()
    }
}

impl KeyParts for Arc<str> {
    ///
    /// # Returns
    /// Type: `Result<Vec<KeySegment>, KeyError>`.
    fn key_segments(&self) -> Result<Vec<KeySegment>, KeyError> {
        Ok(vec![KeySegment::Text(Arc::clone(self))])
    }
}

impl KeyParts for u64 {
    ///
    /// # Returns
    /// Type: `Result<Vec<KeySegment>, KeyError>`.
    fn key_segments(&self) -> Result<Vec<KeySegment>, KeyError> {
        Ok(vec![KeySegment::Unsigned(*self)])
    }
}

impl KeyParts for [u8] {
    ///
    /// # Returns
    /// Type: `Result<Vec<KeySegment>, KeyError>`.
    fn key_segments(&self) -> Result<Vec<KeySegment>, KeyError> {
        Ok(vec![KeySegment::Binary(Arc::from(self))])
    }
}

impl KeyParts for Arc<[u8]> {
    ///
    /// # Returns
    /// Type: `Result<Vec<KeySegment>, KeyError>`.
    fn key_segments(&self) -> Result<Vec<KeySegment>, KeyError> {
        Ok(vec![KeySegment::Binary(Arc::clone(self))])
    }
}

impl KeyParts for Vec<u8> {
    ///
    /// # Returns
    /// Type: `Result<Vec<KeySegment>, KeyError>`.
    fn key_segments(&self) -> Result<Vec<KeySegment>, KeyError> {
        self.as_slice().key_segments()
    }
}

/// Canonical length-prefixed key, format version 1.
///
/// Wire contract: version byte; u64-be domain length and UTF-8 domain bytes;
/// u64-be namespace length and UTF-8 namespace bytes; u64-be segment count;
/// then for each segment one kind byte, u64-be payload length and payload.
/// Kind bytes 1/2/3 mean text/binary/unsigned respectively. Each unsigned
/// payload is exactly one u64-be word. All lengths count bytes, not characters.
/// Concatenation, runtime hashes and source locations never determine identity.
///
/// # Type parameters
/// - `D`: Identity-domain type. Bound or type: `KeyDomain`.
pub struct StableKey<D: KeyDomain> {
    encoded: Arc<[u8]>,
    domain: PhantomData<fn() -> D>,
}

impl<D: KeyDomain> Clone for StableKey<D> {
    ///
    /// # Returns
    /// Type: `Self`.
    fn clone(&self) -> Self {
        Self {
            encoded: Arc::clone(&self.encoded),
            domain: PhantomData,
        }
    }
}

impl<D: KeyDomain> PartialEq for StableKey<D> {
    ///
    /// # Parameters
    /// - `other` (`&Self`): Second value used for comparison or composition.
    ///
    /// # Returns
    /// Type: `bool`.
    /// Returns whether the requested condition holds.
    fn eq(&self, other: &Self) -> bool {
        self.encoded == other.encoded
    }
}

impl<D: KeyDomain> Eq for StableKey<D> {}

impl<D: KeyDomain> std::hash::Hash for StableKey<D> {
    ///
    /// # Type parameters
    /// - `H`: Bounds: `std::hash::Hasher`.
    ///
    /// # Parameters
    /// - `state` (`&mut H`): Current or replacement state owned by this operation.
    ///
    /// # Returns
    /// Type: `()`.
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.encoded.hash(state);
    }
}

impl<D: KeyDomain> std::fmt::Debug for StableKey<D> {
    ///
    /// # Parameters
    /// - `formatter` (`&mut std::fmt::Formatter<'_>`): Destination formatter used by the Rust formatting trait.
    ///
    /// # Returns
    /// Type: `std::fmt::Result`.
    /// Returns `std::fmt::Result`.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("StableKey")
            .field("domain", &D::TAG)
            .field("encoded", &self.encoded)
            .finish()
    }
}

impl<D: KeyDomain> StableKey<D> {
    const VERSION: u8 = 1;
    const TEXT: u8 = 1;
    const BINARY: u8 = 2;
    const UNSIGNED: u8 = 3;
    const WORD: usize = std::mem::size_of::<u64>();

    /// Construct a `StableKey` from its required inputs and explicit policies.
    ///
    /// # Type parameters
    /// - `D`: Stable-key domain type.
    ///
    /// # Parameters
    /// - `namespace` (`&KeyNamespace<D>`): The namespace supplied to this operation.
    /// - `segments` (`impl IntoIterator<Item = KeySegment>`): The segments supplied to this operation.
    ///
    /// # Returns
    /// Type: `Result<Self, KeyError>`.
    pub fn new(
        namespace: &KeyNamespace<D>,
        segments: impl IntoIterator<Item = KeySegment>,
    ) -> Result<Self, KeyError> {
        let segments: Vec<_> = segments.into_iter().collect();
        let segment_refs = || segments.iter().map(KeySegmentRef::from);
        let required = Self::encoded_size(namespace, segments.len(), segment_refs())?;

        let mut encoded = Vec::new();
        encoded
            .try_reserve_exact(required)
            .map_err(|_| KeyError::AllocationFailed)?;
        Self::write_encoded(namespace, segments.len(), segment_refs(), |bytes| {
            encoded.extend_from_slice(bytes);
            Ok::<(), KeyError>(())
        })?;
        Ok(Self {
            encoded: Arc::from(encoded),
            domain: PhantomData,
        })
    }

    fn encoded_size<'a>(
        namespace: &KeyNamespace<D>,
        count: usize,
        segments: impl Iterator<Item = KeySegmentRef<'a>>,
    ) -> Result<usize, KeyError> {
        let mut required = 1usize;
        for bytes in [D::TAG.as_bytes(), namespace.name.as_bytes()] {
            let _ = u64::try_from(bytes.len()).map_err(|_| KeyError::LengthOverflow)?;
            required = required
                .checked_add(Self::WORD)
                .and_then(|count| count.checked_add(bytes.len()))
                .ok_or(KeyError::LengthOverflow)?;
        }
        let _ = u64::try_from(count).map_err(|_| KeyError::LengthOverflow)?;
        required = required
            .checked_add(Self::WORD)
            .ok_or(KeyError::LengthOverflow)?;
        for segment in segments {
            let length = match segment {
                KeySegmentRef::Text(text) => text.len(),
                KeySegmentRef::Binary(bytes) => bytes.len(),
                KeySegmentRef::Unsigned(_) => Self::WORD,
            };
            let _ = u64::try_from(length).map_err(|_| KeyError::LengthOverflow)?;
            required = required
                .checked_add(1 + Self::WORD)
                .and_then(|count| count.checked_add(length))
                .ok_or(KeyError::LengthOverflow)?;
        }

        Ok(required)
    }

    fn write_encoded<'a, E: From<KeyError>>(
        namespace: &KeyNamespace<D>,
        count: usize,
        segments: impl Iterator<Item = KeySegmentRef<'a>>,
        mut write: impl FnMut(&[u8]) -> Result<(), E>,
    ) -> Result<(), E> {
        write(&[Self::VERSION])?;
        Self::write_bytes(D::TAG.as_bytes(), &mut write)?;
        Self::write_bytes(namespace.name.as_bytes(), &mut write)?;
        Self::write_word(count, &mut write)?;
        for segment in segments {
            match segment {
                KeySegmentRef::Text(text) => {
                    write(&[Self::TEXT])?;
                    Self::write_bytes(text.as_bytes(), &mut write)?;
                }
                KeySegmentRef::Binary(bytes) => {
                    write(&[Self::BINARY])?;
                    Self::write_bytes(bytes, &mut write)?;
                }
                KeySegmentRef::Unsigned(value) => {
                    write(&[Self::UNSIGNED])?;
                    Self::write_bytes(&value.to_be_bytes(), &mut write)?;
                }
            }
        }
        Ok(())
    }

    /// Reject noncanonical or cross-domain encoded identities before use.
    ///
    /// # Type parameters
    /// - `D`: Identity-domain type. Bound: `KeyDomain`.
    ///
    /// # Parameters
    /// - `encoded` (`Arc<[u8]>`): The encoded supplied to this operation.
    ///
    /// # Returns
    /// Type: `Result<Self, KeyError>`.
    pub fn from_encoded(encoded: Arc<[u8]>) -> Result<Self, KeyError> {
        Self::validate_encoded(&encoded)?;
        Ok(Self {
            encoded,
            domain: PhantomData,
        })
    }

    fn validate_encoded(encoded: &[u8]) -> Result<(), KeyError> {
        let mut cursor = 0;
        match Self::read_exact(encoded, &mut cursor, 1)?.first() {
            Some(version) if *version == Self::VERSION => {}
            _ => return Err(KeyError::UnsupportedVersion),
        }
        let domain = Self::read_bytes(encoded, &mut cursor)?;
        if domain != D::TAG.as_bytes() {
            return Err(KeyError::WrongDomain);
        }
        if domain.is_empty() {
            return Err(KeyError::EmptyDomain);
        }
        let namespace = Self::read_bytes(encoded, &mut cursor)?;
        if namespace.is_empty() {
            return Err(KeyError::EmptyNamespace);
        }
        std::str::from_utf8(namespace).map_err(|_| KeyError::InvalidUtf8)?;
        let segments = Self::read_word(encoded, &mut cursor)?;
        for _ in 0..segments {
            let kind = Self::read_exact(encoded, &mut cursor, 1)?
                .first()
                .copied()
                .ok_or(KeyError::Truncated)?;
            let payload = Self::read_bytes(encoded, &mut cursor)?;
            match kind {
                Self::TEXT => {
                    std::str::from_utf8(payload).map_err(|_| KeyError::InvalidUtf8)?;
                }
                Self::BINARY => {}
                Self::UNSIGNED if payload.len() == Self::WORD => {}
                _ => return Err(KeyError::InvalidSegment),
            }
        }
        if cursor != encoded.len() {
            return Err(KeyError::TrailingBytes);
        }
        Ok(())
    }

    #[must_use]
    /// Borrow the stable encoded bytes without transferring ownership.
    ///
    /// # Type parameters
    /// - `D`: Identity-domain type. Bound: `KeyDomain`.
    ///
    /// # Returns
    /// Type: `&[u8]`.
    pub fn as_bytes(&self) -> &[u8] {
        &self.encoded
    }

    #[must_use]
    /// Transfer the owned encoded bytes to the caller.
    ///
    /// # Type parameters
    /// - `D`: Identity-domain type. Bound: `KeyDomain`.
    ///
    /// # Returns
    /// Type: `Arc<[u8]>`.
    pub fn into_bytes(self) -> Arc<[u8]> {
        self.encoded
    }

    fn write_word<E: From<KeyError>>(
        value: usize,
        write: &mut impl FnMut(&[u8]) -> Result<(), E>,
    ) -> Result<(), E> {
        let value = u64::try_from(value).map_err(|_| KeyError::LengthOverflow)?;
        write(&value.to_be_bytes())
    }

    fn write_bytes<E: From<KeyError>>(
        bytes: &[u8],
        write: &mut impl FnMut(&[u8]) -> Result<(), E>,
    ) -> Result<(), E> {
        Self::write_word(bytes.len(), write)?;
        write(bytes)
    }

    ///
    /// # Type parameters
    /// - `'a`: Lifetime parameter.
    ///
    /// # Parameters
    /// - `source` (`&'a [u8]`): Underlying provider or input from which values are read.
    /// - `cursor` (`&mut usize`): The cursor supplied to this operation.
    /// - `length` (`usize`): The length supplied to this operation.
    ///
    /// # Returns
    /// Type: `Result<&'a [u8], KeyError>`.
    fn read_exact<'a>(
        source: &'a [u8],
        cursor: &mut usize,
        length: usize,
    ) -> Result<&'a [u8], KeyError> {
        let end = cursor.checked_add(length).ok_or(KeyError::LengthOverflow)?;
        let slice = source.get(*cursor..end).ok_or(KeyError::Truncated)?;
        *cursor = end;
        Ok(slice)
    }

    ///
    /// # Parameters
    /// - `source` (`&[u8]`): Underlying provider or input from which values are read.
    /// - `cursor` (`&mut usize`): The cursor supplied to this operation.
    ///
    /// # Returns
    /// Type: `Result<u64, KeyError>`.
    fn read_word(source: &[u8], cursor: &mut usize) -> Result<u64, KeyError> {
        let bytes: [u8; std::mem::size_of::<u64>()] = Self::read_exact(source, cursor, Self::WORD)?
            .try_into()
            .map_err(|_| KeyError::Truncated)?;
        Ok(u64::from_be_bytes(bytes))
    }

    ///
    /// # Type parameters
    /// - `'a`: Lifetime parameter.
    ///
    /// # Parameters
    /// - `source` (`&'a [u8]`): Underlying provider or input from which values are read.
    /// - `cursor` (`&mut usize`): The cursor supplied to this operation.
    ///
    /// # Returns
    /// Type: `Result<&'a [u8], KeyError>`.
    fn read_bytes<'a>(source: &'a [u8], cursor: &mut usize) -> Result<&'a [u8], KeyError> {
        let length = Self::read_word(source, cursor)?;
        let length = usize::try_from(length).map_err(|_| KeyError::LengthOverflow)?;
        Self::read_exact(source, cursor, length)
    }
}
