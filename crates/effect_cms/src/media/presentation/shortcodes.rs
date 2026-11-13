//! Consumer-store media shortcode resolution through the canonical content catalog.
use std::sync::Arc;
use effect::{Effect,effect};
use crate::content::{ShortcodeCatalog,ShortcodeResolver,ShortcodeToken,ContentRenderOptions,RenderedContentFragment,shortcode_arg};
use crate::media::assets::{MediaAssetStore,MediaAssetURLResolver,media_asset_primary_url,media_asset_thumbnail_url,media_asset_display_name};
use super::{EmbeddedMediaProps,embedded_media_block,view_text};

pub struct MediaAssetShortcodeRenderer<E:Send+'static,R:Send+Sync+'static>{
    pub assets:Option<Arc<dyn MediaAssetStore<E,R>>>,pub resolve:Option<MediaAssetURLResolver>,pub image_kind:String,
}
impl<E:Send+'static,R:Send+Sync+'static> Clone for MediaAssetShortcodeRenderer<E,R>{fn clone(&self)->Self{Self{assets:self.assets.clone(),resolve:self.resolve.clone(),image_kind:self.image_kind.clone()}}}
pub fn new_media_asset_shortcode_renderer<E:Send+'static,R:Send+Sync+'static>(store:Option<Arc<dyn MediaAssetStore<E,R>>>,resolve:Option<MediaAssetURLResolver>)->MediaAssetShortcodeRenderer<E,R>{MediaAssetShortcodeRenderer{assets:store,resolve,image_kind:"image".into()}}
impl<E:Send+'static,R:Send+Sync+'static> MediaAssetShortcodeRenderer<E,R>{
    pub fn render_media_shortcode(&self,id:crate::commands::InputBytes,_options:ContentRenderOptions)->Effect<RenderedContentFragment,E,R>{
        let renderer=self.clone();
        effect!{let renderer=renderer.clone();let id=id.clone();
            match renderer.assets.clone(){
                None=>RenderedContentFragment::default(),
                Some(assets)=>{
                    let asset=assets.get_by_id(id.trim())?;
                    match asset{
                        None=>RenderedContentFragment::default(),
                        Some(asset)=>{
                            let image_kind=if renderer.image_kind.trim().is_empty(){"image"}else{renderer.image_kind.trim()};
                            let props=EmbeddedMediaProps{kind:asset.kind.trim().into(),url:view_text(media_asset_primary_url(&asset,renderer.resolve.as_ref())),thumbnail:view_text(media_asset_thumbnail_url(&asset,image_kind,renderer.resolve.as_ref())),
                                alt:view_text(asset.alt.trim()),title:view_text(media_asset_display_name(&asset)),description:view_text(asset.description.trim()),provider:view_text(asset.provider.trim()),caption:String::new()};
                            RenderedContentFragment{html:embedded_media_block(&props),block:true,cache_key:format!("media|{}",view_text(id.trim())),..Default::default()}
                        }
                    }
                }
            }
        }
    }
}
impl<E:Send+'static,R:Send+Sync+'static> ShortcodeResolver<E,R> for MediaAssetShortcodeRenderer<E,R>{
    fn name(&self)->String{"media".into()}
    fn resolve(&self,token:ShortcodeToken,options:ContentRenderOptions)->Effect<RenderedContentFragment,E,R>{
        let id=shortcode_arg(&token,&["id","value"]);
        if id.is_empty(){return effect::effect_sync!{body:||RenderedContentFragment::default()};}
        self.render_media_shortcode(id.into(),options)
    }
}
impl<E:Send+'static,R:Send+Sync+'static> ShortcodeCatalog<E,R> for MediaAssetShortcodeRenderer<E,R>{fn resolvers(&self)->Vec<Arc<dyn ShortcodeResolver<E,R>>>{vec![Arc::new(self.clone())]}}
/// Consumer-owned catalog registration; the CMS installs only the supplied resolver.
pub fn register_media_shortcode_catalog<E:Send+'static,R:Send+Sync+'static>(register:Option<&(dyn Fn(Arc<dyn ShortcodeCatalog<E,R>>)+Send+Sync)>,store:Option<Arc<dyn MediaAssetStore<E,R>>>,resolve:Option<MediaAssetURLResolver>){
    if let Some(register)=register{register(Arc::new(new_media_asset_shortcode_renderer(store,resolve)));}
}
