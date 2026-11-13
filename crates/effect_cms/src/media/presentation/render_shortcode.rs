use maud::{html,Markup};
use crate::attachments::MediaAttachmentFormItem;
use web_components::components::shared;
use web_components::components::unstyled::{badge,card};
use web_components::components::unstyled::{checkbox,dialog,icons,input,select};
use super::*;
use super::render::{attrs,first,text,hidden,get_form,media_button,MediaRenderContext};

pub fn media_insert_toolbar_button(c:MediaRenderContext<'_>,p:&MediaInsertToolbarButtonProps)->Markup {
    media_button(html!{(icons::Image(&icons::IconProps{class:"w-3 h-3".into(),..Default::default()}))(if p.label.trim().is_empty(){c.t("media.shortcode.insert_button")}else{p.label.trim().to_owned()})},"button",shared::VARIANT_OUTLINE,"gap-2 border-border bg-background text-foreground hover:border-primary/35 hover:bg-primary/5 hover:text-primary",attrs(&[("data-on:click",&open_media_fragment_action(p.open_url.trim(),p.owner_form_id.trim()))]),false)
}
/// Source helper composed with the existing localized-tab URL producer.
pub fn media_localized_insert_helper(c:MediaRenderContext<'_>,props:Option<&crate::forms::LocalizedMediaToolsProps>,tab:&crate::forms::LocalizedTab,field_label:&str)->Markup {
    html!{@if let Some(p)=props {div class="localized-helper-panel localized-helper-panel--insert" {div class="localized-helper-row" {
        div class="localized-helper-copy" {p class="localized-helper-title" {(icons::Image(&icons::IconProps{class:"w-4 h-4".into(),..Default::default()}))span {(c.t("media.shortcode.insert_heading"))}}@if !p.help_text.trim().is_empty(){p class="localized-helper-help" {(&p.help_text)}}}
        div class="localized-helper-actions" {(media_insert_toolbar_button(c,&MediaInsertToolbarButtonProps {root_id:text(&p.root_id),open_url:crate::forms::localized_media_open_url(p,tab,field_label),owner_form_id:text(&p.owner_form_id),attachment_id:text(&p.attachment_id),editor_field_key:p.field_key.clone(),editor_field_id:tab.field_id.clone(),editor_label:field_label.into(),label:p.label.clone()}))}
    }}}}
}
pub fn media_shortcode_modal(c:MediaRenderContext<'_>,p:&ShortcodeToolsProps)->Markup {
    let form_id=shortcode_filter_form_id(p);let filter_action=format!("const form = document.getElementById('{}'); if (form instanceof HTMLFormElement) {{ form.requestSubmit(); }}",js_string(&form_id));
    let dismiss=media_dismiss_action("shortcodeModalOpen",&p.root_id);
    let content=html!{
        div class="flex items-start gap-3 border-b border-border px-4 py-4" {
            span class="mt-0.5 flex h-8 w-8 shrink-0 items-center justify-center rounded-md bg-primary/10 text-primary" {(icons::CodeBrackets(&icons::IconProps{class:"w-[15px] h-[15px]".into(),..Default::default()}))}
            div class="min-w-0 flex-1 space-y-1" {
                (dialog::DialogTitle(&dialog::DialogTitleProps{id:format!("{}-title",p.dialog_id),level:"h2".into(),class:"text-lg font-semibold tracking-[-0.02em] text-foreground".into(),..Default::default()},html!{(c.t("media.shortcode.modal_title"))}))
                (dialog::DialogDescription(&dialog::DialogDescriptionProps{id:format!("{}-description",p.dialog_id),class:"text-sm text-muted-foreground".into(),..Default::default()},html!{(c.t("media.shortcode.modal_description"))}))
            }
            (dialog::DialogClose(&dialog::DialogCloseProps{dialog_id:p.dialog_id.clone(),signal:"shortcodeModalOpen".into(),after_close_action:media_mount_clear_action(&p.root_id),show_icon:true,attributes:attrs(&[("style","position:static")]),class:"rounded-full border border-border text-muted-foreground hover:bg-muted hover:text-foreground".into(),..Default::default()},Markup::default()))
        }
        div class="border-b border-border bg-muted/10 px-4 py-3" {div class="flex flex-wrap items-center gap-2 text-xs text-muted-foreground" {
            span {(c.t("media.shortcode.active_editor_description"))}
            (badge::Badge(&badge::BadgeProps{variant:shared::VARIANT_OUTLINE,size:shared::SIZE_SM,pill:true,class:"border-border bg-background text-foreground".into(),..Default::default()},html!{(&p.target_label)}))
        }}
        (get_form(p.library_url.trim(),"border-b border-border px-4 py-4",&form_id,html!{
            (shortcode_library_hidden_fields(p))(hidden("page","1"))
            div class="space-y-3" {div class="grid gap-3 sm:grid-cols-[minmax(0,1fr)_12rem]" {
                (input::Input(&input::InputProps{input_type:input::INPUT_TYPE_SEARCH,name:"search".into(),value:p.search.clone(),placeholder:c.t("media.picker.search_placeholder"),size:shared::SIZE_MD,full_width:true,class:"h-10 rounded-md px-4 text-sm".into(),attributes:attrs(&[("data-on:input__debounce.250ms",&filter_action),("aria-label",&c.t("media.picker.search_placeholder"))]),..Default::default()}))
                (select::Select(&select::SelectProps{name:"sort".into(),value:media_picker_sort_value(&p.sort),size:shared::SIZE_MD,class:"h-10 rounded-md".into(),options:[("newest","media.picker.sort_newest"),("oldest","media.picker.sort_oldest"),("alpha","media.picker.sort_alpha")].into_iter().map(|(v,k)|select::SelectOption::new(c.t(k),v)).collect(),attributes:attrs(&[("data-on:change",&filter_action),("aria-label",&c.t("media.picker.sort_label"))]),..Default::default()}))
            } div class="flex flex-wrap items-center gap-2" {@for kind in ["","image","video","document"] {(shortcode_kind_pill(c,p,kind))}}}
        }))
        div class="media-ui-shortcode-workspace flex flex-col gap-4 sm:grid sm:grid-cols-[minmax(0,1fr)_21rem] sm:gap-0" {
            div class="media-ui-shortcode-list px-4 py-4 sm:border-r sm:border-border sm:px-5" {
                @if p.items.is_empty(){(card::Card(&card::CardProps{class:"rounded-[0.9rem] border border-dashed border-border bg-muted/10 shadow-none".into(),..Default::default()},card::CardContent(&card::CardContentProps{class:"space-y-3 px-4 py-6 text-center".into(),..Default::default()},html!{p class="text-base font-semibold text-foreground" {(c.t("media.shortcode.empty_title"))}p class="text-sm leading-6 text-muted-foreground" {(c.t("media.shortcode.empty_description"))}p class="text-xs leading-5 text-muted-foreground" {(c.t("media.shortcode.empty_hint"))}})))}
                @else {div class="media-ui-picker-grid media-ui-shortcode-grid" {@for item in &p.items {(media_shortcode_attachment_button(c,item))}}
                    (media_library_pagination(c,&MediaLibraryPaginationProps{form_id:form_id.clone(),page:p.page,page_size:p.page_size,total:p.total,pages:p.pages,item_count:p.items.len() as i64,previous_label:c.t("common.pagination.previous"),next_label:c.t("common.pagination.next"),class:"media-ui-shortcode-pagination mt-4 border-t border-border pt-4".into()}))
                }
            }
            div class="media-ui-shortcode-sidebar border-t border-border bg-muted/10 px-4 py-4 sm:border-l sm:border-t-0 sm:border-border sm:px-5" data-show="$shortcodeAssetID !== ''" style="display:none" {
                div class="space-y-3" {
                    div class="space-y-2" {p class="text-[11px] font-semibold uppercase tracking-[0.16em] text-muted-foreground" {(c.t("media.shortcode.generated_title"))}
                        code class="media-ui-shortcode-preview block rounded-md border border-border bg-background px-3 py-2.5 font-mono text-[11px] leading-5 text-foreground" data-shortcode-preview="true" data-text=(shortcode_preview_expr()) data-attr:data-shortcode-value=(shortcode_preview_expr()) {(c.t("media.shortcode.generated_placeholder"))}
                    }
                    div class="grid gap-2 sm:grid-cols-2 md:grid-cols-2" {
                        (select::SelectWithLabel(&select::SelectProps{id:format!("{}-shortcode-size",p.root_id),name:"shortcode_size".into(),value:"full".into(),size:shared::SIZE_MD,options:["full","large","medium","thumbnail"].into_iter().map(|v|select::SelectOption::new(c.t(&format!("media.shortcode.option_size_{v}")),v)).collect(),class:"bg-background".into(),attributes:attrs(&[("data-bind","shortcodeSize")]),..Default::default()},&c.t("media.shortcode.field_size"),""))
                        (select::SelectWithLabel(&select::SelectProps{id:format!("{}-shortcode-align",p.root_id),name:"shortcode_align".into(),value:"none".into(),size:shared::SIZE_MD,options:["none","left","right","center"].into_iter().map(|v|select::SelectOption::new(c.t(&format!("media.shortcode.option_align_{v}")),v)).collect(),class:"bg-background".into(),attributes:attrs(&[("data-bind","shortcodeAlign")]),..Default::default()},&c.t("media.shortcode.field_alignment"),""))
                    }
                    div class="grid gap-2 sm:grid-cols-2 md:grid-cols-2" {
                        label class="flex items-center gap-3 rounded-md border border-border bg-background px-3 py-2.5" {
                            (checkbox::Checkbox(&checkbox::CheckboxProps{id:format!("{}-shortcode-caption",p.root_id),name:"shortcode_caption".into(),size:shared::SIZE_SM,class:"rounded border-border".into(),attributes:attrs(&[("data-bind","shortcodeCaption")]),..Default::default()}))
                            span class="text-sm font-medium text-foreground" {(c.t("media.shortcode.field_caption"))}
                        }
                        label class="flex items-center gap-3 rounded-md border border-border bg-background px-3 py-2.5" {
                            (checkbox::Checkbox(&checkbox::CheckboxProps{id:format!("{}-shortcode-link",p.root_id),name:"shortcode_link".into(),size:shared::SIZE_SM,class:"rounded border-border".into(),attributes:attrs(&[("data-bind","shortcodeLink"),("data-attr:disabled","$shortcodeAssetKind !== 'image' ? 'true' : null")]),..Default::default()}))
                            span class="text-sm font-medium text-foreground" {(c.t("media.shortcode.field_link"))}
                        }
                    }
                    (input::InputWithLabel(&input::InputProps{input_type:input::INPUT_TYPE_TEXT,id:format!("{}-shortcode-class",p.root_id),name:"shortcode_class".into(),placeholder:c.t("media.shortcode.field_class_placeholder"),size:shared::SIZE_MD,full_width:true,class:"bg-background font-mono text-sm".into(),attributes:attrs(&[("data-bind","shortcodeClass")]),..Default::default()},&c.t("media.shortcode.field_class"),""))
                    p class="text-[11px] leading-5 text-muted-foreground" {(c.t("media.shortcode.footer_note"))}
                }
            }
        }
        div class="media-ui-shortcode-actions flex items-center justify-end gap-2 border-t border-border bg-card px-4 py-4" {div class="flex items-center gap-2" {
            (media_button(html!{(c.t("common.cancel"))},"button",shared::VARIANT_GHOST,"text-muted-foreground hover:bg-muted hover:text-foreground",attrs(&[("data-on:click",&dismiss)]),false))
            (media_shortcode_action_button(c, &p.root_id, None))
            (media_shortcode_action_button(c, &p.root_id, Some(&p.target_field_id)))
        }}
    };
    html!{div data-signals=(media_shortcode_signals(p)) data-on:load=(media_scroll_lock_action()) {
        (dialog::Dialog(&dialog::DialogProps{id:p.dialog_id.clone(),open:true,signal:"shortcodeModalOpen".into(),class:"media-ui-overlay z-[1000001]".into(),role:"dialog".into(),aria_labelled_by:format!("{}-title",p.dialog_id),aria_described_by:format!("{}-description",p.dialog_id),..Default::default()},html!{
            (dialog::DialogOverlay(&dialog::DialogOverlayProps{dialog_id:p.dialog_id.clone(),class:"z-[1000002] bg-background/70 backdrop-blur-sm".into(),attributes:attrs(&[("style","z-index:1000002;"),("data-on:click",&dismiss)])}))
            (dialog::DialogContent(&dialog::DialogContentProps{dialog_id:p.dialog_id.clone(),size:shared::SIZE_LG,disable_full_width:true,class:"media-ui-shortcode-dialog z-[1000003] overflow-hidden rounded-[0.95rem] border border-border bg-card p-0 shadow-xl".into(),attributes:attrs(&[("style","z-index:1000003;")]),..Default::default()},content))
        }))
    }}
}
fn media_shortcode_attachment_button(c:MediaRenderContext<'_>,i:&MediaAttachmentFormItem)->Markup {
    let id=text(&i.asset_id);let title=text(&i.title);let alt=text(&i.alt);let alt_override=text(&i.alt_override);
    let trimmed=crate::validation::trim_unicode_whitespace(i.asset_id.as_bytes());
    let short=crate::byte_text::encoding_text(&trimmed[..trimmed.len().min(12)]);
    let meta=[i.status.trim(),short.trim()].into_iter().filter(|v|!v.is_empty()).collect::<Vec<_>>().join(" • ");
    html!{button type="button" class="media-ui-picker-card group flex h-full flex-col overflow-hidden rounded-[0.4rem] border bg-card text-left transition focus-within:ring-2 focus-within:ring-primary/20 hover:border-primary/35 hover:bg-primary/5" data-on:click=(format!("$shortcodeAssetID = '{}'; $shortcodeAssetKind = '{}'",media_js_string(&id),media_js_string(i.kind.trim()))) data-attr:data-media-selected=(format!("$shortcodeAssetID === '{}' ? 'true' : 'false'",media_js_string(&id))) {
        (media_library_card(c,&MediaLibraryCardProps{title:first(&[&title,&id]).into(),meta,preview_url:media_attachment_preview_url(i),alt:first(&[&alt_override,&alt,&title]).into(),kind:i.kind.clone(),..Default::default()}))
    }}
}
fn shortcode_library_hidden_fields(p:&ShortcodeToolsProps)->Markup {html!{@for (k,v) in [("root_id",&p.root_id),("dialog_id",&p.dialog_id),("scope",&p.scope),("owner_form_id",&p.owner_form_id),("field_id",&p.target_field_id),("field_label",&p.target_label)] {(hidden(k,v.trim()))}}}
fn shortcode_kind_pill(c:MediaRenderContext<'_>,p:&ShortcodeToolsProps,kind:&str)->Markup {
    let active=p.kind.trim().eq_ignore_ascii_case(kind.trim());let class=if active{"border-primary bg-primary text-primary-foreground hover:border-primary hover:bg-primary hover:text-primary-foreground"}else{"border-border bg-background text-foreground hover:border-primary/35 hover:bg-primary/5 hover:text-primary"};
    let label=if kind.is_empty(){c.t("media.filter.all")}else{c.t(&format!("media.kind.{kind}"))};
    media_button(html!{(label)},"submit",shared::VARIANT_OUTLINE,class,attrs(&[("form",&shortcode_filter_form_id(p)),("name","kind"),("value",kind.trim())]),false)
}
pub fn media_shortcode_signals(p:&ShortcodeToolsProps)->String {
    let id=p.items.first().map(|i|text(&i.asset_id)).unwrap_or_default();let kind=p.items.first().map(|i|i.kind.trim()).unwrap_or("");
    format!("{{ shortcodeModalOpen: true, shortcodeAssetID: '{}', shortcodeAssetKind: '{}', shortcodeSize: 'full', shortcodeAlign: 'none', shortcodeCaption: false, shortcodeLink: false, shortcodeClass: '' }}",media_js_string(id.trim()),media_js_string(kind))
}
pub fn shortcode_filter_form_id(p:&ShortcodeToolsProps)->String {format!("{}-library-form",first(&[&p.root_id,"media-shortcode-root"]))}
pub fn shortcode_preview_expr()->String {
    r##"'[media id="' + ($shortcodeAssetID || 'asset-id') + '" size="' + ($shortcodeSize || 'full') + '" align="' + ($shortcodeAlign || 'none') + '"' + ($shortcodeCaption ? ' caption="true"' : '') + (($shortcodeLink && $shortcodeAssetKind === 'image') ? ' link="file"' : '') + (($shortcodeClass || '').trim() ? ' class="' + $shortcodeClass.trim() + '"' : '') + ']'"##.into()
}
pub fn shortcode_copy_action(root_id:&str)->String {format!(r##"const root = document.getElementById('{}'); const preview = root?.querySelector('[data-shortcode-preview="true"]'); const shortcode = String(preview?.dataset.shortcodeValue || preview?.textContent || '').trim(); if (shortcode !== '' && !shortcode.includes('id="asset-id"')) {{ navigator.clipboard.writeText(shortcode).catch((error) => console.error('shortcode copy failed', error)); }}"##,media_js_string(root_id.trim()))}
pub fn shortcode_insert_action(root_id:&str,target_field_id:&str)->String {
    format!(r##"const root = document.getElementById('{}'); const preview = root?.querySelector('[data-shortcode-preview="true"]'); const shortcode = String(preview?.dataset.shortcodeValue || preview?.textContent || '').trim(); const field = document.getElementById('{}'); if ((field instanceof HTMLTextAreaElement || field instanceof HTMLInputElement) && shortcode !== '' && !shortcode.includes('id="asset-id"')) {{ const insert = new CustomEvent('content-editor-insert', {{cancelable: true, detail: {{text: shortcode}}}}); if (field.dispatchEvent(insert)) {{ const start = typeof field.selectionStart === 'number' ? field.selectionStart : field.value.length; const end = typeof field.selectionEnd === 'number' ? field.selectionEnd : start; field.setRangeText(shortcode, start, end, 'end'); field.dispatchEvent(new Event('input', {{ bubbles: true }})); }} $shortcodeModalOpen = false; {}; requestAnimationFrame(() => requestAnimationFrame(() => {{ if (!document.querySelector('.media-ui-overlay')) {{ const editor = field.nextElementSibling?.querySelector('.cm-content'); (editor instanceof HTMLElement ? editor : field).focus(); }} }})); }}"##,media_js_string(root_id.trim()),media_js_string(target_field_id.trim()),media_mount_clear_action(root_id.trim()))
}

/// Shortcode-specific hooks stay with the shortcode workspace rather than generic controls.
fn media_shortcode_action_button(c: MediaRenderContext<'_>, root_id: &str, target: Option<&str>) -> Markup {
    let insert = target.is_some();
    let props = web_components::components::unstyled::button::ButtonProps {
        variant: if insert { shared::VARIANT_PRIMARY } else { shared::VARIANT_OUTLINE },
        size: shared::SIZE_SM,
        class: if insert { "shadow-none" } else { "border-border bg-background text-foreground hover:border-primary/35 hover:bg-primary/5 hover:text-primary" }.into(),
        ..Default::default()
    };
    let action = match target { Some(id) => shortcode_insert_action(root_id,id), None => shortcode_copy_action(root_id) };
    html! { button type="button" class=(props.build_classes())
        data-shortcode-copy-button=[(!insert).then_some("true")]
        data-shortcode-insert-button=[insert.then_some("true")]
        data-shortcode-root-id=(root_id) data-shortcode-target-id=[target]
        data-attr:disabled="$shortcodeAssetID === '' ? 'true' : null" data-on:click=(action) {
        (c.t(if insert { "media.shortcode.insert_attachment_button" } else { "media.shortcode.copy_button" }))
    } }
}
