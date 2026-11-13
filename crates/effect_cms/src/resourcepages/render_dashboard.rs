use maud::{html,Markup};
use web_components::components::{shared,unstyled::{resourceui,form,input,button,icons,badge,card,heading,emptystate}};
use crate::{dashboards::*,rendering::*,definitions::*};
use super::{render_contracts::*,render_helpers::*,render_list::{resource_pagination,resource_action_label,resource_delete_action_attributes}};

pub fn resource_dashboard_search_form(c:ResourceRenderContext<'_>,search:&DashboardSearchVM)->Markup {
    html!{@if dashboard_search_enabled(search){(form::Form(&form::FormProps{method:"GET".into(),action:search.action.clone(),class:"max-w-xl".into(),attributes:[("role".into(),"search".into()),("aria-label".into(),dashboard_search_aria_label(search,&c.t("common.search")))].into(),..Default::default()},input::CommandSearchInput(&input::CommandSearchInputProps{id:search.id.clone(),name:dashboard_search_name(search),value:text(&search.value),placeholder:dashboard_search_placeholder(search,&c.t("common.search")),aria_label:dashboard_search_aria_label(search,&c.t("common.search")),size:shared::SIZE_MD,shortcut_label:dashboard_search_shortcut_label(search,&c.t("common.search_shortcut")),enable_global_shortcut:true,full_width:true,..Default::default()})))}}
}
pub fn media_dashboard_workbench(c:ResourceRenderContext<'_>,p:&ContentDashboardPageVM,token:&str,field:&str)->Markup {
    let stats=shared_values(&p.stats);let filters=shared_values(&p.filters);let rows=shared_values(&p.primary_section.rows);
    html!{div class="space-y-5" {
        (resourceui::Toolbar(&resourceui::ToolbarProps::default(),html!{
            (resourceui::Split(&resourceui::SplitProps::default(),html!{div class="min-w-0 flex-1" {(resource_dashboard_search_form(c,&p.search))}@if !stats.is_empty(){(resourceui::StatGroup(&resourceui::StatGroupProps::default(),html!{@for stat in &stats {(media_dashboard_stat_chip(stat))}}))}}))
            @if !filters.is_empty(){(resourceui::FilterGroup(&resourceui::FilterGroupProps{aria_label:c.t("common.filter"),class:"mt-4 border-t border-border pt-4".into(),..Default::default()},html!{@for filter in &filters {(media_dashboard_filter_chip(filter))}}))}
        }))
        (resourceui::Panel(&resourceui::PanelProps{title:p.primary_section.title.clone(),description:media_dashboard_result_description(c,&p.search),no_padding:true,..Default::default()},html!{
            @if rows.is_empty(){div class="p-8" {(emptystate::EmptyState(&emptystate::EmptyStateProps{title:p.primary_section.empty_title.clone(),description:p.primary_section.empty_description.clone(),..Default::default()},Markup::default()))}}
            @else {(resourceui::ResultList(&resourceui::ResultListProps::default(),html!{@for row in &rows {(media_dashboard_workbench_row(c,row,token,field))}}))(media_dashboard_pagination(c,&p.primary_section.pagination))}
        }))
    }}
}
pub fn media_dashboard_stat_chip(stat:&DashboardStatVM)->Markup {resourceui::StatChip(&resourceui::StatChipProps{label:stat.label.clone(),value:stat.value.clone(),icon:dashboard_stat_icon_component(&stat.icon),..Default::default()})}
pub fn media_dashboard_filter_chip(filter:&DashboardFilterChipVM)->Markup {resourceui::FilterChip(&resourceui::FilterChipProps{label:filter.label.clone(),count:filter.count.clone(),href:filter.href.clone(),active:filter.active,..Default::default()})}
pub fn media_dashboard_workbench_row(c:ResourceRenderContext<'_>,row:&DashboardRowVM,token:&str,field:&str)->Markup {resourceui::ResultRow(&resourceui::ResultRowProps{title:row.title.clone(),title_href:row.title_href.clone(),secondary_text:row.secondary_text.clone(),meta_text:row.meta_text.clone(),preview:Some(media_dashboard_preview(row)),badges:Some(media_dashboard_badges(row)),actions:Some(media_dashboard_row_actions(c,row,token,field)),..Default::default()})}
pub fn media_dashboard_preview(row:&DashboardRowVM)->Markup {resourceui::PreviewFrame(&resourceui::PreviewFrameProps{src:row.preview_url.clone(),alt:row.preview_alt.clone(),fallback:Some(media_dashboard_preview_icon(&row.preview_kind)),..Default::default()})}
pub fn media_dashboard_preview_icon(kind:&str)->Markup {let props=icons::IconProps{size:shared::SIZE_MD,..Default::default()};if kind.trim()=="image"{icons::Image(&props)}else{icons::FileText(&props)}}
pub fn media_dashboard_badge(b:&DashboardRowBadgeVM)->Markup {html!{@if !b.label.trim().is_empty(){(badge::Badge(&badge::BadgeProps{variant:shared::VARIANT_OUTLINE,size:shared::SIZE_SM,pill:true,..Default::default()},html!{(&b.label)}))}}}
pub fn media_dashboard_badges(row:&DashboardRowVM)->Markup {let badges=shared_values(&row.badges);html!{@if !badges.is_empty(){div class="mt-3 flex flex-wrap gap-2" {@for badge in &badges {(media_dashboard_badge(badge))}}}}}
pub fn media_dashboard_row_actions(c:ResourceRenderContext<'_>,row:&DashboardRowVM,token:&str,field:&str)->Markup {resourceui::ActionGroup(&resourceui::ActionGroupProps::default(),html!{@for a in shared_values(&row.actions){(media_dashboard_row_action_with_title(c,&a,&row.title,token,field))}})}
pub fn media_dashboard_row_action(c:ResourceRenderContext<'_>,action:&DashboardRowActionVM,token:&str,field:&str)->Markup {media_dashboard_row_action_with_title(c,action,"",token,field)}
fn media_dashboard_row_action_with_title(_c:ResourceRenderContext<'_>,a:&DashboardRowActionVM,title:&str,token:&str,field:&str)->Markup {
    if a.href.trim().is_empty()||a.label.trim().is_empty(){return Markup::default()}
    let icon=media_dashboard_action_icon(&a.kind);
    if a.kind.as_str()==RESOURCE_ITEM_ACTION_KIND_DELETE {return form::FormWithCSRF(&form::FormProps{method:"POST".into(),action:a.href.clone(),class:"inline".into(),..Default::default()},token,field,button::ButtonWithLeadingIcon(&button::ButtonProps{button_type:"submit",variant:shared::VARIANT_DESTRUCTIVE,size:shared::SIZE_SM,attributes:resource_delete_action_attributes(&a.confirm_message,&a.label,title),..Default::default()},icon,html!{(&a.label)}))}
    let mut attributes=std::collections::BTreeMap::from([("aria-label".into(),resource_action_label(&a.label,title))]);
    if a.kind.as_str()==RESOURCE_ITEM_ACTION_KIND_EXTERNAL_LINK {attributes.insert("target".into(),"_blank".into());attributes.insert("rel".into(),"noreferrer".into());}
    button::ButtonWithLeadingIcon(&button::ButtonProps{as_element:shared::ELEMENT_A,href:a.href.clone(),variant:shared::VARIANT_OUTLINE,size:shared::SIZE_SM,attributes,..Default::default()},icon,html!{(&a.label)})
}
pub fn media_dashboard_action_icon(kind:&ResourceItemActionKind)->Markup {let p=icons::IconProps{size:shared::SIZE_SM,..Default::default()};match kind.as_str(){RESOURCE_ITEM_ACTION_KIND_DELETE=>icons::Trash(&p),RESOURCE_ITEM_ACTION_KIND_EXTERNAL_LINK=>icons::Link2(&p),_=>icons::Edit(&p)}}
pub fn dashboard_stat_icon_component(icon:&DashboardStatIcon)->Option<Markup> {let p=icons::IconProps{size:shared::SIZE_SM,..Default::default()};match icon.0.as_str(){DASHBOARD_STAT_ICON_MEDIA=>Some(icons::Image(&p)),DASHBOARD_STAT_ICON_CHECK=>Some(icons::Check(&p)),DASHBOARD_STAT_ICON_DRAFT=>Some(icons::Edit(&p)),DASHBOARD_STAT_ICON_CALENDAR=>Some(icons::Calendar(&p)),DASHBOARD_STAT_ICON_COMMENT=>Some(icons::MessageCircle(&p)),DASHBOARD_STAT_ICON_SETTINGS=>Some(icons::Settings(&p)),_=>None}}
pub fn media_dashboard_result_description(c:ResourceRenderContext<'_>,search:&DashboardSearchVM)->String {let value=text(search.value.trim());if value.is_empty(){String::new()}else{format!("{}: {value}",c.t("common.search"))}}
pub fn media_dashboard_pagination(c:ResourceRenderContext<'_>,p:&DashboardPaginationVM)->Markup {
    html!{@if p.total_pages>1 {(resourceui::PaginationBar(&resourceui::PaginationBarProps{aria_label:c.t_args("common.pagination.page_label",&[p.current_page.to_string(),p.total_pages.to_string()]),..Default::default()},html!{
        @if p.current_page>1 {(button::ButtonWithLeadingIcon(&button::ButtonProps{as_element:shared::ELEMENT_A,href:crate::media::presentation::media_dashboard_page_href(c.request,&p.base_href,&p.page_param,p.current_page-1),variant:shared::VARIANT_OUTLINE,size:shared::SIZE_SM,..Default::default()},icons::ChevronLeft(&icons::IconProps{size:shared::SIZE_SM,..Default::default()}),html!{(c.t("common.pagination.previous"))}))}
        span class="text-sm text-muted-foreground" {(p.current_page) " / " (p.total_pages)}
        @if p.current_page<p.total_pages {(button::ButtonWithTrailingIcon(&button::ButtonProps{as_element:shared::ELEMENT_A,href:crate::media::presentation::media_dashboard_page_href(c.request,&p.base_href,&p.page_param,p.current_page+1),variant:shared::VARIANT_OUTLINE,size:shared::SIZE_SM,..Default::default()},icons::ChevronRight(&icons::IconProps{size:shared::SIZE_SM,..Default::default()}),html!{(c.t("common.pagination.next"))}))}
    }))}}
}
pub fn dashboard_action_icon(action:&DashboardActionVM)->Markup {let mut icon=action.icon.trim();if icon.is_empty()&&action.href.to_ascii_lowercase().contains("/new"){icon="plus"}icons::IconFromAlias(icon,&icons::IconProps{size:shared::SIZE_SM,..Default::default()}).unwrap_or_default()}
pub fn resource_dashboard_header_actions(actions:&[DashboardActionVM])->Markup {
    let filtered:Vec<_>=actions.iter().filter(|a|a.location.as_str()==DASHBOARD_ACTION_LOCATION_HEADER&&!a.label.trim().is_empty()&&!a.href.trim().is_empty()).collect();
    html!{@if !filtered.is_empty(){div class="flex flex-wrap items-center gap-3" {@for (index,a) in filtered.iter().enumerate(){(button::ButtonWithLeadingIcon(&button::ButtonProps{as_element:shared::ELEMENT_A,href:a.href.clone(),variant:resource_dashboard_action_variant(index),size:shared::SIZE_MD,..Default::default()},dashboard_action_icon(a),html!{(&a.label)}))}}}}
}
pub fn resource_dashboard_page_body(c:ResourceRenderContext<'_>,p:&ResourceDashboardPageVM,o:&ResourcePageShellOptions)->Markup {html!{(resource_dashboard_search_form(c,&p.search))@for s in shared_values(&p.sections){@if resource_dashboard_section_has_content(&s){(resource_dashboard_section_view(c,&s,&o.csrf_token,&o.csrf_field_name))}}}}
pub fn resource_dashboard_section_view(c:ResourceRenderContext<'_>,s:&ResourceDashboardSectionVM,token:&str,field:&str)->Markup {
    let metrics=shared_values(&s.metrics);let items=shared_values(&s.items);let actions=shared_values(&s.actions);
    card::Card(&card::CardProps{class:"rounded-xl border border-border shadow-sm".into(),..Default::default()},card::CardContent(&card::CardContentProps{class:"space-y-4 p-6".into(),..Default::default()},html!{
        @if !s.title.trim().is_empty(){div class="space-y-1" {(heading::H2("lg",false,"font-semibold",html!{(&s.title)}))@if !s.description.trim().is_empty(){p class="text-sm text-muted-foreground" {(&s.description)}}}}
        @if !metrics.is_empty(){div class="grid gap-4 sm:grid-cols-2 xl:grid-cols-4" {@for m in &metrics {div class="rounded-xl border border-border bg-card p-4" {p class="text-sm text-muted-foreground" {(&m.label)}p class="mt-2 text-3xl font-semibold text-foreground" {(&m.value)}@if !m.description.trim().is_empty(){p class="mt-2 text-sm text-muted-foreground" {(&m.description)}}}}}}
        @if !items.is_empty(){div class="space-y-4" {@for i in &items {(resource_dashboard_item(c,i,token,field))}}}
        @if !actions.is_empty(){div class="flex flex-wrap gap-3" {@for a in &actions {(button::Button(&button::ButtonProps{as_element:shared::ELEMENT_A,href:a.href.clone(),variant:shared::VARIANT_OUTLINE,size:shared::SIZE_SM,..Default::default()},html!{(&a.label)}))}}}
        (resource_pagination(c,&s.pagination))
    }))
}
pub fn resource_dashboard_item(_c:ResourceRenderContext<'_>,i:&ResourceListItemVM,token:&str,field:&str)->Markup {
    let href=resource_first_href(i);let title=text(&i.title);
    html!{div class="rounded-xl border border-border bg-card p-4" {div class="flex flex-wrap items-start justify-between gap-3" {
        div class="min-w-0 space-y-1" {@if !href.is_empty(){a href=(href) class="break-words text-base font-semibold text-foreground [overflow-wrap:anywhere] hover:text-primary" {(title)}}@else{p class="break-words text-base font-semibold text-foreground [overflow-wrap:anywhere]" {(title)}}
            @if !i.summary.trim().is_empty(){p class="break-words text-sm text-muted-foreground [overflow-wrap:anywhere]" {(text(&i.summary))}}
            @if !shared_values(&i.meta).is_empty(){div class="flex flex-wrap gap-x-3 gap-y-1 text-sm text-muted-foreground" {@for m in &shared_values(&i.meta) {div class="break-words [overflow-wrap:anywhere]" {@if let Some(component)=&m.component {(component)}@else{(&m.value)}}}}}
        }
        div class="flex flex-wrap items-center gap-2" {@if !i.status.trim().is_empty(){(badge::Badge(&badge::BadgeProps{variant:shared::VARIANT_OUTLINE,size:shared::SIZE_SM,pill:true,..Default::default()},html!{(&i.status)}))}
            @for a in &shared_values(&i.actions) {@if a.kind.as_str()==RESOURCE_ITEM_ACTION_KIND_DELETE {(form::FormWithCSRF(&form::FormProps{method:"POST".into(),action:a.href.clone(),class:"inline".into(),..Default::default()},token,field,button::Button(&button::ButtonProps{button_type:"submit",variant:shared::VARIANT_DESTRUCTIVE,size:shared::SIZE_SM,attributes:resource_delete_button_attrs(&a.confirm_message),..Default::default()},html!{(&a.label)})))}@else{(button::Button(&button::ButtonProps{as_element:shared::ELEMENT_A,href:a.href.clone(),variant:shared::VARIANT_GHOST,size:shared::SIZE_SM,..Default::default()},html!{(&a.label)}))}}
        }
    }}}
}
