//! Stable media presenter identities.

use sha1::{Digest, Sha1};

/// Derive the source's stable media identity from provider and raw URL.
#[must_use]
pub fn stable_media_id(provider: &str, raw_url: &str) -> String {
    let mut hasher = Sha1::new();
    hasher.update(provider.trim().as_bytes());
    hasher.update(b"|");
    hasher.update(raw_url.trim().as_bytes());
    let digest = hasher.finalize();
    digest
        .iter()
        .take(12)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
