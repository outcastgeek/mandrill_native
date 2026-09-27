const std = @import("std");
const fx = @import("effect");

const AppError = union(enum) { invalid_port: u16 };
const Env = struct { allocator: std.mem.Allocator, io: std.Io, port: u16 };
const Port = fx.Effect(u16, AppError, Env);
const Build = fx.EffectBuilder(AppError, Env);
// Application configuration chosen by this consumer, never a library default.
const configured_port: u16 = 8080;

fn readPort(env: *const Env) u16 {
    return env.port;
}

fn validatePort(port: u16) Port {
    return if (port == 0) Build.fail(u16, .{ .invalid_port = port }) else Build.succeed(port);
}

pub fn main(init: std.process.Init) void {
    const env: Env = .{ .allocator = init.gpa, .io = init.io, .port = configured_port };
    const program = comptime Build.bind(Port.service(readPort), validatePort);
    const exit = fx.run(&env, program);
    switch (exit) {
        .success => |port| std.debug.assert(port == configured_port),
        .fail => |problem| std.debug.print("Invalid port: {d}\n", .{problem.invalid_port}),
        .interrupted => std.debug.print("Application interrupted\n", .{}),
        .die => |defect| std.debug.print("Application defect: {s}\n", .{defect.message}),
        .finalization_failed => std.debug.print("Cleanup failed\n", .{}),
    }
}
