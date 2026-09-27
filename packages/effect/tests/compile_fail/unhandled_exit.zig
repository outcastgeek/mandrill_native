const fx = @import("effect");
pub fn main() void {
    const result = fx.run(&{}, fx.Effect(u16, void, void).succeed(3));
    switch (result) {
        .success => {},
        .fail => {},
        .die => {},
        .finalization_failed => {},
    }
}
