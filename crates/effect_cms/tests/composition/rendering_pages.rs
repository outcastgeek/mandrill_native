use std::{collections::BTreeMap,sync::Arc};
use effect::{Exit,effect_sync,effect_try_sync,effect_future};
use effect_cms::{definitions::*,rendering::*,resourcepages::*,dashboards};
use maud::html;
use super::support::*;
#[path="../support/native_http.rs"] mod native_http;

#[test]
fn item_builders_skip_blank_keys_keep_meta_positions_and_children()->Result<(),String>{
    let request=request("/")?;
    let definition=ResourceListItemDefinition{meta:SharedList::new(vec![ResourceListItemMetaDefinition{label:"Author".into(),..Default::default()},ResourceListItemMetaDefinition{label_key:"date".into(),..Default::default()}]),..Default::default()};
    let states=vec![ResourceListItemState{key:" ".into(),title:"ignored".into(),..Default::default()},ResourceListItemState{key:" item ".into(),title:" Title ".into(),meta:vec![resource_meta_state_link("Ada","/ada")],children:vec![ResourceListItemState{key:"child".into(),title:"Nested".into(),..Default::default()}],..Default::default()}];
    let items=build_resource_list_items(&definition,&states,&request,None);assert_eq!(items.len(),1);let item=items.first().ok_or("item")?;assert_eq!(item.key,"item");assert_eq!(item.title,"Title");let meta=item.meta.as_ref().ok_or("meta")?.to_vec();assert_eq!(meta.len(),2);assert_eq!(meta.first().ok_or("meta")?.href,"/ada");assert_eq!(meta.last().ok_or("second meta")?.label,"date");assert!(meta.last().ok_or("second")?.value.is_empty());assert_eq!(item.children.as_ref().ok_or("children")?.len(),1);Ok(())
}

#[test]
fn list_frame_recovers_expected_failure_but_preserves_interruption()->Result<(),String>{
    let request=request("/?page=2")?;let kind=ResourceKind::new("article");
    let source:ListPageSource<String,()>=Arc::new(|_,_,_|effect_try_sync!{body:||Err::<ResourceListPageVM,_>("load failed".into())});
    let options=FrameBuilderOptions{default_page_size:3,rendering:Some(RenderingCallbacks{list_page:Some(source),detail_page:None}),..Default::default()};
    let frame=run(build_resource_list_frame(&request,&kind,false,2,0,7,true,true,options))?;assert_eq!(frame.total_pages,3);assert_eq!(frame.page,2);assert!(frame.page_vm.title.is_empty());assert!(frame.is_logged_in&&frame.can_create);
    let source:ListPageSource<String,()>=Arc::new(|_,_,_|effect_future!{exit_with_context;body:|_,_|async{Exit::Interrupted}});
    let options=FrameBuilderOptions{rendering:Some(RenderingCallbacks{list_page:Some(source),detail_page:None}),..Default::default()};
    let exit=futures_executor::block_on(build_resource_list_frame(&request,&kind,false,1,5,0,false,false,options).run(Arc::new(())));assert!(matches!(exit,Exit::Interrupted));Ok(())
}

#[test]
fn definition_page_updates_same_seo_handle_and_hreflangs()->Result<(),String>{
    let request=request("/articles")?;let kind=ResourceKind::new("article");let seo=SEOView::new(effect_cms::seo::SEOProps{title:"retained".into(),..Default::default()});let retained=seo.clone();
    let options=DefinitionResourcePageBuilderOptions::<String,()>{list_seo:Some(Arc::new(move|_,_|{let seo=seo.clone();effect_sync!{body:move||seo.clone()}})),route_set:Some(Arc::new(|_,_|ResourceRouteSet{list:"articles".into(),..Default::default()})),hreflang_path:Some(Arc::new(|_|"/articles".into())),hreflangs_for_path:Some(Arc::new(|_,_|vec![effect_cms::seo::Hreflang{lang:"fr".into(),url:"/fr/articles".into()}])),..Default::default()};
    let page=run(build_definition_list_page(&request,&kind,false,1,5,0,ResourceListPageState::default(),options))?;
    assert!(page.seo_view.is_some());let handle=retained.shared_handle().ok_or("retained SEO")?;let props=run::<_,String>(handle.with(Clone::clone))?;assert_eq!(props.title,"retained");assert_eq!(props.hreflangs.len(),1);Ok(())
}

#[test]
fn detail_sections_apply_override_preserve_html_and_skip_empty_body()->Result<(),String>{
    let request=request("/")?;
    let declared=ResourceDetailDefinition{sections:SharedList::new(vec![ResourceDetailSectionDefinition{key:ResourceSectionKey::new("body"),title_key:"Body".into(),..Default::default()},ResourceDetailSectionDefinition{key:ResourceSectionKey::new("empty"),..Default::default()}]),..Default::default()};
    let state=BTreeMap::from([(ResourceSectionKey::new("body"),ResourceDetailSectionState{title:"Override".into(),body:"<strong>authored</strong>".into(),body_is_html:true,..Default::default()})]);
    let sections=build_resource_detail_sections(&declared,&state,&request,None);let first=sections.first().ok_or("section")?;assert_eq!(first.title,"Override");assert!(first.body_is_html);assert!(resource_has_detail_content(first));assert_eq!(sections.len(),1);assert!(!resource_has_detail_content(&SectionVM::default()));Ok(())
}

#[test]
fn resource_form_renderer_preserves_domain_attributes_and_editor_requirement()->Result<(),String>{
    let c=ResourceRenderContext{request:None,translate:None,translate_args:None};
    let mut field=ResourceFieldVM{kind:ResourceFieldKind::new(RESOURCE_FIELD_KIND_TYPED_VALUE),label:"Typed".into(),attrs:BTreeMap::from([("data-client:custom".into(),"<value>&\"".into())]),..Default::default()};
    assert!(resource_form_field_view(c,&field,&Default::default()).is_err());
    field.prepared_editor=Some(html!{textarea name="body" {"prepared"}});
    let markup=resource_form_field_view(c,&field,&Default::default()).map_err(|e|e.to_string())?.into_string();assert!(markup.contains("data-client:custom=\"&lt;value&gt;&amp;&quot;\""));assert!(markup.contains("<textarea"));
    let actions=management_form_submit_actions(&[ManagementFormActionProps{name:FORM_SUBMIT_ACTION_FIELD_NAME.into(),value:"publish".into(),label:"Publish".into(),..Default::default()}],"").into_string();assert!(actions.contains("data-localized-submit-control"));assert!(actions.contains("name=\"submit_action\"")||actions.contains(&format!("name=\"{FORM_SUBMIT_ACTION_FIELD_NAME}\"")));assert!(actions.contains("value=\"publish\""));Ok(())
}

#[test]
fn approved_maud_attribute_spread_escapes_values_and_rejects_invalid_names(){
    let attrs=BTreeMap::from([("data-on:click".to_owned(),"a < b && c == \"yes\"".to_owned()),("data-plugin".into(),"custom".into())]);
    let html=web_components::html!{button class="fixed" ..(&attrs) {"Run"}}.into_string();assert!(html.contains("data-on:click=\"a &lt; b &amp;&amp; c == &quot;yes&quot;\""));assert!(html.contains("class=\"fixed\""));
    for name in ["", "two names", "bad=key", "bad>key", "bad/key", "bad\"key"]{let bad=BTreeMap::from([(name,"value")]);let caught=std::panic::catch_unwind(||web_components::html!{div ..(&bad) {}});assert!(caught.is_err(),"accepted invalid attribute name {name:?}");}
}

#[test]
fn select_sync_records_and_initialized_form_updates_retain_protocol()->Result<(),String>{
    let action=select_option_data_sync_action(&[SelectOptionDataSyncField{field_id:"title".into(),data_key:"title".into(),use_selected_value:false},SelectOptionDataSyncField{field_id:"slug".into(),use_selected_value:true,..Default::default()}]);assert!(action.contains("selectedOption.dataset[dataKey]"));assert!(action.contains("['slug','',true]"));
    let attrs=resource_select_data_sync_attrs(&[SelectOptionDataSyncField{field_id:"title".into(),..Default::default()}]);assert_eq!(attrs.get("data-on:load"),attrs.get("data-on:change"));assert!(!attrs.contains_key("data-on-load"));
    let options=resource_field_options_from_records(&[ResourceFieldOptionRecord{value:" ".into(),..Default::default()},ResourceFieldOptionRecord{value:" one ".into(),attrs:BTreeMap::from([("data-title".into(),"One".into())]),..Default::default()}]);assert_eq!(options.len(),1);assert_eq!(options.first().ok_or("option")?.label,"one");
    let mut form=effect_server::ByteFormData{values:BTreeMap::new()};let mut post=effect_server::ByteFormData{values:BTreeMap::new()};set_request_form_value(Some(&mut form),Some(&mut post)," title "," Value ");assert_eq!(form.values,post.values);assert_eq!(form.values.get(b"title".as_slice()),Some(&vec![b"Value".to_vec()]));Ok(())
}

#[test]
fn search_and_pagination_deliver_ordered_native_sse_events()->Result<(),String>{
    assert_eq!(resource_search_sse_url("/items?b=2#top"),"/items?b=2&patch=replace#top");assert_eq!(resource_append_sse_url("/items?patch=replace&b=2"),"/items?b=2");assert_eq!(resource_search_sse_url("/%ZZ"),"/%ZZ");
    let handler:native_http::Handler<String>=Arc::new(|_|effect_sync!{body:||write_resource_search_replacement("#items",html!{div {"one\ntwo"}},"next","/items?page=2&patch=replace","loading","hasMore",true)});
    let response=native_http::deliver(handler,Arc::new(|e|e.to_string()))?;assert_eq!(response.status,http::StatusCode::OK);assert_eq!(response.headers.get(http::header::CONTENT_TYPE).and_then(|v|v.to_str().ok()),Some("text/event-stream; charset=utf-8"));
    assert!(response.body.contains("data: selector #items\n"));assert!(response.body.contains("data: elements <div>one\ndata: elements two</div>"));assert!(response.body.find("datastar-patch-elements")<response.body.find("datastar-patch-signals"));assert!(response.body.contains("/items?page=2"));assert!(!response.body.contains("patch=replace"));
    for has_more in [true,false]{let handler:native_http::Handler<String>=Arc::new(move|_|effect_sync!{body:move||write_paginated_component_stream("#items",&[html!{p {"first"}},html!{p {"second"}}],"/items?page=3","next","loading","hasMore",has_more)});let response=native_http::deliver(handler,Arc::new(|e|e.to_string()))?;assert!(response.body.contains("data: mode append"));assert_eq!(response.body.matches("event: datastar-patch-elements").count(),2);if has_more{assert!(response.body.contains("page=3"));}else{assert!(response.body.contains("\"hasMore\":false"));assert!(!response.body.contains("page=3"));}}
    Ok(())
}

#[test]
fn dashboard_actions_filter_invalid_and_preserve_order()->Result<(),String>{
    let location=DashboardActionLocation::new("header");let actions=vec![dashboards::ActionVM{label:" ".into(),href:"/skip".into(),location:location.clone(),..Default::default()},dashboards::ActionVM{label:"Create".into(),href:"/new".into(),location:location.clone(),..Default::default()},dashboards::ActionVM{label:"Other".into(),href:"/other".into(),location:DashboardActionLocation::new("tool"),..Default::default()}];
    let mapped=dashboards::map_dashboard_actions(&actions,&location,Some(|a:dashboards::ActionVM|a.href)).ok_or("mapped actions")?;assert_eq!(mapped,vec!["/new"]);assert_eq!(dashboards::first_dashboard_action(&actions,&location).ok_or("first")?.label,"Create");assert!(dashboards::dashboard_actions_for_location(&actions,&DashboardActionLocation::new("absent")).is_none());Ok(())
}

#[test]
fn prepared_form_and_maud_reply_cross_the_native_http_boundary() -> Result<(), String> {
    use effect::effect;
    use effect::memory::{BufferView, Bytes, MemoryError, MemoryPlan, Partition, RegionId};
    use effect_cms::forms::{
        InitializeForm, MergedFormCapacity, MergedFormPolicy, PreparedMergedForms,
    };
    use effect_server::{BodyLimit, MultipartPolicy, ServerError};
    use std::num::NonZeroUsize;

    let region = RegionId::named("cms-http-fixture");
    let total = Bytes::new(128 * 1024);
    let capacity = [Partition { id: region, capacity: total }];
    let memory = MemoryPlan::new(total, &capacity)
        .and_then(|plan| plan.metadata(region))
        .and_then(MemoryPlan::prepare)
        .map_err(|error| error.to_string())?;
    let one = NonZeroUsize::MIN;
    let forms = Arc::new(
        PreparedMergedForms::prepare(&memory, region, MergedFormCapacity {
            forms: one,
            pairs: one.saturating_add(3),
            bytes: Bytes::new(128),
            query_segments: one.saturating_add(3),
            body_segments: one.saturating_add(3),
        }).map_err(|error| error.to_string())?,
    );
    let output = Arc::new(memory.buffers(region, 1, Bytes::new(1024))
        .map_err(|error| error.to_string())?);
    let memory = memory.seal();
    let urlencoded = policy();
    let form_policy = MergedFormPolicy {
        urlencoded,
        multipart: MultipartPolicy {
            body: BodyLimit::new(one),
            part_bytes: one,
            header_bytes: one,
            parts: one,
            field_bytes: one,
        },
        multipart_fields: one,
        multipart_values: urlencoded.values,
        file_memory: 0,
    };

    let response = native_http::deliver_at(
        Arc::new(move |request| {
            let request = request.clone();
            let forms = forms.clone();
            let output = output.clone();
            effect! {
                let request = request.clone();
                let forms = forms.clone();
                let output = output.clone();
                let first = forms.read(&request, form_policy, InitializeForm::ParseForm, std::convert::identity)?;
                assert_eq!(first.values(b"title").collect::<Vec<_>>(), [b"<news>".as_slice()]);
                let probe = forms.read::<ServerError, native_http::Env>(
                    &request, form_policy, InitializeForm::ParseForm, std::convert::identity,
                );
                let held = probe.result()?;
                assert!(matches!(held, Err(ServerError::Memory(MemoryError::NoSlot))));
                drop(first);
                let restored = forms.read(&request, form_policy, InitializeForm::ParseForm, std::convert::identity)?;
                assert!(restored.diagnostic().is_none());
                effect_try_sync! {
                    once;
                    body: move || {
                        let mut output = output.try_checkout().map_err(ServerError::Memory)?;
                        output.clear();
                        let markup = html! { h1 { "<news>" } };
                        write_maud_markup(&mut output, markup, Bytes::new(4096)).map_err(ServerError::Memory)?;
                        let held = BufferView::new(output.share());
                        Ok(effect_server::reply! { lease(held, http::HeaderValue::from_static("text/html")); })
                    },
                }?
            }
        }),
        Arc::new(std::convert::identity),
        http::Method::GET,
        "/?title=%3Cnews%3E".into(),
        String::new(),
        None,
    )?;
    assert_eq!(response.status, http::StatusCode::OK);
    assert_eq!(response.headers[http::header::CONTENT_TYPE], "text/html");
    assert_eq!(response.body, "<h1>&lt;news&gt;</h1>");
    assert_eq!(memory.report().reserved, total);
    Ok(())
}
