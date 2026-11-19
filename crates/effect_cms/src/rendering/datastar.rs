//! Shared Datastar framing; all element payloads are serialized Maud components.
use maud::Markup;
use effect::memory::{Buffer, BufferLease, BufferView, MemoryError, SlotPool};
use effect::{Effect, Exit, NativeContext};
use std::num::NonZeroUsize;
use std::sync::Arc;
use effect_server::{BodyWriter, Reply, reply};
use http::{header,HeaderName,HeaderValue};
fn visit_field<E>(
    name: &str,
    mut remaining: &str,
    mut append: impl FnMut(&str) -> Result<(), E>,
) -> Result<(), E> {
    loop {
        let end = remaining.find(['\r', '\n']).unwrap_or(remaining.len());
        let (line, separator) = remaining.split_at(end);
        for part in ["data: ", name, " ", line, "\n"] {
            append(part)?;
        }
        if separator.is_empty() {
            return Ok(());
        }
        remaining = separator
            .strip_prefix("\r\n")
            .or_else(|| separator.strip_prefix(['\r', '\n']))
            .unwrap_or_default();
    }
}

fn field(event: &mut String, name: &str, value: &str) {
    let Ok(()) = visit_field(name, value, |part| {
        event.push_str(part);
        Ok::<_, std::convert::Infallible>(())
    });
}

pub(crate) fn component_patch_lease(
    mut output: BufferLease<'static>,
    selector: &str,
    mode: &str,
    markup: &Markup,
) -> Result<BufferView<'static>, MemoryError> {
    output.clear();
    output.extend_from_slice(b"event: datastar-patch-elements\n")?;
    {
        let mut append = |value: &str| output.extend_from_slice(value.as_bytes());
        for (name, value) in [("selector", selector), ("mode", mode), ("elements", markup.0.as_str())] {
            if !value.is_empty() {
                visit_field(name, value, &mut append)?;
            }
        }
        visit_field("ifmissing", "ignore", append)?;
    }
    output.extend_from_slice(b"\n")?;
    Ok(BufferView::new(output.share()))
}

pub(crate) fn signal_patch_lease(
    mut output: BufferLease<'static>,
    signals: &str,
) -> Result<BufferView<'static>, MemoryError> {
    output.clear();
    output.extend_from_slice(b"event: datastar-patch-signals\n")?;
    if !signals.is_empty() {
        visit_field("signals", signals, |value| output.extend_from_slice(value.as_bytes()))?;
    }
    output.extend_from_slice(b"\n")?;
    Ok(BufferView::new(output.share()))
}

#[derive(Clone)]
struct PreparedPatches {
    selector: Arc<str>,
    mode: Arc<str>,
    components: Arc<[Markup]>,
    signals: Arc<str>,
    frames: Arc<SlotPool<'static, Buffer<'static>>>,
}

type MapCapacity<E> = dyn Fn(MemoryError) -> E + Send + Sync;

async fn send_patch<E: Send + 'static, R: Send + Sync + 'static>(
    frame: BufferView<'static>,
    writer: &BodyWriter<E, R>,
    context: &NativeContext,
    environment: Arc<R>,
) -> Exit<(), E> {
    match context.run(writer.write_lease(frame), environment.clone()).await {
        Exit::Success(()) => context.run(writer.flush(), environment).await,
        failure @ (Exit::Fail(_) | Exit::Die(_) | Exit::Interrupted | Exit::FinalizationFailed(_)) => failure,
    }
}

fn prepared_patch_body<E: Send + 'static, R: Send + Sync + 'static>(
    patches: PreparedPatches,
    first: Effect<BufferLease<'static>, E, R>,
    writer: BodyWriter<E, R>,
    errors: Arc<MapCapacity<E>>,
) -> Effect<(), E, R> {
    effect::effect_future! {
        exit_with_context;
        body: move |environment: Arc<R>, context| {
            let patches = patches.clone();
            let first = first.clone();
            let writer = writer.clone();
            let errors = errors.clone();
            async move {
                let mut initial = match context.run(first, environment.clone()).await {
                    Exit::Success(frame) => Some(frame),
                    Exit::Fail(error) => return Exit::Fail(error),
                    Exit::Die(defect) => return Exit::Die(defect),
                    Exit::Interrupted => return Exit::Interrupted,
                    Exit::FinalizationFailed(failure) => return Exit::FinalizationFailed(failure),
                };
                for component in patches.components.iter() {
                    if context.cancellation().is_requested() {
                        return Exit::Interrupted;
                    }
                    let frame = initial.take().map_or_else(|| patches.frames.try_checkout(), Ok)
                        .and_then(|output| component_patch_lease(output, &patches.selector, &patches.mode, component));
                    let frame = match frame {
                        Ok(frame) => frame,
                        Err(error) => return Exit::Fail(errors(error)),
                    };
                    match send_patch(frame, &writer, &context, environment.clone()).await {
                        Exit::Success(()) => (),
                        failure @ (Exit::Fail(_) | Exit::Die(_) | Exit::Interrupted | Exit::FinalizationFailed(_)) => return failure,
                    }
                }
                if context.cancellation().is_requested() {
                    return Exit::Interrupted;
                }
                match initial.take().map_or_else(|| patches.frames.try_checkout(), Ok)
                    .and_then(|output| signal_patch_lease(output, &patches.signals))
                {
                    Ok(frame) => send_patch(frame, &writer, &context, environment).await,
                    Err(error) => Exit::Fail(errors(error)),
                }
            }
        },
    }
}

/// Scope one finite sequence of Maud element patches followed by its signal patch.
pub(crate) fn prepared_event_stream<E: Send + 'static, R: Send + Sync + 'static>(
    (selector, mode, components, signals): (Arc<str>, Arc<str>, Arc<[Markup]>, Arc<str>),
    frames: Arc<SlotPool<'static, Buffer<'static>>>,
    maximum_components: NonZeroUsize,
    on_error: impl Fn(MemoryError) -> E + Send + Sync + 'static,
) -> Effect<Reply<E, R>, E, R> {
    let count = components.len();
    let patches = PreparedPatches { selector, mode, components, signals, frames: frames.clone() };
    let errors: Arc<MapCapacity<E>> = Arc::new(on_error);
    let refusal = errors.clone();
    let reserve = effect::effect_try_sync! {
        body {
            if count > maximum_components.get() {
                return Err(MemoryError::CapacityExceeded {
                    resource: 0,
                    requested: count,
                    available: maximum_components.get(),
                    control: false,
                });
            }
            frames.try_checkout()
        },
        on_error |error| { refusal(error) },
    };
    effect::effect_map! {
        program: reserve,
        map: move |frame| {
            let first = effect::effect_sync! { once; body: move || frame, };
            let patches = patches.clone();
            let errors = errors.clone();
            let body = effect_server::stream! {
                body: move |writer| prepared_patch_body(patches.clone(), first.clone(), writer, errors.clone()),
            };
            reply! {
                header(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
                header(header::CONNECTION, HeaderValue::from_static("keep-alive"));
                header(HeaderName::from_static("x-accel-buffering"), HeaderValue::from_static("no"));
                sse(body);
            }
        },
    }
}
pub(crate) fn component_patch_event(selector:&str,mode:&str,component:&Markup)->String{
    let mut event=String::from("event: datastar-patch-elements\n");
    if !selector.is_empty(){field(&mut event,"selector",selector);}
    if !mode.is_empty(){field(&mut event,"mode",mode);}
    if !component.0.as_str().is_empty(){field(&mut event,"elements",component.0.as_str());}
    field(&mut event,"ifmissing","ignore");event.push('\n');event
}
pub(crate) fn signal_patch_event(signals:&str)->String{
    let mut event=String::from("event: datastar-patch-signals\n");
    if !signals.is_empty(){field(&mut event,"signals",signals);}
    event.push('\n');event
}
pub(crate) fn event_reply<E:Send+'static,R:Send+Sync+'static>(event:String)->Reply<E,R>{
    reply! {header(header::CONTENT_TYPE,HeaderValue::from_static("text/event-stream"));
        header(header::CACHE_CONTROL,HeaderValue::from_static("no-cache"));
        header(header::CONNECTION,HeaderValue::from_static("keep-alive"));
        header(HeaderName::from_static("x-accel-buffering"),HeaderValue::from_static("no"));text(event);}
}
/// Native body ownership preserves event write/flush ordering and terminal failures.
pub(crate) fn event_stream_reply<E:Send+'static,R:Send+Sync+'static>(events:Vec<String>)->Reply<E,R>{
    let body=effect_server::stream! {body |writer| {
        let events=events.clone();let writer=writer.clone();
        for event in events {if !event.is_empty(){writer.write(event.into())?;writer.flush()?;}}
    }};
    reply! {header(header::CACHE_CONTROL,HeaderValue::from_static("no-cache"));
        header(header::CONNECTION,HeaderValue::from_static("keep-alive"));
        header(HeaderName::from_static("x-accel-buffering"),HeaderValue::from_static("no"));sse(body);}
}
