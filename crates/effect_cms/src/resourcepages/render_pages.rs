use maud::{html,Markup};
use web_components::components::{shared,unstyled::{container,heading,button,form,card,icons,resourceui}};
use crate::{forms::ResourceFormPageVM,rendering::{ResourceListPageVM,ResourceDetailPageVM},dashboards::ResourceDashboardPageVM,routing::ManagementBreadcrumbVM};
use super::{render_contracts::*,render_helpers::*,render_form::*,render_list::*,render_dashboard::*};

pub fn resource_page_breadcrumb(c:ResourceRenderContext<'_>,title:&str,breadcrumb:&ManagementBreadcrumbVM,enabled:bool,render:Option<&ResourceBreadcrumbRenderer>)->Markup {
    html!{@if enabled&&!breadcrumb.dashboard_route.is_empty(){@if let Some(render)=render {(render(&ResourcePageBreadcrumbProps{request:c.request.cloned(),dashboard_route:breadcrumb.dashboard_route.clone(),dashboard_label:breadcrumb.dashboard_label.clone(),current:resource_page_title(title,breadcrumb),..Default::default()}))}}}
}
pub fn resource_form_page_content(c:ResourceRenderContext<'_>,p:&ResourceFormPageVM,o:&ResourcePageShellOptions)->Result<Markup,super::ResourceRenderError> {
    Ok(container::Container(&container::ContainerProps{full_bleed:true,centered:true,padding:"lg",class:"max-w-4xl py-8".into(),..Default::default()},html!{div class="space-y-6" {
        (resource_page_breadcrumb(c,&p.title,&p.breadcrumb,true,o.breadcrumb.as_ref()))
        div class="flex items-center justify-between gap-4" {div class="space-y-1" {(heading::H1("2xl",false,"font-heading font-bold",html!{(resource_page_title(&p.title,&p.breadcrumb))}))@if !p.breadcrumb.current_subtitle.trim().is_empty(){p class="text-sm text-muted-foreground" {(&p.breadcrumb.current_subtitle)}}}
            @if !p.cancel_href.trim().is_empty(){(button::ButtonWithLeadingIcon(&button::ButtonProps{as_element:shared::ELEMENT_A,href:p.cancel_href.clone(),variant:shared::VARIANT_GHOST,size:shared::SIZE_SM,..Default::default()},icons::Close(&icons::IconProps{size:shared::SIZE_SM,..Default::default()}),html!{(c.t("common.cancel"))}))}
        }
        (resource_form_page_body(c,p,o)?)
    }}))
}
pub fn resource_form_page_body(c:ResourceRenderContext<'_>,p:&ResourceFormPageVM,o:&ResourcePageShellOptions)->Result<Markup,super::ResourceRenderError> {
    if p.kind.as_str()=="media"{return media_resource_form_workbench(c,p,o)}
    Ok(html!{
        @if !p.status.issues.is_empty(){(resource_validation_summary(c,&p.status))}@else if !p.status.message.trim().is_empty(){@if let Some(alert)=&o.status_alert {(alert(&p.status.message))}}
        (card::Card(&card::CardProps{class:"rounded-xl border border-border shadow-sm".into(),..Default::default()},card::CardContent(&card::CardContentProps{no_padding:true,class:"p-6".into(),..Default::default()},form::FormWithCSRF(&form::FormProps{method:"POST".into(),action:p.action_href.clone(),class:"space-y-6".into(),attributes:resource_form_page_attributes(p,o.form_attributes.as_ref()),..Default::default()},&o.csrf_token,&o.csrf_field_name,html!{
            (resource_form_sections(c,&p.sections,&o.form_render_options)?)
            @if let Some(alerts)=&o.live_submit_alerts {(alerts("mt-2"))}
            @if let Some(actions)=&o.form_submit_actions {(actions(&resource_form_action_props(&p.actions),""))}
        }))))
        @if let Some(roots)=&o.media_modal_roots {@for manager in resource_form_media_managers(p){(roots(&manager.mounts))}}
    })
}
pub fn media_resource_form_workbench(c:ResourceRenderContext<'_>,p:&ResourceFormPageVM,o:&ResourcePageShellOptions)->Result<Markup,super::ResourceRenderError> {
    Ok(resourceui::FormLayout(&resourceui::FormLayoutProps{class:"media-resource-workbench".into(),..Default::default()},html!{
        div class="min-w-0 space-y-6" {
            @if !p.status.issues.is_empty(){(resource_validation_summary(c,&p.status))}@else if !p.status.message.trim().is_empty(){@if let Some(alert)=&o.status_alert {(alert(&p.status.message))}}
            (resourceui::Panel(&resourceui::PanelProps{title:media_workbench_title(p,o),no_padding:true,class:"media-resource-form-panel".into(),..Default::default()},form::FormWithCSRF(&form::FormProps{method:"POST".into(),action:p.action_href.clone(),class:"space-y-5 p-5 sm:p-6".into(),attributes:resource_form_page_attributes(p,o.form_attributes.as_ref()),..Default::default()},&o.csrf_token,&o.csrf_field_name,html!{
                (media_resource_form_sections(c,&p.sections,&o.form_render_options)?)
                @if let Some(alerts)=&o.live_submit_alerts {(alerts("mt-2"))}
                @if o.form_submit_actions.is_some()||(o.inline_media_actions&&!p.cancel_href.trim().is_empty()){div class="media-resource-actions border-t border-border pt-5" {
                    @if o.inline_media_actions&&!p.cancel_href.trim().is_empty(){(button::Button(&button::ButtonProps{as_element:shared::ELEMENT_A,href:p.cancel_href.clone(),variant:shared::VARIANT_OUTLINE,size:shared::SIZE_MD,..Default::default()},html!{(c.t("common.cancel"))}))}
                    @if let Some(actions)=&o.form_submit_actions {(actions(&resource_form_action_props(&p.actions),""))}
                }}
            })))
            @if let Some(roots)=&o.media_modal_roots {@for manager in resource_form_media_managers(p){(roots(&manager.mounts))}}
        }
        aside class="space-y-4 lg:sticky lg:top-24 lg:self-start" {(media_resource_preview_panel(c,p))
            @if !o.inline_media_actions&&!p.cancel_href.trim().is_empty(){(resourceui::Panel(&resourceui::PanelProps{title:c.t("common.actions"),no_padding:true,..Default::default()},html!{div class="space-y-3 p-4" {
                (button::ButtonWithLeadingIcon(&button::ButtonProps{as_element:shared::ELEMENT_A,href:p.cancel_href.clone(),variant:shared::VARIANT_OUTLINE,size:shared::SIZE_SM,full_width:true,..Default::default()},icons::Close(&icons::IconProps{size:shared::SIZE_SM,..Default::default()}),html!{(c.t("common.cancel"))}))
            }}))}
        }
    }))
}
pub fn media_workbench_title(p:&ResourceFormPageVM,o:&ResourcePageShellOptions)->String {if o.inline_media_actions{String::new()}else{resource_page_title(&p.title,&p.breadcrumb)}}
pub fn resource_list_page_content(c:ResourceRenderContext<'_>,p:&ResourceListPageVM,o:&ResourcePageShellOptions)->Markup {
    container::Container(&container::ContainerProps{centered:true,padding:"lg",class:"max-w-7xl py-8".into(),..Default::default()},html!{div class="space-y-6" {
        (resource_page_breadcrumb(c,&p.title,&p.breadcrumb,p.manage_view,o.breadcrumb.as_ref()))
        div class="flex flex-wrap items-start justify-between gap-4" {div class="space-y-1" {(heading::H1("2xl",false,"font-heading font-bold",html!{(resource_page_title(&p.title,&p.breadcrumb))}))@if !p.subtitle.trim().is_empty(){p class="text-sm text-muted-foreground" {(&p.subtitle)}}}
            @if !p.new_href.trim().is_empty()&&p.can_create{(button::Button(&button::ButtonProps{as_element:shared::ELEMENT_A,href:p.new_href.clone(),variant:shared::VARIANT_PRIMARY,size:shared::SIZE_MD,..Default::default()},html!{(c.t("common.new"))}))}
        }
        (resource_list_page_body(c,p,o))
    }})
}
pub fn resource_detail_page_content(c:ResourceRenderContext<'_>,p:&ResourceDetailPageVM,o:&ResourcePageShellOptions)->Markup {
    container::Container(&container::ContainerProps{centered:true,padding:"lg",class:"max-w-5xl py-8".into(),..Default::default()},html!{div class="space-y-6" {
        (resource_page_breadcrumb(c,&p.title,&p.breadcrumb,p.manage_view,o.breadcrumb.as_ref()))
        div class="flex flex-wrap items-start justify-between gap-4" {div class="space-y-2" {(resource_back_button(c,&p.back_href))(heading::H1("2xl",false,"font-heading font-bold",html!{(resource_page_title(&p.title,&p.breadcrumb))}))}(resource_detail_actions(c,p,&o.csrf_token,&o.csrf_field_name))}
        (resource_detail_page_body(c,p))
    }})
}
pub fn resource_dashboard_page_content(c:ResourceRenderContext<'_>,p:&ResourceDashboardPageVM,o:&ResourcePageShellOptions)->Markup {
    container::Container(&container::ContainerProps{full_bleed:true,centered:true,padding:"lg",class:"max-w-7xl py-8".into(),..Default::default()},html!{div class="space-y-6" {
        (resource_page_breadcrumb(c,&p.title,&p.breadcrumb,true,o.breadcrumb.as_ref()))
        div class="space-y-2" {(heading::H1("2xl",false,"font-heading font-bold",html!{(resource_page_title(&p.title,&p.breadcrumb))}))@if !p.subtitle.trim().is_empty(){p class="text-sm text-muted-foreground" {(&p.subtitle)}}}
        (resource_dashboard_header_actions(&shared_values(&p.actions)))(resource_dashboard_page_body(c,p,o))
    }})
}
