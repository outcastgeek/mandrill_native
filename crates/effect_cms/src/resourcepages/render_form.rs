use maud::{html,Markup};
use web_components::components::{shared,unstyled::{input,select,form,alert,heading,icons,datetimepicker,durationpicker,resourceui}};
use crate::{forms::*,definitions::*};
use super::{render_contracts::*,render_helpers::*};

pub fn management_form_action_icon_component(icon:&str)->Markup {icons::IconFromAlias(icon,&icons::IconProps{size:shared::SIZE_SM,..Default::default()}).unwrap_or_default()}
pub fn management_form_submit_actions(actions:&[ManagementFormActionProps],class:&str)->Markup {
    html!{@if !actions.is_empty(){div class=(shared::build_classes(&["flex flex-wrap justify-end gap-3 pt-4",class])) {@for action in actions {@if !action.label.is_empty(){
        (web_components::components::unstyled::button::ButtonWithLeadingIcon(&web_components::components::unstyled::button::ButtonProps{button_type:"submit",variant:action.variant,size:shared::SIZE_MD,attributes:localized_submit_control_attributes([("name".into(),action.name.clone()),("value".into(),action.value.clone())].into()),..Default::default()},management_form_action_icon_component(&action.icon),html!{(&action.label)}))
    }}}}}
}
pub fn resource_form_sections(c:ResourceRenderContext<'_>,sections:&[ResourceFormSectionVM],options:&ResourceFormRenderOptions)->Result<Markup,super::ResourceRenderError> {Ok(html!{@for section in sections {(resource_form_section_view(c,section,options)?)}})}
pub fn resource_form_section_view(c:ResourceRenderContext<'_>,s:&ResourceFormSectionVM,o:&ResourceFormRenderOptions)->Result<Markup,super::ResourceRenderError> {
    Ok(html!{@for f in &s.fields {@if f.kind.as_str()==RESOURCE_FIELD_KIND_HIDDEN {input type="hidden" name=(resource_field_name(f)) value=(text(&f.value));}}
        @if resource_has_visible_fields(s){div class="space-y-4" {
            @if !s.title.trim().is_empty(){div class="space-y-1" {(heading::H2("lg",false,"font-semibold",html!{(&s.title)}))@if !s.description.trim().is_empty(){p class="text-sm text-muted-foreground" {(&s.description)}}}}
            div class="space-y-4" {@for f in &s.fields {@if f.kind.as_str()!=RESOURCE_FIELD_KIND_HIDDEN {(resource_form_field_view(c,f,o)?)}}}
        }}
    })
}
pub fn resource_form_field_view(c:ResourceRenderContext<'_>,f:&ResourceFieldVM,o:&ResourceFormRenderOptions)->Result<Markup,super::ResourceRenderError> {
    if f.relation.valid(){return Ok(html!{(relation_control(&f.relation))@if !f.help_text.trim().is_empty(){p id=(format!("{}-help",resource_field_id(f))) class="text-sm text-muted-foreground" {(f.help_text.trim())}}});}
    let help=first(&[&f.help_text,&f.description]);
    let input_props=input::InputProps{input_type:if f.kind.as_str()==RESOURCE_FIELD_KIND_URL{input::INPUT_TYPE_URL}else{input::INPUT_TYPE_TEXT},name:resource_field_name(f),id:resource_field_id(f),placeholder:f.placeholder.clone(),value:text(&f.value),required:f.required,disabled:f.disabled,error:f.error,error_message:f.error_message.clone(),show_error:f.error,aria_invalid:f.error,size:shared::SIZE_MD,full_width:!matches!(f.kind.as_str(),RESOURCE_FIELD_KIND_TEXT_AREA|RESOURCE_FIELD_KIND_COLLECTION),attributes:resource_attributes(&f.attrs),..Default::default()};
    Ok(match f.kind.as_str(){
        RESOURCE_FIELD_KIND_TYPED_VALUE=>form::Fieldset(&form::FieldsetProps{disabled:f.disabled,aria_invalid:f.error.to_string(),attributes:resource_attributes(&f.attrs),..Default::default()},html!{legend {(&f.label)} (super::prepared_field_editor(f)?) @if !help.is_empty(){p {(help)}} @if !f.error_message.is_empty(){p role="alert" {(&f.error_message)}}}),
        RESOURCE_FIELD_KIND_TAGS=>input::CommaSeparatedInputWithLabel(&input_props,&f.label,help),
        RESOURCE_FIELD_KIND_TEXT_AREA|RESOURCE_FIELD_KIND_COLLECTION=>input::TextAreaWithLabel(&input_props,&f.label,help,resource_field_rows(f,6) as i32),
        RESOURCE_FIELD_KIND_SELECT=>select::SelectWithLabel(&select::SelectProps{name:resource_field_name(f),id:resource_field_id(f),value:text(&f.value),required:f.required,disabled:f.disabled,error:f.error,error_message:f.error_message.clone(),size:shared::SIZE_MD,options:resource_select_options(&f.options),placeholder:f.placeholder.clone(),placeholder_disabled:false,attributes:resource_attributes(&f.attrs),..Default::default()},&f.label,help),
        RESOURCE_FIELD_KIND_LOCALIZED=>html!{@if let (Some(tabs),Some(render))=(&f.localized_tabs,&o.localized_tabs){(render(tabs.as_ref()))(resource_field_error_message(f))}},
        RESOURCE_FIELD_KIND_MEDIA=>html!{@if let (Some(manager),Some(render))=(&f.media_manager,&o.media_attachment_manager){(render(manager.as_ref()))(resource_field_error_message(f))}},
        RESOURCE_FIELD_KIND_DATE_TIME=>datetimepicker::DateTimePickerWithLabel(&datetimepicker::DateTimePickerProps{id:resource_field_id(f),name:resource_field_name(f),value:text(&f.value),time_zone_name:first(&[&f.secondary_name,"time_zone"]).into(),time_zone_value:f.secondary_value.clone(),size:shared::SIZE_MD,required:f.required,error:f.error,error_message:f.error_message.clone(),full_width:true,locale_aware_use_24_hour:true,locale_aware_week_starts_on:true,attributes:resource_attributes(&f.attrs),..Default::default()}.with_locale_preferences(c.t("component.datetimepicker.use_24_hour")!="false",resource_week_starts_on(&c.t("component.datetimepicker.week_starts_on"))),&f.label,help),
        RESOURCE_FIELD_KIND_DURATION=>durationpicker::DurationPickerWithLabel(&durationpicker::DurationPickerProps{id:resource_field_id(f),name:resource_field_name(f),value:text(&f.value),display_value:resource_field_display_value(f),placeholder:f.placeholder.clone(),helper_text:help.into(),required:f.required,error:f.error,error_message:f.error_message.clone(),size:shared::SIZE_MD,full_width:true,..Default::default()},&f.label,""),
        _=>input::InputWithLabel(&input_props,&f.label,help),
    })
}
pub fn resource_validation_summary(c:ResourceRenderContext<'_>,s:&FormStatusVM)->Markup {
    alert::Alert(&alert::AlertProps{variant:shared::VARIANT_DESTRUCTIVE,class:"border border-destructive bg-destructive-muted text-foreground".into(),role:"alert".into(),aria_live:"assertive".into(),..Default::default()},html!{div class="space-y-2" {div class="space-y-1" {p class="text-sm font-semibold text-destructive" {(c.t(crate::validation::VALIDATION_SUMMARY_TITLE_KEY))}@if !s.message.trim().is_empty(){p class="text-sm leading-6 text-foreground" {(&s.message)}}}ul class="list-disc space-y-1 pl-5 text-sm leading-6 text-foreground" {@for issue in &s.issues {@if !issue.message.trim().is_empty(){li {(&issue.message)}}}}}})
}
pub fn resource_field_error_message(f:&ResourceFieldVM)->Markup {html!{@if f.error&&!f.error_message.trim().is_empty(){p class="mt-1.5 text-xs leading-relaxed text-destructive" role="alert" {(&f.error_message)}}}}
pub fn media_resource_form_sections(c:ResourceRenderContext<'_>,sections:&[ResourceFormSectionVM],o:&ResourceFormRenderOptions)->Result<Markup,super::ResourceRenderError> {
    Ok(html!{@for s in sections {@for f in &s.fields {@if f.kind.as_str()==RESOURCE_FIELD_KIND_HIDDEN {input type="hidden" name=(resource_field_name(f)) value=(text(&f.value));}}
        @if resource_has_visible_fields(s){(resourceui::FormSection(&resourceui::FormSectionProps{title:s.title.clone(),description:s.description.clone(),body_class:media_resource_form_section_grid_class(s).into(),class:"media-resource-section".into(),..Default::default()},html!{@for f in &s.fields {@if f.kind.as_str()!=RESOURCE_FIELD_KIND_HIDDEN {div class=(media_resource_form_field_class(f)) {(resource_form_field_view(c,f,o)?)}}}}))}
    }})
}
pub fn media_resource_form_section_grid_class(s:&ResourceFormSectionVM)->&'static str {match s.key.as_str(){"identity"|"metadata"=>"grid gap-4 sm:grid-cols-2",_=>"space-y-4"}}
pub fn media_resource_form_field_class(f:&ResourceFieldVM)->&'static str {match resource_field_name(f).trim(){"url"|"title"|"description"|"alt"|"thumbnail_url"=>"sm:col-span-2",_=>""}}
pub fn media_resource_field_value(p:&ResourceFormPageVM,name:&str)->String {if name.trim().is_empty(){return String::new()}p.sections.iter().flat_map(|s|&s.fields).find(|f|resource_field_name(f).trim().eq_ignore_ascii_case(name.trim())).map(|f|text(f.value.trim())).unwrap_or_default()}
pub fn media_resource_preview_kind(p:&ResourceFormPageVM)->String {crate::media::assets::normalize_media_kind(media_resource_field_value(p,"kind"))}
pub fn media_resource_preview_url(p:&ResourceFormPageVM)->String {let thumb=media_resource_field_value(p,"thumbnail_url");if !thumb.trim().is_empty(){thumb.trim().into()}else if media_resource_preview_kind(p)=="image"{media_resource_field_value(p,"url").trim().into()}else{String::new()}}
pub fn media_resource_preview_meta(name:&str,label:&str,value:&str)->Markup {html!{div class="min-w-0" data-media-preview-meta=(name) hidden[value.trim().is_empty()] {dt class="text-xs font-medium uppercase tracking-wide text-muted-foreground" {(label)}dd class="mt-1 break-words font-mono text-xs text-foreground [overflow-wrap:anywhere]" {(value)}}}}
pub fn media_resource_preview_panel(c:ResourceRenderContext<'_>,p:&ResourceFormPageVM)->Markup {
    let preview=media_resource_preview_url(p);let alt=media_resource_field_value(p,"alt");let title=media_resource_field_value(p,"title");let fallback=c.t("media.preview.alt_fallback");let kind=media_resource_preview_kind(p);
    resourceui::Panel(&resourceui::PanelProps{title:c.t("media.inline_create.live_preview"),description:c.t("media.inline_create.preview_meta_hint"),body_class:"space-y-4 p-4".into(),no_padding:true,class:"media-resource-preview".into(),..Default::default()},html!{
        div class="flex aspect-video w-full items-center justify-center overflow-hidden rounded-lg border border-border bg-background text-muted-foreground" data-media-preview-frame data-alt-fallback=(fallback) {
            @if !preview.is_empty(){img src=(&preview) alt=(first(&[&alt,&title,&fallback])) class="h-full w-full object-cover";}
            span data-media-preview-fallback hidden[!preview.is_empty()] aria-hidden="true" {
                span data-media-preview-icon="image" hidden[kind!="image"] {(super::render_dashboard::media_dashboard_preview_icon("image"))}
                span data-media-preview-icon="file" hidden[kind=="image"] {(super::render_dashboard::media_dashboard_preview_icon("document"))}
            }
        }
        dl class="space-y-3 text-sm" {@for (name,key) in [("kind","media.form.kind_label"),("source_kind","media.form.source_kind_label"),("status","media.form.status_label"),("url","media.form.url_label")] {(media_resource_preview_meta(name,&c.t(key),&media_resource_field_value(p,name)))}}
    })
}

fn resource_week_starts_on(value:&str)->usize{match value.trim().to_lowercase().as_str(){"monday"|"mon"=>1,"tuesday"|"tue"=>2,"wednesday"|"wed"=>3,"thursday"|"thu"=>4,"friday"|"fri"=>5,"saturday"|"sat"=>6,_=>0}}
