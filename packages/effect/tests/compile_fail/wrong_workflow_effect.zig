const std = @import("std");
const effect = @import("effect");

const Env = struct { allocator: std.mem.Allocator, io: std.Io };
const Builder = effect.WorkflowBuilder(void, Env);

fn next(value: u8) @TypeOf(Builder.succeed(@as(u8, 0))) {
    return Builder.succeed(value);
}

pub fn main() void {
    _ = Builder.bind(effect.Effect(u8, void, Env).succeed(1), next);
}
