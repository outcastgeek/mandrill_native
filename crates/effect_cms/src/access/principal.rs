//! The access principal and the role rule.
//!
//! The principal declarations the CMS reads: the authenticated principal and the
//! role-membership rule.
//!
//! # The principal representation
//!
//! Only the principal's roles participate in this owner's rule. The local workspace
//! identity crate models authenticated principals through claims identities; this
//! owner therefore carries a Rust-owned principal value with the principal identity
//! fields and a required consumer-supplied resolver that produces it. Authentication is not
//! asserted here: an [`AccessPrincipal`] is a value, not proof of a verified session,
//! and only the consumer's identity integration can produce one from a request.
//!
//! # Role matching
//!
//! Each configured role is trimmed, blank entries are dropped, an empty
//! configured set and matches a principal role after trimming it. Those branches are
//! preserved. The nil-principal branch becomes unrepresentable because the rule takes
//! a borrowed principal.

use std::collections::BTreeSet;
use std::sync::Arc;

/// The default unauthorized message key.
pub const DEFAULT_UNAUTHORIZED_MESSAGE_KEY: &str = "error.unauthorized_message";

/// The default forbidden message key.
pub const DEFAULT_FORBIDDEN_MESSAGE_KEY: &str = "error.forbidden_message";

/// One authenticated principal as the access owner reads it.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AccessPrincipal {
    user_id: String,
    user_name: String,
    email: String,
    roles: Vec<String>,
    security_stamp: String,
    authentication_method: String,
}

impl AccessPrincipal {
    /// Build a principal value from the consumer's identity integration.
    ///
    /// The consumer owns authentication; this constructor records the resulting
    /// identity data and grants no authority by itself.
    #[must_use]
    pub fn new(
        user_id: impl Into<String>,
        user_name: impl Into<String>,
        email: impl Into<String>,
        roles: Vec<String>,
        security_stamp: impl Into<String>,
        authentication_method: impl Into<String>,
    ) -> Self {
        Self {
            user_id: user_id.into(),
            user_name: user_name.into(),
            email: email.into(),
            roles,
            security_stamp: security_stamp.into(),
            authentication_method: authentication_method.into(),
        }
    }

    /// The user identifier.
    #[must_use]
    pub fn user_id(&self) -> &str {
        self.user_id.as_str()
    }

    /// The user name.
    #[must_use]
    pub fn user_name(&self) -> &str {
        self.user_name.as_str()
    }

    /// The user's email.
    #[must_use]
    pub fn email(&self) -> &str {
        self.email.as_str()
    }

    /// The user's roles, in source order.
    #[must_use]
    pub fn roles(&self) -> &[String] {
        self.roles.as_slice()
    }

    /// The user's security stamp.
    #[must_use]
    pub fn security_stamp(&self) -> &str {
        self.security_stamp.as_str()
    }

    /// How the user authenticated.
    #[must_use]
    pub fn authentication_method(&self) -> &str {
        self.authentication_method.as_str()
    }
}

/// Decides whether a principal may access a surface.
pub type PrincipalAccessRule = Arc<dyn Fn(&AccessPrincipal) -> bool + Send + Sync + 'static>;

/// Build a rule that admits any principal holding one of the configured roles.
///
/// Configured roles are trimmed and blank entries are dropped; an empty configured
/// set admits nobody. Principal roles are trimmed before comparison.
#[must_use]
pub fn principal_has_any_role(roles: &[String]) -> PrincipalAccessRule {
    let allowed: BTreeSet<String> = roles
        .iter()
        .map(|role| role.trim().to_owned())
        .filter(|role| !role.is_empty())
        .collect();
    Arc::new(move |principal: &AccessPrincipal| {
        if allowed.is_empty() {
            return false;
        }
        principal
            .roles()
            .iter()
            .any(|role| allowed.contains(role.trim()))
    })
}
