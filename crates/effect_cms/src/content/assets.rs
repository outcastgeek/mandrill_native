//! Content asset (CDN) URL resolution.
//!
//! The owner supplies the CDN base variable name, the base-URL join rule, the content
//! asset URL, the absolute content asset URL and the absoluteness test.
//!
//! # Configuration is an explicit typed value
//!
//! A library domain must not read the process environment, so the CDN base is an
//! explicit [`ContentAssetConfig`] built by the consumer at composition time from
//! [`CONTENT_ASSET_BASE_URL_ENV_KEY`]; the environment is read there, not here. The
//! configured base is trimmed, and an unset variable is the same as an empty base.
//!
//! # Details
//!
//! - a blank path resolves to no URL, before the base is consulted;
//! - an already absolute value (`http://`, `https://`, `data:`) is returned
//!   unchanged, so a CDN base never rewrites an absolute asset;
//! - an empty base returns the trimmed path, otherwise trailing slashes of the base
//!   and leading slashes of the path are normalized into exactly one separator;
//! - the absolute URL keeps an already absolute resolved URL and otherwise joins the
//!   caller-supplied public base URL with the resolved value.
//!
//! # Cross-owner boundary
//!
//! The request's public base URL is the routing owner's rule, not this owner's.
//! [`absolute_content_asset_url`] therefore takes the public base URL as a
//! caller-supplied value: the routing/transport owner computes it from the request
//! and passes it here. Reimplementing the forwarded-host/scheme rule inside the
//! content owner would duplicate another owner's policy, so it is not done; a
//! request-shaped convenience wrapper can be published together with the routing
//! owner (recorded in the crate README and the progress ledger).

/// The environment variable that names the CDN base URL.
pub const CONTENT_ASSET_BASE_URL_ENV_KEY: &str = "CDN_URL";

/// The typed content asset configuration.
///
/// The value is the trimmed CDN base URL; an empty base means no CDN is
/// configured, which is the unset-variable state.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ContentAssetConfig {
    /// The trimmed CDN base URL.
    base_url: String,
}

impl ContentAssetConfig {
    /// Build the configuration from a CDN base URL.
    ///
    /// The value is trimmed; `None` is the unset variable.
    #[must_use]
    pub fn from_base_url(base_url: Option<&str>) -> Self {
        Self {
            base_url: base_url.unwrap_or_default().trim().to_owned(),
        }
    }

    /// The trimmed CDN base URL, empty when no CDN is configured.
    #[must_use]
    pub fn base_url(&self) -> &str {
        &self.base_url
    }
}

/// Resolve a content asset path against the configured CDN base.
#[must_use]
pub fn content_asset_url(config: &ContentAssetConfig, path: &str) -> String {
    resolve_content_asset_url(config.base_url(), path)
}

/// Resolve a content asset path against a supplied base URL.
///
/// Base-URL join rule: blank paths resolve to nothing, absolute
/// values are returned unchanged, an empty base returns the trimmed value and
/// otherwise the two are joined with exactly one separator.
#[must_use]
pub fn resolve_content_asset_url(base_url: &str, path: &str) -> String {
    let base = base_url.trim();
    let value = path.trim();
    if value.is_empty() {
        return String::new();
    }
    if is_absolute_asset_url(value) {
        return value.to_owned();
    }
    if base.is_empty() {
        return value.to_owned();
    }
    let base = base.trim_end_matches('/');
    let value = value.trim_start_matches('/');
    let mut resolved = String::with_capacity(base.len() + 1 + value.len());
    resolved.push_str(base);
    resolved.push('/');
    resolved.push_str(value);
    resolved
}

/// Resolve a content asset path to an absolute public URL.
///
/// `public_base_url` is the routing owner's `BaseURLFromRequest(r, "")` result,
/// supplied by the caller; an already absolute resolved URL is kept and a blank
/// resolved URL stays blank.
#[must_use]
pub fn absolute_content_asset_url(
    config: &ContentAssetConfig,
    public_base_url: &str,
    path: &str,
) -> String {
    let resolved = content_asset_url(config, path);
    if resolved.is_empty() {
        return String::new();
    }
    if is_absolute_asset_url(&resolved) {
        return resolved;
    }
    resolve_content_asset_url(public_base_url, &resolved)
}

/// Whether the value is already an absolute asset URL.
#[must_use]
pub fn is_absolute_asset_url(raw: &str) -> bool {
    let value = raw.trim();
    value.starts_with("http://") || value.starts_with("https://") || value.starts_with("data:")
}
