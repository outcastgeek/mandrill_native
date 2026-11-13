//! Ordered picker, attachment, inline-creation and shortcode request flows.
use super::*;
use crate::attachments::{attach_asset_ids_to_attachment_inputs, promote_attachment_input,
    remove_attachment_input, media_selected_asset_ids, initialize_media_form, InitializeForm};
use crate::media::surface::{MediaPatch,media_datastar_request,media_query_from_request};
use effect_server::{BodyStatus,reply};
use http::{StatusCode,HeaderName,HeaderValue};

pub(crate) type FormFlow<E,R> = Arc<dyn Fn(&Request,Arc<ByteFormData>)->Effect<Reply<E,R>,E,R> + Send + Sync>;

pub fn resolve_media_flow_surface(request:&Request,form:&ByteFormData,resolve:Option<&MediaSurfaceResolver>)->MediaSurfaceRequest {
    resolve.map(|resolve| resolve(request,form)).unwrap_or_default()
}
pub fn render_media_flow_error<E:Send+'static,R:Send+Sync+'static>(request:&Request,status:u16,message:&str,render:Option<&MediaFlowErrorRenderer<E,R>>)->Effect<Reply<E,R>,E,R> {
    if let Some(render)=render { return render(request,status,message.trim()); }
    effect_sync! { body: move || {
        let code=StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        reply! { status(BodyStatus::new(code).unwrap_or(BodyStatus::Ok));
            header(HeaderName::from_static("x-content-type-options"),HeaderValue::from_static("nosniff"));
            text(format!("{}\n",code.canonical_reason().unwrap_or_default())); }
    }}
}
pub(crate) fn redirect<E:Send+'static,R:Send+Sync+'static>(request:&Request,path:&str)->Effect<Reply<E,R>,E,R> {
    let path=path.trim().to_owned();
    let resolve=Arc::new(move |request:&Request| crate::access::resolved_redirect_target(request,&path)) as crate::access::AccessPathBuilder;
    crate::access::new_redirect_access_response(Some(resolve),BodyStatus::new(StatusCode::SEE_OTHER).unwrap_or(BodyStatus::Ok))(request)
}
pub(crate) fn form_flow<E:Send+'static,R:Send+Sync+'static>(request:&Request,io:MediaFlowIO<E,R>,bad:String,run:FormFlow<E,R>)->Effect<Reply<E,R>,E,R> {
    let request=request.clone();
    effect! {
        let request=request.clone(); let io=io.clone(); let bad=bad.clone(); let run=run.clone();
        let initialize=match io.form.clone() {
            Some(form)=>effect_sync!{body:move || (ByteFormData{values:form.values.clone()},None)},
            None=>{
                let on_error=io.on_form_error.clone();
                initialize_media_form(&request,io.form_policy,InitializeForm::ParseForm,move |error|on_error(error))
            }
        };
        let initialized=initialize?;
        if initialized.1.is_some() { render_media_flow_error(&request,400,&bad,io.write_error.as_ref())? }
        else { run(&request,Arc::new(initialized.0))? }
    }
}
fn role(raw:&InputBytes,normalize:Option<&RoleNormalizer>)->InputBytes {
    normalize.map(|normalize|normalize(raw.as_bytes()).trim()).unwrap_or_else(||raw.trim())
}
fn default_role(request:&Request,surface:&MediaSurfaceRequest,resolve:Option<&MediaDefaultRoleResolver>)->InputBytes {
    resolve.map(|resolve|resolve(request,surface).trim()).unwrap_or_default()
}
fn with_default_role(request:&Request,mut surface:MediaSurfaceRequest,resolve:Option<&MediaDefaultRoleResolver>)->MediaSurfaceRequest {
    let fallback=default_role(request,&surface,resolve);
    surface.role=[surface.role,fallback].into_iter().map(|role|role.trim()).find(|role|!role.is_empty()).unwrap_or_default();surface
}
fn value(request:&Request,form:&ByteFormData,key:&str)->InputBytes {
    form.values.get(key.as_bytes()).and_then(|values|values.first()).map(|value|InputBytes::from(value.as_slice()).trim())
        .filter(|value|!value.is_empty()).unwrap_or_else(||crate::queries::request_query_value(request,key))
}

pub fn handle_media_picker_search<E:Send+'static,R:Send+Sync+'static>(request:&Request,options:MediaPickerSearchFlowOptions<E,R>)->Effect<Reply<E,R>,E,R> {
    if !media_datastar_request(request) { return redirect(request,&options.redirect_path); }
    let io=options.io.clone();let bad=options.bad_request_message.clone();
    form_flow(request,io,bad,Arc::new(move |request,form|{
        let request=request.clone();let form=form.clone();let options=options.clone();
        effect! {
            let request=request.clone();let form=form.clone();let options=options.clone();
            match (options.build_page,options.render_picker,options.io.write_patch) {
                (Some(build),Some(render),Some(write))=>{
                    let page=build(&request,media_query_from_request(&request,Some(&form),options.page_size),String::new(),form).either()?;
                    match page {
                        Ok(page)=>write(&request,page.root_id.clone().into(),"inner".into(),render(page))?,
                        Err(_)=>render_media_flow_error(&request,500,&options.internal_error_message,options.io.write_error.as_ref())?,
                    }
                }
                _=>render_media_flow_error(&request,500,&options.internal_error_message,options.io.write_error.as_ref())?,
            }
        }
    }))
}

pub fn handle_media_picker_attach<E:Send+'static,R:Send+Sync+'static>(request:&Request,options:MediaPickerAttachFlowOptions<E,R>)->Effect<Reply<E,R>,E,R> {
    let io=options.io.clone();let bad=options.bad_request_message.clone();
    form_flow(request,io,bad,Arc::new(move |request,form|{
        let request=request.clone();let form=form.clone();let options=options.clone();
        effect! {
            let request=request.clone();let form=form.clone();let options=options.clone();
            match (options.build_page,options.render_picker,options.current_inputs,options.preview_items,
                options.build_manager,options.render_manager,options.io.write_patch,options.io.write_patches) {
                (Some(build),Some(render_picker),Some(current),Some(preview),Some(manager),Some(render),Some(write),Some(writes))=>{
                    let surface=with_default_role(&request,resolve_media_flow_surface(&request,&form,options.surface.as_ref()),options.default_role.as_ref());
                    let requested=current(&request,surface.clone(),form.clone()).either()?;
                    match requested {
                        Err(_)=>render_media_flow_error(&request,500,&options.internal_error_message,options.io.write_error.as_ref())?,
                        Ok(inputs)=>{
                            let selected=media_selected_asset_ids(Some(&form));
                            if selected.is_empty() {
                                let page=build(&request,media_query_from_request(&request,Some(&form),options.page_size),options.no_selection_message,form).either()?;
                                match page { Ok(page)=>write(&request,page.root_id.clone().into(),"inner".into(),render_picker(page))?,
                                    Err(_)=>render_media_flow_error(&request,500,&options.internal_error_message,options.io.write_error.as_ref())? }
                            } else {
                                let mut roles=options.role_options;
                                if roles.normalize_role.is_none(){roles.normalize_role=options.normalize_role.clone();}
                                let next=attach_asset_ids_to_attachment_inputs(&inputs,&selected,role(&surface.role,options.normalize_role.as_ref()),&roles);
                                let items=preview(&request,next).either()?;
                                match items {
                                    Err(_)=>render_media_flow_error(&request,500,&options.internal_error_message,options.io.write_error.as_ref())?,
                                    Ok(items)=>match manager(&request,&form,surface.clone(),items) {
                                        None=>render_media_flow_error(&request,500,&options.internal_error_message,options.io.write_error.as_ref())?,
                                        Some(manager)=>{
                                            let section=manager.mounts.section_id.clone();let root=manager.mounts.picker_root_id.clone();
                                            let component=render(manager);
                                            let focus=crate::media::presentation::media_picker_return_focus(&crate::byte_text::encoding_text(section.as_bytes()),&crate::byte_text::encoding_text(surface.role.as_bytes()));
                                            writes(&request,vec![MediaPatch{root_id:section,mode:String::new(),component},MediaPatch{root_id:root,mode:"inner".into(),component:focus}])?
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                _=>render_media_flow_error(&request,500,&options.internal_error_message,options.io.write_error.as_ref())?,
            }
        }
    }))
}

pub fn handle_media_attachment_mutation<E:Send+'static,R:Send+Sync+'static>(request:&Request,options:MediaAttachmentMutationFlowOptions<E,R>)->Effect<Reply<E,R>,E,R> {
    let io=options.io.clone();let bad=options.bad_request_message.clone();
    form_flow(request,io,bad,Arc::new(move |request,form|{
        let request=request.clone();let form=form.clone();let options=options.clone();
        effect! {
            let request=request.clone();let form=form.clone();let options=options.clone();
            match (options.current_inputs,options.preview_items,options.build_manager,options.render_manager,options.io.write_patch) {
                (Some(current),Some(preview),Some(manager),Some(render),Some(write))=>{
                    let surface=resolve_media_flow_surface(&request,&form,options.surface.as_ref());
                    let requested=current(&request,surface.clone(),form.clone()).either()?;
                    match requested {
                        Err(_)=>render_media_flow_error(&request,500,&options.internal_error_message,options.io.write_error.as_ref())?,
                        Ok(inputs)=>{
                            let mut roles=options.role_options;
                            if roles.normalize_role.is_none(){roles.normalize_role=options.normalize_role.clone();}
                            let id=value(&request,&form,"asset_id");
                            let next=match value(&request,&form,"mutation").as_bytes() {
                                b"promote"=>promote_attachment_input(&inputs,&id,&roles),
                                b"remove"=>remove_attachment_input(&inputs,&id,options.normalize_role.as_deref()),
                                _=>inputs,
                            };
                            let items=preview(&request,next).either()?;
                            match items {
                                Err(_)=>render_media_flow_error(&request,500,&options.internal_error_message,options.io.write_error.as_ref())?,
                                Ok(items)=>match manager(&request,&form,surface,items) {
                                    Some(manager)=>write(&request,manager.mounts.section_id.clone(),"outer".into(),render(manager))?,
                                    None=>render_media_flow_error(&request,500,&options.internal_error_message,options.io.write_error.as_ref())?,
                                }
                            }
                        }
                    }
                }
                _=>render_media_flow_error(&request,500,&options.internal_error_message,options.io.write_error.as_ref())?,
            }
        }
    }))
}

pub fn valid_media_inline_create_flow<I,E:Send+'static,R:Send+Sync+'static>(options:&MediaInlineCreateFlowOptions<I,E,R>)->bool {
    options.parse_input.is_some()&&options.validate_input.is_some()&&options.create_asset.is_some()
        &&options.render_create.is_some()&&options.current_inputs.is_some()&&options.preview_items.is_some()
        &&options.build_manager.is_some()&&options.render_manager.is_some()&&options.io.write_patch.is_some()
        &&options.io.write_patches.is_some()&&options.io.write_html_status.is_some()
}

pub fn handle_media_inline_create<I:Clone+Send+'static,E:Send+'static,R:Send+Sync+'static>(request:&Request,options:MediaInlineCreateFlowOptions<I,E,R>)->Effect<Reply<E,R>,E,R> {
    let io=options.io.clone();let bad=options.bad_request_message.clone();
    form_flow(request,io,bad,Arc::new(move |request,form|{
        let request=request.clone();let form=form.clone();let options=options.clone();
        effect! {
            let request=request.clone();let form=form.clone();let options=options.clone();
            if !valid_media_inline_create_flow(&options) { render_media_flow_error(&request,500,&options.internal_error_message,options.io.write_error.as_ref())? }
            else { run_inline_create(&request,form,options)? }
        }
    }))
}
fn run_inline_create<I:Clone+Send+'static,E:Send+'static,R:Send+Sync+'static>(request:&Request,form:Arc<ByteFormData>,options:MediaInlineCreateFlowOptions<I,E,R>)->Effect<Reply<E,R>,E,R> {
    let request=request.clone();
    effect! {
        let request=request.clone();let form=form.clone();let options=options.clone();
        let surface=resolve_media_flow_surface(&request,&form,options.surface.as_ref());
        let current=options.current_inputs.as_ref().expect("admitted current inputs")(&request,surface.clone(),form.clone()).either()?;
        match current {
            Err(_)=>render_media_flow_error(&request,500,&options.internal_error_message,options.io.write_error.as_ref())?,
            Ok(inputs)=>{
                let preview=options.preview_items.as_ref().expect("admitted preview")(&request,inputs.clone()).either()?;
                match preview {
                    Err(_)=>render_media_flow_error(&request,500,&options.internal_error_message,options.io.write_error.as_ref())?,
                    Ok(current_items)=>{
                        let input=options.parse_input.as_ref().expect("admitted parser")(&request,&form);
                        let message=options.validate_input.as_ref().expect("admitted validator")(&request,&input).trim().to_owned();
                        if !message.is_empty() {
                            let rendered=options.render_create.as_ref().expect("admitted renderer")(&request,&form,input,message,current_items);
                            write_inline_render(&request,422,rendered,&options.io)?
                        } else {
                            let created=options.create_asset.as_ref().expect("admitted creator")(&request,input.clone()).either()?;
                            match created {
                                Err(_)=>{
                                    let rendered=options.render_create.as_ref().expect("admitted renderer")(&request,&form,input,options.internal_error_message.clone(),current_items);
                                    write_inline_render(&request,500,rendered,&options.io)?
                                }
                                Ok(id)=>{
                                    let surface=with_default_role(&request,surface,options.default_role.as_ref());
                                    let mut roles=options.role_options.clone();if roles.normalize_role.is_none(){roles.normalize_role=options.normalize_role.clone();}
                                    let next=attach_asset_ids_to_attachment_inputs(&inputs,&[id.trim()],role(&surface.role,options.normalize_role.as_ref()),&roles);
                                    let preview=options.preview_items.as_ref().expect("admitted preview")(&request,next).either()?;
                                    match preview {
                                        Err(_)=>render_media_flow_error(&request,500,&options.internal_error_message,options.io.write_error.as_ref())?,
                                        Ok(items)=>match options.build_manager.as_ref().expect("admitted manager")(&request,&form,surface.clone(),items) {
                                            None=>render_media_flow_error(&request,500,&options.internal_error_message,options.io.write_error.as_ref())?,
                                            Some(manager)=>{
                                                let section=manager.mounts.section_id.clone();
                                                let component=options.render_manager.as_ref().expect("admitted manager renderer")(manager);
                                                if media_datastar_request(&request) {
                                                    let focus=crate::media::presentation::media_picker_return_focus(&crate::byte_text::encoding_text(section.as_bytes()),&crate::byte_text::encoding_text(surface.role.as_bytes()));
                                                    options.io.write_patches.as_ref().expect("admitted patches")(&request,vec![MediaPatch{root_id:section,mode:String::new(),component},MediaPatch{root_id:surface.root_id,mode:"inner".into(),component:focus}])?
                                                } else { options.io.write_html_status.as_ref().expect("admitted html")(&request,201,component)? }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
fn write_inline_render<E:Send+'static,R:Send+Sync+'static>(request:&Request,status:u16,rendered:MediaInlineCreateRender,io:&MediaFlowIO<E,R>)->Effect<Reply<E,R>,E,R> {
    if media_datastar_request(request) { io.write_patch.as_ref().expect("admitted patch")(request,rendered.root_id,"inner".into(),rendered.component) }
    else { io.write_html_status.as_ref().expect("admitted html")(request,status,rendered.component) }
}

pub fn handle_media_inline_preview<I:Send+'static,E:Send+'static,R:Send+Sync+'static>(request:&Request,options:MediaInlinePreviewFlowOptions<I,E,R>)->Effect<Reply<E,R>,E,R> {
    let io=options.io.clone();let bad=options.bad_request_message.clone();
    form_flow(request,io,bad,Arc::new(move |request,form|{
        let request=request.clone();let form=form.clone();let options=options.clone();
        effect! {
            let request=request.clone();let form=form.clone();let options=options.clone();
            match (options.parse_input,options.render_preview) {
                (Some(parse),Some(render))=>{
                    let rendered=render(&request,&form,parse(&request,&form));
                    if media_datastar_request(&request) {
                        match options.io.write_patch {Some(write)=>write(&request,rendered.root_id,"outer".into(),rendered.component)?,None=>render_media_flow_error(&request,500,"",options.io.write_error.as_ref())?}
                    } else {
                        match options.io.write_html {Some(write)=>write(&request,rendered.component)?,None=>render_media_flow_error(&request,500,"",options.io.write_error.as_ref())?}
                    }
                }
                _=>render_media_flow_error(&request,500,"",options.io.write_error.as_ref())?,
            }
        }
    }))
}

pub fn handle_media_shortcode_modal<E:Send+'static,R:Send+Sync+'static>(request:&Request,options:MediaShortcodeModalFlowOptions<E,R>)->Effect<Reply<E,R>,E,R> {
    if !media_datastar_request(request) { return redirect(request,&options.redirect_path); }
    let io=options.io.clone();let bad=options.bad_request_message.clone();
    form_flow(request,io,bad,Arc::new(move |request,form|{
        let request=request.clone();let form=form.clone();let options=options.clone();
        effect! {
            let request=request.clone();let form=form.clone();let options=options.clone();
            match (options.load_items,options.render_modal,options.io.write_patch) {
                (Some(load),Some(render),Some(write))=>{
                    let surface=resolve_media_flow_surface(&request,&form,options.surface.as_ref());
                    let query=media_query_from_request(&request,Some(&form),options.page_size);
                    let result=load(&request,query.clone()).either()?;
                    match result {
                        Err(_)=>render_media_flow_error(&request,500,&options.internal_error_message,options.io.write_error.as_ref())?,
                        Ok((items,total))=>{
                            let root=crate::queries::first_non_blank(&[crate::byte_text::encoding_text(surface.root_id.as_bytes()),options.default_root_id]);
                            let dialog=crate::queries::first_non_blank(&[crate::byte_text::encoding_text(value(&request,&form,"dialog_id").as_bytes()),format!("{root}-dialog")]);
                            let field=crate::byte_text::encoding_text(value(&request,&form,"field_id").as_bytes());
                            let label=crate::queries::first_non_blank(&[crate::byte_text::encoding_text(value(&request,&form,"field_label").as_bytes()),field.clone()]);
                            let mounts=options.mounts_for_scope.map(|resolve|resolve(surface.scope.as_bytes())).unwrap_or_default();
                            let owner=crate::queries::first_non_blank(&[crate::byte_text::encoding_text(surface.owner_form_id.as_bytes()),crate::byte_text::encoding_text(mounts.owner_form_id.as_bytes())]);
                            let limit=if query.limit>0{query.limit}else{[options.default_page_size,options.page_size,1].into_iter().find(|value|*value>0).unwrap_or(1)};
                            let pages=options.total_pages.map(|resolve|resolve(total,limit)).unwrap_or(0);
                            let vm=ShortcodeToolsProps{root_id:root.clone(),dialog_id:dialog,scope:crate::byte_text::encoding_text(surface.scope.as_bytes()),owner_form_id:owner,
                                library_url:options.library_url.trim().into(),target_field_id:field,target_label:label,items,
                                search:crate::byte_text::encoding_text(query.search.as_bytes()),kind:crate::byte_text::encoding_text(query.kind.as_bytes()),sort:crate::byte_text::encoding_text(query.sort.as_bytes()),
                                page:crate::rendering::resource_list_current_page(query.offset,limit,options.default_page_size),page_size:query.limit,total,pages,..ShortcodeToolsProps::default()};
                            write(&request,root.into(),"inner".into(),render(vm))?
                        }
                    }
                }
                _=>render_media_flow_error(&request,500,&options.internal_error_message,options.io.write_error.as_ref())?,
            }
        }
    }))
}
