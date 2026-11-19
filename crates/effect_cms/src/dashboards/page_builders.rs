use std::sync::Arc;

use effect::{Effect, effect, effect_sync};
use effect_server::Request;
use crate::definitions::{ResourceKind, SharedList};
use super::{ResourceDashboardPageVM, ResourceDashboardSectionVM};

#[derive(Clone, Debug, Default)]
pub struct ResourceDashboardPageState {
    pub sections: Option<SharedList<ResourceDashboardSectionVM>>,
}

pub type ResourceDashboardPageStateBuilder = Arc<dyn Fn(ResourceDashboardPageVM) -> ResourceDashboardPageState + Send + Sync + 'static>;
pub type DashboardPageSource<C, E, R> = Arc<dyn Fn(&C, Option<&Request>, &ResourceKind) -> Effect<Option<ResourceDashboardPageVM>, E, R> + Send + Sync + 'static>;

fn detached_sections(state: ResourceDashboardPageState) -> Option<SharedList<ResourceDashboardSectionVM>> {
    state.sections.and_then(|sections| {
        let values = sections.to_vec();
        if values.is_empty() { None } else { Some(SharedList::new(values)) }
    })
}

/// Build a dashboard page with a supplied state.
pub fn build_resource_dashboard_page<C: Send + Sync + 'static, E: Send + 'static, R: Send + Sync + 'static>(context: Arc<C>, request: Option<Request>, kind: ResourceKind, state: ResourceDashboardPageState, source: Option<DashboardPageSource<C, E, R>>) -> Effect<ResourceDashboardPageVM, E, R> {
    let build: ResourceDashboardPageStateBuilder = Arc::new(move |_| state.clone());
    build_resource_dashboard_page_with_state(context, request, kind, source, Some(build))
}

/// Build a dashboard page, deriving state from the resolved page.
pub fn build_resource_dashboard_page_with_state<C: Send + Sync + 'static, E: Send + 'static, R: Send + Sync + 'static>(context: Arc<C>, request: Option<Request>, kind: ResourceKind, source: Option<DashboardPageSource<C, E, R>>, build_state: Option<ResourceDashboardPageStateBuilder>) -> Effect<ResourceDashboardPageVM, E, R> {
    effect! {
        let context = context.clone();
        let request = request.clone();
        let kind = kind.clone();
        let source = source.clone();
        let build_state = build_state.clone();
        let mut page = match source {
            Some(source) => match effect! { either; program: source(&context, request.as_ref(), &kind), }? {
                Ok(Some(page)) => page,
                Ok(None) | Err(_) => ResourceDashboardPageVM::default(),
            },
            None => ResourceDashboardPageVM::default(),
        };
        let state_page = page.clone();
        let state = effect_sync! { body: move || build_state.as_ref().map_or_else(ResourceDashboardPageState::default, |build| build(state_page.clone())) }?;
        page.sections = detached_sections(state);
        page
    }
}
