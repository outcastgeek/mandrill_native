//! Relation lookup: typed target selection, consumer-bound scope admission and
//! closed publication outcomes.
//!
//! Relation lookup owns typed target selection, consumer-bound scope admission and
//! closed publication outcomes, plus the relation constructors re-exported through
//! `queries`.
//!
//! # Ownership replaces one-shot stage claims
//!
//! Per-call atomic claims would stop a published stage from being replayed or
//! forged through interfaces. Private, owned values — resolution, admission and
//! observation — consumed by the next private function provide the same guarantee:
//! a stage cannot be reused, cloned into a successor or constructed outside this
//! module. The public outcome values are produced only by this owner's flow.
//!
//! # Consumer capabilities
//!
//! All related data access is consumer-supplied and effectful:
//! [`RelationScopeAdmissionFunc`] reuses the application's own scope authority before
//! any count/page or selected-label hydration, and [`RelationLookupFunc`] performs
//! the actual lookup. Both are required constructor inputs of
//! [`RelationLookupBinding`]; there is no lookup by service name, no optional
//! capability, no setter and no runtime registry. The CMS owns no relation store,
//! SQL, backend, schema, migration or pool.
//!
//! # Branches preserved
//!
//! - Request, definitions and a constructor-bound target are required; a missing
//!   request object is the nil-request rejection.
//! - Request cancellation is observed before resolution, before and after scope
//!   admission, after lookup and again before publication; each observation produces
//!   the failed outcome carrying the cancellation cause.
//! - Unknown kinds, unknown relations and a binding that does not match its
//!   construction are rejected with the owner's messages.
//! - A transport decode error, an unknown mode, a blank inverse source identity, a
//!   non-positive page or page size, an integer-overflowing page offset, an invalid
//!   or repeated selected target and a scope-admission refusal are rejected; the
//!   admission failure keeps its consumer cause and adds the issue text.
//! - A zero observation, an observation of a different admitted query, a total below
//!   the page length, a page longer than the configured size, a changed source
//!   revision, invalid/repeated/mismatched records, unrequested or duplicated
//!   hydrated selections, overlapping or visible missing selections and unaccounted
//!   requested selections fail publication with the owner's messages.
//! - An observation that reports unavailability publishes the unavailable outcome;
//!   an observation that reports failure publishes the failed outcome with its cause.
//!   Every observation kind — records, unavailable and failed — is checked against the
//!   admitted query before it is accepted.
//! - A stop observation reported without a cause receives the owner's own
//!   diagnostic instead of a consumer value.
//!
//! # Deliberately not modelled
//!
//! An unconstructed default service publishes no outcome and invokes no callback.
//! Constructed services require their checked registry and bound capabilities. The relation key's defensive "empty field" and "unknown scalar
//! type" checks are likewise enforced by the typed constructors. A stop observation
//! takes an optional consumer cause: an absent cause receives the owner's own
//! diagnostic, so no cause branch is removed. The text wrapped by the
//! `relation selection token` message now comes
//! from the local JSON reader, so the prefix and refusal categories match while the
//! nested parser text is the local reader's.
//!
//! # Effect reuse
//!
//! Descriptions are lazy: constructing the lookup performs no resolution, no registry
//! read and no cancellation check. The staged workflow is built with `effect!`, uses
//! the kernel's sync constructors for pure lifts, and copies its staged inputs when a
//! run starts, so re-running a description does not consume a required input.

use crate::commands::InputBytes;
use std::collections::BTreeSet;
use std::fmt;
use std::sync::Arc;

use effect::{Effect, effect, effect_sync};
use effect_server::Request;
use serde::Serialize;

use crate::definitions::{
    Registry, ResourceFieldKey, ResourceKind, ResourceRelationDefinition,
    ResourceRelationDescriptor, resolve_resource_relation,
};

use super::list_query::{
    PAGE_QUERY_PARAM, ParseNamedPageFunc, QueryValuesRef, RequestLanguageFunc, SEARCH_QUERY_PARAM,
    check_capacity, check_request_bytes, request_query_value, request_query_value_untrimmed,
};
use crate::forms::{PreparedMergedForm, PreparedMergedForms};
use effect::memory::{Bytes, MemoryError};
use effect_server::ServerError;
use std::num::NonZeroUsize;

/// The `choices` lookup mode.
pub const RELATION_LOOKUP_CHOICES: &str = "choices";

/// The `inverse` lookup mode.
pub const RELATION_LOOKUP_INVERSE: &str = "inverse";

/// A relation-lookup contract the owner refused, with the refusal text.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RelationContractError {
    message: String,
}

impl RelationContractError {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    /// The message text for this refusal.
    #[must_use]
    pub fn message(&self) -> &str {
        self.message.as_str()
    }
}

impl fmt::Display for RelationContractError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.message.as_str())
    }
}

impl std::error::Error for RelationContractError {}

/// One immutable, typed target key component.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RelationKeyValue {
    field: ResourceFieldKey,
    kind: RelationKeyKind,
    text: InputBytes,
    integer: i64,
    boolean: bool,
}

/// The scalar type of one relation key component.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RelationKeyKind {
    Text,
    Integer,
    Boolean,
}

impl RelationKeyKind {
    fn name(self) -> &'static str {
        match self {
            RelationKeyKind::Text => "string",
            RelationKeyKind::Integer => "int",
            RelationKeyKind::Boolean => "bool",
        }
    }
}

impl RelationKeyValue {
    /// The field this component identifies.
    #[must_use]
    pub fn field(&self) -> &ResourceFieldKey {
        &self.field
    }

    /// The string value, when this component is a string key.
    #[must_use]
    pub fn string_value(&self) -> Option<&[u8]> {
        match self.kind {
            RelationKeyKind::Text => Some(self.text.as_bytes()),
            RelationKeyKind::Integer | RelationKeyKind::Boolean => None,
        }
    }

    /// The integer value, when this component is an integer key.
    #[must_use]
    pub fn int_value(&self) -> Option<i64> {
        match self.kind {
            RelationKeyKind::Integer => Some(self.integer),
            RelationKeyKind::Text | RelationKeyKind::Boolean => None,
        }
    }

    /// The boolean value, when this component is a boolean key.
    #[must_use]
    pub fn bool_value(&self) -> Option<bool> {
        match self.kind {
            RelationKeyKind::Boolean => Some(self.boolean),
            RelationKeyKind::Text | RelationKeyKind::Integer => None,
        }
    }
}

/// Build one string target key component.
///
/// # Errors
///
/// Returns [`RelationContractError`] when the field is empty.
pub fn new_relation_string_key(
    field: ResourceFieldKey,
    value: impl Into<InputBytes>,
) -> Result<RelationKeyValue, RelationContractError> {
    new_relation_key(field, RelationKeyKind::Text, value.into(), 0, false)
}

/// Build one integer target key component.
///
/// # Errors
///
/// Returns [`RelationContractError`] when the field is empty.
pub fn new_relation_int_key(
    field: ResourceFieldKey,
    value: i64,
) -> Result<RelationKeyValue, RelationContractError> {
    new_relation_key(
        field,
        RelationKeyKind::Integer,
        InputBytes::default(),
        value,
        false,
    )
}

/// Build one boolean target key component.
///
/// # Errors
///
/// Returns [`RelationContractError`] when the field is empty.
pub fn new_relation_bool_key(
    field: ResourceFieldKey,
    value: bool,
) -> Result<RelationKeyValue, RelationContractError> {
    new_relation_key(
        field,
        RelationKeyKind::Boolean,
        InputBytes::default(),
        0,
        value,
    )
}

/// Build one target key component.
fn new_relation_key(
    field: ResourceFieldKey,
    kind: RelationKeyKind,
    text: InputBytes,
    integer: i64,
    boolean: bool,
) -> Result<RelationKeyValue, RelationContractError> {
    if field.as_str().is_empty() {
        return Err(RelationContractError::new(
            "relation key: field is required",
        ));
    }
    Ok(RelationKeyValue {
        field,
        kind,
        text,
        integer,
        boolean,
    })
}

/// One typed target identity within an exact relation declaration.
///
/// A selection grants no write permission and no right to reveal a target's label.
#[derive(Clone, Debug, Default)]
pub struct RelationSelection {
    value: Option<Arc<RelationSelectionValue>>,
}

/// The admitted selection payload.
#[derive(Debug)]
struct RelationSelectionValue {
    source: ResourceKind,
    relation: String,
    declaration: ResourceRelationDefinition,
    target: ResourceKind,
    tag: String,
    keys: Vec<RelationKeyValue>,
    identity: String,
}

/// One serialized target key component, in the JSON field names.
#[derive(Serialize)]
struct RelationKeyIdentity<'value> {
    #[serde(rename = "Field")]
    field: &'value str,
    #[serde(rename = "Type")]
    kind: &'value str,
    #[serde(rename = "Text")]
    text: &'value InputBytes,
    #[serde(rename = "Integer")]
    integer: i64,
    #[serde(rename = "Boolean")]
    boolean: bool,
}

/// One serialized target identity, in the JSON field names.
#[derive(Serialize)]
struct RelationTargetIdentity<'value> {
    #[serde(rename = "Target")]
    target: &'value str,
    #[serde(rename = "Tag")]
    tag: &'value str,
    #[serde(rename = "Keys")]
    keys: Vec<RelationKeyIdentity<'value>>,
}

/// Decoding retains encounter order, scalar-null no-ops and reused array slots.
#[derive(Default)]
struct RelationKeyToken {
    field: String,
    kind: String,
    text: String,
    integer: i64,
    boolean: bool,
}
#[derive(Default)]
struct RelationTargetToken {
    target: String,
    tag: String,
    keys: Vec<RelationKeyToken>,
    keys_len: usize,
}


fn token_field_key(key: &str) -> String {
    key.chars()
        .map(|ch| match ch {
            '\u{212a}' => 'k',
            '\u{017f}' => 's',
            _ => ch.to_ascii_lowercase(),
        })
        .collect()
}
struct KeyTokenSeed<'a>(&'a mut RelationKeyToken);
impl<'de> serde::de::DeserializeSeed<'de> for KeyTokenSeed<'_> {
    type Value = ();
    fn deserialize<D: serde::Deserializer<'de>>(self, decoder: D) -> Result<(), D::Error> {
        decoder.deserialize_option(self)
    }
}
impl<'de> serde::de::Visitor<'de> for KeyTokenSeed<'_> {
    type Value = ();
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a relation key object or null")
    }
    fn visit_none<E: serde::de::Error>(self) -> Result<(), E> {
        Ok(())
    }
    fn visit_some<D: serde::Deserializer<'de>>(self, decoder: D) -> Result<(), D::Error> {
        decoder.deserialize_map(self)
    }
    fn visit_map<M: serde::de::MapAccess<'de>>(self, mut map: M) -> Result<(), M::Error> {
        while let Some(key) = map.next_key::<String>()? {
            match token_field_key(&key).as_str() {
                "field" => {
                    if let Some(value) = map.next_value::<Option<String>>()? {
                        self.0.field = value;
                    }
                }
                "type" => {
                    if let Some(value) = map.next_value::<Option<String>>()? {
                        self.0.kind = value;
                    }
                }
                "text" => {
                    if let Some(value) = map.next_value::<Option<String>>()? {
                        self.0.text = value;
                    }
                }
                "integer" => {
                    if let Some(value) = map.next_value::<Option<i64>>()? {
                        self.0.integer = value;
                    }
                }
                "boolean" => {
                    if let Some(value) = map.next_value::<Option<bool>>()? {
                        self.0.boolean = value;
                    }
                }
                _ => return Err(serde::de::Error::custom(format!("unknown field {key:?}"))),
            }
        }
        Ok(())
    }
}
struct KeysTokenSeed<'a>(&'a mut RelationTargetToken);
impl<'de> serde::de::DeserializeSeed<'de> for KeysTokenSeed<'_> {
    type Value = ();
    fn deserialize<D: serde::Deserializer<'de>>(self, decoder: D) -> Result<(), D::Error> {
        decoder.deserialize_option(self)
    }
}
impl<'de> serde::de::Visitor<'de> for KeysTokenSeed<'_> {
    type Value = ();
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a relation keys array or null")
    }
    fn visit_none<E: serde::de::Error>(self) -> Result<(), E> {
        self.0.keys.clear();
        self.0.keys_len = 0;
        Ok(())
    }
    fn visit_some<D: serde::Deserializer<'de>>(self, decoder: D) -> Result<(), D::Error> {
        decoder.deserialize_seq(self)
    }
    fn visit_seq<S: serde::de::SeqAccess<'de>>(self, mut sequence: S) -> Result<(), S::Error> {
        let mut index = 0;
        loop {
            let existed = index < self.0.keys.len();
            if !existed {
                self.0.keys.push(RelationKeyToken::default());
            }
            if sequence
                .next_element_seed(KeyTokenSeed(&mut self.0.keys[index]))?
                .is_none()
            {
                if !existed {
                    self.0.keys.pop();
                }
                break;
            }
            index += 1;
        }
        if index == 0 {
            self.0.keys.clear();
        }
        self.0.keys_len = index;
        Ok(())
    }
}
struct TargetTokenSeed<'a>(&'a mut RelationTargetToken);
impl<'de> serde::de::DeserializeSeed<'de> for TargetTokenSeed<'_> {
    type Value = ();
    fn deserialize<D: serde::Deserializer<'de>>(self, decoder: D) -> Result<(), D::Error> {
        decoder.deserialize_option(self)
    }
}
impl<'de> serde::de::Visitor<'de> for TargetTokenSeed<'_> {
    type Value = ();
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a relation selection object or null")
    }
    fn visit_none<E: serde::de::Error>(self) -> Result<(), E> {
        Ok(())
    }
    fn visit_some<D: serde::Deserializer<'de>>(self, decoder: D) -> Result<(), D::Error> {
        decoder.deserialize_map(self)
    }
    fn visit_map<M: serde::de::MapAccess<'de>>(self, mut map: M) -> Result<(), M::Error> {
        while let Some(key) = map.next_key::<String>()? {
            match token_field_key(&key).as_str() {
                "target" => {
                    if let Some(value) = map.next_value::<Option<String>>()? {
                        self.0.target = value;
                    }
                }
                "tag" => {
                    if let Some(value) = map.next_value::<Option<String>>()? {
                        self.0.tag = value;
                    }
                }
                "keys" => map.next_value_seed(KeysTokenSeed(self.0))?,
                _ => return Err(serde::de::Error::custom(format!("unknown field {key:?}"))),
            }
        }
        Ok(())
    }
}

/// Admit one typed target selection for a relation declaration.
///
/// # Errors
///
/// Returns [`RelationContractError`] with the refusal message when the descriptor
/// is invalid, the tag does not belong to a finite target set, the
/// target or key is missing, a key is repeated, or the keys do not match the
/// declared mapping.
pub fn new_relation_selection(
    descriptor: &ResourceRelationDescriptor,
    tag: &str,
    keys: Vec<RelationKeyValue>,
) -> Result<RelationSelection, RelationContractError> {
    if !descriptor.is_valid() {
        return Err(RelationContractError::new(
            "relation selection: invalid descriptor",
        ));
    }
    let Some(source) = descriptor.source_kind().cloned() else {
        return Err(RelationContractError::new(
            "relation selection: invalid descriptor",
        ));
    };
    let Some(definition) = descriptor.definition() else {
        return Err(RelationContractError::new(
            "relation selection: invalid descriptor",
        ));
    };
    let mut target = definition.target.clone();
    let mut mapping = definition.keys.clone();
    if !definition.targets.is_empty() {
        target = ResourceKind::default();
        for candidate in &definition.targets {
            if candidate.tag == tag {
                target = candidate.target.clone();
                mapping = candidate.keys.clone();
                break;
            }
        }
    } else if !tag.is_empty() {
        return Err(RelationContractError::new(
            "relation selection: unexpected target tag",
        ));
    }
    if target.as_str().is_empty() || keys.is_empty() {
        return Err(RelationContractError::new(
            "relation selection: declared target and key are required",
        ));
    }
    let mut seen: BTreeSet<ResourceFieldKey> = BTreeSet::new();
    for key in &keys {
        if !seen.insert(key.field.clone()) {
            return Err(RelationContractError::new(
                "relation selection: invalid or repeated typed key",
            ));
        }
    }
    if !mapping.is_empty() {
        if mapping.len() != keys.len() {
            return Err(RelationContractError::new(
                "relation selection: incomplete target key",
            ));
        }
        for (key, declared) in keys.iter().zip(mapping.iter()) {
            if key.field != declared.target || key.kind.name() != declared.r#type {
                return Err(RelationContractError::new(
                    "relation selection: target key does not match declaration",
                ));
            }
        }
    }
    let identity = json_identity(
        serde_json::to_string(&RelationTargetIdentity {
            target: target.as_str(),
            tag,
            keys: keys
                .iter()
                .map(|key| RelationKeyIdentity {
                    field: key.field.as_str(),
                    kind: key.kind.name(),
                    text: &key.text,
                    integer: key.integer,
                    boolean: key.boolean,
                })
                .collect(),
        })
        .map_err(|error| RelationContractError::new(error.to_string()))?,
    );
    Ok(RelationSelection {
        value: Some(Arc::new(RelationSelectionValue {
            source,
            relation: definition.name.clone(),
            declaration: definition,
            target,
            tag: tag.to_owned(),
            keys,
            identity,
        })),
    })
}

/// HTML-safe JSON string escaping.
///
/// The escaping covers `<`, `>`, `&`, U+2028 and U+2029. The published selection
/// identity is compared with generated tokens, so the same escaping is applied to the
/// serialized identity; those characters never occur outside string literals in the
/// serialized form, so the substitution is exact.
fn json_identity(json: String) -> String {
    let mut escaped = String::with_capacity(json.len());
    for character in json.chars() {
        match character {
            '<' => escaped.push_str("\\u003c"),
            '>' => escaped.push_str("\\u003e"),
            '&' => escaped.push_str("\\u0026"),
            '\u{2028}' => escaped.push_str("\\u2028"),
            '\u{2029}' => escaped.push_str("\\u2029"),
            _ => escaped.push(character),
        }
    }
    escaped
}

/// Decode one previously published selection identity.
///
/// The decoded identity grants no lookup or mutation authority: the declaration is
/// rechecked against the supplied descriptor.
///
/// # Errors
///
/// Returns [`RelationContractError`] with the refusal message when the token is not
/// exactly one JSON value, carries unknown fields, mixes key value
/// kinds, names an unsupported key type, or belongs to a different declaration.
/// Field names use source Unicode simple folding; duplicate members, scalar null
/// values and reused Keys array slots follow the source decoder. Native JSON syntax
/// error wording remains serde's, wrapped in the source refusal prefix.
pub fn parse_relation_selection(
    descriptor: &ResourceRelationDescriptor,
    raw: impl AsRef<[u8]>,
) -> Result<RelationSelection, RelationContractError> {
    let document =
        crate::json_text::normalize_json_string_tokens(raw.as_ref()).ok_or_else(|| {
            RelationContractError::new("relation selection token: invalid JSON string")
        })?;
    let document = crate::json_text::normalize_integer_negative_zero(&document);
    let mut decoder = serde_json::Deserializer::from_slice(&document);
    let mut token = RelationTargetToken::default();
    serde::de::DeserializeSeed::deserialize(TargetTokenSeed(&mut token), &mut decoder).map_err(
        |error| RelationContractError::new(format!("relation selection token: {error}")),
    )?;
    decoder.end().map_err(|_| {
        RelationContractError::new("relation selection token: expected exactly one value")
    })?;
    let mut keys: Vec<RelationKeyValue> = Vec::with_capacity(token.keys_len);
    for component in token.keys.into_iter().take(token.keys_len) {
        let field = ResourceFieldKey::new(component.field.as_str());
        let key = match component.kind.as_str() {
            "string" => {
                if component.integer != 0 || component.boolean {
                    return Err(RelationContractError::new(
                        "relation selection token: mixed string key values",
                    ));
                }
                new_relation_string_key(field, component.text)?
            }
            "int" => {
                if !component.text.is_empty() || component.boolean {
                    return Err(RelationContractError::new(
                        "relation selection token: mixed integer key values",
                    ));
                }
                new_relation_int_key(field, component.integer)?
            }
            "bool" => {
                if !component.text.is_empty() || component.integer != 0 {
                    return Err(RelationContractError::new(
                        "relation selection token: mixed boolean key values",
                    ));
                }
                new_relation_bool_key(field, component.boolean)?
            }
            other => {
                return Err(RelationContractError::new(format!(
                    "relation selection token: unsupported key type {other:?}"
                )));
            }
        };
        keys.push(key);
    }
    let selection = new_relation_selection(descriptor, token.tag.as_str(), keys)?;
    if selection.target().as_str() != token.target {
        return Err(RelationContractError::new(
            "relation selection token: target does not match its declaration",
        ));
    }
    Ok(selection)
}

impl RelationSelection {
    /// Whether this selection was admitted.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.value.is_some()
    }

    /// Whether the complete immutable declaration, including key mappings, matches.
    #[must_use]
    pub fn belongs_to(&self, descriptor: &ResourceRelationDescriptor) -> bool {
        self.matches(descriptor)
    }

    /// The selected target kind, empty for an unadmitted selection.
    #[must_use]
    pub fn target(&self) -> ResourceKind {
        self.value
            .as_ref()
            .map_or_else(ResourceKind::default, |value| value.target.clone())
    }

    /// The declared target tag, empty for an unadmitted selection.
    #[must_use]
    pub fn tag(&self) -> String {
        self.value
            .as_ref()
            .map_or_else(String::new, |value| value.tag.clone())
    }

    /// A detached copy of the typed key components.
    #[must_use]
    pub fn keys(&self) -> Vec<RelationKeyValue> {
        self.value
            .as_ref()
            .map(|value| value.keys.clone())
            .unwrap_or_default()
    }

    /// The published selection identity, empty for an unadmitted selection.
    #[must_use]
    pub fn identity(&self) -> String {
        self.identity_ref().to_owned()
    }

    fn identity_ref(&self) -> &str {
        self.value.as_ref().map_or("", |value| value.identity.as_str())
    }

    /// Whether this selection was admitted for the supplied declaration.
    fn matches(&self, descriptor: &ResourceRelationDescriptor) -> bool {
        let Some(value) = self.value.as_ref() else {
            return false;
        };
        if !descriptor.is_valid() {
            return false;
        }
        let Some(source) = descriptor.source_kind() else {
            return false;
        };
        let Some(definition) = descriptor.definition_ref() else {
            return false;
        };
        value.source == *source
            && value.relation == definition.name
            && same_relation_definition(&value.declaration, definition)
    }
}

/// One presentation record supplied by the scoped lookup.
#[derive(Clone, Debug, Default)]
pub struct RelationLookupRecord {
    value: Option<Arc<RelationLookupRecordValue>>,
}

/// The admitted record payload.
#[derive(Debug)]
struct RelationLookupRecordValue {
    selection: RelationSelection,
    label: String,
    href: String,
}

/// Build one lookup record from an admitted selection.
///
/// # Errors
///
/// Returns [`RelationContractError`] when the selection was not admitted.
pub fn new_relation_lookup_record(
    selection: RelationSelection,
    label: String,
    href: String,
) -> Result<RelationLookupRecord, RelationContractError> {
    if !selection.is_valid() {
        return Err(RelationContractError::new(
            "relation lookup record: invalid selection",
        ));
    }
    Ok(RelationLookupRecord {
        value: Some(Arc::new(RelationLookupRecordValue {
            selection,
            label,
            href,
        })),
    })
}

impl RelationLookupRecord {
    /// Whether this record carries an admitted selection.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.value
            .as_ref()
            .is_some_and(|value| value.selection.is_valid())
    }

    /// The record's selection.
    #[must_use]
    pub fn selection(&self) -> RelationSelection {
        self.value
            .as_ref()
            .map(|value| value.selection.clone())
            .unwrap_or_default()
    }

    fn selection_ref(&self) -> Option<&RelationSelection> {
        self.value.as_ref().map(|value| &value.selection)
    }

    /// The supplied label.
    #[must_use]
    pub fn label(&self) -> String {
        self.value
            .as_ref()
            .map_or_else(String::new, |value| value.label.clone())
    }

    /// The supplied href.
    #[must_use]
    pub fn href(&self) -> String {
        self.value
            .as_ref()
            .map_or_else(String::new, |value| value.href.clone())
    }
}

/// The relation lookup mode.
///
/// The value is an open string; the
/// two constants are the only admitted spellings and any other value is rejected by
/// admission.
#[derive(Clone, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct RelationLookupMode(String);

impl RelationLookupMode {
    /// Admit the caller-supplied value exactly as written.
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// The value exactly as written.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl From<&str> for RelationLookupMode {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl fmt::Display for RelationLookupMode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.0.as_str())
    }
}

/// Shared typed transport failure; cloning retains the same original error value.
/// The lock permits Send-only causes without imposing Clone or Sync on the cause.
pub struct RelationLookupDecodeError<E>{value:Arc<std::sync::Mutex<E>>}
impl<E> RelationLookupDecodeError<E>{
    pub fn new(error:E)->Self{Self{value:Arc::new(std::sync::Mutex::new(error))}}
    pub fn with_error<A>(&self,read:impl FnOnce(&E)->A)->A{read(&self.value.lock().unwrap_or_else(std::sync::PoisonError::into_inner))}
    pub fn same_cause(&self,other:&Self)->bool{Arc::ptr_eq(&self.value,&other.value)}
}
impl<E> Clone for RelationLookupDecodeError<E>{fn clone(&self)->Self{Self{value:self.value.clone()}}}
impl<E:fmt::Debug> fmt::Debug for RelationLookupDecodeError<E>{fn fmt(&self,f:&mut fmt::Formatter<'_>)->fmt::Result{self.with_error(|error|fmt::Debug::fmt(error,f))}}
impl<E:fmt::Display> fmt::Display for RelationLookupDecodeError<E>{fn fmt(&self,f:&mut fmt::Formatter<'_>)->fmt::Result{self.with_error(|error|fmt::Display::fmt(error,f))}}
impl<E> PartialEq for RelationLookupDecodeError<E>{fn eq(&self,other:&Self)->bool{self.same_cause(other)}}
impl<E> Eq for RelationLookupDecodeError<E>{}
impl<E:std::error::Error+'static> std::error::Error for RelationLookupDecodeError<E>{}

/// Transport input for one relation lookup.
///
/// This carries input, not lookup or write authority; selected values are decoded
/// by the generated typed transport adapter, and a decode failure travels as
/// [`RelationLookupInput::decode_error`].
pub struct RelationLookupInput<E> {
    /// Transport decode failure, when the adapter could not decode the selection.
    pub decode_error: Option<RelationLookupDecodeError<E>>,
    /// The request whose cancellation, page and locale inputs are used.
    pub request: Option<Request>,
    /// Source resource kind.
    pub source: ResourceKind,
    /// Relation name.
    pub relation: String,
    /// Source record identity.
    pub source_id: InputBytes,
    /// Source revision.
    pub revision: InputBytes,
    /// Requested lookup mode.
    pub mode: RelationLookupMode,
    /// Decoded selected targets.
    pub selected: Vec<RelationSelection>,
}

impl<E> Clone for RelationLookupInput<E>{fn clone(&self)->Self{Self{decode_error:self.decode_error.clone(),request:self.request.clone(),source:self.source.clone(),relation:self.relation.clone(),source_id:self.source_id.clone(),revision:self.revision.clone(),mode:self.mode.clone(),selected:self.selected.clone()}}}
impl<E> Default for RelationLookupInput<E>{fn default()->Self{Self{decode_error:None,request:None,source:Default::default(),relation:String::new(),source_id:Default::default(),revision:Default::default(),mode:Default::default(),selected:Vec::new()}}}

/// The immutable query handed to the bound scope admission and lookup.
#[derive(Clone, Debug, Default)]
pub struct RelationQuery {
    value: Option<Arc<RelationQueryValue>>,
}

/// The admitted query payload.
#[derive(Debug)]
struct RelationQueryValue {
    descriptor: ResourceRelationDescriptor,
    source_id: InputBytes,
    revision: InputBytes,
    search: InputBytes,
    locale: InputBytes,
    mode: RelationLookupMode,
    page: i64,
    page_size: i64,
    selected: Vec<RelationSelection>,
}

impl RelationQuery {
    /// Whether this query was admitted.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.value
            .as_ref()
            .is_some_and(|value| value.descriptor.is_valid())
    }

    /// A copy of the admitted relation declaration.
    #[must_use]
    pub fn descriptor(&self) -> ResourceRelationDescriptor {
        self.value
            .as_ref()
            .map_or_else(ResourceRelationDescriptor::default, |value| {
                value.descriptor.clone()
            })
    }

    /// The source record identity.
    #[must_use]
    pub fn source_id(&self) -> InputBytes {
        self.value
            .as_ref()
            .map_or_else(InputBytes::default, |value| value.source_id.clone())
    }

    /// The source revision.
    #[must_use]
    pub fn revision(&self) -> InputBytes {
        self.value
            .as_ref()
            .map_or_else(InputBytes::default, |value| value.revision.clone())
    }

    /// The admitted search value.
    #[must_use]
    pub fn search(&self) -> InputBytes {
        self.value
            .as_ref()
            .map_or_else(InputBytes::default, |value| value.search.clone())
    }

    /// The admitted locale.
    #[must_use]
    pub fn locale(&self) -> InputBytes {
        self.value
            .as_ref()
            .map_or_else(InputBytes::default, |value| value.locale.clone())
    }

    /// The admitted lookup mode.
    #[must_use]
    pub fn mode(&self) -> RelationLookupMode {
        self.value
            .as_ref()
            .map_or_else(RelationLookupMode::default, |value| value.mode.clone())
    }

    /// The admitted page number.
    #[must_use]
    pub fn page(&self) -> i64 {
        self.value.as_ref().map_or(0, |value| value.page)
    }

    /// The admitted page size.
    #[must_use]
    pub fn page_size(&self) -> i64 {
        self.value.as_ref().map_or(0, |value| value.page_size)
    }

    /// A detached copy of the requested selections.
    #[must_use]
    pub fn selected(&self) -> Vec<RelationSelection> {
        self.value
            .as_ref()
            .map(|value| value.selected.clone())
            .unwrap_or_default()
    }

    /// The declaration's query scope.
    #[must_use]
    pub fn scope(&self) -> String {
        self.descriptor()
            .definition()
            .map_or_else(String::new, |definition| definition.scope)
    }

    /// The declared query scope of one finite target.
    ///
    /// A single-target declaration answers only for an empty tag; a finite target
    /// set answers only for its own tags, and an unknown tag yields the empty
    /// string.
    #[must_use]
    pub fn scope_for_target(&self, tag: &str) -> String {
        let Some(definition) = self.descriptor().definition() else {
            return String::new();
        };
        if definition.targets.is_empty() && tag.is_empty() {
            return definition.scope;
        }
        for target in &definition.targets {
            if target.tag == tag {
                return target.scope.clone();
            }
        }
        String::new()
    }

    /// Build an admitted query.
    ///
    /// The parameters are the admitted query's fields in order; the
    /// caller supplies exactly the values it already holds, so no builder is added.
    #[allow(clippy::too_many_arguments)]
    fn admit(
        descriptor: ResourceRelationDescriptor,
        source_id: InputBytes,
        revision: InputBytes,
        search: InputBytes,
        locale: InputBytes,
        mode: RelationLookupMode,
        page: i64,
        page_size: i64,
        selected: Vec<RelationSelection>,
    ) -> Self {
        Self {
            value: Some(Arc::new(RelationQueryValue {
                descriptor,
                source_id,
                revision,
                search,
                locale,
                mode,
                page,
                page_size,
                selected,
            })),
        }
    }

    /// Whether the supplied payload is this query's own admitted payload.
    fn same_inner(&self, other: &Arc<RelationQueryValue>) -> bool {
        self.value
            .as_ref()
            .is_some_and(|value| Arc::ptr_eq(value, other))
    }

    /// The admitted payload, when this query is valid.
    fn inner(&self) -> Option<&Arc<RelationQueryValue>> {
        self.value.as_ref()
    }
}

/// Reuses application scope authority before any lookup work.
pub type RelationScopeAdmissionFunc<E, R> =
    Arc<dyn Fn(&RelationQuery) -> Effect<(), E, R> + Send + Sync + 'static>;

/// Performs the scoped lookup and reports its observation.
pub type RelationLookupFunc<E, R> = Arc<
    dyn Fn(&RelationQuery) -> Effect<RelationLookupObservation<E>, E, R> + Send + Sync + 'static,
>;

/// One constructor-bound target capability.
///
/// The binding names the source/relation it was constructed for, so a lookup cannot
/// be dispatched by a service-name string.
pub struct RelationLookupBinding<E: Send + 'static, R: Send + Sync + 'static> {
    value: Option<Arc<RelationLookupBindingValue<E, R>>>,
}

/// The bound capability payload.
struct RelationLookupBindingValue<E: Send + 'static, R: Send + Sync + 'static> {
    source: ResourceKind,
    relation: String,
    admit: RelationScopeAdmissionFunc<E, R>,
    lookup: RelationLookupFunc<E, R>,
}

impl<E: Send + 'static, R: Send + Sync + 'static> Default for RelationLookupBinding<E, R> {
    fn default() -> Self {
        Self { value: None }
    }
}

impl<E: Send + 'static, R: Send + Sync + 'static> Clone for RelationLookupBinding<E, R> {
    fn clone(&self) -> Self {
        Self {
            value: self.value.clone(),
        }
    }
}

/// Bind one relation's scope admission and lookup capabilities.
///
/// # Errors
///
/// Returns [`RelationContractError`] when the source or relation name is blank. The
/// two capability callbacks are required arguments, so an absent callback is a
/// compile-time error rather than a runtime refusal.
pub fn new_relation_lookup_binding<E: Send + 'static, R: Send + Sync + 'static>(
    source: ResourceKind,
    relation: impl Into<String>,
    admit: RelationScopeAdmissionFunc<E, R>,
    lookup: RelationLookupFunc<E, R>,
) -> Result<RelationLookupBinding<E, R>, RelationContractError> {
    let relation = relation.into().trim().to_owned();
    if source.as_str().is_empty() || relation.is_empty() {
        return Err(RelationContractError::new(
            "relation lookup binding: source, relation, scope admission and lookup are required",
        ));
    }
    Ok(RelationLookupBinding {
        value: Some(Arc::new(RelationLookupBindingValue {
            source,
            relation,
            admit,
            lookup,
        })),
    })
}

impl<E: Send + 'static, R: Send + Sync + 'static> RelationLookupBinding<E, R> {
    /// Whether this binding was admitted.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.value.is_some()
    }
}

/// Reuses the owner's configured paging and locale inputs.
#[derive(Clone, Default)]
pub struct RelationLookupOptions {
    /// Configured page size.
    pub default_page_size: i64,
    /// Optional named-page parser.
    pub parse_named_page: Option<ParseNamedPageFunc>,
    /// Optional request-language resolver.
    pub request_language: Option<RequestLanguageFunc>,
}

/// Selected bounds for a relation page, selected targets and metadata.
#[derive(Clone, Copy)]
pub struct PreparedRelationLimits {
    /// Maximum size of a page requested from the bound provider.
    pub page_size: NonZeroUsize,
    /// Maximum selected targets admitted for independent hydration.
    pub selections: NonZeroUsize,
    /// Maximum bytes in the request path and encoded query.
    pub request: Bytes,
    /// Maximum input metadata and maximum published provider metadata.
    pub metadata: Bytes,
}

/// Resources used by the selected relation lookup path.
pub struct PreparedRelationLookup<E> {
    storage: Arc<PreparedMergedForms>,
    limits: PreparedRelationLimits,
    on_refusal: Arc<dyn Fn(ServerError) -> E + Send + Sync>,
}

impl<E> PreparedRelationLookup<E> {
    /// Bind prepared query storage and the consumer's typed refusal mapping.
    pub fn new(
        storage: Arc<PreparedMergedForms>,
        limits: PreparedRelationLimits,
        on_refusal: impl Fn(ServerError) -> E + Send + Sync + 'static,
    ) -> Self {
        Self { storage, limits, on_refusal: Arc::new(on_refusal) }
    }
}

/// The scoped relation lookup service.
pub struct RelationLookupService<E: Send + 'static, R: Send + Sync + 'static> {
    value:Option<Arc<RelationLookupServiceValue<E,R>>>,
    prepared: Option<Arc<PreparedRelationLookup<E>>>,
}
struct RelationLookupServiceValue<E:Send+'static,R:Send+Sync+'static>{
    resources: Arc<dyn Registry + Send + Sync>,
    binding: RelationLookupBinding<E, R>,
    options: RelationLookupOptions,
}

impl<E: Send + 'static, R: Send + Sync + 'static> RelationLookupService<E, R> {
    /// Construct the scoped relation lookup service.
    ///
    /// The definition registry, the constructor-bound lookup capability and a
    /// positive configured page size are all required.
    ///
    /// # Errors
    ///
    /// Returns [`RelationContractError`] with the refusal message when the binding is
    /// unconstructed or the configured page size is not positive.
    pub fn new(
        resources: Arc<dyn Registry + Send + Sync>,
        binding: RelationLookupBinding<E, R>,
        options: RelationLookupOptions,
    ) -> Result<Self, RelationContractError> {
        if !binding.is_valid() || options.default_page_size <= 0 {
            return Err(RelationContractError::new(
                "relation lookup service: definitions, bound lookup and positive configured page size are required",
            ));
        }
        Ok(Self {
            value: Some(Arc::new(RelationLookupServiceValue { resources, binding, options })),
            prepared: None,
        })
    }

    /// Select bounded decoding and fanout for the existing lookup entry point.
    #[must_use]
    pub fn with_prepared_work(mut self, prepared: Arc<PreparedRelationLookup<E>>) -> Self {
        self.prepared = Some(prepared);
        self
    }
}

impl<E: Send + 'static, R: Send + Sync + 'static> Clone for RelationLookupService<E, R> {
    fn clone(&self) -> Self {
        Self { value: self.value.clone(), prepared: self.prepared.clone() }
    }
}

impl<E: Send + 'static, R: Send + Sync + 'static> Default for RelationLookupService<E, R> {
    fn default() -> Self {
        Self { value: None, prepared: None }
    }
}

/// Construct the scoped relation lookup service.
///
/// # Errors
///
/// Returns [`RelationContractError`] under the same conditions as
/// [`RelationLookupService::new`].
pub fn new_relation_lookup_service<E: Send + 'static, R: Send + Sync + 'static>(
    resources: Arc<dyn Registry + Send + Sync>,
    binding: RelationLookupBinding<E, R>,
    options: RelationLookupOptions,
) -> Result<RelationLookupService<E, R>, RelationContractError> {
    RelationLookupService::new(resources, binding, options)
}

/// Why a lookup did not publish records.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RelationLookupCause<E> {
    /// The transport adapter's original typed failure.
    Decode(RelationLookupDecodeError<E>),
    /// The consumer's expected failure value.
    Consumer(E),
    /// An owner contract violation, with the message text.
    Message(String),
    /// The request's cancellation source.
    Cancelled,
}

/// An observation the scoped lookup returns.
///
/// A zero observation is a failed lookup, never a successful empty page. A stop
/// observation carries the admitted query payload and a cause entry: a consumer cause
/// when one was supplied, otherwise the owner's missing-cause diagnostic.
pub struct RelationLookupObservation<E> {
    value: Option<RelationObservationValue<E>>,
}

/// One observation payload.
enum RelationObservationValue<E> {
    /// A scoped page and independently hydrated selections.
    Records {
        query: Arc<RelationQueryValue>,
        rows: Vec<RelationLookupRecord>,
        selected: Vec<RelationLookupRecord>,
        missing: Vec<RelationSelection>,
        total: i64,
        revision: InputBytes,
    },
    /// The consumer reported that lookup is unavailable.
    Unavailable {
        query: Arc<RelationQueryValue>,
        cause: RelationLookupCause<E>,
    },
    /// The consumer reported a lookup failure.
    Failed {
        query: Arc<RelationQueryValue>,
        cause: RelationLookupCause<E>,
    },
}

impl<E> Default for RelationLookupObservation<E> {
    fn default() -> Self {
        Self { value: None }
    }
}

impl<E> RelationLookupObservation<E> {
    /// The observation payload, absent for the zero observation.
    fn into_value(self) -> Option<RelationObservationValue<E>> {
        self.value
    }
}

/// Snapshot one scoped page and independently hydrated selections.
///
/// # Errors
///
/// Returns [`RelationContractError`] when the query was not admitted.
pub fn observe_relation_lookup<E>(
    query: &RelationQuery,
    rows: Vec<RelationLookupRecord>,
    selected: Vec<RelationLookupRecord>,
    missing: Vec<RelationSelection>,
    total: i64,
    revision: InputBytes,
) -> Result<RelationLookupObservation<E>, RelationContractError> {
    let Some(query) = query.inner() else {
        return Err(RelationContractError::new(
            "relation lookup observation: invalid query",
        ));
    };
    Ok(RelationLookupObservation {
        value: Some(RelationObservationValue::Records {
            query: Arc::clone(query),
            rows,
            selected,
            missing,
            total,
            revision,
        }),
    })
}

/// Report that lookup is unavailable for the admitted query.
///
/// An unadmitted query yields the zero observation, which is a failed lookup, not a
/// published outcome. An absent cause yields the owner's own diagnostic instead of a
/// consumer value.
#[must_use]
pub fn observe_relation_lookup_unavailable<E>(
    query: &RelationQuery,
    cause: Option<E>,
) -> RelationLookupObservation<E> {
    observe_relation_stop(query, cause, true)
}

/// Report a lookup failure for the admitted query.
///
/// An unadmitted query yields the zero observation, which is a failed lookup, not a
/// published outcome. An absent cause yields the owner's own diagnostic instead of a
/// consumer value.
#[must_use]
pub fn observe_relation_lookup_failed<E>(
    query: &RelationQuery,
    cause: Option<E>,
) -> RelationLookupObservation<E> {
    observe_relation_stop(query, cause, false)
}

/// The diagnostic for a stop observation reported without a cause.
const MISSING_STOP_CAUSE: &str = "relation lookup stopped without a diagnostic cause";

/// Build one stop observation for an admitted query.
///
/// The observation keeps the admitted query payload, exactly as the owner binds
/// `query.value` for every observation kind, so a callback cannot hand back another
/// query's stop observation.
fn observe_relation_stop<E>(
    query: &RelationQuery,
    cause: Option<E>,
    unavailable: bool,
) -> RelationLookupObservation<E> {
    let Some(query) = query.inner() else {
        return RelationLookupObservation::default();
    };
    let cause = cause.map_or_else(
        || RelationLookupCause::Message(MISSING_STOP_CAUSE.to_owned()),
        RelationLookupCause::Consumer,
    );
    let value = if unavailable {
        RelationObservationValue::Unavailable {
            query: Arc::clone(query),
            cause,
        }
    } else {
        RelationObservationValue::Failed {
            query: Arc::clone(query),
            cause,
        }
    };
    RelationLookupObservation { value: Some(value) }
}

/// A published relation lookup result.
///
/// The result is an immutable view published only after validation of the exact
/// admitted query, bound observation and requested selection identities.
#[derive(Clone, Debug, Default)]
pub struct RelationLookupResult {
    publication: Option<Arc<RelationLookupPublication>>,
}

/// The validated publication payload.
#[derive(Debug)]
struct RelationLookupPublication {
    query: RelationQuery,
    rows: Vec<RelationLookupRecord>,
    selected: Vec<RelationLookupRecord>,
    missing: Vec<RelationSelection>,
    total: i64,
    revision: InputBytes,
}

impl RelationLookupResult {
    /// Whether this result carries a validated publication.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.publication
            .as_ref()
            .is_some_and(|publication| publication.query.is_valid())
    }

    /// The admitted query this result was published for.
    #[must_use]
    pub fn query(&self) -> RelationQuery {
        self.publication
            .as_ref()
            .map(|publication| publication.query.clone())
            .unwrap_or_default()
    }

    /// A detached copy of the page records.
    #[must_use]
    pub fn records(&self) -> Vec<RelationLookupRecord> {
        self.publication
            .as_ref()
            .map(|publication| publication.rows.clone())
            .unwrap_or_default()
    }

    /// A detached copy of the hydrated selected records.
    #[must_use]
    pub fn selected(&self) -> Vec<RelationLookupRecord> {
        self.publication
            .as_ref()
            .map(|publication| publication.selected.clone())
            .unwrap_or_default()
    }

    /// A detached copy of the selected targets whose labels are unavailable.
    #[must_use]
    pub fn missing(&self) -> Vec<RelationSelection> {
        self.publication
            .as_ref()
            .map(|publication| publication.missing.clone())
            .unwrap_or_default()
    }

    /// The total number of scoped records.
    #[must_use]
    pub fn total(&self) -> i64 {
        self.publication
            .as_ref()
            .map_or(0, |publication| publication.total)
    }

    /// The observation's source revision.
    #[must_use]
    pub fn revision(&self) -> InputBytes {
        self.publication
            .as_ref()
            .map_or_else(InputBytes::default, |publication| {
                publication.revision.clone()
            })
    }
}

/// A lookup that published records.
///
/// No `Default` is derived: a default value would let a consumer fabricate a
/// successful outcome with no validated publication behind it. The payload is private,
/// so only this owner's publication flow constructs one.
#[derive(Clone, Debug)]
pub struct RelationLookupFound {
    result: RelationLookupResult,
}

impl RelationLookupFound {
    /// The published result.
    #[must_use]
    pub fn result(&self) -> &RelationLookupResult {
        &self.result
    }
}

/// A lookup that published no records.
///
/// No `Default` is derived, for the same reason as [`RelationLookupFound`]: a
/// fabricated empty success would carry no validated publication.
#[derive(Clone, Debug)]
pub struct RelationLookupEmpty {
    result: RelationLookupResult,
}

impl RelationLookupEmpty {
    /// The published result.
    #[must_use]
    pub fn result(&self) -> &RelationLookupResult {
        &self.result
    }
}

/// A lookup the owner rejected.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RelationLookupRejected<E> {
    issues: Vec<String>,
    cause: Option<RelationLookupCause<E>>,
}

impl<E> RelationLookupRejected<E> {
    /// The rejection issues.
    #[must_use]
    pub fn issues(&self) -> Vec<String> {
        self.issues.clone()
    }

    /// The rejection cause.
    #[must_use]
    pub fn cause(&self) -> Option<&RelationLookupCause<E>> {
        self.cause.as_ref()
    }
}

/// A lookup the consumer reported as unavailable.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RelationLookupUnavailable<E> {
    cause: RelationLookupCause<E>,
}

impl<E> RelationLookupUnavailable<E> {
    /// The unavailability cause.
    #[must_use]
    pub fn cause(&self) -> &RelationLookupCause<E> {
        &self.cause
    }
}

/// A lookup that failed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RelationLookupFailed<E> {
    cause: RelationLookupCause<E>,
}

impl<E> RelationLookupFailed<E> {
    /// The failure cause.
    #[must_use]
    pub fn cause(&self) -> &RelationLookupCause<E> {
        &self.cause
    }
}

/// The closed relation lookup outcome.
pub enum RelationLookupOutcome<E> {
    /// Records were published.
    Found(RelationLookupFound),
    /// No records were published.
    Empty(RelationLookupEmpty),
    /// The owner rejected the lookup.
    Rejected(RelationLookupRejected<E>),
    /// The consumer reported lookup as unavailable.
    Unavailable(RelationLookupUnavailable<E>),
    /// The lookup failed.
    Failed(RelationLookupFailed<E>),
}

impl<E> RelationLookupOutcome<E> {
    /// The published result, for the publishing outcomes.
    #[must_use]
    pub fn result(&self) -> Option<&RelationLookupResult> {
        match self {
            RelationLookupOutcome::Found(found) => Some(found.result()),
            RelationLookupOutcome::Empty(empty) => Some(empty.result()),
            RelationLookupOutcome::Rejected(_)
            | RelationLookupOutcome::Unavailable(_)
            | RelationLookupOutcome::Failed(_) => None,
        }
    }

    /// The failure cause, for the non-publishing outcomes.
    #[must_use]
    pub fn cause(&self) -> Option<&RelationLookupCause<E>> {
        match self {
            RelationLookupOutcome::Found(_) | RelationLookupOutcome::Empty(_) => None,
            RelationLookupOutcome::Rejected(rejected) => rejected.cause(),
            RelationLookupOutcome::Unavailable(unavailable) => Some(unavailable.cause()),
            RelationLookupOutcome::Failed(failed) => Some(failed.cause()),
        }
    }
}

/// A lookup the owner refused with one issue.
fn rejected<E>(issue: &str) -> RelationLookupOutcome<E> {
    RelationLookupOutcome::Rejected(RelationLookupRejected {
        issues: vec![issue.to_owned()],
        cause: None,
    })
}

/// A lookup that failed for an owner contract reason.
fn failed_message<E>(message: &str) -> RelationLookupOutcome<E> {
    RelationLookupOutcome::Failed(RelationLookupFailed {
        cause: RelationLookupCause::Message(message.to_owned()),
    })
}

/// A lookup that failed because the request was cancelled.
fn failed_cancelled<E>() -> RelationLookupOutcome<E> {
    RelationLookupOutcome::Failed(RelationLookupFailed {
        cause: RelationLookupCause::Cancelled,
    })
}

/// A lookup that failed for the consumer's own reason.
fn failed_consumer<E>(cause: E) -> RelationLookupOutcome<E> {
    RelationLookupOutcome::Failed(RelationLookupFailed {
        cause: RelationLookupCause::Consumer(cause),
    })
}

/// One resolved lookup input, bound to the constructor-bound capabilities.
struct RelationResolution<E: Send + 'static, R: Send + Sync + 'static> {
    request: Request,
    descriptor: ResourceRelationDescriptor,
    admit: RelationScopeAdmissionFunc<E, R>,
    lookup: RelationLookupFunc<E, R>,
    options: RelationLookupOptions,
    source_id: InputBytes,
    revision: InputBytes,
    mode: RelationLookupMode,
    selected: Vec<RelationSelection>,
    decode_error: Option<RelationLookupDecodeError<E>>,
}

/// One admitted lookup query.
struct RelationAdmission<E: Send + 'static, R: Send + Sync + 'static> {
    request: Request,
    lookup: RelationLookupFunc<E, R>,
    query: RelationQuery,
}

/// One observed lookup ready for publication.
struct RelationObserved<E: Send + 'static, R: Send + Sync + 'static> {
    admission: RelationAdmission<E, R>,
    rows: Vec<RelationLookupRecord>,
    selected: Vec<RelationLookupRecord>,
    missing: Vec<RelationSelection>,
    total: i64,
    revision: InputBytes,
}

impl<E: Send + 'static, R: Send + Sync + 'static> Clone for RelationAdmission<E, R> {
    fn clone(&self) -> Self {
        Self {
            request: self.request.clone(),
            lookup: self.lookup.clone(),
            query: self.query.clone(),
        }
    }
}

impl<E, R> RelationLookupService<E, R>
where
    E: fmt::Display + Send + 'static,
    R: Send + Sync + 'static,
{
    /// Run the scoped lookup and publish its closed outcome.
    ///
    /// The returned effect is lazy. Consumer scope-admission and lookup failures
    /// arrive as expected failures on the capability boundary and are mapped into the
    /// rejected/failed outcomes with their consumer cause; defects,
    /// interruption and cleanup failures keep their own effect channels.
    /// The unconstructed default service repeatedly yields None without callbacks.
    pub fn lookup(&self, input: RelationLookupInput<E>) -> Effect<Option<RelationLookupOutcome<E>>, E, R> {
        if self.value.is_none(){return effect_sync!{body:||None};}
        let service = self.clone();
        effect! {
            let service = service.clone();
            let prepared = service.prepared.clone();
            // Resolution reads the request cancellation source and queries the
            // consumer's definition registry, so it runs as part of the description's
            // execution: a registry update between construction and running is seen,
            // and constructing the description performs no work.
            let (resolution, fields) = match prepare_lookup_fields(&service, &input, prepared.as_deref()) {
                Ok(fields) => (resolve_relation_lookup(&service, &input), fields),
                Err(outcome) => (Err(outcome), None),
            };
            let admitted = match resolution {
                Ok(resolution) => admit_relation_query(resolution, fields.as_ref(), prepared.as_deref())?,
                Err(outcome) => Err(outcome),
            };
            let observed = match admitted {
                Ok(admission) => execute_relation_lookup(admission)?,
                Err(outcome) => Err(outcome),
            };
            let outcome = match observed {
                Ok(observed) => publish_relation_lookup(observed, prepared.as_deref()),
                Err(outcome) => outcome,
            };
            drop(fields);
            Some(outcome)
        }
    }
}

fn prepare_lookup_fields<E: Send + 'static, R: Send + Sync + 'static>(
    service: &RelationLookupService<E, R>,
    input: &RelationLookupInput<E>,
    prepared: Option<&PreparedRelationLookup<E>>,
) -> Result<Option<PreparedMergedForm>, RelationLookupOutcome<E>> {
    let (Some(service), Some(request), Some(prepared)) =
        (service.value.as_ref(), input.request.as_ref(), prepared)
    else {
        return Ok(None);
    };
    if request.cancellation().is_requested() || input.decode_error.is_some() {
        return Ok(None);
    }
    let limits = prepared.limits;
    let fields = (|| {
        check_request_bytes(request, limits.request)?;
        if input.selected.len() > limits.selections.get()
            || service.options.default_page_size.unsigned_abs() > limits.page_size.get() as u64
        {
            return Err(ServerError::Memory(MemoryError::NoSlot));
        }
        let mut metadata = [
            input.source.as_str().len(), input.relation.len(), input.source_id.as_bytes().len(),
            input.revision.as_bytes().len(), input.mode.as_str().len(),
        ].into_iter().chain(input.selected.iter().map(|selection| selection.identity_ref().len()));
        let bytes = metadata.try_fold(Bytes::new(0), |bytes, size| bytes.checked_add(Bytes::new(size)))
            .map_err(ServerError::Memory)?;
        check_capacity(bytes.get(), limits.metadata.get())?;
        prepared.storage.read_query(request)
    })();
    fields.map(Some).map_err(|error| failed_consumer((prepared.on_refusal)(error)))
}

/// Resolve the input against the definitions and the constructor-bound capability.
fn resolve_relation_lookup<E: Send + 'static, R: Send + Sync + 'static>(
    service: &RelationLookupService<E, R>,
    input: &RelationLookupInput<E>,
) -> Result<RelationResolution<E, R>, RelationLookupOutcome<E>> {
    let Some(service)=service.value.as_ref() else{return Err(rejected("relation lookup service is not constructed"))};
    let Some(binding) = service.binding.value.as_ref() else {
        return Err(rejected(
            "relation lookup requires request, definitions and a bound target",
        ));
    };
    let Some(request) = input.request.clone() else {
        return Err(rejected(
            "relation lookup requires request, definitions and a bound target",
        ));
    };
    if request.cancellation().is_requested() {
        return Err(failed_cancelled());
    }
    let mut descriptor = None;
    service
        .resources
        .visit_definition(&input.source, &mut |definition| {
            descriptor = Some(resolve_resource_relation(definition, input.relation.as_str()));
        })
        .map_err(|error| rejected(&error.to_string()))?;
    let descriptor = match descriptor.ok_or_else(|| {
        rejected(&crate::definitions::UnknownResourceKind::new(input.source.clone()).to_string())
    })? {
        Ok(descriptor) => descriptor,
        Err(error) => return Err(rejected(&error.to_string())),
    };
    let relation_name = descriptor
        .definition_ref()
        .map_or("", |definition| definition.name.as_str());
    if binding.source != input.source || binding.relation != relation_name {
        return Err(rejected(
            "relation lookup does not match its constructor-bound source and relation",
        ));
    }
    Ok(RelationResolution {
        request,
        descriptor,
        admit: Arc::clone(&binding.admit),
        lookup: Arc::clone(&binding.lookup),
        options: service.options.clone(),
        source_id: input.source_id.clone(),
        revision: input.revision.clone(),
        mode: input.mode.clone(),
        selected: input.selected.clone(),
        decode_error: input.decode_error.clone(),
    })
}

/// Admit the lookup query and run the consumer's scope admission.
fn admit_relation_query<E, R>(
    resolution: RelationResolution<E, R>,
    fields: Option<&PreparedMergedForm>,
    prepared: Option<&PreparedRelationLookup<E>>,
) -> Effect<Result<RelationAdmission<E, R>, RelationLookupOutcome<E>>, E, R>
where
    E: fmt::Display + Send + 'static,
    R: Send + Sync + 'static,
{
    if !resolution.descriptor.is_valid() {
        return effect_sync! { body: move || {
            Err(rejected(
                "relation lookup resolution is missing or already consumed",
            ))
        }, };
    }
    if let Some(decode_error) = resolution.decode_error.clone() {
        return effect_sync! { once; body: move || {
            Err(RelationLookupOutcome::Rejected(RelationLookupRejected {
                issues: vec!["invalid selected-target transport".to_owned()],
                cause: Some(RelationLookupCause::Decode(decode_error)),
            }))
        }, };
    }
    let mode = resolution.mode.as_str();
    if mode != RELATION_LOOKUP_CHOICES && mode != RELATION_LOOKUP_INVERSE {
        return effect_sync! { body: move || {
            Err(rejected("relation lookup mode must be choices or inverse"))
        }, };
    }
    if mode == RELATION_LOOKUP_INVERSE && resolution.source_id.trim().is_empty() {
        return effect_sync! { body: move || {
            Err(rejected(
                "inverse lookup requires its source record identity",
            ))
        }, };
    }
    let request = resolution.request.clone();
    let page = match &resolution.options.parse_named_page {
        Some(parse) => parse(&request, PAGE_QUERY_PARAM),
        None => {
            let raw = fields.map_or_else(
                || request_query_value_untrimmed(&request, PAGE_QUERY_PARAM),
                |fields| fields.values(PAGE_QUERY_PARAM.as_bytes()).next()
                    .map(InputBytes::from).unwrap_or_default(),
            );
            if raw.is_empty() {
                1
            } else {
                match raw.to_str().ok().and_then(|raw| raw.parse::<i64>().ok()) {
                    Some(page) => page,
                    None => {
                        return effect_sync! { body: move || {
                            Err(rejected("relation lookup page must be an integer"))
                        }, };
                    }
                }
            }
        }
    };
    let page_size = resolution.options.default_page_size;
    if page <= 0 || page_size <= 0 {
        return effect_sync! { body: move || Err(rejected("relation lookup pagination must be positive")), };
    }
    if page - 1 > i64::MAX / page_size {
        return effect_sync! { body: move || {
            Err(rejected(
                "relation lookup page offset exceeds the integer representation",
            ))
        }, };
    }
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    for selection in &resolution.selected {
        if !selection.matches(&resolution.descriptor) || !seen.insert(selection.identity_ref()) {
            return effect_sync! { body: move || {
                Err(rejected(
                    "selected target is invalid, duplicated or belongs to a different relation declaration",
                ))
            }, };
        }
    }
    drop(seen);
    let locale = match &resolution.options.request_language {
        Some(resolve) => {
            let locale = resolve(&request);
            let locale = crate::validation::trim_unicode_whitespace(locale.as_bytes());
            if let Some(prepared) = prepared
                && let Err(error) = check_capacity(locale.len(), prepared.limits.metadata.get())
            {
                let outcome = failed_consumer((prepared.on_refusal)(error));
                return effect_sync! { once; body: move || Err(outcome), };
            }
            InputBytes::from(locale)
        }
        None => InputBytes::default(),
    };
    let search = fields.map_or_else(
        || request_query_value(&request, SEARCH_QUERY_PARAM),
        |fields| QueryValuesRef::Prepared(fields).trimmed(SEARCH_QUERY_PARAM),
    );
    let query = RelationQuery::admit(
        resolution.descriptor,
        resolution.source_id,
        resolution.revision,
        search,
        locale,
        resolution.mode,
        page,
        page_size,
        resolution.selected,
    );
    if request.cancellation().is_requested() {
        return effect_sync! { body: move || Err(failed_cancelled()), };
    }
    let admission = RelationAdmission { request: resolution.request, lookup: resolution.lookup, query };
    let admit = resolution.admit;
    let callback_query = admission.query.clone();
    let check = admission.request.clone();
    effect! {
        let admission = admission.clone();
        let check = check.clone();
        let attempted = effect! { either; program: admit(&callback_query), }?;
        match attempted {
            Ok(()) => {
                if check.cancellation().is_requested() {
                    Err(failed_cancelled())
                } else {
                    Ok(admission)
                }
            }
            Err(error) => {
                if check.cancellation().is_requested() {
                    Err(failed_cancelled())
                } else {
                    let issue = format!("relation lookup scope rejected: {error}");
                    Err(RelationLookupOutcome::Rejected(RelationLookupRejected {
                        issues: vec![issue],
                        cause: Some(RelationLookupCause::Consumer(error)),
                    }))
                }
            }
        }
    }
}

/// Execute the scoped lookup and validate its observation against the admission.
fn execute_relation_lookup<E, R>(
    admission: RelationAdmission<E, R>,
) -> Effect<Result<RelationObserved<E, R>, RelationLookupOutcome<E>>, E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    if admission.request.cancellation().is_requested() {
        return effect_sync! { body: move || Err(failed_cancelled()), };
    }
    let lookup = Arc::clone(&admission.lookup);
    let callback_query = admission.query.clone();
    let request = admission.request.clone();
    effect! {
        let admission = admission.clone();
        let request = request.clone();
        let attempted = effect! { either; program: lookup(&callback_query), }?;
        match attempted {
            Ok(observation) => {
                if request.cancellation().is_requested() {
                    Err(failed_cancelled())
                } else {
                    finish_observation(admission, observation)
                }
            }
            Err(error) => {
                if request.cancellation().is_requested() {
                    Err(failed_cancelled())
                } else {
                    Err(failed_consumer(error))
                }
            }
        }
    }
}

/// Split one observation into records or a stop outcome.
fn finish_observation<E: Send + 'static, R: Send + Sync + 'static>(
    admission: RelationAdmission<E, R>,
    observation: RelationLookupObservation<E>,
) -> Result<RelationObserved<E, R>, RelationLookupOutcome<E>> {
    match observation.into_value() {
        None => Err(failed_message(
            "relation lookup returned no observation or a different query's observation",
        )),
        Some(RelationObservationValue::Records {
            query,
            rows,
            selected,
            missing,
            total,
            revision,
        }) => {
            if !admission.query.same_inner(&query) {
                return Err(failed_message(
                    "relation lookup returned no observation or a different query's observation",
                ));
            }
            Ok(RelationObserved {
                admission,
                rows,
                selected,
                missing,
                total,
                revision,
            })
        }
        Some(RelationObservationValue::Unavailable { query, cause }) => {
            if !admission.query.same_inner(&query) {
                return Err(failed_message(
                    "relation lookup returned no observation or a different query's observation",
                ));
            }
            Err(RelationLookupOutcome::Unavailable(
                RelationLookupUnavailable { cause },
            ))
        }
        Some(RelationObservationValue::Failed { query, cause }) => {
            if !admission.query.same_inner(&query) {
                return Err(failed_message(
                    "relation lookup returned no observation or a different query's observation",
                ));
            }
            Err(RelationLookupOutcome::Failed(RelationLookupFailed {
                cause,
            }))
        }
    }
}

/// Publish the validated observation as a closed outcome.
fn publish_relation_lookup<E: Send + 'static, R: Send + Sync + 'static>(
    observed: RelationObserved<E, R>,
    prepared: Option<&PreparedRelationLookup<E>>,
) -> RelationLookupOutcome<E> {
    let RelationObserved {
        admission,
        rows,
        selected,
        missing,
        total,
        revision,
    } = observed;
    let query = admission.query;
    let Some(query_arc) = query.inner().cloned() else {
        return failed_message("relation lookup publication does not match its admitted query");
    };
    let descriptor = query_arc.descriptor.clone();
    if !descriptor.is_valid() {
        return failed_message("relation lookup publication does not match its admitted query");
    }
    if admission.request.cancellation().is_requested() {
        return failed_cancelled();
    }
    let row_count = i64::try_from(rows.len()).unwrap_or(i64::MAX);
    if total < row_count || row_count > query_arc.page_size {
        return failed_message("relation lookup returned inconsistent page or total");
    }
    if !query_arc.revision.is_empty() && revision != query_arc.revision {
        return failed_message("relation lookup source revision changed");
    }
    if let Some(prepared) = prepared
        && let Err(error) = check_published_relation(&rows, &selected, &missing, &revision, prepared.limits)
    {
        return failed_consumer((prepared.on_refusal)(error));
    }
    if let Err(message) = validate_relation_records(&descriptor, &rows) {
        return failed_message(message);
    }
    if let Err(message) = validate_relation_records(&descriptor, &selected) {
        return failed_message(message);
    }
    let requested: BTreeSet<&str> = query_arc
        .selected
        .iter()
        .map(RelationSelection::identity_ref)
        .collect();
    let mut accounted: BTreeSet<&str> = BTreeSet::new();
    for record in &selected {
        let identity = record.selection_ref().map_or("", RelationSelection::identity_ref);
        if !requested.contains(identity) || !accounted.insert(identity) {
            return failed_message("relation lookup hydrated an unrequested or repeated selection");
        }
    }
    for selection in &missing {
        let identity = selection.identity_ref();
        if !selection.belongs_to(&descriptor)
            || !requested.contains(identity)
            || !accounted.insert(identity)
        {
            return failed_message(
                "relation lookup returned an invalid, repeated or overlapping missing selection",
            );
        }
        for record in &rows {
            if record.selection_ref().is_some_and(|selection| selection.identity_ref() == identity) {
                return failed_message("relation lookup marked a visible page record unavailable");
            }
        }
    }
    if accounted.len() != requested.len() {
        return failed_message("relation lookup omitted requested selection availability");
    }
    drop(accounted);
    drop(requested);
    let has_records = !rows.is_empty() || !selected.is_empty();
    let result = RelationLookupResult {
        publication: Some(Arc::new(RelationLookupPublication {
            query,
            rows,
            selected,
            missing,
            total,
            revision,
        })),
    };
    if has_records {
        RelationLookupOutcome::Found(RelationLookupFound { result })
    } else {
        RelationLookupOutcome::Empty(RelationLookupEmpty { result })
    }
}

fn check_published_relation(
    rows: &[RelationLookupRecord],
    selected: &[RelationLookupRecord],
    missing: &[RelationSelection],
    revision: &InputBytes,
    limits: PreparedRelationLimits,
) -> Result<(), ServerError> {
    let fanout = selected.len().checked_add(missing.len())
        .ok_or(ServerError::Memory(MemoryError::ArithmeticOverflow))?;
    if fanout > limits.selections.get() {
        return Err(ServerError::Memory(MemoryError::NoSlot));
    }
    let mut sizes = rows.iter().chain(selected).filter_map(|record| record.value.as_ref())
        .flat_map(|record| [record.selection.identity_ref().len(), record.label.len(), record.href.len()])
        .chain(missing.iter().map(|selection| selection.identity_ref().len()));
    let bytes = sizes.try_fold(Bytes::new(revision.as_bytes().len()), |bytes, size| bytes.checked_add(Bytes::new(size)))
        .map_err(ServerError::Memory)?;
    check_capacity(bytes.get(), limits.metadata.get())
}

/// Validate one record collection against the admitted declaration.
fn validate_relation_records(
    descriptor: &ResourceRelationDescriptor,
    records: &[RelationLookupRecord],
) -> Result<(), &'static str> {
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    for record in records {
        if !record.selection_ref().is_some_and(|selection| {
            selection.belongs_to(descriptor) && seen.insert(selection.identity_ref())
        }) {
            return Err(
                "relation lookup returned an invalid, repeated or mismatched target record",
            );
        }
    }
    Ok(())
}

/// Structural equality of two detached relation declarations.
fn same_relation_definition(
    left: &ResourceRelationDefinition,
    right: &ResourceRelationDefinition,
) -> bool {
    if left.name != right.name
        || left.cardinality != right.cardinality
        || left.inverse_of != right.inverse_of
        || left.on_target_delete != right.on_target_delete
        || left.query_binding != right.query_binding
        || left.scope != right.scope
        || left.target != right.target
        || left.discriminator != right.discriminator
        || left.optional != right.optional
        || left.owned != right.owned
        || left.through != right.through
        || left.operations != right.operations
    {
        return false;
    }
    if left.targets.len() != right.targets.len() {
        return false;
    }
    for (candidate, declared) in left.targets.iter().zip(right.targets.iter()) {
        if candidate.scope != declared.scope
            || candidate.tag != declared.tag
            || candidate.target != declared.target
            || candidate.keys.len() != declared.keys.len()
        {
            return false;
        }
        for (key, other) in candidate.keys.iter().zip(declared.keys.iter()) {
            if key.source != other.source
                || key.target != other.target
                || key.r#type != other.r#type
            {
                return false;
            }
        }
    }
    if left.keys.len() != right.keys.len() {
        return false;
    }
    for (key, other) in left.keys.iter().zip(right.keys.iter()) {
        if key.source != other.source || key.target != other.target || key.r#type != other.r#type {
            return false;
        }
    }
    let editor = &left.editor;
    let other = &right.editor;
    editor.control == other.control
        && editor.label_query == other.label_query
        && editor.locale_key == other.locale_key
        && editor.save_mode == other.save_mode
        && editor.removal_operation == other.removal_operation
        && editor.label_field == other.label_field
        && editor.order_field == other.order_field
        && editor.columns == other.columns
        && editor.minimum == other.minimum
        && editor.maximum == other.maximum
}
