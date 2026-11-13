//! Deterministic media plans seeded through consumer-owned stores.

use std::{collections::BTreeMap, sync::Arc};
use effect::{AtomicRef, Effect, effect, effect_try_sync, effect_for_each, effect_map};
use maud::html;
use uuid::Uuid;
use crate::{attachments::{MediaAttachmentStoreRecord, media_shortcode}, commands::InputBytes};
use crate::media::{assets::{MediaAssetRecord, MediaAssetStore, ContentMediaAttachmentStore,
    normalize_media_kind, normalize_media_source_kind, media_asset_primary_url},
    contentmedia::{ContentMediaItem, ContentVideo, ContentVideoProvider, ContentVideoMetadata,
    marshal_content_media_json, marshal_content_video_json}};

/// One declared demo asset.
#[derive(Clone, Debug, Default)]
pub struct DemoMediaAssetSpec {
    pub key: String, pub kind: String, pub source_kind: String,
    pub url: InputBytes, pub provider: InputBytes, pub title: InputBytes,
    pub description: InputBytes, pub thumbnail_url: InputBytes, pub alt: InputBytes,
    pub duration: i64,
}
/// One declared demo attachment.
#[derive(Clone, Debug, Default)]
pub struct DemoMediaAttachmentSpec {
    pub asset_key: String, pub role: InputBytes, pub title_override: InputBytes,
    pub caption: InputBytes, pub alt_override: InputBytes,
}
/// The assets, attachments and inline references for one content record.
#[derive(Clone, Debug, Default)]
pub struct DemoContentMediaPlan {
    pub assets: Vec<DemoMediaAssetSpec>, pub attachments: Vec<DemoMediaAttachmentSpec>,
    pub inline_shortcode_keys: Vec<String>, pub featured_video_key: String,
}
/// A deterministic library over supplied shared asset specifications.
#[derive(Clone, Debug, Default)]
pub struct DemoMediaLibrary {
    pub namespace: String, pub shared_assets: BTreeMap<String, DemoMediaAssetSpec>,
    pub created_by: String,
}

/// Derive a stable namespaced identity.
pub fn stable_demo_id(namespace: &str, parts: &[&str]) -> String {
    let namespace = if namespace.trim().is_empty() { "contentmgmt/demo" } else { namespace.trim() };
    let name = format!("{namespace}/{}", parts.iter().map(|part| part.trim()).collect::<Vec<_>>().join("|"));
    Uuid::new_v5(&Uuid::NAMESPACE_URL, name.as_bytes()).to_string()
}
impl DemoMediaLibrary {
    pub fn asset_id(&self, key: &str) -> String { stable_demo_id(&self.namespace, &["media-asset", key]) }
    pub fn content_id(&self, owner_type: &str, slug: &str) -> String {
        stable_demo_id(&self.namespace, &["content", owner_type, slug])
    }
    pub fn attachment_id(&self, owner_type: &str, owner_id: &str, key: &str, position: usize) -> String {
        stable_demo_id(&self.namespace, &["media-attachment", owner_type, owner_id, key, &position.to_string()])
    }
    pub fn shortcode(&self, key: &str, role: &str) -> String {
        if key.trim().is_empty() { String::new() } else { crate::byte_text::encoding_text(media_shortcode(&self.asset_id(key), role).as_bytes()) }
    }
    pub fn asset_record(&self, spec: &DemoMediaAssetSpec) -> MediaAssetRecord {
        MediaAssetRecord {
            id: self.asset_id(&spec.key), kind: normalize_media_kind(&spec.kind),
            source_kind: normalize_media_source_kind(&spec.source_kind), status: "ready".into(),
            url: spec.url.trim(), provider: spec.provider.trim(), title: spec.title.trim(),
            description: spec.description.trim(), thumbnail_url: spec.thumbnail_url.trim(),
            alt: spec.alt.trim(), duration: (spec.duration > 0).then(||Arc::new(std::sync::atomic::AtomicI64::new(spec.duration))),
            created_by: if self.created_by.trim().is_empty() { "system".into() } else { self.created_by.trim().into() },
            ..MediaAssetRecord::default()
        }
    }
    pub fn upsert_asset<E: Send + 'static, R: Send + Sync + 'static>(
        &self, store: Option<Arc<dyn MediaAssetStore<E, R>>>, spec: DemoMediaAssetSpec,
        unavailable: Arc<dyn Fn(&str) -> E + Send + Sync>,
    ) -> Effect<MediaAssetRecord, E, R> {
        let library = self.clone();
        effect! {
            let library = library.clone(); let store = store.clone(); let spec = spec.clone();
            let unavailable = unavailable.clone();
            let store = effect_try_sync! { body: move || store.clone().ok_or_else(|| unavailable("demo media library: media asset store is nil")) }?;
            let record = library.asset_record(&spec);
            let existing = store.get_by_id(record.id.clone().into())?;
            let create = existing.is_none();
            let record = match existing {
                None => record,
                Some(mut existing) => {
                    existing.kind = record.kind; existing.source_kind = record.source_kind;
                    existing.url = record.url; existing.status = record.status;
                    existing.provider = record.provider; existing.title = record.title;
                    existing.description = record.description; existing.thumbnail_url = record.thumbnail_url;
                    existing.alt = record.alt; existing.duration = record.duration;
                    existing.cdn_url = InputBytes::default(); existing.created_by = record.created_by;
                    existing
                }
            };
            let handle = AtomicRef::new(record);
            let write = if create { store.create(handle.clone()) } else { store.update(handle.clone()) };
            write?;
            handle.with(Clone::clone)?
        }
    }
    pub fn spec_for_key(&self, plan: &DemoContentMediaPlan, key: &str) -> Option<DemoMediaAssetSpec> {
        let key = key.trim();
        if key.is_empty() { return None; }
        plan.assets.iter().find(|spec| spec.key.trim() == key).cloned()
            .or_else(|| self.shared_assets.get(key).cloned())
    }
    pub fn seed_content_media<E: Send + 'static, R: Send + Sync + 'static>(
        &self, assets: Option<Arc<dyn MediaAssetStore<E,R>>>,
        attachments: Option<Arc<dyn ContentMediaAttachmentStore<E,R>>>,
        owner_type: String, owner_id: String, hero: DemoMediaAssetSpec, plan: DemoContentMediaPlan,
        unavailable: Arc<dyn Fn(&str) -> E + Send + Sync>,
    ) -> Effect<BTreeMap<String,MediaAssetRecord>,E,R> {
        let library = self.clone();
        effect! {
            let library = library.clone(); let assets = assets.clone(); let attachments = attachments.clone();
            let owner_type = owner_type.clone(); let owner_id = owner_id.clone(); let hero = hero.clone();
            let plan = plan.clone(); let unavailable = unavailable.clone(); let missing = unavailable.clone();
            let attachments = effect_try_sync! { body: move || attachments.clone().ok_or_else(|| missing("demo media library: content media attachment store is nil")) }?;
            let hero_asset = library.upsert_asset(assets.clone(), hero.clone(), unavailable.clone())?;
            let mut by_key = BTreeMap::new();
            by_key.insert(hero.key.clone(), hero_asset.clone());
            let specs = seed_specs(&library,&plan);
            let seeded = upsert_seed_specs(library.clone(),assets.clone(),unique_seed_specs(&hero.key,specs),unavailable.clone())?;
            by_key.extend(seeded);
            let rows = seed_attachment_rows(&library,&owner_type,&owner_id,&hero,hero_asset,&plan,&by_key);
            attachments.replace_by_owner(owner_type.into(),owner_id.into(),rows)?;
            by_key
        }
    }
    pub fn ensure_markdown_shortcodes(&self, content: &str, keys: &[String]) -> String {
        let missing = self.missing_shortcodes(content,keys);
        if missing.is_empty() { return content.trim().into(); }
        let section = format!("## Demo Media\n\n{}",missing.join("\n\n"));
        if content.trim().is_empty() { section } else { format!("{}\n\n{section}",content.trim()) }
    }
    pub fn ensure_html_shortcodes(&self, content: &str, keys: &[String]) -> String {
        let missing = self.missing_shortcodes(content,keys);
        if missing.is_empty() { return content.trim().into(); }
        html! {
            (maud::PreEscaped(content.trim()))
            @if !content.trim().is_empty() { "\n" }
            @for (index,shortcode) in missing.iter().enumerate() {
                @if index > 0 { "\n" }
                p { (shortcode) }
            }
        }.into_string()
    }
    fn missing_shortcodes(&self, content: &str, keys: &[String]) -> Vec<String> {
        keys.iter().map(|key| self.shortcode(key,"inline"))
            .filter(|code| !code.is_empty() && !content.trim().contains(code)).collect()
    }
    pub fn featured_video_json(&self, plan: &DemoContentMediaPlan, assets: &BTreeMap<String,MediaAssetRecord>) -> Result<Option<String>,serde_json::Error> {
        let Some(asset) = assets.get(plan.featured_video_key.trim()) else { return Ok(None); };
        if !asset.kind.eq_ignore_ascii_case("video") { return Ok(None); }
        let video = ContentVideo {
            id: asset.id.clone().into(), title: first_non_blank(&[asset.title.clone(),asset.description.clone(),"Demo video".into()]),
            url: media_asset_primary_url(asset,None),
            provider: ContentVideoProvider { name: first_non_blank(&[asset.provider.clone(),"Video".into()]), base_url: demo_provider_base_url(&asset.provider).into() },
            metadata: ContentVideoMetadata { description: asset.description.clone(), thumbnail: asset.thumbnail_url.trim(), duration: asset.duration.as_ref().map(|value|value.load(std::sync::atomic::Ordering::SeqCst)).unwrap_or(0), ..ContentVideoMetadata::default() },
            ..ContentVideo::default()
        };
        marshal_content_video_json(Some(&video))
    }
    pub fn plan_media_json(&self, hero_key: &str, plan: &DemoContentMediaPlan, assets: &BTreeMap<String,MediaAssetRecord>) -> Result<String,serde_json::Error> {
        let mut keys = vec![hero_key.trim().to_owned()];
        keys.extend(plan.attachments.iter().map(|spec| spec.asset_key.trim().to_owned()));
        demo_content_media_json(&keys,assets)
    }
}
pub fn demo_content_media_json(keys: &[String], assets: &BTreeMap<String,MediaAssetRecord>) -> Result<String,serde_json::Error> {
    let items = keys.iter().filter_map(|key| assets.get(key.trim())).map(|asset| ContentMediaItem {
        kind: asset.kind.trim().into(), url: media_asset_primary_url(asset,None), thumbnail: asset.thumbnail_url.trim(),
        alt: asset.alt.trim(), title: asset.title.trim(), description: asset.description.trim(), provider: asset.provider.trim(),
        duration: asset.duration.as_ref().map(|value|value.load(std::sync::atomic::Ordering::SeqCst)).unwrap_or(0), ..ContentMediaItem::default()
    }).collect::<Vec<_>>();
    marshal_content_media_json(&items)
}
pub fn demo_provider_base_url(provider: impl AsRef<[u8]>) -> &'static str {
    match crate::byte_text::lowercase(crate::validation::trim_unicode_whitespace(provider.as_ref())).as_str() {
        "youtube" => "https://www.youtube.com/", "vimeo" => "https://vimeo.com/",
        "dailymotion" => "https://www.dailymotion.com/", "twitch" => "https://www.twitch.tv/", _ => "",
    }
}
pub fn int_value(value: Option<i64>) -> i64 { value.unwrap_or(0) }
fn first_non_blank(values: &[InputBytes]) -> InputBytes { values.iter().map(InputBytes::trim).find(|value| !value.is_empty()).unwrap_or_default() }

fn seed_specs(library:&DemoMediaLibrary,plan:&DemoContentMediaPlan)->Vec<DemoMediaAssetSpec>{
            let mut specs = plan.assets.clone();
            for spec in &plan.attachments {
                if let Some(spec) = library.spec_for_key(&plan, &spec.asset_key) { specs.push(spec); }
            }
            for key in &plan.inline_shortcode_keys {
                if let Some(spec) = library.spec_for_key(&plan, key) { specs.push(spec); }
            }
            if let Some(spec) = library.spec_for_key(&plan, &plan.featured_video_key) { specs.push(spec); }
    specs
}

fn seed_attachment_rows(library:&DemoMediaLibrary,owner_type:&str,owner_id:&str,hero:&DemoMediaAssetSpec,hero_asset:MediaAssetRecord,plan:&DemoContentMediaPlan,by_key:&BTreeMap<String,MediaAssetRecord>)->Vec<MediaAttachmentStoreRecord>{
            let mut rows = vec![MediaAttachmentStoreRecord {
                id: library.attachment_id(&owner_type,&owner_id,&hero.key,0).into(),
                media_asset_id: hero_asset.id.into(), role: "hero".into(),
                ..MediaAttachmentStoreRecord::default()
            }];
            for (index,spec) in plan.attachments.iter().enumerate() {
                if let Some(asset) = by_key.get(spec.asset_key.trim()) {
                    rows.push(MediaAttachmentStoreRecord {
                        id: library.attachment_id(&owner_type,&owner_id,&spec.asset_key,index+1).into(),
                        media_asset_id: asset.id.clone().into(), role: spec.role.trim(),
                        title_override: spec.title_override.trim(), caption: spec.caption.trim(),
                        alt_override: spec.alt_override.trim(),
                    });
                }
            }
    rows
}

fn unique_seed_specs(hero_key:&str,specs:Vec<DemoMediaAssetSpec>)->Vec<DemoMediaAssetSpec>{
    let mut seen=std::collections::BTreeSet::from([hero_key.to_owned()]);
    specs.into_iter().filter(|spec|{let key=spec.key.trim();!key.is_empty()&&seen.insert(key.to_owned())}).collect()
}
fn upsert_seed_specs<E:Send+'static,R:Send+Sync+'static>(library:DemoMediaLibrary,assets:Option<Arc<dyn MediaAssetStore<E,R>>>,specs:Vec<DemoMediaAssetSpec>,unavailable:Arc<dyn Fn(&str)->E+Send+Sync>)->Effect<Vec<(String,MediaAssetRecord)>,E,R>{
    let upsert=move |spec:DemoMediaAssetSpec|{
        let key=spec.key.trim().to_owned();
        effect_map!{program:library.upsert_asset(assets.clone(),spec,unavailable.clone()),map:move |asset|{(key.clone(),asset)}}
    };
    effect_for_each!{values:specs,body:upsert}
}
