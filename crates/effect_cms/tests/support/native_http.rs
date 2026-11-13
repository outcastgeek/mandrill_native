//! Real native transport for CMS Reply producers; all tasks are joined and drained.
use std::{num::NonZeroUsize,sync::Arc,time::Duration};
use effect::{AdmissionCapacity,Cancellation,Diagnostics,DiagnosticCapacity,DiagnosticOverflow,Effect,Exit,HasClock,HasLogger,HasTaskSpawner,Logger,LogLevel,effect_try_future,effect_sync,service};
use effect_server::{BodyPolicy,HostPolicy,Request,RequestEnv,Reply,ServerError,ServerPolicy,RequestBinding,HttpProtocols,HasTokio,endpoint,registered,router,server};
use effect_tokio::{TokioTasks,TokioClock};
use http_body_util::BodyExt;
pub struct TransportServices{runtime:Arc<tokio::runtime::Runtime>,tasks:TokioTasks,clock:TokioClock,logger:Logger}
impl HasTaskSpawner for TransportServices{type Tasks=TokioTasks;fn task_spawner(&self)->&TokioTasks{&self.tasks}}
impl HasClock for TransportServices{type Clock=TokioClock;fn clock(&self)->&TokioClock{&self.clock}}
impl HasTokio for TransportServices{fn tokio(&self)->&tokio::runtime::Handle{self.runtime.handle()}}
impl HasLogger for TransportServices{fn logger(&self)->&Logger{&self.logger}}
pub type Env=RequestEnv<TransportServices>;
pub type Handler<E>=Arc<dyn Fn(&Request)->Effect<Reply<E,Env>,E,Env>+Send+Sync>;
pub struct Response{pub status:http::StatusCode,pub headers:http::HeaderMap,pub body:String}
pub fn deliver<E:std::fmt::Debug+Send+'static>(handler:Handler<E>,native_error:Arc<dyn Fn(ServerError)->E+Send+Sync>)->Result<Response,String>{
    deliver_at(handler,native_error,http::Method::GET,"/".into(),String::new(),None)
}
/// Exercise the same native host with a real method, form body and optional typed ID capture.
pub fn deliver_at<E:std::fmt::Debug+Send+'static>(handler:Handler<E>,native_error:Arc<dyn Fn(ServerError)->E+Send+Sync>,method:http::Method,uri:String,body:String,capture:Option<effect_server::Route<String>>)->Result<Response,String>{
    let runtime=Arc::new(tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().map_err(|e|e.to_string())?);
    let tasks=TokioTasks::new(runtime.clone(),AdmissionCapacity::Bounded(NonZeroUsize::MIN.saturating_add(31))).map_err(|e|format!("{e:?}"))?;
    let diagnostics=Diagnostics::new(DiagnosticCapacity::Bounded(NonZeroUsize::MIN.saturating_add(15)),DiagnosticOverflow::DiscardOldest);
    let services=Arc::new(TransportServices{runtime:runtime.clone(),tasks:tasks.clone(),clock:TokioClock::new(runtime.clone()),logger:Logger::new(LogLevel::Debug,diagnostics,|values|Ok(values.clone()),|_|Ok(()))});
    let initial=Request::from_native(http::Request::new(effect_server::native::axum::body::Body::empty()),Cancellation::new());
    let environment=Arc::new(RequestEnv{services:services.clone(),request:initial});
    let binding=RequestBinding::new(move|request|{let services=services.clone();effect_sync!{body:move||Arc::new(RequestEnv{services:services.clone(),request:request.clone()})}});
    let methods=effect_server::HttpMethodSet::new(vec![method.clone()]).map_err(|e|e.to_string())?;
    let endpoint=endpoint!{
        let handler=handler.clone();
        let request=service!{env:|env:&Env|env.request.clone()}?;
        handler(&request)?
    };
    let path=uri.split('?').next().unwrap_or("/");
    let routes=match capture{
        Some(route)=>router!{methods(methods,route,move |_:String|endpoint.clone())},
        None=>router!{methods(methods,path,registered!{endpoint(endpoint)})},
    }.map_err(|e|format!("{e:?}"))?;
    let policy_error=native_error.clone();
    let policy=ServerPolicy::new(BodyPolicy{frames:NonZeroUsize::MIN.saturating_add(7),chunk_bytes:NonZeroUsize::MIN.saturating_add(16383)},HostPolicy{drain:Duration::from_secs(3)},move|e|policy_error(e),|_|effect_sync!{body:||effect_server::reply!{text("native error".to_owned())}});
    let server=server!{policy:policy,environment:binding,listen:std::net::SocketAddr::from(([127,0,0,1],0)),protocols:HttpProtocols::Http1,routes:routes}.map_err(|e|format!("{e:?}"))?;
    let program=server.with_host(move|host|{
        let native_error=native_error.clone();let method=method.clone();let uri=uri.clone();let body=body.clone();
        effect_try_future!{once;body:move |_|async move{
            let stream=tokio::net::TcpStream::connect(host.address).await.map_err(|e|ServerError::Invalid(e.to_string()))?;
            let (mut client,connection)=hyper::client::conn::http1::handshake(hyper_util::rt::TokioIo::new(stream)).await.map_err(|e|ServerError::Invalid(e.to_string()))?;
            let driver=tokio::spawn(connection);
            let request=http::Request::builder().method(method).uri(format!("http://{}{uri}",host.address)).header(http::header::CONTENT_TYPE,"application/x-www-form-urlencoded").body(http_body_util::Full::new(effect_server::native::bytes::Bytes::from(body))).map_err(|e|ServerError::Invalid(e.to_string()))?;
            let response=client.send_request(request).await.map_err(|e|ServerError::Invalid(e.to_string()))?;
            let status=response.status();let headers=response.headers().clone();
            let bytes=response.into_body().collect().await.map_err(|e|ServerError::Invalid(e.to_string()))?.to_bytes();
            drop(client);driver.await.map_err(|e|ServerError::Invalid(e.to_string()))?.map_err(|e|ServerError::Invalid(e.to_string()))?;
            host.stop();host.drained().await;
            Ok(Response{status,headers,body:String::from_utf8(bytes.to_vec()).map_err(|e|ServerError::Invalid(e.to_string()))?})
        },on_error:move |error|native_error(error)}
    });
    let exit=runtime.block_on(program.run(environment));runtime.block_on(tasks.stop().wait());
    match exit{Exit::Success(response)=>Ok(response),Exit::Fail(error)=>Err(format!("HTTP effect failed: {error:?}")),Exit::Die(d)=>Err(format!("HTTP defect: {d:?}")),Exit::Interrupted=>Err("HTTP interrupted".into()),Exit::FinalizationFailed(f)=>Err(format!("HTTP cleanup: {f:?}"))}
}
