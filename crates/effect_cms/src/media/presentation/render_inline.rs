use maud::{html,Markup};
use web_components::components::shared;
use web_components::components::unstyled::{heading,card};
use web_components::components::unstyled::{drawer,form,icons,input,stepper};
use super::*;
use super::render::{attrs,hidden,media_button,MediaRenderContext};

pub fn media_inline_create_drawer(c:MediaRenderContext<'_>,p:&MediaInlineCreateProps)->Markup {
    let refresh=media_form_get_action(p.preview_url.trim(),&format!("{}-form",p.dialog_id.trim()));
    let dismiss=media_dismiss_action("mediaCreateOpen",&p.root_id);
    let form_id=format!("{}-form",p.dialog_id.trim());
    let form_children=html! {
        (hidden("surface","inline"))
        @for (k,v) in [("root_id",&p.root_id),("picker_root_id",&p.picker_root_id),("scope",&p.scope),("role",&p.role_default),("owner_form_id",&p.owner_form_id),("owner_content_id",&p.owner_content_id),("target",&p.attachment_id),("preview_root_id",&p.preview_root_id)] {(hidden(k,v.trim()))}
        input type="hidden" name="kind" data-bind="mediaCreateType";
        input type="hidden" name="source_kind" data-bind="mediaCreateMode";
        (media_attachment_payload_inputs(&p.attachments))
        @if !p.error_msg.trim().is_empty() {div class="rounded-xl border border-destructive/35 bg-destructive-muted px-4 py-3 text-sm text-destructive-foreground" {(p.error_msg.trim())}}
        div data-show="$mediaCreateStep === 1" class="space-y-4" {
            div class="space-y-1.5" {h3 tabindex="-1" data-media-step-heading="1" class="text-base font-semibold text-media-ink" {(c.t("media.inline_create.step1_title"))}p class="text-sm leading-relaxed text-media-ink-muted" {(c.t("media.inline_create.step1_description"))}}
            div class="grid grid-cols-1 gap-3 sm:grid-cols-3" {@for kind in ["image","video","document"] {(media_type_option_card(c,kind,&c.t(&format!("media.inline_create.type_{kind}_title")),&c.t(&format!("media.inline_create.type_{kind}_body")),&p.kind,&refresh))}}
            (card::Card(&card::CardProps {class:"media-ui-panel rounded-2xl border-media-border shadow-none".into(),..Default::default()},card::CardContent(&card::CardContentProps{class:"flex gap-3 p-4".into(),..Default::default()},html!{(icons::Info(&icons::IconProps{class:"mt-0.5 shrink-0 text-media-ink-muted w-4 h-4".into(),..Default::default()}))p class="text-xs leading-relaxed text-media-ink-muted" {(c.t("media.inline_create.reusable_note"))}})))
        }
        div data-show="$mediaCreateStep === 2" class="space-y-4" {
            div class="flex items-center gap-2 text-xs text-media-ink-muted" {
                (inline_back(c,1)) span class="text-media-ink-faint" {"·"}
                span class="inline-flex items-center rounded-full border border-media-border bg-media-surface-2 px-2 py-0.5 text-[11px] font-medium text-media-ink-muted" data-text=(kind_text(c)) {}
            }
            div class="space-y-1.5" {h3 tabindex="-1" data-media-step-heading="2" class="text-base font-semibold text-media-ink" {(c.t("media.inline_create.step2_title"))}p class="text-sm leading-relaxed text-media-ink-muted" {(c.t("media.inline_create.step2_description"))}}
            div class="inline-flex rounded-xl border media-ui-divider p-1" {(media_mode_button(c,"remote",&p.source_kind,&refresh))(media_mode_button(c,"embed",&p.source_kind,&refresh))}
        }
        div data-show="$mediaCreateStep === 3" class="space-y-4" {
            div class="flex items-center gap-2 text-xs text-media-ink-muted" {(inline_back(c,2))span class="text-media-ink-faint" {"·"}span class="inline-flex items-center rounded-full border border-media-border bg-media-surface-2 px-2 py-0.5 text-[11px] font-medium text-media-ink-muted" data-text=(kind_mode_text(c)) {}}
            div class="space-y-1.5" {h3 tabindex="-1" data-media-step-heading="3" class="text-base font-semibold text-media-ink" data-text=(mode_text(c,"step3_title")) {}p class="text-sm leading-relaxed text-media-ink-muted" data-text=(mode_text(c,"step3_description")) {}}
            (input::InputWithLabel(&input::InputProps{input_type:input::INPUT_TYPE_TEXT,name:"url".into(),id:format!("{}-url",p.dialog_id),value:p.url.trim().into(),placeholder:c.t("media.inline_create.url_placeholder"),required:true,size:shared::SIZE_MD,full_width:true,class:"media-ui-input".into(),attributes:attrs(&[("data-bind","mediaCreateURL"),("data-on:input__debounce.250ms",&refresh),("data-on:change",&refresh)]),..Default::default()},&c.t("media.inline_create.url_label"),""))
            (media_inline_create_preview(c,&p.preview))
        }
        div data-show="$mediaCreateStep === 4" class="space-y-4" {
            div class="flex items-center gap-2 text-xs text-media-ink-muted" {(inline_back(c,3))span class="text-media-ink-faint" {"·"}span class="inline-flex items-center rounded-full border border-media-border bg-media-surface-2 px-2 py-0.5 text-[11px] font-medium text-media-ink-muted" {(c.t("media.inline_create.review_badge"))}}
            div class="space-y-1.5" {h3 tabindex="-1" data-media-step-heading="4" class="text-base font-semibold text-media-ink" {(c.t("media.inline_create.review_title"))}p class="text-sm leading-relaxed text-media-ink-muted" {(c.t("media.inline_create.review_description"))}}
            div class="grid gap-4 md:grid-cols-2" {
                @for (name,suffix,label,value,kind) in [("title","title","title",&p.title,input::INPUT_TYPE_TEXT),("thumbnail_url","thumbnail","thumbnail",&p.thumbnail_url,input::INPUT_TYPE_TEXT),("alt","alt","alt",&p.alt,input::INPUT_TYPE_TEXT),("duration","duration","duration",&p.duration,input::INPUT_TYPE_NUMBER)] {
                    (input::InputWithLabel(&input::InputProps{input_type:kind,name:name.into(),id:format!("{}-{suffix}",p.dialog_id),value:value.trim().into(),placeholder:c.t(&format!("media.inline_create.{label}_placeholder")),size:shared::SIZE_MD,full_width:true,class:"media-ui-input".into(),..Default::default()},&c.t(&format!("media.inline_create.{label}_label")),""))
                }
            }
            (input::TextAreaWithLabel(&input::InputProps{name:"description".into(),id:format!("{}-description-field",p.dialog_id),value:p.description.trim().into(),placeholder:c.t("media.inline_create.description_placeholder"),size:shared::SIZE_MD,class:"media-ui-input min-h-28".into(),..Default::default()},&c.t("media.inline_create.description_label"),"",4))
        }
    };
    drawer::Drawer(&drawer::DrawerProps {open:true,signal_name:"mediaCreateOpen".into(),after_close_action:media_mount_clear_action(&p.root_id),title:c.t("media.inline_create.title"),position:drawer::DRAWER_POSITION_RIGHT,panel_width:"media-ui-inline-create-panel".into(),show_header:false,show_close_button:false,class:"media-ui-overlay".into(),panel_class:"media-ui-drawer-panel border-l border-media-border".into(),content_class:"flex h-full flex-col overflow-hidden".into(),attributes:attrs(&[("data-signals",&media_inline_signals(p)),("data-on:load",&media_scroll_lock_action()),("style","z-index:1000001;")]),..Default::default()},html!{
        div class="flex items-center justify-between border-b media-ui-divider px-5 py-4" {div class="flex items-center gap-3" {
            (stepper::Stepper(&stepper::Props{class:"gap-1.5".into(),items:["step_choose_type","step_choose_mode","step_paste_source","step_review_save"].into_iter().enumerate().map(|(index,key)|{let condition=format!("$mediaCreateStep >= {}",index+1);stepper::Item{label:c.t(&format!("media.inline_create.{key}")),attributes:attrs(&[("data-class:size-2.5",&condition),("data-class:border-primary",&condition),("data-class:bg-primary",&condition)]),..Default::default()}}).collect(),..Default::default()}))
            (heading::H2("base",false,"font-semibold text-media-ink",html!{(c.t("media.inline_create.title"))}))
        } (media_button(html!{(icons::Close(&icons::IconProps{class:"w-[14px] h-[14px]".into(),..Default::default()}))},"button",shared::VARIANT_GHOST,"media-ui-drawer-close h-8 w-8 p-0 text-media-ink-faint hover:bg-media-surface-2 hover:text-media-ink",attrs(&[("aria-label",&c.t("component.dialog.close_aria_label")),("data-drawer-close","true"),("data-on:click",&dismiss)]),false))}
        div class="flex-1 overflow-y-auto px-5 py-5" {
            (form::FormWithCSRF(&form::FormProps{method:"POST".into(),action:p.action_url.clone(),datastar_post:p.action_url.clone(),class:"space-y-6".into(),attributes:attrs(&[("id",&form_id),("data-effect",r#"const step = $mediaCreateStep; if (el.dataset.mediaStep && el.dataset.mediaStep !== String(step)) { queueMicrotask(() => el.querySelector('[data-media-step-heading="' + step + '"]')?.focus()) } el.dataset.mediaStep = String(step)"#)]),..Default::default()},&p.csrf_token,&p.csrf_field_name,form_children))
        }
        div class="border-t media-ui-divider px-5 py-4" {div class="flex items-center justify-between gap-3" {
            (media_button(html!{(c.t("common.cancel"))},"button",shared::VARIANT_GHOST,"text-media-ink-muted hover:bg-media-surface-2 hover:text-media-ink",attrs(&[("data-on:click",&dismiss)]),false))
            div class="flex items-center gap-2" {
                (media_button(html!{(c.t("media.inline_create.continue_button"))},"button",shared::VARIANT_PRIMARY,"shadow-none",attrs(&[("data-show","$mediaCreateStep === 1 || $mediaCreateStep === 2"),("data-on:click","if ($mediaCreateStep === 1 && $mediaCreateType !== '') { $mediaCreateStep = 2 } else if ($mediaCreateStep === 2 && $mediaCreateMode !== '') { $mediaCreateStep = 3 }"),("data-attr:disabled","($mediaCreateStep === 1 && $mediaCreateType === '') || ($mediaCreateStep === 2 && $mediaCreateMode === '')")]),false))
                (media_button(html!{(c.t("media.inline_create.review_button"))},"button",shared::VARIANT_PRIMARY,"shadow-none",attrs(&[("data-show","$mediaCreateStep === 3"),("data-on:click","if ($mediaCreateURL !== '') { $mediaCreateStep = 4 }"),("data-attr:disabled","$mediaCreateURL === ''")]),false))
                (media_button(html!{(c.t("media.inline_create.create_attach_button"))},"submit",shared::VARIANT_PRIMARY,"shadow-none",attrs(&[("data-show","$mediaCreateStep === 4"),("form",&form_id)]),false))
            }
        }}
    })
}
fn inline_back(c:MediaRenderContext<'_>,step:usize)->Markup {media_button(html!{(icons::ChevronLeft(&icons::IconProps{class:"w-3 h-3".into(),..Default::default()}))(c.t("common.back"))},"button",shared::VARIANT_GHOST,"h-auto gap-1 px-0 py-0 text-xs text-media-ink-muted hover:bg-transparent hover:text-media-ink",attrs(&[("data-on:click",&format!("if ($mediaCreateStep > 1) {{ $mediaCreateStep = {step} }}"))]),false)}
fn kind_text(c:MediaRenderContext<'_>)->String {format!("$mediaCreateType === 'video' ? '{}' : ($mediaCreateType === 'document' ? '{}' : '{}')",js_string(&c.t("media.kind.video")),js_string(&c.t("media.kind.document")),js_string(&c.t("media.kind.image")))}
fn kind_mode_text(c:MediaRenderContext<'_>)->String {
    let mode=format!("($mediaCreateMode === 'embed' ? '{}' : '{}')",js_string(&c.t("media.source.embed")),js_string(&c.t("media.source.remote")));
    format!("$mediaCreateType === 'video' ? '{} · ' + {mode} : ($mediaCreateType === 'document' ? '{} · ' + {mode} : '{} · ' + {mode})",js_string(&c.t("media.kind.video")),js_string(&c.t("media.kind.document")),js_string(&c.t("media.kind.image")))
}
fn mode_text(c:MediaRenderContext<'_>,key:&str)->String {format!("$mediaCreateMode === 'embed' ? '{}' : '{}'",js_string(&c.t(&format!("media.inline_create.{key}_embed"))),js_string(&c.t(&format!("media.inline_create.{key}_remote"))))}
pub fn media_inline_signals(p:&MediaInlineCreateProps)->String {
    let kind=p.kind.trim();let mode=p.source_kind.trim();let url=p.url.trim();let step=if kind.is_empty(){1}else if mode.is_empty(){2}else if url.is_empty(){3}else{4};
    format!("{{ mediaCreateOpen: true, mediaCreateStep: {step}, mediaCreateType: '{}', mediaCreateMode: '{}', mediaCreateURL: '{}' }}",media_js_string(kind),media_js_string(mode),media_js_string(url))
}
fn option_action(signal:&str,value:&str,step:usize,preview:&str)->String {
    let action=format!("${} = '{}'; if ($mediaCreateStep < {step}) {{ $mediaCreateStep = {step} }}",signal.trim(),media_js_string(value));if preview.trim().is_empty(){action}else{format!("{action}; {}",preview.trim())}
}
fn media_type_option_card(c:MediaRenderContext<'_>,value:&str,title:&str,body:&str,fallback:&str,preview:&str)->Markup {
    html!{button type="button" class="media-ui-type-card flex min-h-36 flex-col items-start gap-3 rounded-2xl p-4 text-left transition-all" aria-pressed=(fallback.trim().eq_ignore_ascii_case(value.trim()).to_string()) data-attr:aria-pressed=(format!("String($mediaCreateType === '{value}')")) data-class:border-media-brand=(format!("$mediaCreateType === '{value}'")) data-class:bg-media-brand-faint=(format!("$mediaCreateType === '{value}'")) data-on:click=(option_action("mediaCreateType",value,2,preview)) {span aria-hidden="true" {(media_type_badge(c.request,c.translate,value,""))}div class="space-y-1" {p class="text-base font-semibold text-media-ink" {(title)}p class="text-sm leading-relaxed text-media-ink-muted" {(body)}}}}
}
/// Media authoring mode selection, including its source theme state.
fn media_mode_button(c:MediaRenderContext<'_>, value:&str, fallback:&str, preview:&str)->Markup {
    let selected = format!("$mediaCreateMode === '{value}'");
    let unselected = format!("$mediaCreateMode !== '{value}'");
    let props = web_components::components::unstyled::button::ButtonProps {
        variant: shared::VARIANT_GHOST, size: shared::SIZE_SM,
        class: "media-ui-mode-option rounded-lg px-3 py-2 text-sm".into(),
        ..Default::default()
    };
    html! { button type="button" class=(props.build_classes())
        aria-pressed=(fallback.trim().eq_ignore_ascii_case(value).to_string())
        data-attr:aria-pressed=(format!("String($mediaCreateMode === '{value}')"))
        data-class:bg-media-brand=(&selected) data-class:text-white=(&selected)
        data-class:bg-transparent=(&unselected) data-class:text-media-ink-muted=(&unselected)
        data-on:click=(option_action("mediaCreateMode",value,3,preview)) {
        (c.t(&format!("media.source.{value}")))
    } }
}
pub fn media_inline_create_preview(c:MediaRenderContext<'_>,p:&MediaInlineCreatePreviewProps)->Markup {
    html!{div id=(p.root_id.trim()) class="space-y-3" data-show="$mediaCreateURL !== ''" {
        @if !p.provider.trim().is_empty(){div class="flex items-center gap-2" {span class="text-xs uppercase tracking-[0.18em] text-media-ink-faint" {(c.t("media.inline_create.detected_provider"))}span class="inline-flex items-center gap-1 rounded-full border border-success/30 bg-success-muted px-2.5 py-1 text-xs font-medium text-success-foreground" {(p.provider.trim())}}}
        (card::Card(&card::CardProps{class:"media-ui-panel rounded-2xl border-media-border shadow-none".into(),..Default::default()},card::CardContent(&card::CardContentProps{class:"space-y-3 p-4".into(),..Default::default()},html!{p class="text-xs uppercase tracking-[0.18em] text-media-ink-faint" {(c.t("media.inline_create.live_preview"))}
            @if p.show_remote_image&&!p.resolved_image_url.trim().is_empty(){img alt=(c.t("media.preview.alt_fallback")) src=(p.resolved_image_url.trim()) class="aspect-video w-full rounded-2xl border media-ui-divider bg-media-surface-2 object-cover";}
            @else {div class="flex aspect-video items-center justify-center rounded-2xl border border-dashed media-ui-divider bg-media-surface-2 px-4 text-center" {div class="space-y-2" {p class="text-sm font-medium text-media-ink" {(c.t(if p.show_video_hint{"media.inline_create.preview_video_hint"}else if p.show_document_hint{"media.inline_create.preview_document_hint"}else{"media.inline_create.preview_image_hint"}))}p class="text-xs text-media-ink-faint" {(c.t("media.inline_create.preview_meta_hint"))}}}}
        })))
    }}
}
