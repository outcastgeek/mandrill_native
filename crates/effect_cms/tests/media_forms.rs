//! Native media, attachments, form state and authoring behavior regressions.
use std::{future::Future, pin::pin, sync::Arc, task::{Context,Poll,Wake,Waker}};
use effect::{Effect,Exit};
struct ThreadWake(std::thread::Thread);
impl Wake for ThreadWake {fn wake(self:Arc<Self>){self.0.unpark()}fn wake_by_ref(self:&Arc<Self>){self.0.unpark()}}
fn block_on<F:Future>(future:F)->F::Output {let waker=Waker::from(Arc::new(ThreadWake(std::thread::current())));let mut cx=Context::from_waker(&waker);let mut future=pin!(future);loop {match future.as_mut().poll(&mut cx){Poll::Ready(v)=>return v,Poll::Pending=>std::thread::park()}}}
fn success<A:Send+'static,E:std::fmt::Debug+Send+'static,R:Send+Sync+'static>(effect:Effect<A,E,R>,env:Arc<R>)->A {match block_on(effect.run(env)){Exit::Success(value)=>value,other=>panic!("expected success; {}",match other {Exit::Success(_)=>"success".into(),Exit::Fail(error)=>format!("failure: {error:?}"),_=>"non-success exit".into()})}}
#[path="media_forms/assets.rs"]mod assets;
#[path="media_forms/render.rs"]mod render;
#[path="media_forms/forms.rs"]mod forms;
#[path="media_forms/authoring.rs"]mod authoring;
#[path="media_forms/flows.rs"]mod flows;
#[path="media_forms/experience.rs"]mod experience;
fn request(uri:&str)->effect_server::Request {effect_server::Request::from_native(http::Request::builder().uri(uri).body(effect_server::native::axum::body::Body::empty()).unwrap(),effect::Cancellation::new())}
fn form_policy()->effect_cms::attachments::MediaFormPolicy {use std::num::NonZeroUsize as N;use effect_server::*;let bound=N::new(4096).unwrap();let values=ByteFieldBounds{count:N::new(32).unwrap(),bytes:bound};effect_cms::attachments::MediaFormPolicy{urlencoded:ByteFormPolicy{body:BodyLimit::new(bound),fields:N::new(64).unwrap(),values},multipart:MultipartPolicy{body:BodyLimit::new(bound),part_bytes:bound,header_bytes:bound,parts:N::new(64).unwrap(),field_bytes:bound},multipart_fields:N::new(64).unwrap(),multipart_values:values,file_memory:4096}}
#[path="support/native_http.rs"]mod native_http;
