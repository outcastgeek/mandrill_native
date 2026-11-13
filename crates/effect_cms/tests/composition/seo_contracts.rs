use std::{collections::BTreeMap,sync::{Arc,atomic::{AtomicUsize,Ordering}}};
use effect::{Exit,effect_sync,effect_try_sync,effect_future};
use effect_cms::{definitions::*,queries::ResourceListQuery,seo::*};
use serde_json::{json,Value};
use super::support::*;

#[test]
fn structured_documents_preserve_schema_positions_video_and_script_escaping()->Result<(),String>{
    let collection:Value=serde_json::from_str(&collection_page_json_ld("Library","Description","/library",11)).map_err(|e|e.to_string())?;
    assert_eq!(collection["@type"],"CollectionPage");assert_eq!(collection["numberOfItems"],11);
    let entries=build_item_list_entries(&["one","skip","three"],Some(&|index,row| (ItemListEntry{url:format!("/{row}"),name:row.to_string(),position:if index==2{9}else{0}},*row!="skip")));
    assert_eq!(entries.iter().map(|e|e.position).collect::<Vec<_>>(),vec![1,9]);
    let video=StructuredVideo{url:"https://video.test/watch".into(),duration_seconds:42,..Default::default()};
    let raw=schema_object_json_ld(SchemaObjectJSONLDInput{schema_type:" CreativeWork ".into(),props:serde_json::from_value(json!({"name":" <script>&\u{2028} ","description":"  ","count":0,"absent":null})).map_err(|e|e.to_string())?,video:Some(video.clone()),trim_string:true});
    let parsed:Value=serde_json::from_str(&raw).map_err(|e|e.to_string())?;assert_eq!(parsed["@type"],"CreativeWork");assert_eq!(parsed["video"]["duration"],"PT42S");assert_eq!(parsed["count"],0);assert!(parsed.get("description").is_none()&&parsed.get("absent").is_none());assert!(!raw.contains("<script>"));assert!(raw.contains("\\u003c"));
    let no_duration:Value=serde_json::from_str(&json_ld(&video_object_json_ld(Some(&StructuredVideo{duration_seconds:0,..video.clone()})).ok_or("video node")?)).map_err(|e|e.to_string())?;assert!(no_duration.get("duration").is_none());
    let mut map=serde_json::Map::from_iter([("@type".into(),json!("Thing"))]);let _=schema_object_json_ld_from_map(&mut map,Some(&video));assert!(map.contains_key("@context")&&map.contains_key("video"));
    let mut props=SEOProps{robots:"noindex".into(),site_name:"Kept".into(),..Default::default()};
    let address=&props as *const SEOProps;
    let returned=structured_seo_props_in_place(&mut props,StructuredSEOPropsInput{title:"Page".into(),jsonld_blocks:vec![raw],structured_video:Some(video),..Default::default()});
    assert_eq!(returned as *const SEOProps,address);assert_eq!(returned.robots,"noindex");assert_eq!(returned.twitter_card,"player");assert!(!returned.open_graph_meta.is_empty()&&!returned.twitter_meta.is_empty());
    Ok(())
}

#[test]
fn sitemap_dates_and_definition_order_match_public_contract()->Result<(),String>{
    for (input,expected) in [(" 2024-02-29 ","2024-02-29"),("2024-01-02T03:04:05+02:00","2024-01-02T01:04:05Z"),("2023-02-29",""),("tomorrow",""),("2024-01-02T03:04:05","")]{assert_eq!(normalize_sitemap_lastmod(input),expected);}
    let instant=chrono::DateTime::parse_from_rfc3339("2024-01-02T03:04:05+02:00").map_err(|e|e.to_string())?.with_timezone(&chrono::Utc);
    assert_eq!(format_sitemap_lastmod_date(Some(instant)),"2024-01-02");assert_eq!(format_sitemap_lastmod_date_time(Some(instant)),"2024-01-02T01:04:05Z");assert_eq!(format_sitemap_lastmod_date(None),"");
    let first=definition();let mut disabled=first.clone();disabled.kind=ResourceKind::new("disabled");disabled.sitemap.enabled=false;let mut last=first.clone();last.kind=ResourceKind::new("last");last.sitemap.route_name="last-sitemap".into();
    let index=definition_sitemap_index(&[first.clone(),disabled.clone(),last]);assert_eq!(index.entries.iter().map(|e|e.kind.as_str()).collect::<Vec<_>>(),vec!["article","last"]);assert!(definition_sitemap_document(&disabled).entries.is_empty());assert_eq!(definition_sitemap_document(&first).entries.len(),1);Ok(())
}

#[test]
fn head_renders_canonical_alternates_and_safe_metadata()->Result<(),String>{
    let props=SEOProps{title:"Title".into(),description:"<unsafe>&\"".into(),url:"https://example.test/a?x=1&y=2".into(),robots:"noindex,follow".into(),keywords:vec!["one".into(),"two".into()],hreflangs:vec![Hreflang{lang:"fr".into(),url:"https://example.test/fr".into()}],jsonld_blocks:vec![collection_page_json_ld("<script>","","/",1)],..Default::default()};
    let html=seo_head(&props).into_string();assert!(html.contains("rel=\"canonical\"")&&html.contains("hreflang=\"fr\""));assert!(html.contains("&lt;unsafe&gt;&amp;&quot;"));assert!(html.contains("application/ld+json"));assert!(!html.contains("<script>"));assert!(html.contains("\\u003cscript\\u003e"));Ok(())
}

#[test]
fn filtered_canonical_hreflangs_keep_repeated_values_and_drop_paging()->Result<(),String>{
    let request=request("http://example.test/en/articles?page=9")?;
    let filter=ResourceFilterDefinition{key:ResourceFilterKey::new("tags"),param:"tag".into(),mode:ResourceFilterMode::new(RESOURCE_FILTER_MODE_MULTI_ALL),..Default::default()};
    let query=ResourceListQuery{search:" alpha ".into(),sort:" newest ".into(),filter_values:BTreeMap::from([(ResourceFilterKey::new("tags"),vec!["rust".into(),"systems".into()])]),..Default::default()};
    let canonical=resource_filtered_canonical_url(&request,"/articles",&query,&[filter.clone()]);assert_eq!(canonical,"/articles?search=alpha&sort=newest&tag=rust&tag=systems");
    let links=resource_filtered_hreflangs(Some(&request),"articles",&query,&[filter],&["fr".into(),"".into(),"x-default".into()]);assert_eq!(links.len(),2);assert!(links.first().ok_or("first")?.url.starts_with("http://example.test/fr/articles?"));assert!(links.last().ok_or("last")?.url.contains("/en/articles?"));assert!(!links.iter().any(|h|h.url.contains("page=")));Ok(())
}

#[test]
fn projection_runs_query_before_render_recovers_only_expected_failures()->Result<(),String>{
    let request=request("http://example.test/articles")?;let kind=ResourceKind::new("article");let calls=Arc::new(AtomicUsize::new(0));
    let seen=calls.clone();let query:SEOQuerySource<(),String,()>=Arc::new(move|_,_|{let seen=seen.clone();effect_sync!{body:move||{seen.fetch_add(1,Ordering::SeqCst);}}});
    let seen=calls.clone();let render:SEOPageSource<(),SEOVM,String,()>=Arc::new(move|_,_,_|{let seen=seen.clone();effect_sync!{body:move||{assert_eq!(seen.fetch_add(1,Ordering::SeqCst),1);Some(SEOVM{title:"projected".into(),canonical_route:"/canonical".into(),indexable:true,..Default::default()})}}});
    let program=resource_seo_props_effect(&request,&kind,Default::default(),Some(query.clone()),Some(render),Arc::new(|p|Some(p.clone())));assert_eq!(calls.load(Ordering::SeqCst),0);assert_eq!(run(program)?.title,"projected");
    let failed:SEOPageSource<(),SEOVM,String,()>=Arc::new(|_,_,_|effect_try_sync!{body:||Err::<Option<SEOVM>,_>("render failed".to_owned())});
    let props=run(resource_seo_props_effect(&request,&kind,Default::default(),Some(query.clone()),Some(failed),Arc::new(|p|Some(p.clone()))))?;assert_eq!(props,SEOProps::default());
    let interrupted:SEOPageSource<(),SEOVM,String,()>=Arc::new(|_,_,_|effect_future!{exit_with_context;body:|_,_|async{Exit::Interrupted}});
    let exit=futures_executor::block_on(resource_seo_props_effect(&request,&kind,Default::default(),Some(query),Some(interrupted),Arc::new(|p|Some(p.clone()))).run(Arc::new(())));assert!(matches!(exit,Exit::Interrupted));
    Ok(())
}
