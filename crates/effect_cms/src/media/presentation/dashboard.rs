//! Media workbench production over the canonical dashboard values and consumer store.
use std::sync::Arc;
use effect::{Effect,effect,effect_sync};
use effect_server::{Request,Reply};
use crate::{commands::InputBytes,definitions::SharedList,dashboards::*,media::{assets::{MediaAssetStore,MediaAssetRecord,media_asset_display_name,media_asset_preview_url},surface::MediaQuery}};
use super::{view_text,media_kind_label,media_status_label};

pub const MEDIA_DASHBOARD_STAT_TOTAL:&str="media-total";
pub const MEDIA_DASHBOARD_STAT_READY:&str="media-ready";
pub const MEDIA_DASHBOARD_STAT_DRAFTS:&str="media-drafts";
pub const MEDIA_DASHBOARD_STAT_ARCHIVED:&str="media-archived";
pub const MEDIA_DASHBOARD_DEFAULT_PAGE_PARAM:&str="page";
pub const MEDIA_DASHBOARD_DEFAULT_PAGE_SIZE:i64=5;
pub type DashboardErrorReporter<E> = Arc<dyn Fn(&str,&E)+Send+Sync>;
pub type TranslateDashboardTextFunc = Arc<dyn Fn(Option<&Request>,&str,&[String])->String+Send+Sync>;
pub type FormatDashboardDateFunc = Arc<dyn Fn(i64)->String+Send+Sync>;
pub type MediaDashboardRecordHref = Arc<dyn Fn(&MediaAssetRecord)->String+Send+Sync>;

pub struct MediaDashboardPageOptions<E:Send+'static,R:Send+Sync+'static>{
    pub request:Option<Request>,pub store:Option<Arc<dyn MediaAssetStore<E,R>>>,pub page_param:String,pub page_size:i64,
    pub library_href:String,pub edit_href:Option<MediaDashboardRecordHref>,pub delete_href:Option<MediaDashboardRecordHref>,
    pub search:DashboardSearchVM,pub translate_text:Option<TranslateDashboardTextFunc>,pub translate_key:Option<crate::localization::TranslateKeyFunc<Request>>,
    pub format_date:Option<FormatDashboardDateFunc>,pub report_error:Option<DashboardErrorReporter<E>>,
}
#[derive(Clone,Default)]
pub struct MediaDashboardPageVM{
    pub search:DashboardSearchVM,pub filters:Vec<DashboardFilterChipVM>,pub stats_grid_class:String,pub stats:Vec<DashboardStatVM>,
    pub recent:DashboardListSectionVM,pub summary:DashboardSummarySectionVM,pub tools:DashboardToolSectionVM,
}
fn translated(request:Option<&Request>,translate:Option<&TranslateDashboardTextFunc>,key:&str,args:&[String])->String{
    let key=key.trim();if key.is_empty(){return String::new();}
    let value=translate.map(|translate|translate(request,key,args)).unwrap_or_else(||args.first().map(|value|crate::text_format::format_one_string(key,value)).unwrap_or_else(||key.to_owned()));
    if value.trim().is_empty(){key.to_owned()}else{value.trim().into()}
}
pub fn count_media_assets<E:Send+'static,R:Send+Sync+'static>(store:Option<Arc<dyn MediaAssetStore<E,R>>>,query:MediaQuery,report:Option<DashboardErrorReporter<E>>)->Effect<i64,E,R>{
    effect!{let store=store.clone();let query=query.clone();let report=report.clone();
        match store{None=>0,Some(store)=>match store.list(query).either()?{
            Ok(result)=>result.map(|result|result.total).unwrap_or(0),Err(error)=>{if let Some(report)=report{report("count media assets",&error);}0}
        }}
    }
}
fn request_page(request:Option<&Request>,param:&str)->i64{
    request.map(|request|crate::queries::request_query_value(request,param)).and_then(|value|value.to_str().ok().and_then(|value|value.parse().ok())).filter(|page|*page>0).unwrap_or(1)
}
fn pages(total:i64,size:i64)->i64{if total<=0||size<=0{1}else{(total/size)+i64::from(total%size!=0)}}
fn href(request:Option<&Request>,base:&str,page_param:&str,filter:Option<(&str,&str)>,page:Option<i64>)->String{
    let base=if base.trim().is_empty(){request.map(|request|request.uri().path()).unwrap_or_default()}else{base.trim()};
    if base.is_empty(){return String::new();}
    if !crate::media::assets::media_url_is_valid(base){return base.into();}
    let encoded_base=crate::content::encode_reference_url(base).unwrap_or_else(||base.to_owned());
    let base=encoded_base.as_str();
    let (without_fragment,fragment)=base.split_once('#').map(|(head,tail)|(head,Some(tail))).unwrap_or((base,None));
    let (path,query)=without_fragment.split_once('?').unwrap_or((without_fragment,""));
    let mut values=request.map(crate::queries::request_query_map).unwrap_or_default();
    for (key,value) in effect_server::decode_url_encoded_pairs_partial(query.as_bytes()).values{values.insert(key.into(),value.into_iter().map(InputBytes::from).collect());}
    if let Some(page)=page{values.insert(page_param.into(),vec![page.max(1).to_string().into()]);}
    else{values.remove(page_param.as_bytes());values.remove(b"status".as_slice());values.remove(b"kind".as_slice());if let Some((key,value))=filter{if !key.is_empty(){values.insert(key.into(),vec![value.into()]);}}}
    let mut target=crate::queries::append_encoded_query(path,&values);
    if query.is_empty()&&without_fragment.ends_with('?')&&without_fragment.bytes().filter(|byte|*byte==b'?').count()==1&&values.is_empty(){target.push('?');}
    if let Some(fragment)=fragment{target.push('#');target.push_str(fragment);}target
}
pub fn media_dashboard_page_href(request:Option<&Request>,base:&str,param:&str,page:i64)->String{href(request,base,if param.trim().is_empty(){"page"}else{param},None,Some(page))}

pub fn media_dashboard_row(request:Option<&Request>,item:&MediaAssetRecord,translate:Option<&TranslateDashboardTextFunc>,translate_key:Option<&crate::localization::TranslateKeyFunc<Request>>,format:Option<&FormatDashboardDateFunc>,edit:Option<&MediaDashboardRecordHref>,delete:Option<&MediaDashboardRecordHref>)->DashboardRowVM{
    let link=|resolve:Option<&MediaDashboardRecordHref>|if item.id.trim().is_empty(){String::new()}else{resolve.map(|resolve|resolve(item).trim().to_owned()).unwrap_or_default()};
    let title_href=link(edit);
    let mut metadata=vec![view_text(item.provider.trim())];
    if item.updated_at>0{if let Some(format)=format{let date=format(item.updated_at);if !date.trim().is_empty(){metadata.push(translated(request,translate,"contentmgmt.media_dashboard.updated_on",&[date.trim().into()]));}}}
    let meta_text=metadata.into_iter().filter(|value|!value.trim().is_empty()).collect::<Vec<_>>().join(" · ");
    let preview_url=view_text(media_asset_preview_url(item,"image",None));
    let label=|kind:bool|request.map(|request|if kind{media_kind_label(translate_key,request,&item.kind)}else{media_status_label(translate_key,request,&item.status)})
        .unwrap_or_else(||if kind{format!("media.kind.{}",crate::media::assets::normalize_media_kind(&item.kind))}else{format!("media.status.{}",crate::media::assets::normalize_media_status(&item.status))});
    let badges=vec![DashboardRowBadgeVM{label:label(true),variant:DashboardRowBadgeVariant(DASHBOARD_ROW_BADGE_VARIANT_OUTLINE.into())},DashboardRowBadgeVM{label:label(false),variant:DashboardRowBadgeVariant(DASHBOARD_ROW_BADGE_VARIANT_SECONDARY.into())}];
    let delete_href=link(delete);let mut actions=Vec::new();
    if !title_href.is_empty(){actions.push(DashboardRowActionVM{kind:"edit".into(),label:translated(request,translate,"common.edit",&[]),href:title_href.clone(),..Default::default()});}
    if !delete_href.is_empty(){actions.push(DashboardRowActionVM{kind:"delete".into(),label:translated(request,translate,"common.delete",&[]),href:delete_href,confirm_message:translated(request,translate,"media.list.delete_confirm",&[])});}
    if !item.url.trim().is_empty(){actions.push(DashboardRowActionVM{kind:"external-link".into(),label:translated(request,translate,"contentmgmt.media_dashboard.open_source",&[]),href:view_text(item.url.trim()),..Default::default()});}
    DashboardRowVM{title:view_text(media_asset_display_name(item)),title_href,secondary_text:view_text(item.url.trim()),meta_text,preview_url,
        preview_alt:if item.alt.trim().is_empty(){view_text(media_asset_display_name(item))}else{view_text(item.alt.trim())},preview_kind:item.kind.trim().into(),badges:Some(SharedList::new(badges)),actions:Some(SharedList::new(actions)),..Default::default()}
}
pub fn media_dashboard_rows(request:Option<&Request>,items:&[MediaAssetRecord],translate:Option<&TranslateDashboardTextFunc>,translate_key:Option<&crate::localization::TranslateKeyFunc<Request>>,format:Option<&FormatDashboardDateFunc>,edit:Option<&MediaDashboardRecordHref>,delete:Option<&MediaDashboardRecordHref>)->Vec<DashboardRowVM>{items.iter().map(|item|media_dashboard_row(request,item,translate,translate_key,format,edit,delete)).collect()}

pub fn build_media_dashboard_page<E:Send+'static,R:Send+Sync+'static>(options:MediaDashboardPageOptions<E,R>)->Effect<MediaDashboardPageVM,E,R>{
    let options=Arc::new(options);
    effect!{let options=options.clone();
        let request=options.request.clone();let page_size=if options.page_size>0{options.page_size}else{5};
        let param=if options.page_param.trim().is_empty(){"page".to_owned()}else{options.page_param.trim().to_owned()};let page=request_page(request.as_ref(),&param);
        let translate_request=request.clone();let translate=options.translate_text.clone();
        let t=move |key:&str|translated(translate_request.as_ref(),translate.as_ref(),key,&[]);
        let mut search=options.search.clone();
        if search.value.trim().is_empty(){search.value=crate::dashboards::dashboard_search_query_value(request.as_ref());}
        if search.name.trim().is_empty(){search.name="search".into();}if search.id.trim().is_empty(){search.id="media-dashboard-search".into();}if search.form_id.trim().is_empty(){search.form_id=format!("{}-form",search.id);}
        if search.placeholder.trim().is_empty(){search.placeholder=t("contentmgmt.media_dashboard.search_placeholder");}if search.aria_label.trim().is_empty(){search.aria_label=search.placeholder.clone();}
        if search.shortcut_label.trim().is_empty(){search.shortcut_label=t("common.search_shortcut");}if search.action.trim().is_empty(){search.action=request.as_ref().map(|request|request.uri().path().trim().to_owned()).unwrap_or_default();}search.enabled=search.enabled||!search.action.trim().is_empty();
        let status=request.as_ref().map(|request|crate::queries::request_query_value(request,"status")).unwrap_or_default();let kind=request.as_ref().map(|request|crate::queries::request_query_value(request,"kind")).unwrap_or_default();
        let recent=match options.store.clone(){None=>None,Some(store)=>match store.list(MediaQuery{search:search.value.trim(),status:status.trim(),kind:kind.trim(),limit:page_size,offset:(page-1).wrapping_mul(page_size),..Default::default()}).either()?{
            Ok(result)=>result,Err(error)=>{if let Some(report)=&options.report_error{report("list recent media",&error);}None}
        }};
        let mut filters=Vec::new();
        if request.is_some(){for (key,value,label) in [("","","contentmgmt.media_dashboard.view_all"),("status","ready","contentmgmt.media_dashboard.ready"),("status","draft","contentmgmt.media_dashboard.drafts"),("status","archived","contentmgmt.media_dashboard.archived"),("kind","image","media.kind.image"),("kind","video","media.kind.video"),("kind","document","media.kind.document")]{
            let label=t(label);let href=href(request.as_ref(),&search.action,&param,Some((key,value)),None);
            let active=match key{"status"=>status.as_bytes().eq_ignore_ascii_case(value.as_bytes()),"kind"=>kind.as_bytes().eq_ignore_ascii_case(value.as_bytes()),_=>status.is_empty()&&kind.is_empty()};
            let mut query=MediaQuery{limit:1,..Default::default()};if key=="status"{query.status=value.into();}if key=="kind"{query.kind=value.into();}
            let count=count_media_assets(options.store.clone(),query,options.report_error.clone())?;
            filters.push(DashboardFilterChipVM{label,href,active,count:count.to_string()});
        }}
        let mut stats=Vec::new();
        for (key,status,label,icon) in [(MEDIA_DASHBOARD_STAT_TOTAL,"","contentmgmt.media_dashboard.total",DASHBOARD_STAT_ICON_MEDIA),(MEDIA_DASHBOARD_STAT_READY,"ready","contentmgmt.media_dashboard.ready",DASHBOARD_STAT_ICON_CHECK),(MEDIA_DASHBOARD_STAT_DRAFTS,"draft","contentmgmt.media_dashboard.drafts",DASHBOARD_STAT_ICON_DRAFT),(MEDIA_DASHBOARD_STAT_ARCHIVED,"archived","contentmgmt.media_dashboard.archived",DASHBOARD_STAT_ICON_SETTINGS)]{
            let label=t(label);let count=count_media_assets(options.store.clone(),MediaQuery{status:status.into(),limit:1,..Default::default()},options.report_error.clone())?;
            stats.push(DashboardStatVM{key:key.into(),label,value:count.to_string(),icon:DashboardStatIcon(icon.into())});
        }
        let recent_section=DashboardListSectionVM{title:t("media.list.title"),empty_title:t("contentmgmt.media_dashboard.empty_title"),empty_description:t("contentmgmt.media_dashboard.empty_description"),pagination:DashboardPaginationVM{base_href:search.action.clone(),page_param:param.into(),current_page:page,total_pages:pages(recent.as_ref().map(|result|result.total).unwrap_or(0),page_size)},
            rows:recent.as_ref().filter(|result|!result.items.is_empty()).map(|result|SharedList::new(media_dashboard_rows(request.as_ref(),&result.items,options.translate_text.as_ref(),options.translate_key.as_ref(),options.format_date.as_ref(),options.edit_href.as_ref(),options.delete_href.as_ref()))),..Default::default()};
        let summary_title=t("contentmgmt.media_dashboard.media_types");let mut summary=Vec::new();
        for kind in ["image","video","document"]{let label=t(&format!("media.kind.{kind}"));let count=count_media_assets(options.store.clone(),MediaQuery{kind:kind.into(),limit:1,..Default::default()},options.report_error.clone())?;summary.push(DashboardSummaryRowVM{label,value:count.to_string()});}
        MediaDashboardPageVM{search,filters,stats_grid_class:"grid grid-cols-2 gap-4 lg:grid-cols-4".into(),stats,recent:recent_section,
            summary:DashboardSummarySectionVM{title:summary_title,rows:Some(SharedList::new(summary))},tools:DashboardToolSectionVM{title:t("contentmgmt.media_dashboard.library_tools"),..Default::default()}}
    }
}
pub fn build_media_content_dashboard_page(shell:ContentDashboardShellVM,page:MediaDashboardPageVM)->ContentDashboardPageVM{
    ContentDashboardPageVM{layout:DashboardLayout(DASHBOARD_LAYOUT_MEDIA_WORKBENCH.into()),page_meta:shell.page_meta,header_actions:shell.header_actions,search:page.search,
        filters:Some(SharedList::new(page.filters)),stats_grid_class:page.stats_grid_class,stats:Some(SharedList::new(page.stats)),primary_section:page.recent,
        summary_section:page.summary,tool_section:DashboardToolSectionVM{title:page.tools.title,actions:shell.tool_actions},..Default::default()}
}

/// Request adapters for the complete media dashboard handler.
pub struct MediaDashboardHandlerOptions<E:Send+'static,R:Send+Sync+'static>{
    pub store:Option<Arc<dyn MediaAssetStore<E,R>>>,
    pub require_access:Option<Arc<dyn Fn(&Request)->Effect<Option<Reply<E,R>>,E,R>+Send+Sync>>,
    pub page_param:String,pub page_size:i64,
    pub library_href:Option<Arc<dyn Fn(&Request)->String+Send+Sync>>,
    pub edit_href:Option<crate::media::management::MediaAssetDetailHref>,pub delete_href:Option<crate::media::management::MediaAssetDetailHref>,
    pub translate_text:Option<TranslateDashboardTextFunc>,pub translate_key:Option<crate::localization::TranslateKeyFunc<Request>>,
    pub format_date:Option<FormatDashboardDateFunc>,pub report_error:Option<DashboardErrorReporter<E>>,
    pub build_shell:Option<Arc<dyn Fn(&Request)->Effect<ContentDashboardShellVM,E,R>+Send+Sync>>,
    pub build_search:Option<Arc<dyn Fn(&Request)->Effect<DashboardSearchVM,E,R>+Send+Sync>>,
    pub render:Option<Arc<dyn Fn(&Request,ContentDashboardPageVM)->Effect<Reply<E,R>,E,R>+Send+Sync>>,
}
pub fn media_dashboard_handler<E:Send+'static,R:Send+Sync+'static>(options:MediaDashboardHandlerOptions<E,R>)->crate::routing::RouteHandler<E,R>{
    let options=Arc::new(options);
    Arc::new(move|request|{
        let request=request.clone();let options=options.clone();
        effect!{let request=request.clone();let options=options.clone();
            let denied=match options.require_access.clone(){Some(require)=>require(&request)?,None=>None};
            match denied{Some(reply)=>reply,None=>match options.render.clone(){
                None=>effect_sync!{body:||{
                    effect_server::reply!{status(effect_server::BodyStatus::new(http::StatusCode::INTERNAL_SERVER_ERROR).unwrap_or(effect_server::BodyStatus::Ok));
                        header(http::HeaderName::from_static("x-content-type-options"),http::HeaderValue::from_static("nosniff"));text("media dashboard renderer is not configured\n".to_owned());}
                }}?,
                Some(render)=>{
                    let shell=match options.build_shell.clone(){Some(build)=>build(&request)?,None=>ContentDashboardShellVM::default()};
                    let library=options.library_href.as_ref().map(|resolve|resolve(&request).trim().to_owned()).unwrap_or_default();
                    let search=match options.build_search.clone(){Some(build)=>build(&request)?,None=>default_media_dashboard_search(&request,options.translate_text.as_ref())};
                    let page=build_media_dashboard_page(MediaDashboardPageOptions{request:Some(request.clone()),store:options.store.clone(),page_param:options.page_param.clone(),page_size:options.page_size,
                        library_href:library,edit_href:dashboard_record_href(&request,&options.edit_href),delete_href:dashboard_record_href(&request,&options.delete_href),search,translate_text:options.translate_text.clone(),translate_key:options.translate_key.clone(),format_date:options.format_date.clone(),report_error:options.report_error.clone()})?;
                    render(&request,build_media_content_dashboard_page(shell,page))?
                }
            }}
        }
    })
}

fn default_media_dashboard_search(request:&Request,translate:Option<&TranslateDashboardTextFunc>)->DashboardSearchVM{
    let t=|key:&str|translated(Some(request),translate,key,&[]);
    new_dashboard_search_vm(DashboardSearchOptions{enabled:true,id:"media-dashboard-search".into(),form_id:"media-dashboard-search-form".into(),name:"search".into(),value:dashboard_search_query_value(Some(request)),placeholder:t("contentmgmt.media_dashboard.search_placeholder"),aria_label:t("contentmgmt.media_dashboard.search_placeholder"),action:request.uri().path().trim().into(),shortcut_label:t("common.search_shortcut")})
}
fn dashboard_record_href(request:&Request,callback:&Option<crate::media::management::MediaAssetDetailHref>)->Option<MediaDashboardRecordHref>{
    callback.clone().map(|callback|{let request=request.clone();Arc::new(move|item:&MediaAssetRecord|callback(&request,item).trim().to_owned())as MediaDashboardRecordHref})
}
