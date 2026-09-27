//! Completion-driven native work. Work owns arbitrary input/results and exposes
//! start(token, done, context), finish(env), deinit(), and a nullable signal value.
//! Accepted start returns 0 and invokes done exactly once after native quiescence;
//! rejection returns nonzero without invoking done. done may precede start return.
const std = @import("std");
const napi = @import("runtime.zig");
const c = napi.c;
const cancel = @import("native_execution").cancellation;
const allocator = std.heap.c_allocator;

pub fn Bindings(comptime Work: type, comptime prepare: fn (c.napi_env, c.napi_callback_info) anyerror!Work, comptime name: [:0]const u8, comptime capacity: usize) type {
    if (c.NAPI_VERSION < 5) @compileError("Native async bindings use napi_add_finalizer and require Node-API 5 or newer");
    if (capacity == 0) @compileError("Native async admission capacity must be positive");
    return struct {
        const Self = @This();
        var active: std.atomic.Value(usize) = .init(0);
        work: Work,
        references: std.atomic.Value(usize) = .init(1),
        token: *const cancel.Token,
        deferred: c.napi_deferred = null,
        callback_ref: c.napi_ref = null,
        keep_alive_ref: c.napi_ref = null,
        tsfn: c.napi_threadsafe_function = null,
        closing: std.atomic.Value(bool) = .init(false),
        cleanup_registered: bool = false,
        status: u32 = 0,
        signal_ref: c.napi_ref = null,
        listener_ref: c.napi_ref = null,

        // The listener can outlive this operation if the signal retains it.
        // Its own reference prevents any callback from using freed native data.
        const Listener = struct {
            token: *const cancel.Token,
            fn call(env: c.napi_env, info: c.napi_callback_info) callconv(.c) c.napi_value {
                var data: ?*anyopaque = null;
                if (c.napi_get_cb_info(env, info, null, null, null, &data) != c.napi_ok) return null;
                const self: *@This() = @ptrCast(@alignCast(data.?));
                _ = cancel.native_cancel_request(self.token);
                var value: c.napi_value = null;
                _ = c.napi_get_undefined(env, &value);
                return value;
            }
            fn finalize(_: c.napi_env, data: ?*anyopaque, _: ?*anyopaque) callconv(.c) void {
                const self: *@This() = @ptrCast(@alignCast(data.?));
                cancel.native_cancel_release(self.token);
                allocator.destroy(self);
            }
        };

        pub fn promise(env: c.napi_env, info: c.napi_callback_info) callconv(.c) c.napi_value {
            return submit(env, info, null, null);
        }

        /// Native error-first callback delivery, sharing the same asynchronous
        /// cancellation and lifetime contract as promise(). No Promise adapter.
        /// The optional token is independently retained for this operation.
        pub fn callback(env: c.napi_env, info: c.napi_callback_info, completion: c.napi_value, token: ?*const cancel.Token) c.napi_value {
            if (completion == null) {
                napi.throwTypeError(env, "native callback is required");
                return null;
            }
            return submit(env, info, completion, token);
        }

        fn submit(env: c.napi_env, info: c.napi_callback_info, completion: c.napi_value, token: ?*const cancel.Token) c.napi_value {
            var count = active.load(.monotonic);
            while (true) {
                if (count >= capacity) {
                    if (completion != null) {
                        invokeCallback(env, completion, .{ .failure = "native async capacity reached" });
                        return napi.undefinedValue(env);
                    }
                    var deferred: c.napi_deferred = null;
                    var value: c.napi_value = null;
                    if (c.napi_create_promise(env, &deferred, &value) != c.napi_ok) return null;
                    napi.rejectPromise(env, deferred, "native async capacity reached");
                    return value;
                }
                count = active.cmpxchgWeak(count, count + 1, .acq_rel, .monotonic) orelse break;
            }
            var work = prepare(env, info) catch |err| {
                _ = active.fetchSub(1, .release);
                if (completion != null) {
                    callbackFailure(env, completion, err);
                    return napi.undefinedValue(env);
                }
                if (err != error.PendingException) napi.throwTypeError(env, @errorName(err));
                return null;
            };
            const self = allocator.create(Self) catch {
                work.deinit();
                _ = active.fetchSub(1, .release);
                if (completion != null) {
                    invokeCallback(env, completion, .{ .failure = "native allocation failed" });
                    return napi.undefinedValue(env);
                }
                napi.throwError(env, "native allocation failed");
                return null;
            };
            const owned_token = token orelse cancel.native_cancel_create();
            if (token != null) cancel.native_cancel_retain(owned_token);
            self.* = .{ .work = work, .token = owned_token };
            var value: c.napi_value = null;
            self.initialize(env, &value, completion) catch |err| {
                if (self.tsfn != null) {
                    _ = c.napi_release_threadsafe_function(self.tsfn, c.napi_tsfn_release);
                } else {
                    if (self.callback_ref != null) _ = c.napi_delete_reference(env, self.callback_ref);
                    self.release();
                }
                if (completion != null) {
                    callbackFailure(env, completion, err);
                    return napi.undefinedValue(env);
                }
                return null;
            };
            _ = self.references.fetchAdd(1, .monotonic); // native callback ownership
            const status = if (cancel.native_cancel_requested(self.token)) @as(u32, 4) else self.work.start(self.token, done, self);
            // Accepted work may have called done already. Do not access self.
            if (status != 0) done(self, status);
            return value;
        }

        fn initialize(self: *Self, env: c.napi_env, value: *c.napi_value, completion: c.napi_value) !void {
            if (completion == null) {
                try napi.check(env, c.napi_create_promise(env, &self.deferred, value));
            } else {
                var kind: c.napi_valuetype = undefined;
                try napi.check(env, c.napi_typeof(env, completion, &kind));
                if (kind != c.napi_function) return error.InvalidCallback;
                try napi.check(env, c.napi_create_reference(env, completion, 1, &self.callback_ref));
                value.* = napi.undefinedValue(env);
            }
            const resource = try napi.makeString(env, name);
            // One terminal event, one reserved slot. No producer ever waits for JS.
            try napi.check(env, c.napi_create_threadsafe_function(env, null, null, resource, 1, 1, self, finalize, self, deliver, &self.tsfn));
            try napi.check(env, c.napi_add_env_cleanup_hook(env, cleanup, self));
            self.cleanup_registered = true;
            if (@hasField(Work, "keep_alive")) {
                try napi.check(env, c.napi_create_reference(env, self.work.keep_alive, 1, &self.keep_alive_ref));
            }
            if (self.work.signal == null) return;
            const signal = self.work.signal;
            try napi.check(env, c.napi_create_reference(env, signal, 1, &self.signal_ref));
            const listener = try allocator.create(Listener);
            listener.* = .{ .token = self.token };
            cancel.native_cancel_retain(self.token);
            var function: c.napi_value = null;
            if (c.napi_create_function(env, "nativeAbort", c.NAPI_AUTO_LENGTH, Listener.call, listener, &function) != c.napi_ok) {
                Listener.finalize(env, listener, null);
                return error.NodeApiFailure;
            }
            if (c.napi_add_finalizer(env, function, listener, Listener.finalize, null, null) != c.napi_ok) {
                Listener.finalize(env, listener, null);
                return error.NodeApiFailure;
            }
            try napi.check(env, c.napi_create_reference(env, function, 1, &self.listener_ref));
            const add = try napi.namedProperty(env, signal, "addEventListener");
            const args = [_]c.napi_value{ try napi.makeString(env, "abort"), function };
            try napi.check(env, c.napi_call_function(env, signal, add, args.len, &args, null));
            var aborted = false;
            try napi.check(env, c.napi_get_value_bool(env, try napi.namedProperty(env, signal, "aborted"), &aborted));
            if (aborted) _ = cancel.native_cancel_request(self.token);
        }

        fn done(data: ?*anyopaque, status: u32) callconv(.c) void {
            const self: *Self = @ptrCast(@alignCast(data.?));
            self.status = status;
            // The native callback owns the TSFN's producer reference and a
            // separate native reference. Environment teardown can discard JS
            // delivery without freeing Work still used by native code.
            if (!self.closing.load(.acquire)) {
                if (c.napi_call_threadsafe_function(self.tsfn, null, c.napi_tsfn_nonblocking) == c.napi_closing) {
                    self.release();
                    return;
                }
            }
            _ = c.napi_release_threadsafe_function(self.tsfn, c.napi_tsfn_release);
            self.release();
        }

        fn cleanup(data: ?*anyopaque) callconv(.c) void {
            const self: *Self = @ptrCast(@alignCast(data.?));
            self.closing.store(true, .release);
            self.cleanup_registered = false;
            _ = cancel.native_cancel_request(self.token);
        }

        fn deliver(env: c.napi_env, _: c.napi_value, context: ?*anyopaque, _: ?*anyopaque) callconv(.c) void {
            const self: *Self = @ptrCast(@alignCast(context.?));
            if (env == null or self.closing.load(.acquire)) return;
            const cancelled = cancel.native_cancel_finish(self.token);
            if (self.callback_ref != null) {
                var completion: c.napi_value = null;
                if (c.napi_get_reference_value(env, self.callback_ref, &completion) != c.napi_ok) return;
                if (cancelled or self.status == 4) {
                    invokeCallback(env, completion, .{ .failure = "native operation cancelled" });
                } else if (self.status != 0) {
                    invokeCallback(env, completion, .{ .failure = name });
                } else {
                    const value = self.work.finish(env) catch |err| {
                        callbackFailure(env, completion, err);
                        return;
                    };
                    invokeCallback(env, completion, .{ .success = value });
                }
                return;
            }
            if (cancelled or self.status == 4) {
                var signal: c.napi_value = null;
                if (self.signal_ref != null and c.napi_get_reference_value(env, self.signal_ref, &signal) == c.napi_ok) {
                    const reason = napi.namedProperty(env, signal, "reason") catch return;
                    _ = c.napi_reject_deferred(env, self.deferred, reason);
                } else napi.rejectPromise(env, self.deferred, "native operation cancelled");
            } else if (self.status != 0) {
                napi.rejectPromise(env, self.deferred, name);
            } else {
                const value = self.work.finish(env) catch |err| {
                    if (err == error.PendingException) {
                        var exception: c.napi_value = null;
                        if (c.napi_get_and_clear_last_exception(env, &exception) == c.napi_ok)
                            _ = c.napi_reject_deferred(env, self.deferred, exception);
                    } else napi.rejectPromise(env, self.deferred, "native result conversion failed");
                    return;
                };
                _ = c.napi_resolve_deferred(env, self.deferred, value);
            }
        }

        fn finalize(env: c.napi_env, data: ?*anyopaque, _: ?*anyopaque) callconv(.c) void {
            const self: *Self = @ptrCast(@alignCast(data.?));
            if (env != null) {
                if (!self.closing.load(.acquire) and self.signal_ref != null and self.listener_ref != null) {
                    var signal: c.napi_value = null;
                    var listener: c.napi_value = null;
                    if (c.napi_get_reference_value(env, self.signal_ref, &signal) == c.napi_ok and c.napi_get_reference_value(env, self.listener_ref, &listener) == c.napi_ok) {
                        const remove = napi.namedProperty(env, signal, "removeEventListener") catch null;
                        const event = napi.makeString(env, "abort") catch null;
                        if (remove != null and event != null) {
                            const args = [_]c.napi_value{ event, listener };
                            _ = c.napi_call_function(env, signal, remove, args.len, &args, null);
                        }
                    }
                }
                if (self.keep_alive_ref != null) _ = c.napi_delete_reference(env, self.keep_alive_ref);
                if (self.callback_ref != null) _ = c.napi_delete_reference(env, self.callback_ref);
                if (self.listener_ref != null) _ = c.napi_delete_reference(env, self.listener_ref);
                if (self.signal_ref != null) _ = c.napi_delete_reference(env, self.signal_ref);
            }
            self.closing.store(true, .release);
            _ = cancel.native_cancel_request(self.token);
            if (env != null and self.cleanup_registered) {
                _ = c.napi_remove_env_cleanup_hook(env, cleanup, self);
                self.cleanup_registered = false;
            }
            self.release();
        }

        fn release(self: *Self) void {
            if (self.references.fetchSub(1, .acq_rel) == 1) self.destroy();
        }

        fn destroy(self: *Self) void {
            _ = cancel.native_cancel_finish(self.token);
            self.work.deinit();
            cancel.native_cancel_release(self.token);
            allocator.destroy(self);
            _ = active.fetchSub(1, .release);
        }
    };
}

const CallbackResult = union(enum) { success: c.napi_value, failure: [:0]const u8, exception: c.napi_value };
fn callbackFailure(env: c.napi_env, completion: c.napi_value, err: anyerror) void {
    if (err == error.PendingException) {
        var exception: c.napi_value = null;
        if (c.napi_get_and_clear_last_exception(env, &exception) == c.napi_ok)
            invokeCallback(env, completion, .{ .exception = exception });
    } else invokeCallback(env, completion, .{ .failure = @errorName(err) });
}
fn invokeCallback(env: c.napi_env, completion: c.napi_value, result: CallbackResult) void {
    var args: [2]c.napi_value = undefined;
    _ = c.napi_get_null(env, &args[0]);
    args[1] = napi.undefinedValue(env);
    switch (result) {
        .success => |value| args[1] = value,
        .exception => |value| args[0] = value,
        .failure => |message| {
            const text = napi.makeString(env, message) catch return;
            if (c.napi_create_error(env, null, text, &args[0]) != c.napi_ok) return;
        },
    }
    const receiver = napi.undefinedValue(env);
    _ = c.napi_call_function(env, receiver, completion, args.len, &args, null);
}
