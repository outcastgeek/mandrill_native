use fx::effect_future;
use mandrill_effect as fx;
fn main() {
    let _: fx::Effect<usize, ()> = effect_future! {
        body |one, two| { async { 1usize } },
    };
}
