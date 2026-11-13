use std::collections::BTreeMap;
use maud::Markup;
use crate::{forms::*,rendering::*,dashboards::*,definitions::*,routing::ManagementBreadcrumbVM,commands::SubmitActionVM,attachments::MediaAttachmentManagerProps};
use web_components::components::{shared,unstyled::select};
use super::render_contracts::*;
pub const FORM_SUBMIT_ACTION_FIELD_NAME:&str="submit_action";
pub const FORM_SUBMIT_ACTION_VALUE_DRAFT:&str="save-draft";
pub const FORM_SUBMIT_ACTION_VALUE_PUBLISH:&str="publish";
pub(super) fn text(value:impl AsRef<[u8]>)->String {crate::byte_text::encoding_text(value.as_ref())}
pub(super) fn first<'a>(values:&[&'a str])->&'a str {values.iter().map(|s|s.trim()).find(|s|!s.is_empty()).unwrap_or("")}
pub fn resource_attributes(values:&BTreeMap<String,String>)->BTreeMap<String,String> {values.iter().filter(|(k,_)|!k.trim().is_empty()).map(|(k,v)|(k.clone(),v.clone())).collect()}
pub fn resource_form_page_attributes(p:&ResourceFormPageVM,build:Option<&ResourceFormAttributesBuilder>)->BTreeMap<String,String> {build.map(|f|f(p)).unwrap_or_default()}
pub fn resource_form_action_props(actions:&[SubmitActionVM])->Vec<ManagementFormActionProps> {
    let publication=actions.iter().any(|a|matches!(a.action.as_str(),RESOURCE_ACTION_DRAFT|RESOURCE_ACTION_PUBLISH));
    actions.iter().filter(|a|!resource_form_action_hidden(&a.action,publication)).map(|a|ManagementFormActionProps{name:FORM_SUBMIT_ACTION_FIELD_NAME.into(),value:resource_form_action_value(&a.action),label:a.label.clone(),icon:resource_form_action_icon(&a.action).into(),variant:resource_form_action_variant(&a.action)}).collect()
}
pub fn resource_form_action_hidden(a:&ResourceAction,publication:bool)->bool {match a.as_str(){RESOURCE_ACTION_TRANSLATE|RESOURCE_ACTION_ATTACH|RESOURCE_ACTION_DELETE=>true,RESOURCE_ACTION_CREATE|RESOURCE_ACTION_UPDATE=>publication,_=>false}}
pub fn resource_form_action_value(a:&ResourceAction)->String {match a.as_str(){RESOURCE_ACTION_DRAFT=>FORM_SUBMIT_ACTION_VALUE_DRAFT,RESOURCE_ACTION_PUBLISH=>FORM_SUBMIT_ACTION_VALUE_PUBLISH,other=>other}.into()}
pub fn resource_form_action_icon(a:&ResourceAction)->&'static str {match a.as_str(){RESOURCE_ACTION_CREATE=>"plus",RESOURCE_ACTION_UPDATE=>"edit",RESOURCE_ACTION_DRAFT=>"file",RESOURCE_ACTION_PUBLISH=>"check",_=>""}}
pub fn resource_form_action_variant(a:&ResourceAction)->&'static str {match a.as_str(){RESOURCE_ACTION_CREATE|RESOURCE_ACTION_UPDATE|RESOURCE_ACTION_PUBLISH=>shared::VARIANT_PRIMARY,_=>shared::VARIANT_OUTLINE}}
pub fn resource_field_options_from_records(records:&[ResourceFieldOptionRecord])->Vec<ResourceFieldOptionVM> {records.iter().filter_map(|r|{let value=r.value.trim();if value.is_empty(){None}else{Some(ResourceFieldOptionVM{value:value.into(),label:first(&[&r.label,value]).into(),attrs:r.attrs.clone(),..Default::default()})}}).collect()}
pub fn resource_select_options(options:&[ResourceFieldOptionVM])->Vec<select::SelectOption> {options.iter().map(|o|select::SelectOption{value:o.value.clone(),label:o.label.clone(),selected:o.selected,attributes:resource_attributes(&o.attrs),..Default::default()}).collect()}
pub fn resource_select_data_sync_attrs(fields:&[SelectOptionDataSyncField])->BTreeMap<String,String> {let action=select_option_data_sync_action(fields);if action.trim().is_empty(){BTreeMap::new()}else{[("data-on:load".into(),action.clone()),("data-on:change".into(),action)].into()}}
pub fn select_option_data_sync_action(fields:&[SelectOptionDataSyncField])->String {
    let targets:Vec<_>=fields.iter().filter(|f|!f.field_id.trim().is_empty()).map(|f|format!("['{}','{}',{}]",js_string(f.field_id.trim()),js_string(f.data_key.trim()),f.use_selected_value)).collect();
    if targets.is_empty(){return String::new()}
    ["if (!(el instanceof HTMLSelectElement)) return;","const form = el.closest('form');","if (!(form instanceof HTMLFormElement)) return;","const selectedOption = el.selectedOptions.length > 0 ? el.selectedOptions[0] : null;","const selectedValue = String(el.value ?? '').trim();","const hasSelection = selectedValue !== '';","const syncField = (field, nextValue, lock, shouldWrite) => {","if (!(field instanceof HTMLInputElement)) return;","if (shouldWrite) field.value = nextValue;","field.readOnly = lock;","field.setAttribute('aria-readonly', lock ? 'true' : 'false');","field.classList.toggle('bg-muted/40', lock);","field.classList.toggle('cursor-not-allowed', lock);","field.dispatchEvent(new Event('input', { bubbles: true }));","field.dispatchEvent(new Event('change', { bubbles: true }));","};",&format!("[{}].forEach(([fieldID, dataKey, useSelectedValue]) => {{",targets.join(",")),"const field = form.querySelector('#' + CSS.escape(fieldID));","const nextValue = useSelectedValue ? selectedValue : (hasSelection && selectedOption instanceof HTMLOptionElement ? String(selectedOption.dataset[dataKey] || '') : '');","const shouldWrite = hasSelection && (useSelectedValue || dataKey !== '');","syncField(field, nextValue, hasSelection, shouldWrite);","});"].join(" ")
}
pub fn resource_form_localized_tabs(p:&ResourceFormPageVM)->Vec<LocalizedTabsProps> {p.sections.iter().flat_map(|s|&s.fields).filter_map(|f|f.localized_tabs.as_ref().map(|v|v.as_ref().clone())).collect()}
pub fn resource_form_media_managers(p:&ResourceFormPageVM)->Vec<MediaAttachmentManagerProps> {p.sections.iter().flat_map(|s|&s.fields).filter_map(|f|f.media_manager.as_ref().map(|v|v.as_ref().clone())).collect()}
pub fn resource_list_has_items(p:&ResourceListPageVM)->bool {p.sections.iter().any(|s|!s.items.is_empty())}
pub fn resource_list_section_items<'a>(sections:&'a [ResourceListSectionVM],key:&ResourceSectionKey)->&'a [ResourceListItemVM] {sections.iter().find(|s|s.key==*key).map(|s|s.items.as_slice()).unwrap_or(&[])}
pub fn resource_detail_section_items<'a>(sections:&'a [ResourceDetailSectionVM],key:&ResourceSectionKey)->&'a [ResourceListItemVM] {sections.iter().find(|s|s.key==*key).map(|s|s.items.as_slice()).unwrap_or(&[])}
pub fn resource_meta_value(meta:&[ResourceMetaVM],index:i64)->String {usize::try_from(index).ok().and_then(|i|meta.get(i)).map(|m|m.value.clone()).unwrap_or_default()}
pub fn resource_first_href(item:&ResourceListItemVM)->String {first(&[&item.detail_href,&item.external_href,&item.action_href]).into()}
pub fn resource_list_section_classes(key:&ResourceSectionKey)->&'static str {match key.as_str(){"featured"=>"grid gap-4","secondary"=>"grid gap-4 md:grid-cols-2",_=>"grid gap-4 md:grid-cols-2 xl:grid-cols-3"}}
pub fn resource_has_visible_fields(section:&ResourceFormSectionVM)->bool {section.fields.iter().any(|f|f.kind.as_str()!=RESOURCE_FIELD_KIND_HIDDEN)}
pub fn resource_field_name(field:&ResourceFieldVM)->String {
    if !field.name.trim().is_empty(){return field.name.trim().into()}
    match field.key.as_str(){RESOURCE_FIELD_KEY_CONTENT_ID=>"content_id",RESOURCE_FIELD_KEY_DATE_TIME=>"date_time",RESOURCE_FIELD_KEY_TIME_ZONE=>"time_zone",other=>other}.into()
}
pub fn resource_field_id(f:&ResourceFieldVM)->String {if !f.input_id.trim().is_empty(){f.input_id.trim().into()}else{resource_field_name(f).replace('_',"-")}}
pub fn resource_field_display_value(f:&ResourceFieldVM)->String {if !f.display_value.trim().is_empty(){f.display_value.trim().into()}else{text(f.value.trim())}}
pub fn resource_field_rows(f:&ResourceFieldVM,fallback:i64)->i64 {if f.rows>0{f.rows}else{fallback}}
pub fn resource_page_title(title:&str,breadcrumb:&ManagementBreadcrumbVM)->String {first(&[title,&breadcrumb.current_label]).into()}
pub fn resource_delete_button_attrs(confirm:&str)->BTreeMap<String,String> {if confirm.trim().is_empty(){BTreeMap::new()}else{[("onclick".into(),format!("return confirm('{}')",js_string(confirm)))].into()}}
pub fn render_component_html(component:Option<&Markup>)->String {component.map(|m|m.clone().into_string().trim().to_owned()).unwrap_or_default()}
pub fn resource_components<T>(values:&[T],render:impl Fn(usize,&T)->Option<Markup>)->Vec<Markup> {values.iter().enumerate().filter_map(|(i,v)|render(i,v)).collect()}
pub fn resource_map_values<T,U>(values:&[T],build:impl Fn(&T)->Option<U>)->Vec<U> {values.iter().filter_map(build).collect()}
pub fn resource_tag_values<T>(tags:&[String],build:impl Fn(&str)->T)->Vec<T> {tags.iter().map(|v|v.trim()).filter(|v|!v.is_empty()).map(build).collect()}
pub fn resource_dashboard_section_has_content(s:&ResourceDashboardSectionVM)->bool {s.metrics.as_ref().is_some_and(|v|!v.is_empty())||s.items.as_ref().is_some_and(|v|!v.is_empty())||s.actions.as_ref().is_some_and(|v|!v.is_empty())}
pub fn resource_dashboard_actions(actions:&[DashboardActionVM],location:&DashboardActionLocation)->Vec<DashboardActionVM> {actions.iter().filter(|a|a.location==*location&&!a.label.trim().is_empty()&&!a.href.trim().is_empty()).cloned().collect()}
pub fn resource_dashboard_action_variant(index:usize)->&'static str {match index{0=>shared::VARIANT_PRIMARY,1=>shared::VARIANT_OUTLINE,_=>shared::VARIANT_GHOST}}
pub fn resource_nested_item_classes(depth:i64)->&'static str {if depth>0{"space-y-3"}else{"space-y-4"}}
pub fn js_string(value:&str)->String {value.replace('\\',"\\\\").replace('\'',"\\'").replace('\n',"\\n").replace('\r',"")}
pub(super) fn shared_values<T:Clone>(values:&Option<SharedList<T>>)->Vec<T> {values.as_ref().map(|v|v.with(|items|items.to_vec())).unwrap_or_default()}
