use fx::{
    codec, effect, effect_all, effect_map, effect_sync, effect_try_sync, service, stream, workflow,
};
use mandrill_effect as fx;
fn main() {
    struct Owned(String);
    let owned = Owned(String::from("Maya"));
    let ingress: fx::Effect<String, &'static str> = effect_try_sync! {
        once; body { Ok(owned.0) },
    };
    let mapped = effect_map! { program: ingress, map |name: String| { name.len() } };
    let _: fx::Effect<Vec<usize>, &'static str> = effect_all! {
        items { item!(mapped); for count in [2usize, 3] { item!(effect_sync! { body { count } }); } },
    };
    let _: fx::Stream<usize, &'static str, ()> = stream! {
        let menu: fx::Stream<usize, &'static str, ()> = stream! { iterable { [1usize].into_iter() }, };
        yield_from!(menu);
        yield 2usize;
    };
    let _: fx::PureCodec<usize, usize> = codec! {
        pure; decode |n: &usize| { Ok(*n) }, encode |n: &usize| { Ok(*n) },
    };
    let _: fx::Effect<usize, &'static str> = service! {
        handle: String::from("Maya"), on_error |error: &'static str| { error },
        run |_name: &String| {
            service! {
                future |name: &String| { let length = name.len(); async move { Ok::<_, &'static str>(length) } },
            }?
        },
    };
    let owned = std::sync::Mutex::new(Some(Owned(String::from("Maya"))));
    let _: fx::Effect<String, &'static str> = effect! {
        bind;
        program: effect_sync! { body { () }, },
        body: move |_| {
            let value = owned.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take();
            effect_try_sync! {
                once;
                body: move || value.map(|value| value.0).ok_or("consumed"),
            }
        },
    };
    let source: fx::Effect<String, &'static str> = effect_sync! { body { String::from("Maya") }, };
    let _: fx::Effect<usize, &'static str> = effect! {
        bind;
        body |name| { effect_sync! { once; body: move || name.len(), } },
        program: source,
    };
    let _: fx::workflow::Durable<String, &'static str, ()> = workflow! {
        result; value: Ok(String::from("owned workflow value")),
    };
}
