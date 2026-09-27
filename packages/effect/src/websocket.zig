const diagnostics = @import("internal/diagnostics.zig");
const std = @import("std");
const outcomes = @import("outcome.zig");
const providers = @import("services.zig");
const Cancellation = @import("internal/runtime.zig").Cancellation;

pub const Operation = enum { connect, send, receive, close_output, close, abort, dispose, sub_protocol };
pub const Error = union(enum) {
    invalid_options: []const u8,
    closed,
    message_too_large: usize,
    invalid_message: []const u8,
    transport: struct { operation: Operation, cause: outcomes.Defect },
    close_timeout,
};
pub const Limits = struct { maximum_message_bytes: usize, receive_buffer_bytes: usize, close_timeout: std.Io.Duration };
pub const Message = union(enum) { text: []const u8, binary: []const u8 };
pub const Kind = enum { text, binary, close };
pub const CloseInfo = struct { status: ?u16, description: ?[]const u8 };
pub const Fragment = struct { kind: Kind, count: usize, end_of_message: bool, close: CloseInfo };
pub const OwnedMessage = union(enum) {
    pub const effect_owned_payload = true;
    text: outcomes.OwnedSlice(u8),
    binary: outcomes.OwnedSlice(u8),
    pub fn deinit(self: *@This()) void {
        switch (self.*) {
            .text, .binary => |*bytes| bytes.deinit(),
        }
        self.* = undefined;
    }
    pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
        return switch (self.*) {
            .text => |bytes| .{ .text = try bytes.clone() },
            .binary => |bytes| .{ .binary = try bytes.clone() },
        };
    }
};
pub const Receive = union(enum) {
    pub const effect_owned_payload = true;
    message: OwnedMessage,
    closed: struct { status: ?u16, description: ?outcomes.OwnedSlice(u8) },
    pub fn deinit(self: *@This()) void {
        switch (self.*) {
            .message => |*message| message.deinit(),
            .closed => |*info| if (info.description) |*description| description.deinit(),
        }
        self.* = undefined;
    }
    pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
        return switch (self.*) {
            .message => |message| .{ .message = try message.clone() },
            .closed => |info| .{ .closed = .{ .status = info.status, .description = if (info.description) |description| try description.clone() else null } },
        };
    }
};
pub const ConnectOptions = struct { uri: []const u8, headers: []const std.http.Header, sub_protocols: []const []const u8, limits: Limits };
pub const Transport = struct { userdata: *anyopaque, connect: *const fn (*anyopaque, ConnectOptions, ?*const Cancellation) outcomes.Exit(NativeSocket, Error) };

/// Providers adapt native sockets to the consumer's I/O runtime. All operations
/// terminate after token cancellation or abort; disposal releases native state.
pub const NativeSocket = struct {
    userdata: *anyopaque,
    vtable: *const VTable,
    pub const VTable = struct {
        send: *const fn (*anyopaque, Kind, []const u8, ?*const Cancellation) outcomes.Exit(void, Error),
        receive: *const fn (*anyopaque, []u8, ?*const Cancellation) outcomes.Exit(Fragment, Error),
        close_output: *const fn (*anyopaque, ?*const Cancellation) outcomes.Exit(void, Error),
        close: *const fn (*anyopaque, ?*const Cancellation) outcomes.Exit(void, Error),
        // Nonblocking and threadsafe: wake/abort native I/O, never join another
        // owner's future. An adapter failure remains a cleanup defect.
        abort: *const fn (*anyopaque) ?outcomes.Defect,
        dispose: *const fn (*anyopaque) outcomes.Exit(void, Error),
        sub_protocol: *const fn (*anyopaque) ?[]const u8,
        is_open: *const fn (*anyopaque) bool,
    };
};

pub fn validate(limits: Limits) outcomes.Exit(void, Error) {
    if (limits.maximum_message_bytes == 0 or limits.receive_buffer_bytes == 0 or limits.receive_buffer_bytes > limits.maximum_message_bytes) return .{ .fail = .{ .invalid_options = diagnostics.message_and_receive_buffer_limits_must_be_positive_buffer_must_fit_a } };
    if (limits.close_timeout.nanoseconds <= 0) return .{ .fail = .{ .invalid_options = diagnostics.close_timeout_must_be_positive } };
    return .{ .success = {} };
}
pub const Connection = struct {
    const Self = @This();
    const Lifetime = enum(u8) { open, closing, ended };
    allocator: std.mem.Allocator,
    io: std.Io,
    clock: providers.Clock,
    native: NativeSocket,
    limits: Limits,
    sending: std.Io.Mutex = .init,
    receiving: std.Io.Mutex = .init,
    closing: std.Io.Mutex = .init,
    gate: std.Io.Mutex = .init,
    drained: std.Io.Condition = .init,
    lifetime: std.atomic.Value(Lifetime) = .init(.open),
    abort_done: std.Io.Event = .unset,
    abort_failure: ?outcomes.Defect = null,
    abort_started: std.atomic.Value(bool) = .init(false),
    active: usize = 0,
    disposed: bool = false,

    pub fn acquire(env: anytype, native: NativeSocket, limits: Limits) outcomes.Exit(*Self, Error) {
        const valid = validate(limits);
        if (valid != .success) return outcomes.castFailure(*Self, valid);
        const self = env.allocator.create(Self) catch return .{ .die = outcomes.Defect.out_of_memory };
        self.* = .{ .allocator = env.allocator, .io = env.io, .clock = undefined, .native = native, .limits = limits };
        if (comptime @hasField(@typeInfo(@TypeOf(env)).pointer.child, "clock")) self.clock = env.clock else self.clock = providers.Clock.fromIo(&self.io);
        if (!native.vtable.is_open(native.userdata)) {
            self.allocator.destroy(self);
            return .{ .fail = .{ .invalid_options = diagnostics.acquired_websocket_must_be_open } };
        }
        return .{ .success = self };
    }
    fn admit(self: *Self) bool {
        self.gate.lockUncancelable(self.io);
        defer self.gate.unlock(self.io);
        if (self.disposed or self.lifetime.load(.acquire) != .open) return false;
        self.active += 1;
        return true;
    }
    fn finish(self: *Self) void {
        self.gate.lockUncancelable(self.io);
        self.active -= 1;
        self.drained.broadcast(self.io);
        self.gate.unlock(self.io);
    }
    fn cancel(data: ?*anyopaque) void {
        const self: *Self = @ptrCast(@alignCast(data.?));
        self.requestAbort();
    }
    fn requestAbort(self: *Self) void {
        self.lifetime.store(.ended, .release);
        if (self.abort_started.swap(true, .acq_rel)) return;
        self.abort_failure = self.native.vtable.abort(self.native.userdata);
        self.abort_done.set(self.io);
    }
    pub fn abort(self: *Self) outcomes.Exit(void, Error) {
        self.requestAbort();
        self.abort_done.waitUncancelable(self.io);
        return if (self.abort_failure) |defect| .{ .fail = .{ .transport = .{ .operation = .abort, .cause = defect } } } else .{ .success = {} };
    }
    pub fn sendToken(self: *Self, message: Message, token: ?*const Cancellation) outcomes.Exit(void, Error) {
        if (!self.admit()) return .{ .fail = .closed };
        defer self.finish();
        var registration: Cancellation.Registration = .{ .data = self, .forward = cancel };
        if (token) |origin| origin.register(&registration);
        defer registration.unregister();
        self.sending.lock(self.io) catch {
            self.requestAbort();
            return .interrupted;
        };
        defer self.sending.unlock(self.io);
        if (self.lifetime.load(.acquire) != .open) return .{ .fail = .closed };
        const bytes = switch (message) {
            .text, .binary => |value| value,
        };
        if (bytes.len > self.limits.maximum_message_bytes) return .{ .fail = .{ .message_too_large = self.limits.maximum_message_bytes } };
        if (message == .text and !std.unicode.utf8ValidateSlice(bytes)) return .{ .fail = .{ .invalid_message = diagnostics.invalid_utf_8_text } };
        const snapshot = self.allocator.dupe(u8, bytes) catch return .{ .die = outcomes.Defect.out_of_memory };
        defer self.allocator.free(snapshot);
        const result = self.native.vtable.send(self.native.userdata, if (message == .text) .text else .binary, snapshot, token);
        if (result != .success) self.requestAbort();
        return result;
    }
    pub fn receiveToken(self: *Self, token: ?*const Cancellation) outcomes.Exit(Receive, Error) {
        if (!self.admit()) return .{ .fail = .closed };
        defer self.finish();
        var registration: Cancellation.Registration = .{ .data = self, .forward = cancel };
        if (token) |origin| origin.register(&registration);
        defer registration.unregister();
        self.receiving.lock(self.io) catch {
            self.requestAbort();
            return .interrupted;
        };
        defer self.receiving.unlock(self.io);
        if (self.lifetime.load(.acquire) != .open) return .{ .fail = .closed };
        const buffer = self.allocator.alloc(u8, self.limits.receive_buffer_bytes) catch return .{ .die = outcomes.Defect.out_of_memory };
        defer self.allocator.free(buffer);
        var output: std.ArrayList(u8) = .empty;
        defer output.deinit(self.allocator);
        var kind: ?Kind = null;
        var successful = false;
        defer if (!successful) self.requestAbort();
        while (true) {
            const received = self.native.vtable.receive(self.native.userdata, buffer, token);
            if (received != .success) return outcomes.castFailure(Receive, received);
            const frame = received.success;
            if (frame.count > buffer.len) return .{ .die = .{ .kind = .invalid_configuration, .message = diagnostics.websocket_provider_returned_a_fragment_larger_than_its_receive_buffer } };
            if (frame.kind == .close) {
                _ = self.lifetime.cmpxchgStrong(.open, .closing, .acq_rel, .acquire);
                const description: ?outcomes.OwnedSlice(u8) = if (frame.close.description) |text| .{ .items = self.allocator.dupe(u8, text) catch return .{ .die = outcomes.Defect.out_of_memory }, .allocator = self.allocator } else null;
                successful = true;
                return .{ .success = .{ .closed = .{ .status = frame.close.status, .description = description } } };
            }
            if (kind != null and kind.? != frame.kind) return .{ .fail = .{ .invalid_message = diagnostics.message_type_changed_during_fragmentation } };
            if (frame.count > self.limits.maximum_message_bytes - output.items.len) return .{ .fail = .{ .message_too_large = self.limits.maximum_message_bytes } };
            output.appendSlice(self.allocator, buffer[0..frame.count]) catch return .{ .die = outcomes.Defect.out_of_memory };
            kind = frame.kind;
            if (!frame.end_of_message) continue;
            if (frame.kind == .text and !std.unicode.utf8ValidateSlice(output.items)) return .{ .fail = .{ .invalid_message = diagnostics.invalid_utf_8_text } };
            const bytes: outcomes.OwnedSlice(u8) = .{ .items = output.toOwnedSlice(self.allocator) catch return .{ .die = outcomes.Defect.out_of_memory }, .allocator = self.allocator };
            successful = true;
            return .{ .success = .{ .message = if (frame.kind == .text) .{ .text = bytes } else .{ .binary = bytes } } };
        }
    }
    pub fn subProtocol(self: *Self) outcomes.Exit(?[]const u8, Error) {
        if (!self.admit()) return .{ .fail = .closed };
        defer self.finish();
        return .{ .success = self.native.vtable.sub_protocol(self.native.userdata) };
    }

    /// Two locally owned native futures race a handshake and the supplied
    /// clock. Every path joins both; timeout aborts before draining I/O.
    fn retainHandshakeCleanup(primary: outcomes.Exit(void, Error), handshake: outcomes.Exit(void, Error)) outcomes.Exit(void, Error) {
        std.debug.assert(primary != .success and primary != .finalization_failed);
        if (handshake == .finalization_failed) {
            var source_primary = handshake.finalization_failed.primary;
            if (source_primary == .failed) outcomes.discardPayload(&source_primary.failed);
            // Timeout/interruption is the primary result; the losing handshake's
            // existing cleanup chain transfers directly without a new allocation.
            var details = handshake.finalization_failed;
            details.primary = primary.primary();
            return .{ .finalization_failed = details };
        }
        var discarded = handshake;
        discarded.deinit();
        return primary;
    }

    pub fn close(self: *Self) outcomes.Exit(void, Error) {
        self.closing.lock(self.io) catch {
            self.requestAbort();
            return .interrupted;
        };
        defer self.closing.unlock(self.io);
        if (self.lifetime.load(.acquire) == .ended) return .{ .success = {} };
        self.lifetime.store(.closing, .release);
        const Race = struct {
            connection: *Self,
            mutex: std.Io.Mutex = .init,
            event: std.Io.Event = .unset,
            winner: ?enum { handshake, timer } = null,
            handshake_result: outcomes.Exit(void, Error) = .interrupted,
            timer_result: providers.Clock.SleepResult = .interrupted,
            fn publish(race: *@This(), winner: @TypeOf(race.winner.?)) void {
                race.mutex.lockUncancelable(race.connection.io);
                if (race.winner == null) {
                    race.winner = winner;
                    race.event.set(race.connection.io);
                }
                race.mutex.unlock(race.connection.io);
            }
            fn handshake(race: *@This()) void {
                const connection = race.connection;
                connection.sending.lock(connection.io) catch {
                    race.publish(.handshake);
                    return;
                };
                race.handshake_result = connection.native.vtable.close_output(connection.native.userdata, null);
                connection.sending.unlock(connection.io);
                if (race.handshake_result == .success) {
                    connection.receiving.lock(connection.io) catch {
                        race.handshake_result = .interrupted;
                        race.publish(.handshake);
                        return;
                    };
                    race.handshake_result = connection.native.vtable.close(connection.native.userdata, null);
                    connection.receiving.unlock(connection.io);
                }
                race.publish(.handshake);
            }
            fn timer(race: *@This()) void {
                race.timer_result = race.connection.clock.sleep(race.connection.limits.close_timeout);
                race.publish(.timer);
            }
        };
        var race: Race = .{ .connection = self };
        var handshake = self.io.concurrent(Race.handshake, .{&race}) catch {
            self.requestAbort();
            return .{ .die = .{ .kind = .provider_unavailable, .message = diagnostics.websocket_close_requires_independent_handshake_progress } };
        };
        var timer = self.io.concurrent(Race.timer, .{&race}) catch {
            self.requestAbort();
            const protection = self.io.swapCancelProtection(.blocked);
            defer _ = self.io.swapCancelProtection(protection);
            handshake.cancel(self.io);
            return retainHandshakeCleanup(.{ .die = .{ .kind = .provider_unavailable, .message = diagnostics.websocket_close_requires_an_independent_timeout_clock } }, race.handshake_result);
        };
        var interrupted = false;
        race.event.wait(self.io) catch {
            interrupted = true;
        };
        const protection = self.io.swapCancelProtection(.blocked);
        defer _ = self.io.swapCancelProtection(protection);
        if (interrupted or race.winner == .timer) {
            self.requestAbort();
            handshake.cancel(self.io);
            timer.cancel(self.io);
            const primary: outcomes.Exit(void, Error) = if (interrupted) .interrupted else switch (race.timer_result) {
                .done => .{ .fail = .close_timeout },
                .interrupted => .interrupted,
                .defect => |defect| .{ .die = defect },
            };
            return retainHandshakeCleanup(primary, race.handshake_result);
        }
        handshake.await(self.io);
        timer.cancel(self.io);
        if (race.handshake_result == .success) self.lifetime.store(.ended, .release) else self.requestAbort();
        return race.handshake_result;
    }
    pub fn dispose(self: *Self) outcomes.Exit(void, Error) {
        const io = self.io;
        const protection = io.swapCancelProtection(.blocked);
        defer _ = io.swapCancelProtection(protection);
        self.requestAbort();
        self.abort_done.waitUncancelable(self.io);
        self.gate.lockUncancelable(self.io);
        while (self.active != 0) self.drained.waitUncancelable(self.io, &self.gate);
        self.disposed = true;
        self.gate.unlock(self.io);
        self.closing.lockUncancelable(self.io);
        self.sending.lockUncancelable(self.io);
        self.receiving.lockUncancelable(self.io);
        const result = self.native.vtable.dispose(self.native.userdata);
        self.receiving.unlock(self.io);
        self.sending.unlock(self.io);
        self.closing.unlock(self.io);
        self.allocator.destroy(self);
        return result;
    }
};
