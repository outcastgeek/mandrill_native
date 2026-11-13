use maud::{html,Markup};
use web_components::components::shared;
use web_components::components::unstyled::{heading,emptystate,badge};
use web_components::components::unstyled::{dialog,icons,input,select};
use super::*;
use super::render::{attrs,hidden,get_form,post_form,media_button,MediaRenderContext};

pub fn media_picker(c:MediaRenderContext<'_>,p:&MediaPickerPageVM,o:&MediaPickerRenderOptions)->Markup {
    let id=format!("{}-dialog",p.root_id);
    let dismiss=format!("{}; {}",media_picker_dismiss_action("mediaPickerOpen",&p.root_id),media_picker_focus_fallback_action(&p.target_id,&p.role_default));
    let payload=|| o.attachment_payload_inputs.as_ref().map(|f|f(&p.current_attachments)).unwrap_or_default();
    let content=html! {
        div class="flex shrink-0 items-start justify-between gap-3 border-b border-border px-5 py-4" {
            div class="flex min-w-0 items-start gap-3" {
                span class="mt-0.5 flex h-8 w-8 shrink-0 items-center justify-center rounded-md bg-primary/10 text-primary" {(icons::GridFourFilled(&icons::IconProps{class:"w-4 h-4".into(),..Default::default()}))}
                div class="min-w-0 space-y-1" {(heading::H2("base",false,"text-[1.05rem] font-semibold leading-tight tracking-[-0.02em] text-foreground",html!{(c.t("media.picker.title"))}))
                    @if !p.context_label.trim().is_empty(){(badge::Badge(&badge::BadgeProps{variant:shared::VARIANT_OUTLINE,size:shared::SIZE_SM,pill:true,class:"border-border bg-muted/20 text-muted-foreground".into(),..Default::default()},html!{(&p.context_label)}))}
                }
            }
            div class="flex shrink-0 items-center gap-2" {
                (get_form(&p.create_url,"contents","",html!{(media_picker_hidden_fields(p))(media_picker_state_hidden_fields(p))(payload())(media_picker_create_button(c))}))
                (dialog::DialogClose(&dialog::DialogCloseProps {dialog_id:id.clone(),signal:"mediaPickerOpen".into(),after_close_action:format!("{}; {}",media_picker_mount_clear_action(&p.root_id),media_picker_focus_fallback_action(&p.target_id,&p.role_default)),show_icon:true,attributes:attrs(&[("style","position:static")]),class:"rounded-full border border-border text-muted-foreground hover:bg-muted hover:text-foreground".into(),..Default::default()},Markup::default()))
            }
        }
        (get_form(&p.filter_action,"shrink-0 border-b border-border px-5 py-4",&p.filter_form_id,html!{
            (media_picker_hidden_fields(p))(hidden("status",&p.status))(payload())(hidden("page","1"))
            div class="space-y-3" {
                @if !p.error_msg.trim().is_empty(){div role="alert" class="rounded-md border border-destructive/30 bg-destructive-muted px-3 py-2 text-sm text-destructive-foreground" {(&p.error_msg)}}
                div class="grid gap-3 sm:grid-cols-[minmax(0,1fr)_12rem]" {
                    (input::Input(&input::InputProps {input_type:input::INPUT_TYPE_SEARCH,name:"search".into(),value:p.search.clone(),placeholder:c.t("media.picker.search_placeholder"),size:shared::SIZE_MD,full_width:true,class:"h-10 rounded-md px-4 text-sm".into(),aria_label:c.t("media.picker.search_placeholder"),..Default::default()}))
                    (select::Select(&select::SelectProps {name:"sort".into(),value:media_picker_sort_value(&p.sort),size:shared::SIZE_MD,class:"h-10 rounded-md".into(),options:[("newest","media.picker.sort_newest"),("oldest","media.picker.sort_oldest"),("alpha","media.picker.sort_alpha")].into_iter().map(|(v,k)|select::SelectOption::new(c.t(k),v)).collect(),attributes:attrs(&[("data-on:change",&p.filter_submit_action)]),aria_label:c.t("media.picker.sort_label"),..Default::default()}))
                }
                @if !p.filter_pills.is_empty(){div class="flex flex-wrap items-center gap-2" {@for pill in &p.filter_pills {(media_button(html!{(&pill.label)},"submit",shared::VARIANT_GHOST,&pill.class,attrs(&[("name","kind"),("value",&pill.value),("form",&p.filter_form_id)]),false))}}}
            }
        }))
        (post_form(&p.attach_url,"flex min-h-0 flex-1 flex-col overflow-hidden",&p.attach_form_id,&o.csrf_token,&o.csrf_field_name,html!{
            (media_picker_hidden_fields(p))(media_picker_state_hidden_fields(p))(hidden("page",&p.page.to_string()))(payload())
            div class="flex min-h-0 flex-1 flex-col px-5 py-4" {
                @if p.items.is_empty(){div class="flex h-full items-center justify-center" {(emptystate::EmptyState(&emptystate::EmptyStateProps {title:c.t("media.picker.empty_title"),description:c.t("media.picker.empty_description"),..Default::default()},Markup::default()))}}
                @else {div class="media-ui-picker-grid" {@for item in &p.items {(media_picker_grid_card(p,item,o))}}}
            }
            div class="shrink-0 border-t border-border bg-card px-5 py-4" {div class="flex flex-wrap items-center justify-between gap-4" {
                @if let Some(pagination)=&o.pagination {(pagination(MediaPickerPaginationProps{form_id:p.filter_form_id.clone(),page:p.page,page_size:p.page_size,total:p.total,pages:p.pages,item_count:p.items.len() as i64,previous_label:c.t("common.pagination.previous"),next_label:c.t("common.pagination.next"),class:"flex-1 flex flex-wrap items-center justify-between gap-4 min-w-0".into()}))}
                div class="flex flex-wrap items-center gap-2" {
                    (media_button(html!{(c.t("common.cancel"))},"button",shared::VARIANT_GHOST,"text-muted-foreground hover:bg-muted hover:text-foreground",attrs(&[("data-on:click",&dismiss)]),false))
                    (media_button(html!{(c.t("media.picker.attach_selected"))},"submit",shared::VARIANT_PRIMARY,"shadow-none",attrs(&[("form",&p.attach_form_id)]),false))
                }
            }}
        }))
    };
    html!{div data-signals=(&p.signals) data-on:load=(media_scroll_lock_action()) {
        (dialog::Dialog(&dialog::DialogProps{id:id.clone(),open:true,signal:"mediaPickerOpen".into(),class:"media-ui-overlay z-[1000001]".into(),role:"dialog".into(),aria_label:c.t("media.picker.title"),..Default::default()},html!{
            (dialog::DialogOverlay(&dialog::DialogOverlayProps{dialog_id:id.clone(),class:"z-[1000002] bg-background/70 backdrop-blur-sm".into(),attributes:attrs(&[("style","z-index:1000002;"),("data-on:click",&dismiss)])}))
            (dialog::DialogContent(&dialog::DialogContentProps{dialog_id:id.clone(),size:shared::SIZE_LG,disable_full_width:true,class:"media-ui-picker-dialog z-[1000003] flex flex-col overflow-hidden rounded-[0.95rem] border border-border bg-card p-0 shadow-xl".into(),attributes:attrs(&[("style","z-index:1000003;")]),..Default::default()},content))
        }))
    }}
}
pub fn media_picker_hidden_fields(p:&MediaPickerPageVM)->Markup {html!{@for (k,v) in [("root_id",&p.root_id),("create_root_id",&p.create_root_id),("scope",&p.scope),("target",&p.target_id),("owner_form_id",&p.owner_form_id),("owner_content_id",&p.owner_content_id),("role",&p.role_default)] {(hidden(k,v))}}}
pub fn media_picker_state_hidden_fields(p:&MediaPickerPageVM)->Markup {html!{(hidden("search",&p.search))(hidden("kind",&p.kind))(hidden("status",&p.status))(hidden("sort",&media_picker_sort_value(&p.sort)))}}
fn media_picker_grid_card(p:&MediaPickerPageVM,item:&MediaPickerItemVM,o:&MediaPickerRenderOptions)->Markup {html!{label id=(format!("{}-choice-{}",p.root_id,item.id)) class=(&item.card_class) data-media-selected=(&item.selected_attr) data-media-card="true" {input class="sr-only peer" type=(&item.selection_type) name=(&item.selection_name) value=(&item.id) form=(&p.attach_form_id) checked[item.checked] disabled[item.disabled]; @if let Some(card)=&o.item_card {(card(item.clone()))} @else {div class="min-w-0 flex-1 space-y-1 px-2.5 py-2 pointer-events-none" {p class="truncate text-[0.72rem] font-medium leading-[1.05rem] text-foreground" {(item.title.trim())} p class="truncate text-[0.64rem] text-muted-foreground" {(item.meta.trim())}}}}}}
pub fn media_picker_focus_fallback_action(section_id:&str,role:&str)->String {format!("queueMicrotask(() => {{ if (document.activeElement === document.body) {{ document.getElementById('{}')?.querySelector('[aria-controls=\"{}-body\"]')?.focus(); }} }})",js_string(section_id),if role.trim()=="hero"{"hero"}else{"additional"})}
pub fn media_picker_return_focus(section_id:&str,role:&str)->Markup {let action=format!("queueMicrotask(() => {{ const target = document.querySelector('.media-ui-picker-dialog [data-media-picker-create]') || document.getElementById('{}')?.querySelector('[aria-controls=\"{}-body\"]'); target?.focus(); el.remove(); }}); {}",js_string(section_id),if role.trim()=="hero"{"hero"}else{"additional"},media_scroll_unlock_when_idle_action());html!{span hidden data-effect=(action) {}}}

/// Picker-specific focus target; generic button styling remains owned by web_components.
fn media_picker_create_button(c: MediaRenderContext<'_>) -> Markup {
    let props = web_components::components::unstyled::button::ButtonProps {
        button_type: "submit", variant: shared::VARIANT_OUTLINE, size: shared::SIZE_SM,
        class: "border-border bg-background text-foreground hover:border-primary/35 hover:bg-primary/5 hover:text-primary".into(),
        ..Default::default()
    };
    html! { button type="submit" class=(props.build_classes()) data-media-picker-create="true" {
        (c.t("media.picker.new_resource"))
    } }
}
