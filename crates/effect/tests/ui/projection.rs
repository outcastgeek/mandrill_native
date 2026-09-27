use mandrill_effect::Effect;

struct Config { port: u16 }
struct Application;

fn main() {
    let program: Effect<u16, (), Config> = Effect::service(|config: &Config| config.port);
    let _wrong = program.provide_some(|_: &Application| String::from("wrong environment"));
}
