//! Protected media management endpoints and mutation recovery.
use std::sync::Arc;
use effect::{Effect,effect,effect_sync,effect_die,effect_map_error,Defect};
use effect_server::{Request,Reply};
use maud::Markup;
use crate::media::{flows::*,runtime::MediaRequestURLResolver};
use crate::commands::InputBytes;
use crate::definitions::{ResourceAction,RESOURCE_ACTION_CREATE,RESOURCE_ACTION_UPDATE,RESOURCE_ACTION_DELETE};
use crate::validation::{ValidationResult,success_validation_result};

pub type MediaAccessGuard<E,R> = Arc<dyn Fn(&Request,bool)->Effect<Option<Reply<E,R>>,E,R> + Send + Sync>;
pub type MediaHTMLWriter<E,R> = Arc<dyn Fn(&Request,Option<Markup>)->Effect<Reply<E,R>,E,R> + Send + Sync>;
pub type MediaItemExists<E,R> = Arc<dyn Fn(InputBytes)->Effect<bool,E,R> + Send + Sync>;

pub struct MediaManagementControllerOptions<I,E:Send+'static,R:Send+Sync+'static> {
    pub resource_kind:crate::definitions::ResourceKind,
    pub inline_surface:String,
    pub list_page_size:i64,
    pub io:MediaFlowIO<E,R>,
    pub require_access:Option<MediaAccessGuard<E,R>>,
    pub library_path:Option<MediaRequestURLResolver>,
    pub request_language:Option<MediaRequestURLResolver>,
    pub mutation_route_path:Option<crate::routing::MutationResultRoutePathBuilder>,
    pub bad_request_message:Option<MediaRequestURLResolver>,
    pub internal_error_message:Option<MediaRequestURLResolver>,
    pub not_found_message:Option<MediaRequestURLResolver>,
    pub render_html:Option<MediaHTMLWriter<E,R>>,
    pub list_page:Option<super::MediaAssetListPageBuilder<E,R>>,
    pub form_page:Option<super::MediaFormPageBuilder<E,R>>,
    pub item_exists:Option<MediaItemExists<E,R>>,
    pub validation:Option<Arc<dyn Fn()->Option<crate::validation::ActionValidationService<E,R>> + Send + Sync>>,
    pub commands:Option<Arc<dyn Fn()->Option<crate::commands::CommandService<E,R>> + Send + Sync>>,
    pub on_mutation_error:Arc<dyn Fn(crate::mutations::MutationFailure<E>)->E + Send + Sync>,
    pub picker_search:Option<Arc<dyn Fn(&Request)->MediaPickerSearchFlowOptions<E,R> + Send + Sync>>,
    pub picker_attach:Option<Arc<dyn Fn(&Request)->MediaPickerAttachFlowOptions<E,R> + Send + Sync>>,
    pub attachment_mutation:Option<Arc<dyn Fn(&Request)->MediaAttachmentMutationFlowOptions<E,R> + Send + Sync>>,
    pub shortcode_modal:Option<Arc<dyn Fn(&Request)->MediaShortcodeModalFlowOptions<E,R> + Send + Sync>>,
    pub inline_create:Option<Arc<dyn Fn(&Request)->MediaInlineCreateFlowOptions<I,E,R> + Send + Sync>>,
    pub inline_preview:Option<Arc<dyn Fn(&Request)->MediaInlinePreviewFlowOptions<I,E,R> + Send + Sync>>,
}
pub struct MediaManagementController<I,E:Send+'static,R:Send+Sync+'static> { options:Arc<MediaManagementControllerOptions<I,E,R>> }
impl<I,E:Send+'static,R:Send+Sync+'static> Clone for MediaManagementController<I,E,R> { fn clone(&self)->Self{Self{options:self.options.clone()}} }
pub fn new_media_management_controller<I,E:Send+'static,R:Send+Sync+'static>(options:MediaManagementControllerOptions<I,E,R>)->MediaManagementController<I,E,R>{MediaManagementController{options:Arc::new(options)}}
impl<I:Default+Clone+Send+'static,E:std::error::Error+Send+'static,R:Send+Sync+'static> MediaManagementController<I,E,R> {
    fn message(&self,request:&Request,resolve:Option<&MediaRequestURLResolver>)->String{resolve.map(|resolve|resolve(request).trim().to_owned()).unwrap_or_default()}
    fn error(&self,request:&Request,status:u16,message:&str)->Effect<Reply<E,R>,E,R>{render_media_flow_error(request,status,message,self.options.io.write_error.as_ref())}
    fn internal(&self,request:&Request)->Effect<Reply<E,R>,E,R>{self.error(request,500,&self.message(request,self.options.internal_error_message.as_ref()))}
    fn with_access(&self,request:&Request,allow:bool,run:crate::routing::RouteHandler<E,R>)->Effect<Reply<E,R>,E,R>{
        let controller=self.clone();let request=request.clone();
        effect!{let controller=controller.clone();let request=request.clone();let run=run.clone();
            let denial=match controller.options.require_access.as_ref(){Some(guard)=>guard(&request,allow)?,None=>None};
            match denial{Some(reply)=>reply,None=>run(&request)?}
        }
    }
    fn form_page(&self,request:&Request,result:ValidationResult)->Effect<Reply<E,R>,E,R>{
        let controller=self.clone();let request=request.clone();
        effect!{let controller=controller.clone();let request=request.clone();let result=result.clone();
            match (controller.options.form_page.as_ref(),controller.options.render_html.as_ref()){
                (Some(build),Some(render))=>match build(&request,result).either()?{
                    Ok(Some(page))=>render(&request,Some(page))?,_=>controller.internal(&request)?,
                },_=>controller.internal(&request)?,
            }
        }
    }
    pub fn list(&self,request:&Request)->Effect<Reply<E,R>,E,R>{
        let controller=self.clone();self.with_access(request,true,Arc::new(move|request|{
            let controller=controller.clone();let request=request.clone();
            effect!{let controller=controller.clone();let request=request.clone();
                match (controller.options.list_page.as_ref(),controller.options.render_html.as_ref()){
                    (Some(build),Some(render))=>match build(&request,crate::media::surface::media_query_from_request(&request,None,controller.options.list_page_size)).either()?{
                        Ok(page)=>render(&request,page)?,Err(_)=>controller.internal(&request)?,
                    },_=>controller.internal(&request)?,
                }
            }
        }))
    }
    pub fn new_form(&self,request:&Request)->Effect<Reply<E,R>,E,R>{
        let controller=self.clone();let datastar=crate::media::surface::media_datastar_request(request);
        self.with_access(request,!datastar,Arc::new(move|request|{
            if !crate::media::surface::media_surface_is_inline(request,&controller.options.inline_surface)||!datastar{
                return controller.form_page(request,success_validation_result(ResourceAction::new(RESOURCE_ACTION_CREATE)));
            }
            let controller=controller.clone();let bad=controller.message(request,controller.options.bad_request_message.as_ref());
            crate::media::flows::form_flow(request,controller.options.io.clone(),bad,Arc::new(move|request,form|{
                let controller=controller.clone();let request=request.clone();let form=form.clone();
                effect!{let controller=controller.clone();let request=request.clone();let form=form.clone();
                    match controller.options.inline_create.as_ref(){
                        None=>controller.internal(&request)?,
                        Some(build)=>{
                            let flow=build(&request);
                            if !valid_media_inline_create_flow(&flow){controller.internal(&request)?}
                            else{
                                let surface=resolve_media_flow_surface(&request,&form,flow.surface.as_ref());
                                let current=flow.current_inputs.as_ref().expect("admitted inputs")(&request,surface,form.clone()).either()?;
                                match current{
                                    Err(_)=>controller.internal(&request)?,
                                    Ok(inputs)=>match flow.preview_items.as_ref().expect("admitted preview")(&request,inputs).either()?{
                                        Err(_)=>controller.internal(&request)?,
                                        Ok(items)=>{
                                            let rendered=flow.render_create.as_ref().expect("admitted create renderer")(&request,&form,I::default(),String::new(),items);
                                            flow.io.write_patch.as_ref().expect("admitted patch")(&request,rendered.root_id,"inner".into(),rendered.component)?
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }))
        }))
    }
    fn exists(&self,request:&Request)->Effect<bool,E,R>{match self.options.item_exists.as_ref(){Some(exists)=>exists(crate::queries::path_value(request,"id").into()),None=>effect_sync!{body:||true}}}
    pub fn edit(&self,request:&Request)->Effect<Reply<E,R>,E,R>{
        let controller=self.clone();self.with_access(request,true,Arc::new(move|request|{
            let controller=controller.clone();let request=request.clone();
            effect!{let controller=controller.clone();let request=request.clone();
                match controller.exists(&request).either()?{
                    Err(_)=>controller.internal(&request)?,
                    Ok(false)=>controller.error(&request,404,&controller.message(&request,controller.options.not_found_message.as_ref()))?,
                    Ok(true)=>controller.form_page(&request,success_validation_result(ResourceAction::new(RESOURCE_ACTION_UPDATE)))?,
                }
            }
        }))
    }
    pub fn create(&self,request:&Request)->Effect<Reply<E,R>,E,R>{
        let controller=self.clone();self.with_access(request,true,Arc::new(move|request|{
            if crate::media::surface::media_surface_is_inline(request,&controller.options.inline_surface){controller.error(request,405,&controller.message(request,controller.options.not_found_message.as_ref()))}
            else{controller.execute_mutation(request,ResourceAction::new(RESOURCE_ACTION_CREATE))}
        }))
    }
    pub fn update(&self,request:&Request)->Effect<Reply<E,R>,E,R>{
        let controller=self.clone();self.with_access(request,true,Arc::new(move|request|{
            let controller=controller.clone();let request=request.clone();
            effect!{let controller=controller.clone();let request=request.clone();
                match controller.exists(&request).either()?{
                    Err(_)=>controller.internal(&request)?,Ok(false)=>controller.error(&request,404,&controller.message(&request,controller.options.not_found_message.as_ref()))?,
                    Ok(true)=>controller.execute_mutation(&request,ResourceAction::new(RESOURCE_ACTION_UPDATE))?,
                }
            }
        }))
    }
    pub fn delete(&self,request:&Request)->Effect<Reply<E,R>,E,R>{let controller=self.clone();self.with_access(request,true,Arc::new(move|request|controller.execute_mutation(request,ResourceAction::new(RESOURCE_ACTION_DELETE))))}
    fn execute_mutation(&self,request:&Request,action:ResourceAction)->Effect<Reply<E,R>,E,R>{
        let controller=self.clone();let request=request.clone();
        effect!{let controller=controller.clone();let request=request.clone();let action=action.clone();
            let validation=controller.options.validation.as_ref().and_then(|resolve|resolve());
            let commands=controller.options.commands.as_ref().and_then(|resolve|resolve());
            let fallback=controller.message(&request,controller.options.bad_request_message.as_ref());
            let language=controller.message(&request,controller.options.request_language.as_ref());
            let validation=match validation{Some(validation)=>validation,None=>effect_die!{defect:Defect::new("media mutation requires validation service")}?};
            let adapter=controller.options.on_mutation_error.clone();
            let attempt=effect_map_error!{
                program:crate::mutations::execute_with_optional_commands(&request,&controller.options.resource_kind,&action,&validation,commands.as_ref(),None).either(),
                map move |failure|{adapter(failure)},
            }?;
            match attempt{
                Ok(applied)=>{
                    let (result,_)=applied.project();
                    crate::routing::redirect_mutation_result(result.as_ref(),&language,controller.options.mutation_route_path.clone())(&request)?
                }
                Err(failure)=>{
                    if failure.is_rejected(){
                        let mut result=failure.validation().clone();
                        if result.message.trim().is_empty(){result.message=fallback;}
                        if action.as_str()==RESOURCE_ACTION_CREATE||action.as_str()==RESOURCE_ACTION_UPDATE{controller.form_page(&request,result)?}
                        else{controller.error(&request,400,&result.message)?}
                    }else{controller.internal(&request)?}
                }
            }
        }
    }
    pub fn search_picker(&self,request:&Request)->Effect<Reply<E,R>,E,R>{
        let controller=self.clone();self.with_access(request,!crate::media::surface::media_datastar_request(request),Arc::new(move|request|{
            match controller.options.picker_search.as_ref(){Some(build)=>handle_media_picker_search(request,build(request)),None=>controller.internal(request)}
        }))
    }
    pub fn attach_picker_selection(&self,request:&Request)->Effect<Reply<E,R>,E,R>{
        let controller=self.clone();self.with_access(request,false,Arc::new(move|request|{
            match controller.options.picker_attach.as_ref(){Some(build)=>handle_media_picker_attach(request,build(request)),None=>controller.internal(request)}
        }))
    }
    pub fn shortcode_modal(&self,request:&Request)->Effect<Reply<E,R>,E,R>{
        let controller=self.clone();self.with_access(request,!crate::media::surface::media_datastar_request(request),Arc::new(move|request|{
            match controller.options.shortcode_modal.as_ref(){Some(build)=>handle_media_shortcode_modal(request,build(request)),None=>controller.internal(request)}
        }))
    }
    pub fn mutate_attachments(&self,request:&Request)->Effect<Reply<E,R>,E,R>{
        let controller=self.clone();self.with_access(request,false,Arc::new(move|request|{
            match controller.options.attachment_mutation.as_ref(){Some(build)=>handle_media_attachment_mutation(request,build(request)),None=>controller.internal(request)}
        }))
    }
    pub fn inline_create_fragment(&self,request:&Request)->Effect<Reply<E,R>,E,R>{
        let controller=self.clone();self.with_access(request,false,Arc::new(move|request|{
            match controller.options.inline_create.as_ref(){Some(build)=>handle_media_inline_create(request,build(request)),None=>{
                let current=controller.clone();let bad=current.message(request,current.options.bad_request_message.as_ref());
                crate::media::flows::form_flow(request,current.options.io.clone(),bad,Arc::new(move|request,_form|current.internal(request)))
            }}
        }))
    }
    pub fn preview_inline_create(&self,request:&Request)->Effect<Reply<E,R>,E,R>{
        let controller=self.clone();self.with_access(request,!crate::media::surface::media_datastar_request(request),Arc::new(move|request|{
            match controller.options.inline_preview.as_ref(){Some(build)=>handle_media_inline_preview(request,build(request)),None=>controller.internal(request)}
        }))
    }
    pub fn handlers(&self)->super::MediaManagementHandlers<E,R>{
        super::MediaManagementHandlers {
            list:Some({let controller=self.clone();Arc::new(move|request|controller.list(request))}),
            new:Some({let controller=self.clone();Arc::new(move|request|controller.new_form(request))}),
            new_post:Some({let controller=self.clone();Arc::new(move|request|controller.create(request))}),
            edit:Some({let controller=self.clone();Arc::new(move|request|controller.edit(request))}),
            edit_post:Some({let controller=self.clone();Arc::new(move|request|controller.update(request))}),
            delete:Some({let controller=self.clone();Arc::new(move|request|controller.delete(request))}),
            picker:Some({let controller=self.clone();Arc::new(move|request|controller.search_picker(request))}),
            picker_attach:Some({let controller=self.clone();Arc::new(move|request|controller.attach_picker_selection(request))}),
            inline_preview:Some({let controller=self.clone();Arc::new(move|request|controller.preview_inline_create(request))}),
            inline_create:Some({let controller=self.clone();Arc::new(move|request|controller.inline_create_fragment(request))}),
            shortcode:Some({let controller=self.clone();Arc::new(move|request|controller.shortcode_modal(request))}),
            attachment_mutate:Some({let controller=self.clone();Arc::new(move|request|controller.mutate_attachments(request))}),
        }
    }
}
