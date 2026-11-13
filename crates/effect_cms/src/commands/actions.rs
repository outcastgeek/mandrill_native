//! Action values and action-label resolution.
//!
//! Action values, the action-label key table and label resolution. The label port is
//! the localization owner's generic [`TranslateKeyFunc`] instantiated with the native
//! request boundary, so no second translation contract is declared.

use std::sync::Arc;

use effect_server::Request;

use crate::definitions::{ResourceAction, ResourceKind};
use crate::localization::TranslateKeyFunc;

/// One action offered by a form.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SubmitActionVM {
    /// The offered action.
    pub action: ResourceAction,
    /// Its localized label.
    pub label: String,
}

/// Translates one action label key over the caller's context.
pub type TranslateActionLabelKeyFunc<C> = TranslateKeyFunc<C>;

/// Resolves the localized label for an action.
pub type ActionLabelResolver =
    Arc<dyn Fn(&Request, &ResourceKind, &ResourceAction) -> String + Send + Sync + 'static>;

/// The translation keys for the named actions.
///
/// Every field is an open string; consumers may leave a key blank and receive the
/// action name as the label.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ActionLabelKeys {
    /// Key for the create and update actions.
    pub save: String,
    /// Key for the delete action.
    pub delete: String,
    /// Key for the publish action.
    pub publish: String,
    /// Key for the draft action.
    pub draft: String,
    /// Key for the attach action.
    pub attach: String,
    /// Key for the translation action.
    pub translate: String,
}

/// Build a resolver from label keys and the caller's translation port.
///
/// A blank key or an absent translator yields the action name. Otherwise the
/// translated label is trimmed and a blank translation falls back to the action
/// name. The request and kind arguments are preserved for the callback contract; the
/// request is the translation context here.
#[must_use]
pub fn new_action_label_resolver(
    keys: ActionLabelKeys,
    translate: Option<TranslateActionLabelKeyFunc<Request>>,
) -> ActionLabelResolver {
    Arc::new(move |request, _kind, action| {
        let key = action_label_key(&keys, action);
        let Some(translate) = translate.as_ref() else {
            return action.as_str().to_owned();
        };
        if key.is_empty() {
            return action.as_str().to_owned();
        }
        let label = translate(request, key.as_str());
        let label = label.trim();
        if label.is_empty() {
            action.as_str().to_owned()
        } else {
            label.to_owned()
        }
    })
}

/// Resolve the label key for an action.
///
/// The returned key is trimmed; an action without a configured key yields an empty
/// string.
#[must_use]
pub fn action_label_key(keys: &ActionLabelKeys, action: &ResourceAction) -> String {
    let value = action.as_str();
    let key = match value {
        crate::definitions::RESOURCE_ACTION_CREATE | crate::definitions::RESOURCE_ACTION_UPDATE => {
            &keys.save
        }
        crate::definitions::RESOURCE_ACTION_DELETE => &keys.delete,
        crate::definitions::RESOURCE_ACTION_PUBLISH => &keys.publish,
        crate::definitions::RESOURCE_ACTION_DRAFT => &keys.draft,
        crate::definitions::RESOURCE_ACTION_ATTACH => &keys.attach,
        crate::definitions::RESOURCE_ACTION_TRANSLATE => &keys.translate,
        _ => return String::new(),
    };
    key.trim().to_owned()
}
