//! Validation path bindings and issue application.
//!
//! A consumer supplies path bindings that map a validated path (or a path segment) to the form
//! field identity and message key the presentation layer renders, and the owner
//! applies the best matching binding to an issue.
//!
//! # Matching rules
//!
//! - The lookup path is the issue path, falling back to the issue field.
//! - A binding whose resource or action is blank matches every resource or action;
//!   a non-blank one matches only its own.
//! - A binding path that equals the issue path scores `500 + length`; a binding path
//!   that prefixes the issue path as a dotted or underscored segment scores its
//!   length. An exact resource match adds `10000` and an exact action match adds
//!   `1000`, and the highest score wins; the first winner wins ties under a strict
//!   `score > best` comparison.
//! - Applying a binding replaces only non-blank members, and the message key is
//!   taken only when the issue has none.

use crate::definitions::{ResourceAction, ResourceKind};

use super::issues::{ValidationIssue, normalize};

/// One supplied path binding.
#[derive(Clone, Debug, Default)]
pub struct Binding {
    /// Resource kind this binding belongs to; blank matches every kind.
    pub resource: ResourceKind,
    /// Action this binding belongs to; blank matches every action.
    pub action: ResourceAction,
    /// Dotted path this binding covers.
    pub path: String,
    /// Field name to report.
    pub field: String,
    /// Input id to report.
    pub field_id: String,
    /// Field key to report.
    pub field_key: String,
    /// Section key to report.
    pub section_key: String,
    /// Message key to report.
    pub message_key: String,
}

/// The consumer-supplied binding catalog.
#[derive(Clone, Debug, Default)]
pub struct Catalog {
    /// Supplied bindings.
    pub bindings: Vec<Binding>,
}

/// The lookup subject for one catalog application.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct LookupOptions {
    /// Resource kind being rendered.
    pub resource: ResourceKind,
    /// Action being rendered.
    pub action: ResourceAction,
}

impl Catalog {
    /// The best binding for one issue, if the catalog supplies one.
    #[must_use]
    pub fn lookup(&self, issue: &ValidationIssue, options: &LookupOptions) -> Option<Binding> {
        let issue = normalize(issue.clone());
        let path = if issue.path.is_empty() {
            issue.field.trim().to_owned()
        } else {
            issue.path.trim().to_owned()
        };
        if path.is_empty() {
            return None;
        }
        let mut best: Option<(Binding, i64)> = None;
        for candidate in &self.bindings {
            let candidate = normalize_binding(candidate);
            if candidate.path.is_empty()
                || !resource_matches(&candidate.resource, &options.resource)
                || !action_matches(&candidate.action, &options.action)
            {
                continue;
            }
            let Some(path_score) = path_score(&candidate.path, &path) else {
                continue;
            };
            let Ok(mut score) = i64::try_from(path_score) else {
                continue;
            };
            if candidate.resource == options.resource {
                score += 10_000;
            }
            if candidate.action == options.action {
                score += 1_000;
            }
            let better = match &best {
                Some((_binding, best_score)) => score > *best_score,
                None => true,
            };
            if better {
                best = Some((candidate, score));
            }
        }
        best.map(|(binding, _score)| binding)
    }

    /// Apply the best binding to one issue.
    #[must_use]
    pub fn apply(&self, issue: ValidationIssue, options: &LookupOptions) -> ValidationIssue {
        let issue = normalize(issue);
        let Some(binding) = self.lookup(&issue, options) else {
            return issue;
        };
        let mut issue = issue;
        if !binding.field.is_empty() {
            issue.field = binding.field;
        }
        if !binding.field_id.is_empty() {
            issue.field_id = binding.field_id;
        }
        if !binding.field_key.is_empty() {
            issue.field_key = binding.field_key;
        }
        if !binding.section_key.is_empty() {
            issue.section_key = binding.section_key;
        }
        if !binding.message_key.is_empty() && issue.message_key.is_empty() {
            issue.message_key = binding.message_key;
        }
        normalize(issue)
    }

    /// Apply the best binding to every issue.
    #[must_use]
    pub fn apply_all(
        &self,
        source: &[ValidationIssue],
        options: &LookupOptions,
    ) -> Vec<ValidationIssue> {
        source
            .iter()
            .cloned()
            .map(|issue| self.apply(issue, options))
            .collect()
    }
}

/// Trim a binding's members and fill its defaulted identity.
fn normalize_binding(binding: &Binding) -> Binding {
    let mut binding = binding.clone();
    binding.path = binding.path.trim().trim_matches('.').to_owned();
    binding.field = binding.field.trim().to_owned();
    binding.field_id = binding.field_id.trim().to_owned();
    binding.field_key = binding.field_key.trim().to_owned();
    binding.section_key = binding.section_key.trim().to_owned();
    binding.message_key = binding.message_key.trim().to_owned();
    if binding.field.is_empty() {
        binding.field = binding.field_key.clone();
    }
    if binding.field_id.is_empty() {
        binding.field_id = binding.field.clone();
    }
    binding
}

/// Whether a binding's resource selector admits the lookup resource.
fn resource_matches(binding: &ResourceKind, target: &ResourceKind) -> bool {
    binding.as_str().is_empty() || binding == target
}

/// Whether a binding's action selector admits the lookup action.
fn action_matches(binding: &ResourceAction, target: &ResourceAction) -> bool {
    binding.as_str().is_empty() || binding == target
}

/// The binding-path score, absent when the binding does not cover the
/// issue path.
fn path_score(binding_path: &str, issue_path: &str) -> Option<usize> {
    let binding_path = binding_path.trim().trim_matches('.');
    let issue_path = issue_path.trim().trim_matches('.');
    if binding_path.is_empty() || issue_path.is_empty() {
        return None;
    }
    if binding_path == issue_path {
        return Some(500 + binding_path.len());
    }
    if issue_path.starts_with(&format!("{binding_path}."))
        || issue_path.starts_with(&format!("{binding_path}_"))
    {
        return Some(binding_path.len());
    }
    None
}
