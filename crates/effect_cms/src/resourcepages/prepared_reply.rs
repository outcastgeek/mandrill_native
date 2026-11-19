//! Prepared publication of native Maud resource pages.

use std::num::NonZeroUsize;
use std::sync::Arc;

use effect::memory::{Buffer, BufferLease, BufferView, Bytes, MemoryError};
use effect::{Effect, effect_map_error};
use effect_server::{HasRequest, Reply, Request, ServerError, server};
use maud::Markup;

use crate::definitions::ResourceKind;
use crate::definitions::SharedList;
use crate::rendering::{RenderingError, ResourceListItemVM};
use crate::{DefinitionServiceSet, PreparedDefinitionServices};

use super::{
    ResourceDashboardPageVM, ResourceDetailPageVM, ResourceFieldVM, ResourceFormPageVM,
    ResourceListPageVM, ResourcePageRenderer, ResourceRenderError,
};

/// A caller-owned resource-page value borrowed by the renderer on each invocation.
#[derive(Clone)]
pub enum ResourcePageModel {
    Form(Arc<ResourceFormPageVM>),
    List(Arc<ResourceListPageVM>),
    Detail(Arc<ResourceDetailPageVM>),
    Dashboard(Arc<ResourceDashboardPageVM>),
}

impl ResourcePageModel {
    fn view(&self) -> ResourcePageRef<'_> {
        match self {
            Self::Form(page) => ResourcePageRef::Form(page),
            Self::List(page) => ResourcePageRef::List(page),
            Self::Detail(page) => ResourcePageRef::Detail(page),
            Self::Dashboard(page) => ResourcePageRef::Dashboard(page),
        }
    }
}

enum ResourcePageRef<'a> {
    Form(&'a ResourceFormPageVM),
    List(&'a ResourceListPageVM),
    Detail(&'a ResourceDetailPageVM),
    Dashboard(&'a ResourceDashboardPageVM),
}

impl ResourcePageRef<'_> {
    fn check_model(&self, limits: ResourcePageRenderLimits) -> Result<(), MemoryError> {
        let mut budget = ModelBudget {
            records: limits.model_records.get(),
        };
        match self {
            Self::Form(page) => {
                budget.count(page.actions.len())?;
                budget.count(page.widgets.len())?;
                budget.count(page.status.issues.len())?;
                budget.count(page.sections.len())?;
                for section in &page.sections {
                    budget.fields(&section.fields)?;
                }
            }
            Self::List(page) => {
                budget.count(page.filters.len())?;
                budget.count(page.active_filter_chips.len())?;
                budget.count(page.filter_state.options.len())?;
                budget.shared(page.pagination.links.as_ref())?;
                budget.count(page.sections.len())?;
                for section in &page.sections {
                    budget.items(&section.items, None)?;
                }
            }
            Self::Detail(page) => {
                budget.count(page.sections.len())?;
                for section in &page.sections {
                    budget.fields(&section.fields)?;
                    budget.items(&section.items, Some(limits.detail_depth))?;
                }
            }
            Self::Dashboard(page) => {
                budget.shared(page.actions.as_ref())?;
                if let Some(sections) = &page.sections {
                    sections.try_with(|sections| {
                        budget.count(sections.len())?;
                        for section in sections {
                            budget.shared(section.metrics.as_ref())?;
                            budget.shared(section.actions.as_ref())?;
                            budget.shared(section.pagination.links.as_ref())?;
                            if let Some(items) = &section.items {
                                items.try_with(|items| budget.items(items, None))??;
                            }
                        }
                        Ok(())
                    })??;
                }
            }
        }
        Ok(())
    }
}

/// Query and page producer used by a prepared definition reply.
#[derive(Clone, Copy)]
pub enum ResourcePageSurface {
    List,
    Detail,
    Dashboard,
}

/// Reply from a definition page, carrying its query/render service failures.
pub type DefinitionPageReply<E, R> = Effect<Reply<RenderingError<E>, R>, RenderingError<E>, R>;

struct ModelBudget {
    records: usize,
}

impl ModelBudget {
    fn count(&mut self, count: usize) -> Result<(), MemoryError> {
        self.records = self.records.checked_sub(count).ok_or(MemoryError::NoSlot)?;
        Ok(())
    }

    fn shared<T>(&mut self, source: Option<&SharedList<T>>) -> Result<(), MemoryError> {
        self.count(source.map_or(0, SharedList::len))
    }

    fn fields(&mut self, fields: &[ResourceFieldVM]) -> Result<(), MemoryError> {
        self.count(fields.len())?;
        for field in fields {
            self.count(field.attrs.len())?;
            self.count(field.errors.len())?;
            self.count(field.options.len())?;
            if let Some(tabs) = &field.localized_tabs {
                self.count(tabs.tabs.len())?;
                for tab in &tabs.tabs {
                    self.count(tab.errors.len())?;
                }
            }
            if let Some(relation) = &field.relation.value {
                self.count(relation.options.len())?;
                self.count(relation.actions.len())?;
                self.count(relation.issues.len())?;
                self.count(relation.rejected_selections.len())?;
                self.count(relation.rows.len())?;
                for row in &relation.rows {
                    if let Some(row) = &row.value {
                        self.count(row.values.len())?;
                        self.count(row.actions.len())?;
                        self.count(row.issues.len())?;
                        self.count(row.rejected_selections.len())?;
                    }
                }
            }
        }
        Ok(())
    }

    fn items(
        &mut self,
        items: &[ResourceListItemVM],
        detail_depth: Option<u8>,
    ) -> Result<(), MemoryError> {
        self.count(items.len())?;
        for item in items {
            self.shared(item.meta.as_ref())?;
            self.shared(item.actions.as_ref())?;
            if let Some(depth) = detail_depth
                && let Some(children) = item.children.as_ref().filter(|list| !list.is_empty())
            {
                let remaining = depth.checked_sub(1).ok_or(MemoryError::NoSlot)?;
                children.try_with(|items| self.items(items, Some(remaining)))??;
            }
        }
        Ok(())
    }
}

/// Failure to admit, render or publish a resource page.
#[derive(Debug)]
pub enum PreparedResourceReplyError {
    Request(ServerError),
    Render(ResourceRenderError),
    Output(MemoryError),
    /// The current resource page cannot be inspected within its admitted limits.
    Model(MemoryError),
}

/// Resource page cardinality, depth and simultaneous native/response byte bounds.
#[derive(Clone, Copy)]
pub struct ResourcePageRenderLimits {
    /// Combined section, row, action, field, option and widget records.
    pub model_records: NonZeroUsize,
    /// Maximum nested detail-item depth, with zero allowing only top-level items.
    pub detail_depth: u8,
    /// The full response slot plus the returned native Maud string capacity.
    pub native_and_response_bytes: Bytes,
}

impl std::fmt::Display for PreparedResourceReplyError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Request(error) => write!(formatter, "{error}"),
            Self::Render(error) => write!(formatter, "{error}"),
            Self::Output(error) => write!(formatter, "{error}"),
            Self::Model(error) => write!(formatter, "{error}"),
        }
    }
}

impl std::error::Error for PreparedResourceReplyError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Request(error) => Some(error),
            Self::Render(error) => Some(error),
            Self::Output(error) | Self::Model(error) => Some(error),
        }
    }
}

/// Append native Maud markup to a fixed buffer under a combined capacity bound.
///
/// Before copying, count the returned Maud string's capacity together with the
/// full physical destination. A refusal preserves existing destination bytes.
pub fn write_maud_markup(
    output: &mut Buffer<'_>,
    markup: Markup,
    maximum_overlap: Bytes,
) -> Result<(), MemoryError> {
    let markup = markup.into_string();
    let overlap = output
        .capacity()
        .checked_add(Bytes::new(markup.capacity()))?;
    check_markup_overlap(overlap, maximum_overlap)?;
    output.extend_from_slice(markup.as_bytes())
}

fn check_markup_overlap(requested: Bytes, capacity: Bytes) -> Result<(), MemoryError> {
    if requested > capacity {
        return Err(MemoryError::ValueTooLarge {
            requested,
            capacity,
        });
    }
    Ok(())
}

/// Invoke a consumer's Maud renderer only after claiming the HTTP reply slot.
/// The renderer keeps its own error type and the reply keeps the actual backing.
pub fn render_prepared_maud_reply<E, R>(
    maximum_overlap: Bytes,
    render: impl Fn(&Request) -> Result<Markup, E> + Send + Sync + 'static,
    on_error: impl Fn(ServerError) -> E + Send + Sync + 'static,
) -> Effect<Reply<E, R>, E, R>
where
    E: Send + 'static,
    R: HasRequest + Send + Sync + 'static,
{
    let render = Arc::new(render);
    let on_error = Arc::new(on_error);
    effect::effect! {
        let render = render.clone();
        let on_error = on_error.clone();
        let request = server! { request; }?;
        effect::effect_try_sync! {
            once;
            body: move || {
                let mut output = request.take_prepared_output_buffer().map_err(|error| on_error(error))?;
                check_markup_overlap(output.capacity(), maximum_overlap)
                    .map_err(|error| on_error(ServerError::Memory(error)))?;
                let markup = render(&request)?;
                write_maud_markup(&mut output, markup, maximum_overlap)
                    .map_err(|error| on_error(ServerError::Memory(error)))?;
                Ok(prepared_html_reply(BufferView::new(output.share())))
            },
        }?
    }
}

fn prepared_html_reply<E: Send + 'static, R: Send + Sync + 'static>(
    body: BufferView<'static>,
) -> Reply<E, R> {
    let media = http::HeaderValue::from_static("text/html; charset=utf-8");
    effect_server::reply! { lease(body, media); }
}

impl ResourcePageRenderer {
    /// Resolve the current request's list query, build its page, and publish the HTML reply.
    pub fn definition_list_reply<E, R>(
        &self,
        services: &DefinitionServiceSet<E, R>,
        kind: &ResourceKind,
    ) -> DefinitionPageReply<E, R>
    where
        E: Send + 'static,
        R: HasRequest + Send + Sync + 'static,
    {
        let renderer = self.clone();
        let queries = services.queries.clone();
        let rendering = services.rendering.clone();
        let kind = kind.clone();
        effect::effect! {
            let renderer = renderer.clone();
            let queries = queries.clone();
            let rendering = rendering.clone();
            let kind = kind.clone();
            let request = server! { request; }?;
            let query = queries.list_query(&request, &kind)?;
            let page = rendering.list_page(&request, &kind, query)?;
            renderer.list_reply(&request, page)?
        }
    }

    /// Take the request's prepared output before invoking shell or component callbacks.
    ///
    /// The reply retains this output through transport. The caller's model and
    /// native Maud component work keep their native ownership. The measured
    /// overlap includes the returned string's allocation capacity and the full
    /// response slot. The native renderer remains responsible for earlier work
    /// inside its invocation.
    pub fn reply_prepared<E: Send + 'static, R: Send + Sync + 'static>(
        &self,
        request: &Request,
        page: ResourcePageModel,
        limits: ResourcePageRenderLimits,
        on_error: impl Fn(PreparedResourceReplyError) -> E + Send + Sync + 'static,
    ) -> Effect<Reply<E, R>, E, R> {
        let renderer = self.clone();
        let request = request.clone();
        effect::effect_try_sync! {
            body: move || {
                let output = take_page_output(&request, limits)?;
                renderer.render_in_output(&request, page.view(), limits, output)
            },
            on_error: on_error
        }
    }

    /// Admit the current request's HTML output before an effectful page-model source.
    /// The output stays reserved during the source and Maud rendering and moves into
    /// the published reply. All expected exits from the source keep their error family.
    pub fn reply_prepared_with<E, R>(
        &self,
        model: Effect<ResourcePageModel, E, R>,
        limits: ResourcePageRenderLimits,
        on_error: impl Fn(PreparedResourceReplyError) -> E + Send + Sync + 'static,
    ) -> Effect<Reply<E, R>, E, R>
    where
        E: Send + 'static,
        R: HasRequest + Send + Sync + 'static,
    {
        self.reply_from_model(model, limits, ResourcePageModel::view, on_error)
    }

    /// Compose the selected definition query with its existing page producer and Maud shell.
    /// Server output is reserved before admission and page production.
    pub fn definition_reply_prepared<E, R>(
        &self,
        services: &DefinitionServiceSet<E, R>,
        prepared: Arc<PreparedDefinitionServices<E>>,
        kind: &ResourceKind,
        surface: ResourcePageSurface,
        limits: ResourcePageRenderLimits,
        on_error: impl Fn(PreparedResourceReplyError) -> RenderingError<E> + Send + Sync + 'static,
    ) -> DefinitionPageReply<E, R>
    where
        E: Send + 'static,
        R: HasRequest + Send + Sync + 'static,
    {
        let queries = services.queries.clone();
        let rendering = services.rendering.clone();
        let kind = kind.clone();
        match surface {
            ResourcePageSurface::List => {
                let model = effect::effect! {
                    let queries = queries.clone();
                    let rendering = rendering.clone();
                    let prepared = prepared.clone();
                    let kind = kind.clone();
                    let request = server! { request; }?;
                    let query = effect_map_error! {
                        program: prepared.list_query(&queries, &request, &kind),
                        map: RenderingError::Consumer,
                    }?;
                    rendering.list_page(&request, &kind, query)?
                };
                self.reply_from_model(model, limits, |page| ResourcePageRef::List(page), on_error)
            }
            ResourcePageSurface::Detail => {
                let model = effect::effect! {
                    let queries = queries.clone();
                    let rendering = rendering.clone();
                    let prepared = prepared.clone();
                    let kind = kind.clone();
                    let request = server! { request; }?;
                    let query = effect_map_error! {
                        program: prepared.detail_query(&queries, &request),
                        map: RenderingError::Consumer,
                    }?;
                    rendering.detail_page(&request, &kind, query)?
                };
                self.reply_from_model(
                    model,
                    limits,
                    |page| ResourcePageRef::Detail(page),
                    on_error,
                )
            }
            ResourcePageSurface::Dashboard => {
                let model = effect::effect! {
                    let queries = queries.clone();
                    let rendering = rendering.clone();
                    let prepared = prepared.clone();
                    let kind = kind.clone();
                    let request = server! { request; }?;
                    let query = effect_map_error! {
                        program: prepared.list_query(&queries, &request, &kind),
                        map: RenderingError::Consumer,
                    }?;
                    rendering.dashboard_page(&request, &kind, query)?
                };
                self.reply_from_model(
                    model,
                    limits,
                    |page| ResourcePageRef::Dashboard(page),
                    on_error,
                )
            }
        }
    }

    fn reply_from_model<A, E, R>(
        &self,
        model: Effect<A, E, R>,
        limits: ResourcePageRenderLimits,
        view: impl for<'a> Fn(&'a A) -> ResourcePageRef<'a> + Copy + Send + Sync + 'static,
        on_error: impl Fn(PreparedResourceReplyError) -> E + Send + Sync + 'static,
    ) -> Effect<Reply<E, R>, E, R>
    where
        A: Send + 'static,
        E: Send + 'static,
        R: HasRequest + Send + Sync + 'static,
    {
        let renderer = self.clone();
        let on_error = Arc::new(on_error);
        effect::effect! {
            let renderer = renderer.clone();
            let model = model.clone();
            let on_error = on_error.clone();
            let request = server! { request; }?;
            let request_for_output = request.clone();
            let on_reserve_error = on_error.clone();
            let output = effect::effect_try_sync! {
                once;
                body: move || take_page_output(&request_for_output, limits),
                on_error: move |error| on_reserve_error(error),
            }?;
            let page = model?;
            effect::effect_try_sync! {
                once;
                body: move || renderer.render_in_output(&request, view(&page), limits, output),
                on_error: move |error| on_error(error),
            }?
        }
    }

    fn render_in_output<E: Send + 'static, R: Send + Sync + 'static>(
        &self,
        request: &Request,
        page: ResourcePageRef<'_>,
        limits: ResourcePageRenderLimits,
        mut output: BufferLease<'static>,
    ) -> Result<Reply<E, R>, PreparedResourceReplyError> {
        page.check_model(limits)
            .map_err(PreparedResourceReplyError::Model)?;
        let markup = match page {
            ResourcePageRef::Form(page) => self
                .form_page(request, page)
                .map_err(PreparedResourceReplyError::Render)?,
            ResourcePageRef::List(page) => self.list_page(request, page),
            ResourcePageRef::Detail(page) => self.detail_page(request, page),
            ResourcePageRef::Dashboard(page) => self.dashboard_page(request, page),
        };
        write_maud_markup(&mut output, markup, limits.native_and_response_bytes)
            .map_err(PreparedResourceReplyError::Output)?;
        Ok(prepared_html_reply(BufferView::new(output.share())))
    }
}

fn take_page_output(
    request: &Request,
    limits: ResourcePageRenderLimits,
) -> Result<BufferLease<'static>, PreparedResourceReplyError> {
    let output = request
        .take_prepared_output_buffer()
        .map_err(PreparedResourceReplyError::Request)?;
    check_markup_overlap(output.capacity(), limits.native_and_response_bytes)
        .map_err(PreparedResourceReplyError::Output)?;
    Ok(output)
}
