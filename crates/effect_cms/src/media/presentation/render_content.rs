use maud::{html, Markup, PreEscaped};
use web_components::components::shared;
use web_components::components::unstyled::{heading,slidepager};
use web_components::components::unstyled::{button};
use super::*;
use super::render::{first,MediaRenderContext};

pub fn video_player_block(p:&VideoPlayerProps)->Markup {
    html! { @if !p.url.trim().is_empty() { section class=(first(&[&p.class,"mb-8"])) {
        video-player url=(&p.url) title=(&p.title) provider=(&p.provider) thumbnail=(&p.thumbnail) description=(&p.description) {}
    } } }
}
pub fn media_player_client_script(asset_url:&str)->Markup { html! { script type="module" src=(asset_url) {} } }
pub fn content_video_detail_section(p:&ContentVideoDetailSectionProps)->Markup { html! { @if let Some(v)=&p.video { (video_player_block(v)) } } }
pub fn content_media_detail_section(c:MediaRenderContext<'_>,p:&ContentMediaDetailSectionProps)->Markup {
    html! { @if let Some(h)=&p.hero { (hero_media_block(h)) }
        (media_slide_section(c,&MediaSlideSectionProps {title:p.slide_title.clone(),eyebrow:p.slide_eyebrow.clone(),subtitle:p.slide_subtitle.clone(),state_var:p.slide_state_var.clone(),items:p.slides.clone()})) }
}
pub fn hero_media_block(p:&HeroMediaProps)->Markup {
    match p.kind.trim().to_ascii_lowercase().as_str() {
        "video"=>video_player_block(&VideoPlayerProps {url:p.url.clone(),title:p.title.clone(),provider:p.provider.clone(),thumbnail:p.thumbnail.clone(),description:p.description.clone(),..Default::default()}),
        "image"=>html! { @if !p.url.trim().is_empty() { section class="mb-8" { img src=(&p.url) alt=(first(&[&p.alt,&p.title])) class="w-full aspect-[21/9] object-cover bg-muted rounded-2xl overflow-hidden border border-border" loading="eager"; } } },
        _=>Markup::default(),
    }
}
pub fn embedded_media_block(p:&EmbeddedMediaProps)->Markup {
    match p.kind.trim().to_ascii_lowercase().as_str() {
        "video"=>html! { div class="my-8" { (video_player_block(&VideoPlayerProps {url:p.url.clone(),title:p.title.clone(),provider:p.provider.clone(),thumbnail:p.thumbnail.clone(),description:p.description.clone(),..Default::default()})) @if !p.caption.trim().is_empty() { p class="mt-2 text-sm text-muted-foreground" { (&p.caption) } } } },
        "image"=>html! { figure class="my-8 space-y-3" { img src=(&p.url) alt=(first(&[&p.alt,&p.title])) class="w-full rounded-2xl border border-border bg-muted object-cover" loading="lazy"; @if !p.caption.trim().is_empty() { figcaption class="text-sm text-muted-foreground" { (&p.caption) } } } },
        _=>Markup::default(),
    }
}
pub fn media_slide_deck(c:MediaRenderContext<'_>,p:&MediaSlideDeckProps)->Markup {
    if p.items.is_empty(){return Markup::default()}
    if p.items.len()==1 {return media_single_item_card(c,&p.items[0])}
    let state=first(&[&p.state_var,"mediaSlide"]);
    let mut pager=slidepager::FullSlidePagerProps(p.items.len() as i32);
    pager.state_var=state.into(); pager.show_dots=true; pager.show_thumbnails=false; pager.class="gap-4".into();
    pager.content_class="media-slide-viewport rounded-[1.35rem] border border-border/40 slide-surface slide-shadow overflow-hidden".into();
    slidepager::SlidePager(&pager,html! { @for (index,item) in p.items.iter().enumerate() { (slidepager::SlidePanel(index as i32,state,media_slide_panel(c,item))) } })
}
pub fn media_slide_section(c:MediaRenderContext<'_>,p:&MediaSlideSectionProps)->Markup {
    html! { @if !p.items.is_empty() { section class="mb-10 space-y-4" { div class="flex items-center gap-3" {
        (heading::H2("xl",false,"font-heading font-semibold",html!{(p.title.trim())}))
        @if !p.eyebrow.trim().is_empty() { span class="font-mono text-xs text-foreground/50" {(p.eyebrow.trim())} }
    } @if !p.subtitle.trim().is_empty() { p class="text-sm leading-6 text-muted-foreground" {(p.subtitle.trim())} }
    (media_slide_deck(c,&MediaSlideDeckProps {state_var:p.state_var.clone(),items:p.items.clone()})) } } }
}
fn uses_video(p:&MediaSlideItem)->bool {p.kind.trim().eq_ignore_ascii_case("video")&&!p.url.trim().is_empty()}
fn primary(c:MediaRenderContext<'_>,p:&MediaSlideItem,class:&str)->Markup {
    if uses_video(p) { return html!{ div class="bg-background/80" { (video_player_block(&VideoPlayerProps {url:p.url.clone(),title:first(&[&p.title,&p.id,&p.url]).into(),provider:p.provider.clone(),thumbnail:p.thumbnail.clone(),class:"mb-0".into(),..Default::default()})) } } }
    if !p.rendered_html.trim().is_empty() {return html!{div class="prose-content slide-prose h-full min-h-[15rem] overflow-auto px-4 py-4 sm:px-5 sm:py-5" {(PreEscaped(&p.rendered_html))}}}
    let preview=if p.kind.trim().eq_ignore_ascii_case("document"){p.thumbnail.trim()}else{first(&[&p.thumbnail,&p.url])};
    let frame=media_preview_frame(c.request,c.translate,preview,first(&[&p.alt,&p.title]),&p.kind,class,&c.t("media.preview.alt_fallback"));
    html! { @if !p.url.trim().is_empty() { a href=(p.url.trim()) target="_blank" rel="noopener noreferrer" class="block h-full" aria-label=(first(&[&p.alt,&p.title,&p.id,&p.url])) {(frame)} } @else {(frame)} }
}
fn details(c:MediaRenderContext<'_>,p:&MediaSlideItem,variant:&'static str,class:&str)->Markup {
    let label=c.t("media.open"); let label=if label=="media.open"{"Open media"}else{&label};
    html! { div class="flex flex-wrap items-center gap-2" {(media_type_badge(c.request,c.translate,&p.kind,""))(provider_chip(&p.provider,""))}
    div class="space-y-2" { h3 class="break-words text-lg font-semibold leading-tight text-foreground sm:text-xl" {(first(&[&p.title,&p.id,&p.url]))} @if !p.description.trim().is_empty() {p class="text-sm leading-6 text-muted-foreground" {(p.description.trim())}} }
    @if !p.url.trim().is_empty() { (button::Button(&button::ButtonProps {as_element:shared::ELEMENT_A,href:p.url.trim().into(),variant,size:shared::SIZE_SM,class:class.into(),attributes:super::render::attrs(&[("target","_blank"),("rel","noopener noreferrer")]),..Default::default()},html!{(label)})) } }
}
fn media_slide_panel(c:MediaRenderContext<'_>,p:&MediaSlideItem)->Markup {
    html!{ article class="slide-panel-inner p-6 sm:p-8" { @if uses_video(p) {
        div class="flex h-full min-h-0 flex-col overflow-hidden rounded-[1rem] border border-border/40 bg-background/80" { (primary(c,p,"aspect-video w-full rounded-none border-0 object-cover")) div class="flex min-w-0 flex-1 flex-col justify-center gap-4 border-t border-border/40 px-5 py-5 sm:px-6" {(details(c,p,shared::VARIANT_PRIMARY,"w-fit"))} }
    } @else { div class="grid h-full min-h-0 gap-5 lg:grid-cols-[minmax(0,1.35fr)_minmax(16rem,0.65fr)] lg:items-center" {div class="min-h-0 overflow-hidden rounded-[1rem] border border-border/40 bg-background/65" {(primary(c,p,"h-full min-h-[15rem] rounded-none border-0 object-cover"))} div class="flex min-w-0 flex-col justify-center gap-4" {(details(c,p,shared::VARIANT_OUTLINE,"w-fit border-border bg-background text-foreground hover:border-primary/35 hover:bg-primary/5 hover:text-primary"))} } } } }
}
fn media_single_item_card(c:MediaRenderContext<'_>,p:&MediaSlideItem)->Markup { html!{ article class="overflow-hidden rounded-[1.35rem] border border-border/40 bg-card/80 slide-shadow" {(primary(c,p,"aspect-video w-full rounded-none border-0 object-cover")) div class="flex min-w-0 flex-col gap-4 border-t border-border/40 px-5 py-5 sm:px-6" {(details(c,p,shared::VARIANT_PRIMARY,"w-fit"))} } } }
