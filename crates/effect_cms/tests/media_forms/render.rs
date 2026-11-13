use effect_cms::{media::{presentation::*,assets::*},attachments::*};
fn context()->MediaRenderContext<'static>{MediaRenderContext::default()}
fn contains(html:&str,markers:&[&str]){for marker in markers{assert!(html.contains(marker),"missing {marker:?}: {html}")}}
#[test]fn single_video_uses_player_before_prepared_shortcode_and_multiple_items_use_pager(){
 let item=MediaSlideItem{id:"recording-1".into(),kind:"video".into(),url:"https://youtu.be/id".into(),provider:"youtube".into(),title:"Recording".into(),rendered_html:"<figure>wrong fallback</figure>".into(),..Default::default()};
 let one=media_slide_deck(context(),&MediaSlideDeckProps{state_var:"recordingSlide".into(),items:vec![item.clone()]}).into_string();contains(&one,&["<video-player","provider=\"youtube\"","Recording"]);assert!(!one.contains("wrong fallback"));assert!(!one.contains("recordingSlideCurrent"));
 let two=media_slide_deck(context(),&MediaSlideDeckProps{state_var:"recordingSlide".into(),items:vec![item.clone(),MediaSlideItem{id:"second".into(),..item}]}).into_string();assert_eq!(two.matches("<video-player").count(),2);contains(&two,&["recordingSlideCurrent","slide-pager"]);
}
#[test]fn document_slide_and_shortcode_picker_never_use_document_as_image(){
 let deck=media_slide_deck(context(),&MediaSlideDeckProps{items:vec![MediaSlideItem{id:"doc".into(),kind:"document".into(),url:"/manual.pdf".into(),title:"Manual".into(),..Default::default()}],..Default::default()}).into_string();contains(&deck,&["href=\"/manual.pdf\"","Manual"]);assert!(!deck.contains("src=\"/manual.pdf\""));
 let modal=media_shortcode_modal(context(),&ShortcodeToolsProps{root_id:"root".into(),dialog_id:"dialog".into(),items:vec![MediaAttachmentFormItem{asset_id:"doc".into(),kind:"document".into(),url:"/manual.pdf".into(),resolved_url:"/cdn/manual.pdf".into(),..Default::default()}],..Default::default()}).into_string();assert!(!modal.contains("src=\"/cdn/manual.pdf\""));contains(&modal,&["data-shortcode-preview","data-shortcode-insert-button","data-on:click="]);
}
#[test]fn attachment_manager_preserves_drag_mutation_and_owner_form_protocol(){
 let html=media_attachment_manager(context(),&MediaAttachmentManagerProps{scope:"primary".into(),mounts:MediaSurfaceMounts{section_id:"attachments".into(),owner_form_id:"content-form".into(),..Default::default()},mutation_url:"/mutate".into(),attachments:vec![MediaAttachmentFormItem{asset_id:"hero".into(),role:"hero".into(),..Default::default()},MediaAttachmentFormItem{asset_id:"gallery".into(),role:"gallery".into(),title:"Gallery".into(),..Default::default()}],..Default::default()}).into_string();
 contains(&html,&["data-media-attachment-list=\"true\"","data-media-attachment-item=\"gallery\"","draggable=\"true\"","data-on:dragstart=","data-on:dragover=","data-on:drop=","data-attr:data-drop-placement=","attachment_captions[]","mutation=remove","content-form","aria-controls=\"additional-body\""]);
 assert!(!html.contains("data-media-attachment-item=\"hero\""));
}
#[test]fn inline_drawer_paths_use_server_preview_and_stable_form_target(){
 let html=media_inline_create_drawer(context(),&MediaInlineCreateProps{dialog_id:"media-create".into(),root_id:"create-root".into(),picker_root_id:"picker-root".into(),action_url:"/create".into(),preview_url:"/preview".into(),preview_root_id:"create-root-preview".into(),kind:"image".into(),source_kind:"remote".into(),url:"/uploads/image.jpg".into(),preview:MediaInlineCreatePreviewProps{root_id:"create-root-preview".into(),show_remote_image:true,resolved_image_url:"/cdn/image.jpg".into(),..Default::default()},..Default::default()}).into_string();
 contains(&html,&["type=\"text\"","data-on:input__debounce.250ms","data-on:load=","mediaScrollLock","media-ui-inline-create-panel","data-drawer-close=\"true\"","data-attr:aria-pressed=","form=\"media-create-form\"","create-root-preview","src=\"/cdn/image.jpg\"","name=\"picker_root_id\" value=\"picker-root\""]);
 assert!(!html.contains("type=\"url\""));assert!(!html.contains("@get(&#39;/cdn/image.jpg"));
}
#[test]fn picker_and_shortcode_content_layer_overlays_and_preserve_selection_forms(){
 let picker=media_picker(context(),&MediaPickerPageVM{root_id:"picker".into(),attach_form_id:"picker-attach".into(),filter_form_id:"picker-filter".into(),items:vec![MediaPickerItemVM{id:"asset".into(),selection_type:"checkbox".into(),selection_name:"asset_ids[]".into(),checked:true,..Default::default()}],..Default::default()},&MediaPickerRenderOptions::default()).into_string();
 contains(&picker,&["z-index:1000002;","z-index:1000003;","form=\"picker-attach\"","name=\"asset_ids[]\"","checked","data-media-picker-create=\"true\""]);
 let modal=media_shortcode_modal(context(),&ShortcodeToolsProps{root_id:"shortcode".into(),dialog_id:"shortcode-dialog".into(),..Default::default()}).into_string();contains(&modal,&["z-index:1000002;","z-index:1000003;","data-signals="]);
}
#[test]fn library_pagination_submits_explicit_filter_form_without_javascript_defaults(){
 let html=media_library_pagination(context(),&MediaLibraryPaginationProps{form_id:"library-filter".into(),page:2,page_size:24,total:70,pages:3,item_count:24,..Default::default()}).into_string();contains(&html,&["form=\"library-filter\"","name=\"page\"","value=\"1\"","value=\"3\"","evt.preventDefault()","form.requestSubmit()"]);
 assert!(media_library_pagination(context(),&MediaLibraryPaginationProps::default()).into_string().is_empty());
}
#[test]fn return_focus_and_scroll_unlock_wait_until_overlay_removal(){
 let hero=media_picker_return_focus("section","hero").into_string();let additional=media_picker_return_focus("section","gallery").into_string();contains(&hero,&["hero-body","queueMicrotask","el.remove()","media-ui-overlay"]);contains(&additional,&["additional-body"]);
 let lock=media_scroll_lock_action();let unlock=media_scroll_unlock_when_idle_action();contains(&lock,&["document.documentElement","body.style.overflow = 'hidden'","mediaScrollLockPreviousOverflow"]);contains(&unlock,&["document.querySelector('.media-ui-overlay')","mediaScrollLockPreviousOverflow","delete html.dataset.mediaScrollLock"]);
}
#[test]fn inline_preview_helper_keeps_stored_path_and_server_resolves_preview(){
 let resolver:MediaAssetURLResolver=std::sync::Arc::new(|raw|[b"https://cdn.example".as_slice(),raw].concat().into());
 let props=build_media_inline_create_props(&MediaInlineCreatePropsOptions{input:MediaAssetFormInput{kind:"image".into(),source_kind:"remote".into(),url:"/stored.jpg".into(),..Default::default()},default_root_id:"root".into(),resolve_url:Some(resolver),..Default::default()});
 assert_eq!(props.url,"/stored.jpg");assert_eq!(props.preview.resolved_image_url,"https://cdn.example/stored.jpg");assert_eq!(props.preview_root_id,props.preview.root_id);assert!(!props.preview_root_id.is_empty());
}
