use maud::{html,Markup,PreEscaped};
use web_components::components::{shared,unstyled::{button,form,heading,input,badge,card,emptystate,icons}};
use crate::{rendering::*,definitions::*};
use super::{render_contracts::*,render_helpers::*};

pub fn resource_list_page_body(c:ResourceRenderContext<'_>,p:&ResourceListPageVM,o:&ResourcePageShellOptions)->Markup {
    html!{(resource_list_toolbar(c,p))@if !resource_list_has_items(p){(resource_list_empty_state(c))}@else{@for s in &p.sections {@if !s.items.is_empty(){(resource_list_section_view(c,&p.kind,s,o.section_container_id.as_ref(),&o.csrf_token,&o.csrf_field_name))}}(resource_pagination(c,&p.pagination))}}
}
pub fn resource_list_toolbar(c:ResourceRenderContext<'_>,p:&ResourceListPageVM)->Markup {
    html!{@if !p.toolbar.action.trim().is_empty()||!p.filter_state.options.is_empty(){div class="space-y-3 rounded-xl border border-border bg-card p-4 shadow-sm" {
        @if !p.toolbar.action.trim().is_empty(){(form::Form(&form::FormProps{method:"GET".into(),action:p.toolbar.action.clone(),class:"grid gap-3 md:grid-cols-[minmax(0,1fr)_auto] md:items-center".into(),attributes:[("role".into(),"search".into()),("aria-label".into(),c.t("common.search"))].into(),..Default::default()},html!{
            (input::CommandSearchInput(&input::CommandSearchInputProps{id:"contentmgmt-search".into(),name:p.toolbar.search_key.as_str().into(),value:text(&p.toolbar.search_value),placeholder:c.t("common.search"),aria_label:c.t("common.search"),size:shared::SIZE_MD,shortcut_label:c.t("common.search_shortcut"),enable_global_shortcut:true,full_width:true,..Default::default()}))
            (button::Button(&button::ButtonProps{button_type:"submit",variant:shared::VARIANT_OUTLINE,size:shared::SIZE_MD,..Default::default()},html!{(c.t("common.filter"))}))
        }))}
        @if !p.filter_state.options.is_empty(){div class="flex flex-wrap gap-2" role="group" aria-label=(c.t("common.filter")) {@for option in &p.filter_state.options {(button::Button(&button::ButtonProps{as_element:shared::ELEMENT_A,href:option.href.clone(),variant:if option.active{shared::VARIANT_PRIMARY}else{shared::VARIANT_OUTLINE},size:shared::SIZE_SM,..Default::default()},html!{(&option.label)}))}}}
    }}}
}
pub fn resource_list_empty_state(c:ResourceRenderContext<'_>)->Markup {emptystate::EmptyState(&emptystate::EmptyStateProps{title:c.t("common.empty_title"),description:c.t("common.empty_description"),..Default::default()},Markup::default())}
pub fn resource_back_button(c:ResourceRenderContext<'_>,href:&str)->Markup {html!{@if !href.trim().is_empty(){(button::ButtonWithLeadingIcon(&button::ButtonProps{as_element:shared::ELEMENT_A,href:href.into(),variant:shared::VARIANT_GHOST,size:shared::SIZE_SM,..Default::default()},icons::ArrowLeft(&icons::IconProps{size:shared::SIZE_SM,..Default::default()}),html!{(c.t("common.back"))}))}}}
pub fn resource_detail_actions(c:ResourceRenderContext<'_>,p:&ResourceDetailPageVM,token:&str,field:&str)->Markup {
    html!{@if p.can_edit {div class="flex flex-wrap gap-2" {
        @if !p.edit_href.trim().is_empty(){(button::ButtonWithLeadingIcon(&button::ButtonProps{as_element:shared::ELEMENT_A,href:p.edit_href.clone(),variant:shared::VARIANT_OUTLINE,size:shared::SIZE_SM,..Default::default()},icons::Edit(&icons::IconProps{size:shared::SIZE_SM,..Default::default()}),html!{(c.t("common.edit"))}))}
        @if !p.delete_action.trim().is_empty(){(form::FormWithCSRF(&form::FormProps{method:"POST".into(),action:p.delete_action.clone(),..Default::default()},token,field,button::ButtonWithLeadingIcon(&button::ButtonProps{button_type:"submit",variant:shared::VARIANT_DESTRUCTIVE,size:shared::SIZE_SM,attributes:resource_delete_button_attrs(&p.delete_confirm),..Default::default()},icons::Trash(&icons::IconProps{size:shared::SIZE_SM,..Default::default()}),html!{(c.t("common.delete"))})))}
    }}}
}
pub fn resource_list_section_view(c:ResourceRenderContext<'_>,kind:&ResourceKind,s:&ResourceListSectionVM,container:Option<&ResourceSectionContainerID>,token:&str,field:&str)->Markup {
    let id=container.map(|f|f(kind,&s.key)).unwrap_or_default();
    html!{div class="space-y-3" {@if !s.title.trim().is_empty(){div class="space-y-1" {(heading::H2("lg",false,"font-semibold",html!{(&s.title)}))@if !s.description.trim().is_empty(){p class="text-sm text-muted-foreground" {(&s.description)}}}}
        div class=(resource_list_section_classes(&s.key)) id=[(!id.is_empty()).then_some(id.as_str())] {@for item in &s.items {(resource_list_item_card_with_actions(c,item,token,field))}}
    }}
}
pub fn resource_list_item_card(c:ResourceRenderContext<'_>,i:&ResourceListItemVM)->Markup {resource_list_item_card_with_actions(c,i,"","")}
pub fn resource_list_item_card_with_actions(_c:ResourceRenderContext<'_>,i:&ResourceListItemVM,token:&str,field:&str)->Markup {
    let href=resource_first_href(i);let title=text(&i.title);let summary=text(&i.summary);let image=text(&i.image_url);
    card::Card(&card::CardProps{class:"overflow-hidden".into(),..Default::default()},html!{
        @if !image.trim().is_empty(){img src=(image) alt=(&title) class="aspect-video w-full object-cover bg-muted";}
        (card::CardContent(&card::CardContentProps{class:"space-y-3 p-5".into(),..Default::default()},html!{div class="space-y-2" {
            div class="flex min-w-0 items-start justify-between gap-3" {
                @if !href.is_empty(){a href=(href) class="min-w-0 break-words text-base font-semibold text-foreground [overflow-wrap:anywhere] hover:text-primary" {(&title)}}@else{p class="min-w-0 break-words text-base font-semibold text-foreground [overflow-wrap:anywhere]" {(&title)}}
                @if !i.status.trim().is_empty(){(badge::Badge(&badge::BadgeProps{variant:shared::VARIANT_OUTLINE,size:shared::SIZE_SM,pill:true,..Default::default()},html!{(&i.status)}))}
            }
            @if !summary.trim().is_empty(){p class="break-words text-sm leading-6 text-muted-foreground [overflow-wrap:anywhere]" {(summary)}}
            @if !shared_values(&i.meta).is_empty(){div class="space-y-1 text-sm text-muted-foreground" {@for m in &shared_values(&i.meta) {div class="break-words [overflow-wrap:anywhere]" {span class="font-medium text-foreground" {(&m.label)}span {": "}@if let Some(component)=&m.component {(component)}@else if !m.href.trim().is_empty(){a href=(&m.href) class="hover:text-primary" {(&m.value)}}@else{span {(&m.value)}}}}}}
            @if !i.action_href.trim().is_empty()&&!i.action_label.trim().is_empty(){div class="pt-1" {(button::Button(&button::ButtonProps{as_element:shared::ELEMENT_A,href:i.action_href.clone(),variant:shared::VARIANT_GHOST,size:shared::SIZE_SM,..Default::default()},html!{(&i.action_label)}))}}
            @if !shared_values(&i.actions).is_empty(){div class="flex flex-wrap gap-2 pt-1" {@for a in &shared_values(&i.actions) {@if !a.href.trim().is_empty()&&!a.label.trim().is_empty(){
                @if a.kind.as_str()==RESOURCE_ITEM_ACTION_KIND_DELETE {(form::FormWithCSRF(&form::FormProps{method:"POST".into(),action:a.href.clone(),class:"inline".into(),..Default::default()},token,field,button::Button(&button::ButtonProps{button_type:"submit",variant:shared::VARIANT_DESTRUCTIVE,size:shared::SIZE_SM,attributes:resource_delete_action_attributes(&a.confirm_message,&a.label,&title),..Default::default()},html!{(&a.label)})))}
                @else {(button::Button(&button::ButtonProps{as_element:shared::ELEMENT_A,href:a.href.clone(),variant:shared::VARIANT_GHOST,size:shared::SIZE_SM,aria_label:resource_action_label(&a.label,&title),..Default::default()},html!{(&a.label)}))}
            }}}}
        }}))
    })
}
pub fn resource_detail_page_body(c:ResourceRenderContext<'_>,p:&ResourceDetailPageVM)->Markup {html!{@for s in &p.sections {@if resource_has_detail_content(s){(resource_detail_section_view(c,s))}}}}
pub fn resource_detail_section_view(c:ResourceRenderContext<'_>,s:&ResourceDetailSectionVM)->Markup {
    card::Card(&card::CardProps{class:"rounded-xl border border-border shadow-sm".into(),..Default::default()},card::CardContent(&card::CardContentProps{class:"space-y-4 p-6".into(),..Default::default()},html!{
        @if !s.title.trim().is_empty(){div class="space-y-1" {(heading::H2("lg",false,"font-semibold",html!{(&s.title)}))@if !s.description.trim().is_empty(){p class="text-sm text-muted-foreground" {(&s.description)}}}}
        @if !s.fields.is_empty(){div class="grid gap-4 md:grid-cols-2" {@for f in &s.fields {div class="min-w-0 space-y-1" {p class="text-sm font-medium text-foreground" {(&f.label)}p class="break-words whitespace-pre-wrap text-sm text-muted-foreground [overflow-wrap:anywhere]" {(resource_field_display_value(f))}}}}}
        @if !s.body.trim().is_empty(){@if s.body_is_html {div class="prose prose-slate max-w-none dark:prose-invert" {(PreEscaped(&s.body))}}@else{div class="whitespace-pre-wrap text-sm leading-7 text-foreground" {(&s.body)}}}
        @if !s.items.is_empty(){div class="space-y-4" {@for i in &s.items {(resource_nested_item(c,i,0))}}}
    }))
}
pub fn resource_nested_item(c:ResourceRenderContext<'_>,i:&ResourceListItemVM,depth:i64)->Markup {
    html!{div class=(resource_nested_item_classes(depth)) {div class="space-y-2 rounded-lg border border-border bg-card p-4" {
        div class="flex flex-wrap items-start justify-between gap-3" {div class="space-y-1" {p class="break-words text-sm font-semibold text-foreground [overflow-wrap:anywhere]" {(text(&i.title))}@if !shared_values(&i.meta).is_empty(){div class="flex flex-wrap gap-x-3 gap-y-1 text-xs text-muted-foreground" {@for m in &shared_values(&i.meta) {div class="break-words [overflow-wrap:anywhere]" {@if let Some(component)=&m.component {(component)}@else{(&m.value)}}}}}}
            @if !i.action_href.trim().is_empty()&&!i.action_label.trim().is_empty(){(button::Button(&button::ButtonProps{as_element:shared::ELEMENT_A,href:i.action_href.clone(),variant:shared::VARIANT_GHOST,size:shared::SIZE_SM,..Default::default()},html!{(&i.action_label)}))}
        }
        @if !i.summary.trim().is_empty(){p class="break-words whitespace-pre-wrap text-sm leading-6 text-foreground [overflow-wrap:anywhere]" {(text(&i.summary))}}
    }@if !shared_values(&i.children).is_empty(){div class="space-y-3 border-l border-border pl-4" {@for child in &shared_values(&i.children) {(resource_nested_item(c,child,depth+1))}}}}}
}
pub fn resource_pagination(c:ResourceRenderContext<'_>,p:&ListPaginationVM)->Markup {
    let class="inline-flex h-10 min-w-10 items-center justify-center rounded-lg border border-border bg-card px-3 text-sm font-medium text-foreground hover:bg-accent hover:text-accent-foreground";
    html!{@if p.total_pages>1 {nav aria-label="Pagination" class="mt-8" data-smooth-preserve-scroll="true" {ul class="flex flex-wrap items-center justify-center gap-2" {
        @if p.has_prev&&!p.prev_href.trim().is_empty(){li {a href=(&p.prev_href) class=(class) {(c.t("common.pagination.previous"))}}}
        @for link in &shared_values(&p.links) {li {@if link.current {span aria-current="page" class="inline-flex h-10 min-w-10 items-center justify-center rounded-lg bg-primary px-3 text-sm font-medium text-primary-foreground" {(link.page)}}@else{a href=(&link.href) class=(class) {(link.page)}}}}
        @if p.has_next&&!p.next_href.trim().is_empty(){li {a href=(&p.next_href) class=(class) {(c.t("common.pagination.next"))}}}
    }}}}
}
pub fn resource_action_label(label:&str,title:&str)->String {if title.trim().is_empty(){label.into()}else{format!("{label}: {title}")}}
pub fn resource_delete_action_attributes(confirm:&str,label:&str,title:&str)->std::collections::BTreeMap<String,String> {let mut attrs=resource_delete_button_attrs(confirm);attrs.insert("aria-label".into(),resource_action_label(label,title));attrs}
