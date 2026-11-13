//! The W5 form mode decision and the form/submit action filters.
//!
//! These are pure functions over declared values:
//!
//! - a mode maps to its action: `new` to create, `edit` to update, and every other
//!   mode to create while keeping the requested mode;
//! - the publication actions (`publish`, `draft`) are hidden unless the definition
//!   declares publication actions, and `delete` is hidden on the new surface;
//! - submit actions are filtered by surface: the new surface drops update/delete and
//!   the edit surface drops create, while every other action is kept in its original
//!   declaration order.

use crate::commands::SubmitActionVM;
use crate::definitions::{
    RESOURCE_ACTION_CREATE, RESOURCE_ACTION_DELETE, RESOURCE_ACTION_DRAFT,
    RESOURCE_ACTION_PUBLISH, RESOURCE_ACTION_UPDATE, RESOURCE_VIEW_MODE_EDIT,
    RESOURCE_VIEW_MODE_NEW, ResourceAction, ResourceViewMode,
};

/// The pure mode decision.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ModeDecision {
    /// The requested surface.
    pub mode: ResourceViewMode,
    /// The action the surface works on.
    pub action: ResourceAction,
}

/// Map the requested mode onto its action.
#[must_use]
pub fn decide_mode(mode: &ResourceViewMode) -> ModeDecision {
    match mode.as_str() {
        RESOURCE_VIEW_MODE_NEW => ModeDecision {
            mode: ResourceViewMode::new(RESOURCE_VIEW_MODE_NEW),
            action: ResourceAction::new(RESOURCE_ACTION_CREATE),
        },
        RESOURCE_VIEW_MODE_EDIT => ModeDecision {
            mode: ResourceViewMode::new(RESOURCE_VIEW_MODE_EDIT),
            action: ResourceAction::new(RESOURCE_ACTION_UPDATE),
        },
        _ => ModeDecision {
            mode: mode.clone(),
            action: ResourceAction::new(RESOURCE_ACTION_CREATE),
        },
    }
}

/// One action offered by a form.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Action {
    /// The offered action.
    pub action: ResourceAction,
    /// Its label.
    pub label: String,
}

/// Keep the actions permitted in the given surface.
///
/// An empty action list stays empty; otherwise each action is kept in declaration
/// order unless it is hidden on the surface.
#[must_use]
pub fn filter_form_actions(
    actions: Vec<Action>,
    mode: &ResourceViewMode,
    has_publication_actions: bool,
) -> Vec<Action> {
    if actions.is_empty() {
        return actions;
    }
    actions
        .into_iter()
        .filter(|action| !hidden_action(&action.action, mode, has_publication_actions))
        .collect()
}

/// Report whether the action is hidden on the surface.
#[must_use]
pub fn hidden_action(
    action: &ResourceAction,
    mode: &ResourceViewMode,
    has_publication_actions: bool,
) -> bool {
    match action.as_str() {
        RESOURCE_ACTION_PUBLISH | RESOURCE_ACTION_DRAFT => !has_publication_actions,
        RESOURCE_ACTION_DELETE => mode.as_str() == RESOURCE_VIEW_MODE_NEW,
        _ => false,
    }
}

/// Filter the submit actions offered on one surface.
///
/// The new surface excludes update and delete, the edit surface excludes create, and
/// the surviving actions keep their declaration order.
#[must_use]
pub fn filter_submit_actions(
    actions: Vec<SubmitActionVM>,
    mode: &ResourceViewMode,
) -> Vec<SubmitActionVM> {
    let mut filtered = Vec::with_capacity(actions.len());
    for action in actions {
        let excluded = match mode.as_str() {
            RESOURCE_VIEW_MODE_NEW => {
                action.action.as_str() == RESOURCE_ACTION_UPDATE
                    || action.action.as_str() == RESOURCE_ACTION_DELETE
            }
            RESOURCE_VIEW_MODE_EDIT => action.action.as_str() == RESOURCE_ACTION_CREATE,
            _ => false,
        };
        if !excluded {
            filtered.push(action);
        }
    }
    filtered
}
