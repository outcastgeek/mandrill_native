use crate::definitions::DashboardActionLocation;

use super::ActionVM;

/// Map the valid actions at a location.
#[must_use]
pub fn map_dashboard_actions<T>(
    actions: &[ActionVM],
    location: &DashboardActionLocation,
    map: Option<impl FnMut(ActionVM) -> T>,
) -> Option<Vec<T>> {
    let mut map = map?;
    let mapped: Vec<T> = actions.iter()
        .filter(|action| &action.location == location && !action.label.trim().is_empty() && !action.href.trim().is_empty())
        .cloned()
        .map(&mut map)
        .collect();
    if mapped.is_empty() { None } else { Some(mapped) }
}

/// Copy the valid actions at a location.
#[must_use]
pub fn dashboard_actions_for_location(
    actions: &[ActionVM],
    location: &DashboardActionLocation,
) -> Option<Vec<ActionVM>> {
    map_dashboard_actions(actions, location, Some(std::convert::identity))
}

/// Find the first valid action at a location.
#[must_use]
pub fn first_dashboard_action(
    actions: &[ActionVM],
    location: &DashboardActionLocation,
) -> Option<ActionVM> {
    actions.iter().find(|action| &action.location == location && !action.label.trim().is_empty() && !action.href.trim().is_empty()).cloned()
}
