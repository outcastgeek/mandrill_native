use effect_cms::{content::*,localization::*,commands::InputBytes};
use std::sync::Arc;

#[test]
fn formats_select_storage_and_contract_body_without_losing_source() {
    assert_eq!(content_format_from_stored_fields(Some("Body"),None),CONTENT_FORMAT_MARKDOWN);
    assert_eq!(content_format_from_stored_fields(Some("Body"),Some("* Body")),CONTENT_FORMAT_ORG);
    assert_eq!(content_format_from_stored_fields(Some("edn:{:body true}"),None),CONTENT_FORMAT_EDN);
    let edn=content_storage_columns_for_format("{:body true}",CONTENT_FORMAT_EDN);
    assert_eq!(edn.markdown.as_deref(),Some("edn:{:body true}")); assert!(edn.suppress_rendered_html); assert_eq!(edn.org,None);
    let org=content_storage_columns_for_format("* Body",CONTENT_FORMAT_ORG);
    assert_eq!(org.org.as_deref(),Some("* Body")); assert_eq!(org.markdown,None); assert!(!org.suppress_rendered_html);
    assert_eq!(source_content_from_stored_fields(Some("edn:{:body true}"),None),"{:body true}");
    let body=contract_content_body_from_localized_draft(&LocalizedContentDraft {content:"* Body".into(),content_format:CONTENT_FORMAT_ORG.into(),..Default::default()});
    assert_eq!(body.org,"* Body"); assert!(body.markdown.is_empty()&&body.edn.is_empty());
}

#[test]
fn payload_mapper_normalizes_languages_and_completeness_requires_every_language() {
    let mapper=new_contract_payload_mapper(ContractPayloadMapperOptions {required_language_keys:vec!["en".into(),"fr".into()],language_key:Some(Arc::new(|lang|lang.trim().to_lowercase().trim_end_matches("-ca").into()))});
    let drafts=vec![LocalizedContentDraft {lang:"en".into(),description:"English summary".into(),content:"English body".into(),content_format:CONTENT_FORMAT_MARKDOWN.into(),..Default::default()},LocalizedContentDraft {lang:"fr-CA".into(),description:"Résumé".into(),content:"* Corps".into(),content_format:CONTENT_FORMAT_ORG.into(),..Default::default()}];
    let mut content=mapper.content_map(&drafts);
    let descriptions=mapper.content_description_map(&drafts);
    assert_eq!(content["en"].markdown,"English body"); assert_eq!(content["fr"].org,"* Corps"); assert_eq!(descriptions["fr"],"Résumé");
    assert!(mapper.content_complete(&content)&&mapper.strings_complete(&descriptions));
    content.remove("fr"); assert!(!mapper.content_complete(&content));
    let mut keys=mapper.required_language_keys(); keys.clear(); assert_eq!(mapper.required_language_keys(),["en","fr"]);
}

#[test]
fn sanitizer_preserves_authored_rich_attributes_and_removes_handlers() {
    let input=r#"<h2 id="headline-9" class="toc-target" onclick="alert(1)">Heading</h2><div class="not-prose mermaid-widget" data-mermaid-widget><pre style="color:#f8f8f2"><code class="language-mermaid" data-lang="mermaid" data-mermaid-source="true">graph TB&#10;A--&gt;B</code></pre></div><span class="math-inline">x^2</span><img src="https://example.com/image.png" onerror="alert(1)">"#;
    let output=sanitize_html(input);
    for expected in ["id=\"headline-9\"","class=\"toc-target\"","data-mermaid-widget","class=\"language-mermaid\"","data-lang=\"mermaid\"","data-mermaid-source=\"true\"","class=\"math-inline\""] {assert!(output.contains(expected),"missing {expected}: {output}");}
    assert!(!output.contains("onclick")&&!output.contains("onerror"));
}

#[test]
fn asset_urls_use_explicit_cdn_then_public_base_and_keep_absolute_inputs() {
    let cdn=ContentAssetConfig::from_base_url(Some("https://cdn.example.com/assets/"));
    assert_eq!(content_asset_url(&cdn,"/images/hero.jpg"),"https://cdn.example.com/assets/images/hero.jpg");
    let unset=ContentAssetConfig::default();
    assert_eq!(absolute_content_asset_url(&unset,"https://example.com/resources","images/hero.jpg"),"https://example.com/resources/images/hero.jpg");
    assert_eq!(absolute_content_asset_url(&unset,"https://example.com","images/hero.jpg"),"https://example.com/images/hero.jpg");
    assert_eq!(absolute_content_asset_url(&unset,"https://example.com","https://media.example.com/hero.jpg"),"https://media.example.com/hero.jpg");
    assert_eq!(content_asset_url(&cdn,""),"");
}

#[test]
fn delimited_json_and_filename_helpers_keep_source_order_and_fallbacks() {
    assert_eq!(slug_filename("launch","md","untitled.md"),"launch.md");
    assert_eq!(slug_filename("",".md","untitled.md"),"untitled.md");
    assert_eq!(split_delimited_values("Ada, Grace\nLinus; Ken"),["Ada","Grace","Linus","Ken"]);
    assert_eq!(json_text_object_pointer("  123 Main  ","formatted").as_deref(),Some(r#"{"formatted":"123 Main"}"#));
    assert_eq!(json_text_object_value(r#"{"formatted":" 123 Main "}"#,"formatted"),"123 Main");
    assert_eq!(json_text_object_value("not-json","formatted"),"");
    assert_eq!(delimited_json_text_pairs("Ada\nGrace","ada@example.test\ngrace@example.test","name","email"),r#"[{"email":"ada@example.test","name":"Ada"},{"email":"grace@example.test","name":"Grace"}]"#);
    assert_eq!(delimited_json_text_pairs("","","name","email"),"[]");
    assert_eq!(json_text_pair_values(r#"[{"name":"Ada","email":"ada@example.test"},{"name":"Grace"}]"#,"name","email"),("Ada\nGrace".into(),"ada@example.test".into()));
}

#[test]
fn localized_collection_distinguishes_absent_values_and_preserves_raw_content_bytes() {
    use std::sync::atomic::{AtomicUsize,Ordering};
    let calls=AtomicUsize::new(0);
    let normalize=|s:&str|{calls.fetch_add(1,Ordering::SeqCst);s.to_lowercase()};
    let langs=vec!["en".to_owned()];
    let absent=collect_localized_content_drafts_from_values(None,&langs,Some(&normalize),None);
    assert!(absent[0].required&&absent[0].content.is_empty()); assert_eq!(calls.load(Ordering::SeqCst),0);
    let empty=LocalizedFieldValues::default();
    let _=collect_localized_content_drafts_from_values(Some(&empty),&langs,Some(&normalize),None);
    assert!(calls.load(Ordering::SeqCst)>0);
    let options=LocalizedRowOptions {row_language:Some(Arc::new(|row:&(&str,&[u8])|row.0.into())),content:Some(Arc::new(|row:&(&str,&[u8])|InputBytes::from(row.1))),..Default::default()};
    let bytes:&[u8]=b" \xff body ";
    let drafts=localized_content_drafts_from_rows(&langs,&[("en",bytes)],&options);
    assert_eq!(drafts[0].content.as_bytes(),bytes);
}

#[test]
fn link_shortcode_catalog_delegates_inputs_and_uses_generic_cache_key() {
    use std::{collections::BTreeMap,sync::atomic::{AtomicUsize,Ordering}};
    let calls=Arc::new(AtomicUsize::new(0));let seen=calls.clone();
    let catalog=new_content_link_shortcode_catalog::<std::fmt::Error,()>(ContentLinkShortcodeCatalogOptions {include_presentation:true,links:vec![ContentLinkShortcodeResolverOptions {name:"document-link".into(),value_keys:vec!["slug".into(),"value".into()],label_keys:vec![],resolve:Some(Arc::new(move |req,opts| {let seen=seen.clone();effect::effect_sync! {body:move || {seen.fetch_add(1,Ordering::SeqCst);assert_eq!(req.value,"platform-refactor");assert_eq!(req.label,"Read it");ContentLinkShortcodeTarget {default_label:"Platform Refactor".into(),href:format!("/{}/documents/{}",opts.lang,req.value),..Default::default()}},}}))}]});
    let resolvers=catalog.resolvers();assert_eq!(resolvers.len(),3);
    let program=resolvers[0].resolve(ShortcodeToken {name:"document-link".into(),args:BTreeMap::from([("slug".into(),"platform-refactor".into()),("label".into(),"Read it".into())]),..Default::default()},ContentRenderOptions {lang:"fr".into(),..Default::default()});
    assert_eq!(calls.load(Ordering::SeqCst),0);
    let fragment=super::run(program).unwrap();let html=fragment.html.into_string();assert!(html.contains("href=\"/fr/documents/platform-refactor\"")&&html.contains("Read it"));assert_eq!(fragment.cache_key,"document-link|platform-refactor|fr");
    let empty=super::run(resolvers[0].resolve(ShortcodeToken::default(),ContentRenderOptions::default())).unwrap();assert!(empty.html.into_string().is_empty());assert_eq!(calls.load(Ordering::SeqCst),1);
}

#[test]
fn localized_content_link_selects_requested_row_and_falls_back_to_first_row() {
    #[derive(Clone)] struct Row {lang:&'static str,slug:&'static str,title:&'static str}
    let resolver=new_localized_content_link_shortcode_resolver::<Row,std::fmt::Error,()>(LocalizedContentLinkShortcodeResolverOptions {name:"document-link".into(),value_keys:vec!["slug".into()],label_keys:vec![],rows:Some(Arc::new(|req,_|effect::effect_sync! {body:move || {assert_eq!(req.value,"platform");vec![Some(Row {lang:"en",slug:"platform",title:"Platform"}),Some(Row {lang:"fr",slug:"plateforme",title:"Plateforme"})]},})),row_options:LocalizedRowOptions {normalize_language:Some(Arc::new(str::to_lowercase)),row_language:Some(Arc::new(|row:&Row|row.lang.into())),..Default::default()},default_label:Some(Arc::new(|row|row.title.into())),href:Some(Arc::new(|row,opts|format!("/{}/documents/{}",opts.lang,row.slug))),cache_key:Some(Arc::new(|row,opts|format!("document-link|{}|{}",row.slug,opts.lang))),block:None});
    let resolve=resolver.resolve.unwrap();
    for (lang,slug,title) in [("fr","plateforme","Plateforme"),("es","platform","Platform")] {let target=super::run(resolve(ContentLinkShortcodeRequest {value:"platform".into(),..Default::default()},ContentRenderOptions {lang:lang.into(),..Default::default()})).unwrap();assert_eq!(target.href,format!("/{lang}/documents/{slug}"));assert_eq!(target.default_label,title);assert_eq!(target.cache_key,format!("document-link|{slug}|{lang}"));}
}

#[test]
fn content_link_target_maps_lookup_and_retains_absence() {
    let resolver=new_content_link_target_resolver::<String,std::fmt::Error,()>(ContentLinkTargetResolverOptions {name:"collection-link".into(),value_keys:vec!["collection".into()],label_keys:vec![],lookup:Some(Arc::new(|req,_|effect::effect_sync! {body:move || if req.value.is_empty(){None}else{Some(req.value.clone())},})),default_label:Some(Arc::new(Clone::clone)),href:Some(Arc::new(|row,opts|format!("/{}/collections/{row}",opts.lang))),cache_key:None,block:None});
    let resolve=resolver.resolve.unwrap();
    let target=super::run(resolve(ContentLinkShortcodeRequest {value:"montreal".into(),..Default::default()},ContentRenderOptions {lang:"en".into(),..Default::default()})).unwrap();assert_eq!(target.href,"/en/collections/montreal");assert_eq!(target.default_label,"montreal");
    assert!(super::run(resolve(ContentLinkShortcodeRequest::default(),ContentRenderOptions::default())).unwrap().href.is_empty());
}
