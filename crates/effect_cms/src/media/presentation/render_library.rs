use maud::{html,Markup};
use web_components::components::shared;
use web_components::components::unstyled::{icons};
use super::*;
use super::render::{attrs,first,media_button,MediaRenderContext};

pub fn media_library_card(c:MediaRenderContext<'_>,p:&MediaLibraryCardProps)->Markup {
    let badge=match p.kind.trim().to_ascii_lowercase().as_str(){"video"=>"bg-info-muted text-info-foreground border-info/30","document"=>"bg-warning-muted text-warning-foreground border-warning/30",_=>"bg-success-muted text-success-foreground border-success/30"};
    html!{div class="relative aspect-[4/3] overflow-hidden bg-muted pointer-events-none" {
        (media_preview_frame(c.request,c.translate,p.preview_url.trim(),p.alt.trim(),p.kind.trim(),"h-full w-full rounded-none border-0 object-cover",&c.t("media.preview.alt_fallback")))
        div class="absolute right-2 top-2" {(media_type_badge(c.request,c.translate,p.kind.trim(),badge))}
        div class="media-ui-picker-selected absolute left-2 top-2 flex h-[1.15rem] w-[1.15rem] items-center justify-center rounded-full border-2 border-card bg-primary text-primary-foreground shadow-sm" {(icons::Check(&icons::IconProps {class:"w-2 h-2".into(),..Default::default()}))}
    } div class="min-w-0 flex-1 space-y-1 px-2.5 py-2 pointer-events-none" {p class="truncate text-[0.72rem] font-medium leading-[1.05rem] text-foreground" {(p.title.trim())} p class="truncate text-[0.64rem] text-muted-foreground" {(p.meta.trim())}}}
}
pub fn media_library_pagination(c:MediaRenderContext<'_>,p:&MediaLibraryPaginationProps)->Markup {
    if p.total<=0{return Markup::default()}
    let size=if p.page_size>0{p.page_size}else{p.item_count};
    let start=if size<=0{1}else{(p.page.max(1)-1)*size+1};
    let end=(start+p.item_count-1).max(0).min(p.total);
    let mut low=(p.page-1).max(1); let high=(low+2).min(p.pages); low=low.min((high-2).max(1));
    html!{div class=(first(&[&p.class,"flex flex-wrap items-center justify-between gap-4"])) {
        div class="space-y-0.5" {
            p class="text-sm text-muted-foreground" {(c.t_args("common.pagination.showing_range",&[start.to_string(),end.to_string(),p.total.to_string()]))}
            p class="text-xs text-muted-foreground" {(c.t_args("common.pagination.page_label",&[p.page.to_string(),p.pages.to_string()]))}
        }
        div class="flex flex-wrap items-center gap-2" {
            (nav(p,p.page-1,true,p.previous_label.trim(),p.page<=1))
            @for page in low..=high {(page_button(p,page,&page.to_string(),false,page==p.page))}
            (nav(p,p.page+1,false,p.next_label.trim(),p.page>=p.pages))
        }
    }}
}
fn pagination_attrs(p:&MediaLibraryPaginationProps,page:i64,current:bool,disabled:bool)->std::collections::BTreeMap<String,String> {
    if disabled||current||page<1||page>p.pages{return attrs(&[])}
    attrs(&[("form",p.form_id.trim()),("name","page"),("value",&page.to_string()),("data-on:click",&media_library_pagination_action(p,page))])
}
pub fn media_library_pagination_action(p:&MediaLibraryPaginationProps,page:i64)->String {
    if p.form_id.trim().is_empty(){return String::new()}
    format!("evt.preventDefault(); const form = document.getElementById('{}'); if (form instanceof HTMLFormElement) {{ const pageInput = form.querySelector('input[name=\"page\"]'); if (pageInput instanceof HTMLInputElement) {{ pageInput.value = '{}'; }} form.requestSubmit(); }}",js_string(p.form_id.trim()),page)
}
fn page_button(p:&MediaLibraryPaginationProps,page:i64,label:&str,disabled:bool,current:bool)->Markup {
    let class=if current{"border-primary bg-primary text-primary-foreground disabled:opacity-100"}else{"border-border bg-background text-muted-foreground hover:border-primary/35 hover:bg-primary/5 hover:text-primary disabled:cursor-not-allowed disabled:border-border disabled:bg-muted/40 disabled:text-muted-foreground"};
    media_button(html!{(label.trim())},"submit",shared::VARIANT_OUTLINE,class,pagination_attrs(p,page,current,disabled),disabled)
}
fn nav(p:&MediaLibraryPaginationProps,page:i64,previous:bool,label:&str,disabled:bool)->Markup {
    let mut a=pagination_attrs(p,page,false,disabled);a.insert("aria-label".into(),label.into());
    let icon=icons::IconProps{class:"w-[14px] h-[14px]".into(),..Default::default()};
    media_button(html!{@if previous {(icons::ChevronLeft(&icon))} @else {(icons::ChevronRight(&icon))} span class="sr-only" {(label)}},"submit",shared::VARIANT_OUTLINE,"h-8 w-8 rounded-md border-border bg-background p-0 text-muted-foreground hover:border-primary/35 hover:bg-primary/5 hover:text-primary disabled:cursor-not-allowed disabled:border-border disabled:bg-muted/40 disabled:text-muted-foreground",a,disabled)
}
