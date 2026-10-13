use fx::codec;
use mandrill_effect as fx;
fn main() {
    let _: fx::PureCodec<usize, usize> = codec! {
        pure; decode |n: &usize| { Ok(*n) },
    };
}
