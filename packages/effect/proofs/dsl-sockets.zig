const std = @import("std");
const fx = @import("effect");
const gates = @import("support/dsl_gate.zig");
const E = enum { expected, mapped };
const Mode = enum { success, failure, defect, interrupted, early_close, pending, late_cancel, acquisition_failure, invalid_limits, invalid_uri, receive_reply };
const Counts = struct {
    live: usize = 0,
    configurations: usize = 0,
    acquired: usize = 0,
    bodies: usize = 0,
    sent: usize = 0,
    disposed: usize = 0,
    aborted: usize = 0,
    closed: usize = 0,
    error_live: usize = 0,
    open: bool = true,
    mode: Mode = .success,
    cleanup_failure: bool = false,
    blocked_dispose: bool = false,
    dispose_started: std.Io.Event = .unset,
    dispose_proceed: std.Io.Event = .unset,
    token: *fx.Cancellation,
    gate: ?*gates.Gate(u32, E) = null,
};
const Env = struct { allocator: std.mem.Allocator, io: std.Io = std.testing.io, cancellation: *fx.Cancellation, async_engine: fx.Executor = undefined, websocket_transport: fx.WebSocket.Transport, counts: *Counts };
const Out = fx.Effect(u32, E, Env);
const Native = fx.Effect(fx.WebSocket.NativeSocket, fx.WebSocket.Error, Env);
const Owner = struct {
    pub const effect_owned_payload = true;
    counts: *Counts,
    pub fn deinit(self: *@This()) void {
        std.debug.assert(self.counts.acquired == self.counts.disposed);
        if (self.counts.gate) |gate| std.debug.assert(gate.joined.load(.acquire) and gate.destroyed.load(.acquire));
        self.counts.live -= 1;
    }
};
const Inputs = struct { owner: Owner, message: fx.OwnedSlice(u8) };
const Mock = struct {
    fn socket(counts: *Counts) fx.WebSocket.NativeSocket {
        return .{ .userdata = counts, .vtable = &.{ .send = send, .receive = receive, .close_output = closeOutput, .close = finish, .abort = abort, .dispose = dispose, .sub_protocol = protocol, .is_open = isOpen } };
    }
    fn borrow(data: *anyopaque) *Counts {
        const counts: *Counts = @ptrCast(@alignCast(data));
        std.debug.assert(counts.live == 1);
        return counts;
    }
    fn send(data: *anyopaque, _: fx.WebSocket.Kind, bytes: []const u8, _: ?*const fx.Cancellation) fx.Exit(void, fx.WebSocket.Error) {
        const counts = borrow(data);
        std.debug.assert(std.mem.eql(u8, bytes, "payload"));
        counts.sent += 1;
        return .{ .success = {} };
    }
    fn receive(data: *anyopaque, buffer: []u8, _: ?*const fx.Cancellation) fx.Exit(fx.WebSocket.Fragment, fx.WebSocket.Error) {
        const counts = borrow(data);
        if (counts.mode == .receive_reply) {
            @memcpy(buffer[0..5], "reply");
            return .{ .success = .{ .kind = .text, .count = 5, .end_of_message = true, .close = .{ .status = null, .description = null } } };
        }
        return .{ .success = .{ .kind = .close, .count = 0, .end_of_message = true, .close = .{ .status = null, .description = null } } };
    }
    fn closeOutput(data: *anyopaque, _: ?*const fx.Cancellation) fx.Exit(void, fx.WebSocket.Error) {
        return if (borrow(data).cleanup_failure) .{ .fail = .closed } else .{ .success = {} };
    }
    fn finish(data: *anyopaque, _: ?*const fx.Cancellation) fx.Exit(void, fx.WebSocket.Error) {
        const counts = borrow(data);
        counts.closed += 1;
        counts.open = false;
        return .{ .success = {} };
    }
    fn abort(data: *anyopaque) ?fx.Defect {
        const counts = borrow(data);
        counts.aborted += 1;
        counts.open = false;
        return if (counts.cleanup_failure) fx.Defect.init("abort failed") else null;
    }
    fn dispose(data: *anyopaque) fx.Exit(void, fx.WebSocket.Error) {
        const counts = borrow(data);
        if (counts.blocked_dispose) {
            counts.dispose_started.set(std.testing.io);
            counts.dispose_proceed.waitUncancelable(std.testing.io);
        }
        counts.disposed += 1;
        return if (counts.cleanup_failure) .{ .fail = .closed } else .{ .success = {} };
    }
    fn protocol(_: *anyopaque) ?[]const u8 {
        return "proof";
    }
    fn isOpen(data: *anyopaque) bool {
        return borrow(data).open;
    }
    fn acquire(counts: *Counts) fx.Exit(fx.WebSocket.NativeSocket, fx.WebSocket.Error) {
        if (counts.mode == .acquisition_failure) return .{ .fail = .closed };
        counts.acquired += 1;
        if (counts.mode == .late_cancel) counts.token.request();
        return .{ .success = socket(counts) };
    }
    fn connect(data: *anyopaque, options: fx.WebSocket.ConnectOptions, _: ?*const fx.Cancellation) fx.Exit(fx.WebSocket.NativeSocket, fx.WebSocket.Error) {
        const counts = borrow(data);
        if (!std.mem.eql(u8, options.uri, "ws://proof.example")) return .{ .die = fx.Defect.init("options were selected more than once") };
        return acquire(counts);
    }
};
const Roles = struct {
    fn baseLimits(counts: *Counts) fx.WebSocket.Limits {
        return .{ .maximum_message_bytes = if (counts.mode == .invalid_limits) 0 else 32, .receive_buffer_bytes = 8, .close_timeout = .fromSeconds(5) };
    }
    fn limits(inputs: *const Inputs, _: *const Env) fx.WebSocket.Limits {
        inputs.owner.counts.configurations += 1;
        return baseLimits(inputs.owner.counts);
    }
    fn options(inputs: *const Inputs, _: *const Env) fx.WebSocket.ConnectOptions {
        const counts = inputs.owner.counts;
        counts.configurations += 1;
        return .{ .uri = if (counts.mode == .invalid_uri) "invalid" else if (counts.configurations == 1) "ws://proof.example" else "ws://wrong.example", .headers = &.{}, .sub_protocols = &.{"proof"}, .limits = baseLimits(counts) };
    }
    fn acquire(inputs: *const Inputs, _: *const Env) Native {
        return Native.fromBorrowed(Inputs, inputs, struct {
            fn run(stored: *const Inputs, _: *const Env) fx.Exit(fx.WebSocket.NativeSocket, fx.WebSocket.Error) {
                return Mock.acquire(stored.owner.counts);
            }
        }.run);
    }
    fn map(_: fx.WebSocket.Error) E {
        return .mapped;
    }
    fn done(_: void, _: *const Env) Out {
        return Out.succeed(7);
    }
    fn body(inputs: anytype, connection: *fx.WebSocket.Connection, env: *const Env) Out {
        const counts = inputs.owner.counts;
        counts.bodies += 1;
        switch (counts.mode) {
            .failure => return Out.fail(.expected),
            .defect => return Out.die(fx.Defect.init("body defect")),
            .interrupted => return Out.interrupted(),
            .pending => return Out.fromAsync(struct {
                fn start(current: *const Env) fx.Async(u32, E).Start {
                    return current.counts.gate.?.start();
                }
            }.start),
            else => {},
        }
        if (counts.mode == .early_close) {
            var close = fx.EffectWebSocket(Env).close(connection);
            var mapped = close.mapErrorOwned(env, map);
            return mapped.flatMapOwned(env, done);
        }
        var send = fx.EffectWebSocket(Env).send(connection, .{ .text = inputs.message.items });
        var mapped = send.mapErrorOwned(env, map);
        return mapped.flatMapOwned(env, done);
    }
};
fn make(env: *const Env, comptime connecting: bool) Out {
    const bytes = env.allocator.dupe(u8, "payload") catch return Out.die(fx.Defect.out_of_memory);
    env.counts.live += 1;
    const inputs: Inputs = .{ .owner = .{ .counts = env.counts }, .message = .{ .items = bytes, .allocator = env.allocator } };
    return if (connecting) fx.dsl.withConnection(env, inputs, .{ .options = Roles.options, .body = Roles.body, .map_error = Roles.map }) else fx.dsl.withSocket(env, inputs, .{ .limits = Roles.limits, .acquire = Roles.acquire, .body = Roles.body, .map_error = Roles.map });
}
fn program(comptime connecting: bool) Out {
    return Out.deferEffect(struct {
        fn create(env: *const Env) Out {
            return make(env, connecting);
        }
    }.create);
}

test "FAC-06 COMP-03 named socket and connection roles preserve exact Effect outcomes native send and early close" {
    inline for (.{ false, true }) |connecting| {
        for ([_]Mode{ .success, .failure, .defect, .interrupted, .early_close, .late_cancel, .acquisition_failure, .invalid_limits }) |mode| {
            var token: fx.Cancellation = .{};
            var counts: Counts = .{ .mode = mode, .token = &token };
            const env: Env = .{ .allocator = std.testing.allocator, .cancellation = &token, .counts = &counts, .websocket_transport = .{ .userdata = &counts, .connect = Mock.connect } };
            var described = make(&env, connecting);
            comptime if (@TypeOf(described) != fx.Effect(u32, E, Env)) @compileError("socket facade must return exact Effect");
            try std.testing.expectEqual(@as(usize, 0), counts.configurations);
            try std.testing.expectEqual(@as(usize, 0), counts.acquired);
            var result = fx.run(&env, described);
            described.deinit();
            defer result.deinit();
            switch (mode) {
                .success, .early_close => try std.testing.expect(result == .success and result.success == 7),
                .failure => try std.testing.expect(result == .fail and result.fail == .expected),
                .defect => try std.testing.expect(result == .die),
                .interrupted, .late_cancel => try std.testing.expect(result == .interrupted),
                .acquisition_failure, .invalid_limits => try std.testing.expect(result == .fail and result.fail == .mapped),
                else => unreachable,
            }
            try std.testing.expectEqual(@as(usize, 1), counts.configurations);
            try std.testing.expectEqual(counts.acquired, counts.disposed);
            try std.testing.expectEqual(@as(usize, 0), counts.live);
            if (mode == .late_cancel or mode == .invalid_limits or mode == .acquisition_failure) try std.testing.expectEqual(@as(usize, 0), counts.bodies);
            if (mode == .success) try std.testing.expectEqual(@as(usize, 1), counts.sent);
            if (mode == .early_close) try std.testing.expectEqual(@as(usize, 1), counts.closed);
        }
    }
}

test "FAC-06 LIFE-07 every socket setup and body allocation failure releases acquired native resources and captures" {
    const Check = struct {
        fn run(allocator: std.mem.Allocator) !void {
            inline for (.{ false, true }) |connecting| {
                var token: fx.Cancellation = .{};
                var counts: Counts = .{ .token = &token };
                const env: Env = .{ .allocator = allocator, .cancellation = &token, .counts = &counts, .websocket_transport = .{ .userdata = &counts, .connect = Mock.connect } };
                var result = fx.run(&env, comptime program(connecting));
                defer result.deinit();
                try std.testing.expectEqual(counts.acquired, counts.disposed);
                try std.testing.expectEqual(@as(usize, 0), counts.live);
                if (result == .die and result.die.kind == .out_of_memory) return error.OutOfMemory;
                try std.testing.expect(result == .success and result.success == 7);
            }
        }
    };
    try std.testing.checkAllAllocationFailures(std.testing.allocator, Check.run, .{});
}

test "FAC-06 LIFE-04 socket cleanup retains mapped close abort disposal failures and the original body failure" {
    inline for (.{ false, true }) |connecting| inline for (.{ false, true }) |body_fails| {
        var token: fx.Cancellation = .{};
        var counts: Counts = .{ .token = &token, .cleanup_failure = true, .mode = if (body_fails) .failure else .success };
        const env: Env = .{ .allocator = std.testing.allocator, .cancellation = &token, .counts = &counts, .websocket_transport = .{ .userdata = &counts, .connect = Mock.connect } };
        var result = fx.run(&env, comptime program(connecting));
        defer result.deinit();
        try std.testing.expect(result == .finalization_failed);
        if (body_fails) try std.testing.expectEqual(E.expected, result.finalization_failed.primary.failed) else try std.testing.expect(result.finalization_failed.primary == .succeeded);
        var failures = result.finalization_failed.iterator();
        var count: usize = 0;
        while (failures.nextFailure()) |failure| {
            try std.testing.expectEqual(E.mapped, failure.release_failure);
            count += 1;
        }
        try std.testing.expectEqual(@as(usize, if (body_fails) 2 else 3), count);
        try std.testing.expectEqual(@as(usize, 1), counts.disposed);
        try std.testing.expectEqual(@as(usize, 0), counts.live);
    };
}

test "FAC-06 connection URI validation maps errors without admitting a native socket" {
    var token: fx.Cancellation = .{};
    var counts: Counts = .{ .token = &token, .mode = .invalid_uri };
    const env: Env = .{ .allocator = std.testing.allocator, .cancellation = &token, .counts = &counts, .websocket_transport = .{ .userdata = &counts, .connect = Mock.connect } };
    var result = fx.run(&env, comptime program(true));
    defer result.deinit();
    try std.testing.expect(result == .fail and result.fail == .mapped);
    try std.testing.expectEqual(@as(usize, 1), counts.configurations);
    try std.testing.expectEqual(@as(usize, 0), counts.acquired);
    try std.testing.expectEqual(@as(usize, 0), counts.bodies);
    try std.testing.expectEqual(@as(usize, 0), counts.live);
}

test "FAC-06 pending socket body joins before native disposal and generated capture destruction" {
    inline for (.{ false, true }) |connecting| inline for (.{ false, true }) |cancel| {
        var engine: fx.NativeIoExecutor = .{ .allocator = std.testing.allocator, .io = std.testing.io, .capabilities = .{ .independent = true, .cpu = false, .blocking = false }, .maximum_owned = 4 };
        defer engine.deinit();
        var token: fx.Cancellation = .{};
        var gate: gates.Gate(u32, E) = .{ .result = .{ .success = 7 } };
        var counts: Counts = .{ .token = &token, .mode = .pending, .gate = &gate };
        const env: Env = .{ .allocator = std.testing.allocator, .cancellation = &token, .async_engine = engine.executor(), .counts = &counts, .websocket_transport = .{ .userdata = &counts, .connect = Mock.connect } };
        var pending = fx.runAsync(&env, comptime program(connecting));
        defer pending.deinit();
        gate.started.waitUncancelable(env.io);
        if (cancel) pending.request() else gate.proceed.set(env.io);
        var result = pending.join();
        defer result.deinit();
        try std.testing.expect(if (cancel) result == .interrupted else result == .success);
        try std.testing.expectEqual(@as(usize, 1), counts.disposed);
        try std.testing.expectEqual(@as(usize, 0), counts.live);
    };
}

test "FAC-06 LIFE-12 delayed socket disposal keeps captures alive through late cancellation" {
    inline for (.{ false, true }) |connecting| {
        var engine: fx.NativeIoExecutor = .{ .allocator = std.testing.allocator, .io = std.testing.io, .capabilities = .{ .independent = true, .cpu = false, .blocking = false }, .maximum_owned = 4 };
        defer engine.deinit();
        var token: fx.Cancellation = .{};
        var counts: Counts = .{ .token = &token, .blocked_dispose = true };
        const env: Env = .{ .allocator = std.testing.allocator, .cancellation = &token, .async_engine = engine.executor(), .counts = &counts, .websocket_transport = .{ .userdata = &counts, .connect = Mock.connect } };
        var pending = fx.runAsync(&env, comptime program(connecting));
        defer pending.deinit();
        counts.dispose_started.waitUncancelable(env.io);
        try std.testing.expectEqual(@as(usize, 1), counts.live);
        pending.request();
        counts.dispose_proceed.set(env.io);
        var result = pending.join();
        defer result.deinit();
        try std.testing.expect(result == .interrupted);
        try std.testing.expectEqual(@as(usize, 1), counts.disposed);
        try std.testing.expectEqual(@as(usize, 0), counts.live);
    }
}

test "FAC-06 LIFE-11 owned incoming socket message survives connection frame and capture teardown until caller cleanup" {
    const Receive = fx.Effect(fx.WebSocket.Receive, E, Env);
    const Body = struct {
        fn run(_: *const Inputs, connection: *fx.WebSocket.Connection, env: *const Env) Receive {
            var received = fx.EffectWebSocket(Env).receive(connection);
            return received.mapErrorOwned(env, Roles.map);
        }
    };
    inline for (.{ false, true }) |connecting| {
        var allocator = std.testing.FailingAllocator.init(std.testing.allocator, .{});
        var token: fx.Cancellation = .{};
        var counts: Counts = .{ .token = &token, .mode = .receive_reply };
        const env: Env = .{ .allocator = allocator.allocator(), .cancellation = &token, .counts = &counts, .websocket_transport = .{ .userdata = &counts, .connect = Mock.connect } };
        const bytes = try env.allocator.dupe(u8, "payload");
        counts.live += 1;
        const inputs: Inputs = .{ .owner = .{ .counts = &counts }, .message = .{ .items = bytes, .allocator = env.allocator } };
        var described = if (connecting) fx.dsl.withConnection(&env, inputs, .{ .options = Roles.options, .body = Body.run, .map_error = Roles.map }) else fx.dsl.withSocket(&env, inputs, .{ .limits = Roles.limits, .acquire = Roles.acquire, .body = Body.run, .map_error = Roles.map });
        var result = fx.run(&env, described);
        described.deinit();
        try std.testing.expect(result == .success and result.success == .message and result.success.message == .text);
        try std.testing.expectEqual(@as(usize, 1), counts.disposed);
        try std.testing.expectEqual(@as(usize, 0), counts.live);
        const remaining = result.success.message.text.items;
        try std.testing.expectEqualStrings("reply", remaining);
        try std.testing.expectEqual(remaining.len, allocator.allocated_bytes - allocator.freed_bytes);
        const before = allocator.freed_bytes;
        const expected = remaining.len;
        result.deinit();
        try std.testing.expectEqual(expected, allocator.freed_bytes - before);
        try std.testing.expectEqual(allocator.allocated_bytes, allocator.freed_bytes);
    }
}

test "FAC-06 LIFE-04 captured error mapper returns owning errors that outlive socket frames and inputs" {
    const Failure = union(enum) {
        pub const effect_owned_payload = true;
        mapped: struct { bytes: fx.OwnedSlice(u8), live: *usize },
        allocation_failed,
        pub fn deinit(self: *@This()) void {
            if (self.* == .mapped) {
                self.mapped.bytes.deinit();
                self.mapped.live.* -= 1;
            }
        }
        pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
            if (self.* == .allocation_failed) return .allocation_failed;
            const bytes = try self.mapped.bytes.clone();
            self.mapped.live.* += 1;
            return .{ .mapped = .{ .bytes = bytes, .live = self.mapped.live } };
        }
    };
    const OwnedOut = fx.Effect(void, Failure, Env);
    const Body = struct {
        fn run(_: *const Inputs, _: *fx.WebSocket.Connection, _: *const Env) OwnedOut {
            return OwnedOut.succeed({});
        }
        fn map(inputs: *const Inputs, _: fx.WebSocket.Error, env: *const Env) Failure {
            std.debug.assert(inputs.owner.counts.live == 1);
            const bytes = env.allocator.dupe(u8, "socket failure") catch return .allocation_failed;
            env.counts.error_live += 1;
            return .{ .mapped = .{ .bytes = .{ .items = bytes, .allocator = env.allocator }, .live = &env.counts.error_live } };
        }
    };
    inline for (.{ false, true }) |connecting| for ([_]Mode{ .acquisition_failure, .success, .invalid_limits }) |mode| {
        var allocator = std.testing.FailingAllocator.init(std.testing.allocator, .{});
        var token: fx.Cancellation = .{};
        var counts: Counts = .{ .token = &token, .mode = mode, .cleanup_failure = mode == .success };
        const env: Env = .{ .allocator = allocator.allocator(), .cancellation = &token, .counts = &counts, .websocket_transport = .{ .userdata = &counts, .connect = Mock.connect } };
        const bytes = try env.allocator.dupe(u8, "payload");
        counts.live += 1;
        const inputs: Inputs = .{ .owner = .{ .counts = &counts }, .message = .{ .items = bytes, .allocator = env.allocator } };
        var described = if (connecting) fx.dsl.withConnection(&env, inputs, .{ .options = Roles.options, .body = Body.run, .map_error = Body.map }) else fx.dsl.withSocket(&env, inputs, .{ .limits = Roles.limits, .acquire = Roles.acquire, .body = Body.run, .map_error = Body.map });
        var result = fx.run(&env, described);
        described.deinit();
        try std.testing.expectEqual(@as(usize, 0), counts.live);
        try std.testing.expectEqual(counts.acquired, counts.disposed);
        if (mode == .success) {
            try std.testing.expect(result == .finalization_failed and result.finalization_failed.primary == .succeeded);
            var iterator = result.finalization_failed.iterator();
            var count: usize = 0;
            while (iterator.nextFailure()) |failure| {
                try std.testing.expect(failure == .release_failure and failure.release_failure == .mapped);
                try std.testing.expectEqualStrings("socket failure", failure.release_failure.mapped.bytes.items);
                count += 1;
            }
            try std.testing.expectEqual(@as(usize, 3), count);
        } else {
            try std.testing.expect(result == .fail and result.fail == .mapped);
            try std.testing.expectEqualStrings("socket failure", result.fail.mapped.bytes.items);
        }
        try std.testing.expectEqual(@as(usize, if (mode == .success) 3 else 1), counts.error_live);
        result.deinit();
        try std.testing.expectEqual(@as(usize, 0), counts.error_live);
        try std.testing.expectEqual(allocator.allocated_bytes, allocator.freed_bytes);
    };
}
