use super::*;
use effect_cms::{attachments::*,media::{assets::*,contentmedia::*,presentation::*}};
use effect_server::ByteFormData;
use std::sync::atomic::{AtomicUsize,Ordering};
fn cdn()->MediaAssetURLResolver {Arc::new(|raw|{if raw.starts_with(b"/"){[b"https://cdn.example/assets".as_slice(),raw].concat().into()}else{raw.into()}})}
#[test]fn primary_preview_resolve_paths_and_preserve_explicit_cdn(){
 let resolver=cdn();let image=MediaAssetRecord{kind:"image".into(),url:"/uploads/demo.jpg".into(),..Default::default()};
 assert_eq!(media_asset_primary_url(&image,Some(&resolver)).as_bytes(),b"https://cdn.example/assets/uploads/demo.jpg");
 assert_eq!(media_asset_preview_url(&image,"image",Some(&resolver)).as_bytes(),b"https://cdn.example/assets/uploads/demo.jpg");
 let video=MediaAssetRecord{kind:"video".into(),url:"/raw.mp4".into(),cdn_url:"https://edge/final.mp4".into(),thumbnail_url:"/thumb.jpg".into(),..Default::default()};
 assert_eq!(media_asset_primary_url(&video,Some(&resolver)).as_bytes(),b"https://edge/final.mp4");
 assert_eq!(media_asset_preview_url(&video,"image",Some(&resolver)).as_bytes(),b"https://cdn.example/assets/thumb.jpg");
 for item in [MediaAssetRecord::default(),MediaAssetRecord{kind:"document".into(),url:"/manual.pdf".into(),..Default::default()}]{assert!(media_asset_preview_url(&item,"image",Some(&resolver)).is_empty())}
 assert!(media_asset_primary_url(&MediaAssetRecord::default(),Some(&resolver)).is_empty());
}
#[test]fn creation_defaults_are_lazy_and_update_preserves_created_metadata(){
 let calls=Arc::new(AtomicUsize::new(0));let count=calls.clone();let id:MediaAssetIdGenerator=Arc::new(move||{count.fetch_add(1,Ordering::SeqCst);"asset-1".into()});let now:MediaAssetNowUnix=Arc::new(||99);
 let program=prepare_record_for_create::<(),()>(MediaAssetRecord{kind:" video ".into(),source_kind:" embed ".into(),status:" ready ".into(),url:" /demo.mp4 ".into(),title:" Demo ".into(),..Default::default()},Some(&id),Some(&now));
 assert_eq!(calls.load(Ordering::SeqCst),0);let record=success(program,Arc::new(()));assert_eq!(calls.load(Ordering::SeqCst),1);
 assert_eq!((record.id.as_str(),record.kind.as_str(),record.source_kind.as_str(),record.status.as_str()),("asset-1","video","embed","ready"));assert_eq!(record.url.as_bytes(),b"/demo.mp4");assert_eq!(record.title.as_bytes(),b"Demo");assert_eq!((record.created_at,record.updated_at),(99,99));
 let later:MediaAssetNowUnix=Arc::new(||120);let record=success(prepare_record_for_update::<(),()>(record,Some(&later)),Arc::new(()));assert_eq!((record.created_at,record.updated_at),(99,120));
}
#[test]fn attachment_store_replace_skips_blank_and_retains_original_positions(){
 let id:MediaAssetIdGenerator=Arc::new(||"attachment-1".into());let now:MediaAssetNowUnix=Arc::new(||42);
 let records=success(prepare_media_attachment_store_records_for_replace::<(),()>(&[MediaAttachmentStoreRecord::default(),MediaAttachmentStoreRecord{media_asset_id:" asset-1 ".into(),role:" hero ".into(),caption:" Caption ".into(),..Default::default()}],Some(&id),Some(&now)),Arc::new(()));
 assert_eq!(records.len(),1);let r=&records[0];assert_eq!(r.record.media_asset_id.as_bytes(),b"asset-1");assert_eq!(r.record.role.as_bytes(),b"hero");assert_eq!(r.record.caption.as_bytes(),b"Caption");assert_eq!((r.position,r.created_at,r.updated_at),(1,42,42));
}
#[test]fn attachment_collection_orders_columns_and_preserves_callback_observations(){
 let mut form=ByteFormData{values:Default::default()};form.values.insert(b"attachment_ids[]".to_vec(),vec![b" a ".to_vec(),Vec::new(),b"b".to_vec()]);form.values.insert(b"attachment_positions[]".to_vec(),vec![b"2".to_vec(),b"0".to_vec(),b"1".to_vec()]);form.values.insert(b"attachment_roles[]".to_vec(),vec![b" hero ".to_vec(),b"blank-row".to_vec(),b"inline".to_vec()]);
 let seen=Arc::new(std::sync::Mutex::new(Vec::new()));let output=seen.clone();let normalizer:RoleNormalizer=Arc::new(move|raw|{output.lock().unwrap().push(raw.to_vec());raw.into()});
 let rows=collect_media_attachment_inputs(Some(&form),Some(normalizer.as_ref()));assert_eq!(rows.iter().map(|i|i.asset_id.as_bytes()).collect::<Vec<_>>(),vec![b"b".as_slice(),b"a".as_slice()]);assert_eq!(*seen.lock().unwrap(),vec![b" hero ".to_vec(),b"blank-row".to_vec(),b"inline".to_vec()]);
 form.values.insert(b"ids".to_vec(),vec![b"fallback".to_vec()]);form.values.insert(b"primary".to_vec(),vec![Vec::new()]);assert_eq!(form_values_with_fallback(Some(&form),"primary","ids"),&[Vec::<u8>::new()]);
}
#[test]fn attachment_promote_dedupe_remove_preserve_one_hero_and_order(){
 let options=MediaAttachmentRoleOptions::default();let input=vec![MediaAttachmentInput{asset_id:"a".into(),role:"hero".into(),..Default::default()},MediaAttachmentInput{asset_id:"b".into(),role:"gallery".into(),..Default::default()},MediaAttachmentInput{asset_id:"a".into(),role:"inline".into(),..Default::default()}];
 let deduped=dedupe_media_attachment_inputs(&input,None);assert_eq!(deduped.len(),2);let promoted=promote_attachment_input(&deduped,"b",&options);assert_eq!(promoted[0].asset_id.as_bytes(),b"b");assert_eq!(promoted[0].role.as_bytes(),b"hero");assert_eq!(promoted[1].asset_id.as_bytes(),b"a");assert_eq!(promoted[1].role.as_bytes(),b"gallery");let removed=remove_attachment_input(&promoted,"a",None);assert_eq!(removed.len(),1);assert_eq!(removed[0].asset_id.as_bytes(),b"b");
}
#[test]fn content_attachment_video_provider_normalizes_and_collection_excludes_embedded(){
 let items=content_media_items_from_attachment_items(&[MediaAttachmentFormItem{asset_id:"video".into(),kind:"video".into(),provider:"YouTube".into(),url:"https://youtube.com/watch?v=1".into(),..Default::default()}]);assert_eq!(items.len(),1);assert_eq!(items[0].provider.as_bytes(),b"youtube");
 let raw=r#"[{"id":"hero","role":"hero","type":"image","url":"/hero.png"},{"id":"inline","type":"video","url":"https://youtube.com/watch?v=1"},{"id":"gallery","type":"image","url":"/gallery.png"}]"#;
 let view=build_content_media_collection_view(&ContentMediaCollectionViewOptions{title:"Story".into(),description:"[media id=\"inline\"]".into(),media_json:raw.into(),resolve_url:Some(cdn()),..Default::default()});
 assert_eq!(view.hero_model.as_ref().unwrap().id.as_bytes(),b"hero");assert_eq!(view.hero_block.as_ref().unwrap().url,"https://cdn.example/assets/hero.png");assert_eq!(view.slide_items.len(),1);assert_eq!(view.slide_items[0].id,"gallery");
}
#[test]fn preview_map_skips_blank_keys_and_trims_both_projections(){let rows=[(" first "," /first.jpg "),("","/blank.jpg")];let map=build_resource_preview_image_map(&rows,Some(&|v:&(&str,&str)|v.0.into()),Some(&|v:&(&str,&str)|v.1.into()));assert_eq!(map.len(),1);assert_eq!(map["first"],"/first.jpg");}
