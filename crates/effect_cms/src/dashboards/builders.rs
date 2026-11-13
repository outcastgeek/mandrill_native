use std::sync::Arc;

use effect::{Effect, effect_sync};
use crate::definitions::{Registry, ResourceDashboardDefinition, ResourceKind, ResourceSectionKey, SharedList, SharedMap, find_resource_definition};
use crate::localization::TranslateKeyFunc;
use crate::rendering::{ListPaginationVM, ResourceListItemVM};
use super::{ActionVM, ResourceDashboardMetricVM, ResourceDashboardSectionVM};

#[derive(Clone, Debug, Default)]
pub struct ResourceDashboardSectionState {
    pub title: String,
    pub description: String,
    pub metrics: Option<SharedList<ResourceDashboardMetricVM>>,
    pub pagination: ListPaginationVM,
    pub items: Option<SharedList<ResourceListItemVM>>,
    pub actions: Option<SharedList<ActionVM>>,
}

pub type ResourceDashboardState = Option<SharedMap<ResourceSectionKey, ResourceDashboardSectionState>>;

fn sections<C>(context: &C, definition: &ResourceDashboardDefinition, state: &ResourceDashboardState, translate: Option<&TranslateKeyFunc<C>>) -> Option<SharedList<ResourceDashboardSectionVM>> {
    if definition.sections.is_empty() { return None; }
    let mut sections = Vec::new();
    for index in 0..definition.sections.len() {
        let Some(def) = definition.sections.with(|values| values.get(index).cloned()) else { continue };
        let value = state.as_ref().and_then(|state| state.get(&def.key));
        let exists = value.is_some();
        let value = value.unwrap_or_default();
        let text = |key: &str| translate.map_or_else(|| key.trim().to_owned(), |translate| translate(context, key));
        let title = crate::queries::first_non_blank(&[value.title, text(&def.title_key)]);
        let description = crate::queries::first_non_blank(&[value.description, text(&def.description_key)]);
        let section = ResourceDashboardSectionVM { key: def.key, title, description, metrics: value.metrics, pagination: value.pagination, items: value.items, actions: value.actions };
        if exists || section.metrics.as_ref().is_some_and(|v| !v.is_empty()) || section.items.as_ref().is_some_and(|v| !v.is_empty()) || section.actions.as_ref().is_some_and(|v| !v.is_empty()) { sections.push(section); }
    }
    Some(SharedList::new(sections))
}

/// Build sections from a declaration and supplied state.
pub fn build_resource_dashboard_sections<C: Send + Sync + 'static, E: Send + 'static, R: Send + Sync + 'static>(context: Arc<C>, definition: ResourceDashboardDefinition, state: ResourceDashboardState, translate: Option<TranslateKeyFunc<C>>) -> Effect<Option<SharedList<ResourceDashboardSectionVM>>, E, R> {
    effect_sync! { body: move || sections(context.as_ref(), &definition, &state, translate.as_ref()) }
}

/// Build sections from a registered declaration.
pub fn build_resource_dashboard_sections_for_kind<C: Send + Sync + 'static, E: Send + 'static, R: Send + Sync + 'static>(context: Arc<C>, resources: Arc<dyn Registry + Send + Sync>, kind: ResourceKind, state: ResourceDashboardState, translate: Option<TranslateKeyFunc<C>>) -> Effect<Option<SharedList<ResourceDashboardSectionVM>>, E, R> {
    effect_sync! { body: move || find_resource_definition(Some(resources.as_ref()), &kind).and_then(|definition| sections(context.as_ref(), &definition.dashboard, &state, translate.as_ref())) }
}
