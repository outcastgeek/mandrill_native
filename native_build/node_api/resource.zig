//! Native-owned asynchronous resources. Native supplies Owner, Input, open and
//! close; all JavaScript lifecycle machinery stays here. close must consume a
//! live Owner and complete exactly once without admission failure or blocking.
//! Native callbacks may run before open/close returns and on foreign threads.
const std = @import("std");
const napi = @import("runtime.zig");
const c = napi.c;
const allocator = std.heap.c_allocator;

pub fn Resource(comptime Native: type, comptime tag: c.napi_type_tag) type {
    if (c.NAPI_VERSION < 8) @compileError("Native resource type tags require Node-API 8 or newer");
    return opaque {
        const ResourceHandle = @This();
        pub fn open(env: c.napi_env, input: Native.Input) !c.napi_value {
            return State.open(env, input);
        }
        pub fn from(env: c.napi_env, object: c.napi_value) !*ResourceHandle {
            return @ptrCast(try State.from(env, object));
        }
        pub fn borrow(self: *ResourceHandle) !*Native.Owner {
            const state: *State = @ptrCast(@alignCast(self));
            return state.borrow();
        }
        pub fn close(self: *ResourceHandle, env: c.napi_env) !c.napi_value {
            const state: *State = @ptrCast(@alignCast(self));
            return state.close(env);
        }
        const State = struct {
            const Self = @This();
            pub const OpenResult = union(enum) { ready: *Native.Owner, failed };
            const Phase = enum(u8) { opening, abandon_open, ready, closing, closed };
            const Event = enum { opened, closed };
            const opened_event: Event = .opened;
            const closed_event: Event = .closed;
            const type_tag = tag;
            phase: std.atomic.Value(Phase) = .init(.opening),
            references: std.atomic.Value(usize) = .init(1), // TSFN; wrapper/native callbacks add ownership
            native_callbacks: std.atomic.Value(usize) = .init(0),
            owner: ?*Native.Owner = null,
            open_failed: bool = false,
            close_status: u32 = 0,
            function: c.napi_threadsafe_function = null,
            object_ref: c.napi_ref = null,
            opening: c.napi_deferred = null,
            closing: c.napi_deferred = null,
            shutting_down: std.atomic.Value(bool) = .init(false),
            delivery_released: std.atomic.Value(bool) = .init(false),
            cleanup_registered: bool = false,

            pub fn open(env: c.napi_env, input: Native.Input) !c.napi_value {
                const self = try allocator.create(Self);
                self.* = .{};
                var promise: c.napi_value = null;
                self.initialize(env, &promise) catch |err| {
                    self.phase.store(.closed, .release);
                    if (self.function != null) {
                        self.delivery_released.store(true, .release);
                        _ = c.napi_release_threadsafe_function(self.function, c.napi_tsfn_release);
                    } else self.release();
                    return err;
                };
                self.retainNative();
                const status = Native.open(input, opened, self);
                if (status != 0) opened(self, .failed);
                return promise;
            }

            fn initialize(self: *Self, env: c.napi_env, promise: *c.napi_value) !void {
                try napi.check(env, c.napi_create_promise(env, &self.opening, promise));
                const name = try napi.makeString(env, "native resource lifecycle");
                // At most two events: open and close. Capacity is reserved up front.
                try napi.check(env, c.napi_create_threadsafe_function(env, null, null, name, 2, 1, self, finalized, self, deliver, &self.function));
                try napi.check(env, c.napi_add_env_cleanup_hook(env, cleanup, self));
                self.cleanup_registered = true;
                const object = try napi.object(env);
                try napi.check(env, c.napi_type_tag_object(env, object, &type_tag));
                try napi.check(env, c.napi_wrap(env, object, self, collected, null, null));
                _ = self.references.fetchAdd(1, .monotonic);
                try napi.check(env, c.napi_create_reference(env, object, 1, &self.object_ref));
            }

            /// Validate the object before interpreting native storage. The returned
            /// wrapper is borrowed within the current Node callback's handle scope.
            pub fn from(env: c.napi_env, object: c.napi_value) !*Self {
                var matches = false;
                try napi.check(env, c.napi_check_object_type_tag(env, object, &type_tag, &matches));
                if (!matches) return error.InvalidNativeResource;
                var data: ?*anyopaque = null;
                try napi.check(env, c.napi_unwrap(env, object, &data));
                return @ptrCast(@alignCast(data orelse return error.InvalidNativeResource));
            }

            /// Borrow only for immediate native submission, which must retain its
            /// own operation ownership before returning. Never store this Owner.
            pub fn borrow(self: *Self) !*Native.Owner {
                if (self.phase.load(.acquire) != .ready) return error.NativeResourceClosed;
                return self.owner.?;
            }

            pub fn close(self: *Self, env: c.napi_env) !c.napi_value {
                if (self.phase.load(.acquire) != .ready) return error.NativeResourceClosed;
                var promise: c.napi_value = null;
                try napi.check(env, c.napi_create_promise(env, &self.closing, &promise));
                try napi.check(env, c.napi_ref_threadsafe_function(env, self.function));
                self.requestClose();
                return promise;
            }

            fn opened(self: *Self, result: OpenResult) void {
                defer self.releaseNative();
                switch (result) {
                    .ready => |owner| self.owner = owner,
                    .failed => self.open_failed = true,
                }
                if (self.owner == null) {
                    self.phase.store(.closed, .release);
                    self.notify(&opened_event);
                    return;
                }
                if (self.phase.cmpxchgStrong(.opening, .ready, .acq_rel, .acquire) != null) {
                    self.phase.store(.closing, .release);
                    self.retainNative();
                    Native.close(self.owner.?, closed, self);
                    return;
                }
                self.notify(&opened_event);
            }

            fn requestClose(self: *Self) void {
                while (true) switch (self.phase.load(.acquire)) {
                    .opening => {
                        if (self.phase.cmpxchgStrong(.opening, .abandon_open, .acq_rel, .acquire) == null) return;
                    },
                    .ready => {
                        if (self.phase.cmpxchgStrong(.ready, .closing, .acq_rel, .acquire) == null) {
                            self.retainNative();
                            Native.close(self.owner.?, closed, self);
                            return;
                        }
                    },
                    .abandon_open, .closing, .closed => return,
                };
            }

            fn closed(self: *Self, status: u32) void {
                defer self.releaseNative();
                self.close_status = status;
                self.phase.store(.closed, .release);
                self.notify(&closed_event);
            }

            fn deliver(env: c.napi_env, _: c.napi_value, context: ?*anyopaque, data: ?*anyopaque) callconv(.c) void {
                const self: *Self = @ptrCast(@alignCast(context.?));
                if (env == null or self.shutting_down.load(.acquire)) return;
                const event: *const Event = @ptrCast(@alignCast(data.?));
                switch (event.*) {
                    .opened => {
                        if (self.open_failed) {
                            napi.rejectPromise(env, self.opening, "native resource open failed");
                        } else {
                            var object: c.napi_value = null;
                            if (c.napi_get_reference_value(env, self.object_ref, &object) == c.napi_ok)
                                _ = c.napi_resolve_deferred(env, self.opening, object);
                            _ = c.napi_unref_threadsafe_function(env, self.function);
                        }
                        self.releaseObject(env);
                    },
                    .closed => if (self.closing != null) {
                        if (self.close_status == 0) {
                            var value: c.napi_value = null;
                            _ = c.napi_get_undefined(env, &value);
                            _ = c.napi_resolve_deferred(env, self.closing, value);
                        } else napi.rejectPromise(env, self.closing, "native resource close failed");
                    },
                }
            }

            fn cleanup(data: ?*anyopaque) callconv(.c) void {
                const self: *Self = @ptrCast(@alignCast(data.?));
                self.shutting_down.store(true, .release);
                self.cleanup_registered = false;
                self.requestClose();
            }
            fn collected(_: c.napi_env, data: ?*anyopaque, _: ?*anyopaque) callconv(.c) void {
                const self: *Self = @ptrCast(@alignCast(data.?));
                self.requestClose();
                self.release();
            }
            fn finalized(env: c.napi_env, data: ?*anyopaque, _: ?*anyopaque) callconv(.c) void {
                const self: *Self = @ptrCast(@alignCast(data.?));
                if (env != null) self.releaseObject(env);
                self.shutting_down.store(true, .release);
                if (env != null and self.cleanup_registered) {
                    _ = c.napi_remove_env_cleanup_hook(env, cleanup, self);
                    self.cleanup_registered = false;
                }
                self.requestClose();
                self.release();
            }
            fn releaseObject(self: *Self, env: c.napi_env) void {
                if (self.object_ref != null) {
                    const reference = self.object_ref;
                    self.object_ref = null;
                    _ = c.napi_delete_reference(env, reference);
                }
            }
            fn retainNative(self: *Self) void {
                _ = self.references.fetchAdd(1, .monotonic);
                _ = self.native_callbacks.fetchAdd(1, .monotonic);
            }
            fn notify(self: *Self, event: *const Event) void {
                if (self.shutting_down.load(.acquire)) return;
                // Each notification owns a producer reference, in addition to
                // the resource's lifetime reference held until native close.
                if (c.napi_acquire_threadsafe_function(self.function) != c.napi_ok) return;
                if (c.napi_call_threadsafe_function(self.function, @constCast(event), c.napi_tsfn_nonblocking) == c.napi_closing) {
                    self.shutting_down.store(true, .release);
                    return;
                }
                _ = c.napi_release_threadsafe_function(self.function, c.napi_tsfn_release);
            }
            fn releaseNative(self: *Self) void {
                if (self.native_callbacks.fetchSub(1, .acq_rel) == 1 and self.phase.load(.acquire) == .closed) {
                    if (!self.delivery_released.swap(true, .acq_rel))
                        _ = c.napi_release_threadsafe_function(self.function, c.napi_tsfn_release);
                }
                self.release();
            }
            fn release(self: *Self) void {
                if (self.references.fetchSub(1, .acq_rel) == 1) {
                    allocator.destroy(self);
                }
            }
        };
    };
}
