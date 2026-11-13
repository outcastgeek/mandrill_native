use maud::{html,Markup};
use crate::attachments::{MediaAttachmentManagerProps,MediaAttachmentFormItem,MediaSurfaceMounts,normalize_media_attachment_role};
use web_components::components::shared;
use web_components::components::unstyled::{card,badge};
use web_components::components::unstyled::{collapsible,icons,input,select};
use super::*;
use super::render::{attrs,first,text,hidden,media_button,MediaRenderContext};

pub fn media_modal_roots(m:&MediaSurfaceMounts)->Markup {html!{(media_mount_root(&text(&m.picker_root_id)))(media_mount_root(&text(&m.create_root_id)))(media_mount_root(&text(&m.shortcode_root_id)))}}
pub fn media_attachment_manager(c:MediaRenderContext<'_>,p:&MediaAttachmentManagerProps)->Markup {html!{div id=(text(&p.mounts.section_id)) class="space-y-4" data-signals="{ mediaHeroOpen: false, mediaAdditionalOpen: false, mediaDraggedId: '', mediaDropTargetId: '', mediaDropPlacement: '' }" {(attachment_section(c,p,true))(attachment_section(c,p,false))}}}
fn attachment_section(c:MediaRenderContext<'_>,p:&MediaAttachmentManagerProps,hero:bool)->Markup {
    let key=if hero{"hero"}else{"additional"};let signal=if hero{"mediaHeroOpen"}else{"mediaAdditionalOpen"};
    let items:Vec<_>=p.attachments.iter().filter(|i|(normalize_media_attachment_role(&i.role)=="hero")==hero).take(if hero {1}else{usize::MAX}).collect();
    let title=c.t(&format!("media.attachments.{key}.title"));let summary=if items.is_empty(){c.t("media.attachments.count_empty")}else if hero||items.len()==1{c.t("media.attachments.count_one")}else{c.t_args("media.attachments.count_many",&[items.len().to_string()])};
    let trigger=collapsible::CollapsibleTrigger(&collapsible::CollapsibleTriggerProps {class:"media-ui-section-trigger".into(),attributes:attrs(&[("aria-controls",&format!("{key}-body")),("data-on:click",&format!("${signal} = !${signal}")),("data-attr:aria-expanded",&format!("${signal} ? 'true' : 'false'")),("data-attr:data-state",&format!("${signal} ? 'open' : 'closed'")),("data-attr:data-expanded",&format!("${signal} ? 'true' : 'false'"))]),..Default::default()},html!{
        div class="flex min-w-0 items-center gap-3" {span class="flex h-9 w-9 shrink-0 items-center justify-center rounded-md border border-border bg-muted/15 text-muted-foreground" {
            @if hero {(icons::Image(&icons::IconProps{class:"w-[15px] h-[15px]".into(),..Default::default()}))} @else {(icons::GridDots(&icons::IconProps{class:"w-[15px] h-[15px]".into(),..Default::default()}))}
        } div class="min-w-0" {div class="flex flex-wrap items-center gap-2" {h3 class="text-[0.98rem] font-semibold tracking-[-0.02em] text-foreground" {(title)}
            (badge::Badge(&badge::BadgeProps{variant:shared::VARIANT_OUTLINE,size:shared::SIZE_SM,pill:true,class:if items.is_empty(){"border-border bg-muted/20 text-muted-foreground"}else{"border-primary/30 bg-primary/10 text-primary"}.into(),..Default::default()},html!{(summary)}))
        } p class="mt-0.5 text-sm leading-6 text-muted-foreground" {(c.t(&format!("media.attachments.{key}.subtitle")))}}}
        span class="flex shrink-0 items-center gap-2 text-xs font-medium text-muted-foreground" {span data-text=(format!("${signal} ? '{}' : '{}'",js_string(&c.t("media.attachments.collapse_section")),js_string(&c.t("media.attachments.expand_section")))) {(c.t("media.attachments.expand_section"))}
            (icons::ChevronDown(&icons::IconProps{class:"h-4 w-4 transition-transform".into(),attributes:attrs(&[("data-class:rotate-180",&format!("${signal}"))]),..Default::default()}))
        }
    });
    let body=html!{div class="space-y-3" {
        @if items.is_empty(){div class="media-ui-empty-inline" {div class="min-w-0 space-y-1" {p class="text-sm font-semibold text-foreground" {(c.t(&format!("media.attachments.{key}.empty_title")))} p class="text-sm leading-6 text-muted-foreground" {(c.t(&format!("media.attachments.{key}.empty_description")))}} div class="flex shrink-0 flex-wrap items-center gap-2 sm:justify-end" {(section_buttons(c,p,hero,true))}}}
        @else {
            @if hero {div class="space-y-2" {p class="text-[11px] font-semibold uppercase tracking-[0.16em] text-muted-foreground" {(c.t("media.attachments.hero.current_label"))} @for (idx,item) in items.iter().enumerate() {(attachment_row(c,p,hero,item,idx))}}}
            @else {div class="media-ui-attachment-list" data-media-attachment-list="true" {@for (idx,item) in items.iter().enumerate(){(attachment_row(c,p,hero,item,idx))}}}
            div class="media-ui-section-footer" {p class="text-sm leading-6 text-muted-foreground" {(c.t(if hero{"media.attachments.hero.replace_hint"}else{"media.attachments.additional.footer_hint"}))} div class="flex flex-wrap items-center gap-2 sm:justify-end" {(section_buttons(c,p,hero,false))}}
        }
    }};
    card::Card(&card::CardProps {class:"overflow-hidden rounded-[0.85rem] border border-border bg-card shadow-none".into(),..Default::default()},collapsible::Collapsible(&collapsible::CollapsibleProps {class:"w-full".into(),..Default::default()},html!{(trigger)(collapsible::CollapsibleContent(&collapsible::CollapsibleContentProps{class:"border-t border-border bg-card px-4 py-4 sm:px-5".into(),attributes:attrs(&[("id",&format!("{key}-body")),("data-show",&format!("${signal}")),("style","display:none"),("data-attr:data-state",&format!("${signal} ? 'open' : 'closed'"))]),..Default::default()},body))}))
}
fn section_buttons(c:MediaRenderContext<'_>,p:&MediaAttachmentManagerProps,hero:bool,empty:bool)->Markup {
    let role=if hero{"hero"}else{"gallery"};let picker=media_action_url(&p.picker_url,role);let create=media_action_url(&p.create_url,role);
    html!{(open_button(&c.t(if hero&&!empty{"media.attachments.change"}else{"media.attachments.attach_existing"}),&picker,&text(&p.mounts.owner_form_id),shared::VARIANT_OUTLINE))
        @if !hero||empty {(open_button(&c.t("media.attachments.new_media"),&create,&text(&p.mounts.owner_form_id),shared::VARIANT_PRIMARY))}
    }
}
fn open_button(label:&str,url:&str,owner:&str,variant:&'static str)->Markup {media_fragment_button(&MediaFragmentButtonProps{label:label.into(),url:url.into(),owner_form_id:owner.into(),variant:variant.into(),size:"sm".into(),button_class:if variant==shared::VARIANT_PRIMARY{"shadow-none"}else{"border-border bg-background text-foreground hover:border-primary/35 hover:bg-primary/5 hover:text-primary"}.into(),..Default::default()})}
pub fn media_attachment_preview_url(item:&MediaAttachmentFormItem)->String {
    let thumb=text(&item.resolved_thumbnail);let thumb_url=text(&item.thumbnail_url);let resolved=text(&item.resolved_url);let url=text(&item.url);
    if item.kind.trim().eq_ignore_ascii_case("document"){first(&[&thumb,&thumb_url]).into()}else{first(&[&thumb,&resolved,&thumb_url,&url]).into()}
}
fn role_label(c:MediaRenderContext<'_>,role:&str)->String {c.t(match role{"inline"=>"media.role.inline","download"=>"media.role.download","hero"=>"media.attachments.hero.title",_=>"media.role.gallery"})}
fn input_id(key:&str,item_id:&str,index:usize,suffix:&str)->String {
    let value=crate::byte_text::lowercase(format!("media-{key}-{index}-{}-{suffix}",item_id.trim()).as_bytes());let mut id=String::new();let mut dash=false;
    for ch in value.chars(){if ch.is_ascii_alphanumeric(){id.push(ch);dash=false}else if !dash{id.push('-');dash=true}}
    id.trim_matches('-').into()
}
fn attachment_row(c:MediaRenderContext<'_>,p:&MediaAttachmentManagerProps,hero:bool,item:&MediaAttachmentFormItem,index:usize)->Markup {
    let key=if hero{"hero"}else{"additional"};let id=text(&item.asset_id);let title=text(&item.title);let display=first(&[&title,&id]);let alt=text(&item.alt);let alt_override=text(&item.alt_override);let shortcode=text(&item.shortcode);
    let provider=text(&item.provider);let supporting=[provider.trim(),item.status.trim(),id.trim()].into_iter().filter(|v|!v.is_empty()).collect::<Vec<_>>().join(" • ");
    let role=if hero{"hero".into()}else{let r=normalize_media_attachment_role(&item.role);if r.is_empty()||r=="hero"{"gallery".into()}else{r}};
    let panel_id=format!("media-{key}-detail-{index}-{}",id.trim());
    let draggable=!hero&&!id.trim().is_empty();let escaped=js_string(id.trim());
    let detail_action=format!("$detailsOpen ? '{}' : '{}'",js_string(&format!("{}: {display}",c.t("media.attachments.hide_details"))),js_string(&format!("{}: {display}",c.t("media.attachments.edit_details"))));
    let detail_labels=format!("$detailsOpen ? '{}' : '{}'",js_string(&c.t("media.attachments.hide_details")),js_string(&c.t("media.attachments.edit_details")));
    html! {div class="media-ui-attach-item" data-signals="{ detailsOpen: false }"
        data-media-attachment-item=[draggable.then_some(id.trim())] draggable=[draggable.then_some("true")]
        data-on:dragstart=[draggable.then(||media_attachment_drag_start_action(&id))]
        data-on:dragover=[draggable.then(||media_attachment_drag_over_action(&id))]
        data-on:drop=[draggable.then(||media_attachment_drop_action(&id))]
        data-on:dragend=[draggable.then_some("$mediaDraggedId = ''; $mediaDropTargetId = ''; $mediaDropPlacement = ''")]
        data-attr:aria-grabbed=[draggable.then(||format!("$mediaDraggedId === '{escaped}' ? 'true' : 'false'"))]
        data-attr:data-dragging=[draggable.then(||format!("$mediaDraggedId === '{escaped}' ? 'true' : 'false'"))]
        data-attr:data-drop-target=[draggable.then(||format!("$mediaDropTargetId === '{escaped}' ? 'true' : 'false'"))]
        data-attr:data-drop-placement=[draggable.then(||format!("$mediaDropTargetId === '{escaped}' ? $mediaDropPlacement : ''"))]
    {
        (hidden("attachment_ids[]",&id))(hidden("attachment_positions[]",&index.to_string()))
        div class="media-ui-attach-row" {
            @if !hero {span class="media-ui-drag-handle" aria-hidden="true" title=(c.t("media.attachments.drag_hint")) {(icons::GripDots(&icons::IconProps{class:"w-[10px] h-4".into(),..Default::default()}))}}
            div class="media-ui-attach-thumb" {(media_preview_frame(c.request,c.translate,&media_attachment_preview_url(item),first(&[&alt_override,&alt,&title]),&item.kind,"h-full w-full rounded-none border-0 object-cover",&c.t("media.preview.alt_fallback")))}
            div class="media-ui-attach-meta" {div class="flex flex-wrap items-center gap-2" {p class="truncate text-sm font-medium text-foreground" {(display)}(media_type_badge(c.request,c.translate,&item.kind,"")) @if !hero&&role!="gallery" {(badge::Badge(&badge::BadgeProps{variant:shared::VARIANT_OUTLINE,size:shared::SIZE_SM,pill:true,class:"border-border bg-muted/20 text-muted-foreground".into(),..Default::default()},html!{(role_label(c,&role))}))}} p class="truncate text-xs text-muted-foreground" {(supporting)}}
            @if !hero&&!shortcode.trim().is_empty(){code class="hidden shrink-0 rounded-md border border-primary/25 bg-primary/10 px-2 py-1 font-mono text-[10px] font-medium text-primary sm:inline-block" title=(shortcode.trim()) {(shortcode.trim())}}
            div class="flex shrink-0 flex-wrap items-center gap-2" {
                (media_button(html!{(icons::Edit(&icons::IconProps{class:"w-[14px] h-[14px]".into(),..Default::default()})) span class="sr-only" data-text=(detail_labels) {(c.t("media.attachments.edit_details"))}},"button",shared::VARIANT_GHOST,"h-8 w-8 rounded-full p-0 text-muted-foreground hover:bg-muted hover:text-foreground",attrs(&[("data-on:click","$detailsOpen = !$detailsOpen"),("data-attr:aria-expanded","$detailsOpen ? 'true' : 'false'"),("aria-controls",&panel_id),("aria-label",&format!("{}: {display}",c.t("media.attachments.edit_details"))),("data-attr:aria-label",&detail_action)]),false))
                (media_button(html!{(icons::Trash(&icons::IconProps{class:"w-[14px] h-[14px]".into(),..Default::default()}))span class="sr-only" {(c.t("media.attachments.remove"))}},"button",shared::VARIANT_GHOST,"h-8 w-8 rounded-full p-0 text-destructive hover:bg-destructive-muted hover:text-destructive-foreground",attrs(&[("data-on:click",&media_mutation_action(&media_mutation_url(p,"remove",&item.asset_id),&text(&p.mounts.owner_form_id))),("aria-label",&format!("{}: {display}",c.t("media.attachments.remove")))]),false))
            }
        }
        div id=(panel_id) class="media-ui-detail-drawer" data-show="$detailsOpen" style="display:none" {
            (hidden("attachment_titles[]",text(&item.title_override).trim()))
            div class="grid gap-3 md:grid-cols-2" {
                div class="md:col-span-2" {(input::InputWithLabel(&input::InputProps{name:"attachment_captions[]".into(),id:input_id(key,&id,index,"caption"),value:text(&item.caption),size:shared::SIZE_MD,full_width:true,class:"bg-background".into(),..Default::default()},&c.t("media.shortcode.field_caption"),""))}
                (input::InputWithLabel(&input::InputProps{name:"attachment_alts[]".into(),id:input_id(key,&id,index,"alt"),value:alt_override,size:shared::SIZE_MD,full_width:true,class:"bg-background".into(),..Default::default()},&c.t("media.inline_create.alt_label"),""))
                @if !hero {(select::SelectWithLabel(&select::SelectProps{name:"attachment_roles[]".into(),id:input_id(key,&id,index,"role"),value:role.clone(),size:shared::SIZE_MD,options:["gallery","inline","download"].into_iter().map(|v|select::SelectOption::new(role_label(c,v),v)).collect(),class:"bg-background".into(),..Default::default()},&c.t("media.fields.role"),""))}
                @else {(hidden("attachment_roles[]",&role)) div class="space-y-1.5" {label class="text-xs font-medium uppercase tracking-[0.12em] text-muted-foreground" {(c.t("media.fields.role"))}div class="rounded-md border border-border bg-background px-3 py-2.5 text-sm text-foreground" {(role_label(c,&role))}}}
            }
            @if !hero {div class="mt-3 flex items-center justify-end gap-2 border-t border-border pt-3" {(media_button(html!{(c.t("media.attachments.make_hero"))},"button",shared::VARIANT_OUTLINE,"border-border bg-background text-foreground hover:border-primary/35 hover:bg-primary/5 hover:text-primary",attrs(&[("data-on:click",&media_mutation_action(&media_mutation_url(p,"promote",&item.asset_id),&text(&p.mounts.owner_form_id)))]),false))}}
        }
    }}
}

pub fn media_attachment_drag_start_action(id:&str)->String {
    let id=id.trim();
    if id.is_empty(){return "$mediaDraggedId = ''".into()}
    r##"evt.dataTransfer.effectAllowed = 'move'; evt.dataTransfer.setData('text/plain', '%s'); $mediaDraggedId = '%s'"##.replace("%s",&js_string(id))
}

pub fn media_attachment_drag_over_action(id:&str)->String {
    let id=id.trim();
    if id.is_empty(){return "evt.preventDefault(); if (evt.dataTransfer) { evt.dataTransfer.dropEffect = 'move'; }".into()}
    r##"evt.preventDefault();
if (evt.dataTransfer) { evt.dataTransfer.dropEffect = 'move'; }
if ($mediaDraggedId === '%s') {
  $mediaDropTargetId = '';
  $mediaDropPlacement = '';
  return;
}
const rect = evt.currentTarget.getBoundingClientRect();
$mediaDropTargetId = '%s';
$mediaDropPlacement = evt.clientY > rect.top + (rect.height / 2) ? 'after' : 'before';"##.replace("%s",&js_string(id))
}

pub fn media_attachment_drop_action(id:&str)->String {
    let id=id.trim();
    if id.is_empty(){return "evt.preventDefault(); $mediaDraggedId = ''".into()}
    r##"evt.preventDefault();
const list = evt.currentTarget.closest('[data-media-attachment-list]');
const draggedId = $mediaDraggedId || (evt.dataTransfer ? evt.dataTransfer.getData('text/plain') : '');
const targetId = '%s';
if (!list || !draggedId || draggedId === targetId) { $mediaDraggedId = ''; $mediaDropTargetId = ''; $mediaDropPlacement = ''; return; }
const dragged = list.querySelector('[data-media-attachment-item="' + draggedId + '"]');
const target = list.querySelector('[data-media-attachment-item="' + targetId + '"]');
if (!dragged || !target || dragged === target) { $mediaDraggedId = ''; $mediaDropTargetId = ''; $mediaDropPlacement = ''; return; }
const rect = target.getBoundingClientRect();
const placement = ($mediaDropTargetId === targetId && ($mediaDropPlacement === 'before' || $mediaDropPlacement === 'after'))
  ? $mediaDropPlacement
  : (evt.clientY > rect.top + (rect.height / 2) ? 'after' : 'before');
list.insertBefore(dragged, placement === 'after' ? target.nextElementSibling : target);
Array.from(list.querySelectorAll('[data-media-attachment-item]')).forEach((row, idx) => {
  const input = row.querySelector('input[name="attachment_positions[]"]');
  if (input instanceof HTMLInputElement) {
    input.value = String(idx);
  }
});
$mediaDraggedId = '';
$mediaDropTargetId = '';
$mediaDropPlacement = '';"##.replace("%s",&js_string(id))
}
