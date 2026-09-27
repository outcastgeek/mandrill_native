//! Small error, export, and consumer-conversion helpers over raw Node-API.
//! Consumers retain full access to `c` and define their own data converters.

const std = @import("std");
const execution = @import("native_execution");
pub const c = @import("node_api_c").c;
pub const c_abi = @import("native_c_abi");

pub const Error = error{
    NodeApiFailure,
    InvalidArgument,
    OutOfRange,
    PendingException,
    OutOfMemory,
};

pub const Converter = struct {
    /// Create a converter for any consumer-owned native type. The shared layer
    /// has no registry or closed list of supported values.
    pub fn Type(comptime Native: type, comptime Context: type) type {
        return struct {
            context: Context,
            decode: *const fn (Context, c.napi_env, c.napi_value) anyerror!Native,
            encode: *const fn (Context, c.napi_env, Native) anyerror!c.napi_value,
        };
    }
};

pub const Function = struct {
    name: [:0]const u8,
    callback: c.napi_callback,
};

pub fn setFunctions(env: c.napi_env, exports: c.napi_value, functions: []const Function) Error!void {
    for (functions) |function| try setFunction(env, exports, function.name, function.callback);
}

/// Reusable Node-API registration entry points. Each addon instantiation reads
/// its own consumer-selected `NAPI_VERSION` from its distinct C import module.
pub fn Module(comptime register: fn (c.napi_env, c.napi_value) anyerror!void) type {
    return struct {
        pub fn apiVersion() callconv(.c) i32 {
            return @intCast(c.NAPI_VERSION);
        }

        pub fn initialize(env: c.napi_env, exports: c.napi_value) callconv(.c) c.napi_value {
            register(env, exports) catch return null;
            return exports;
        }
    };
}

pub fn selectedVersion(env: c.napi_env, info: c.napi_callback_info) callconv(.c) c.napi_value {
    _ = info;
    var result: c.napi_value = null;
    check(env, c.napi_create_uint32(env, c.NAPI_VERSION, &result)) catch return null;
    return result;
}

/// Runs consumer-owned synchronous work and converts ordinary native failures
/// into JavaScript exceptions. The context and result type remain entirely
/// consumer-defined.
pub fn runSync(
    comptime Context: type,
    env: c.napi_env,
    context: Context,
    comptime run: fn (c.napi_env, Context) anyerror!c.napi_value,
    failure_message: [:0]const u8,
) c.napi_value {
    return run(env, context) catch {
        throwError(env, failure_message);
        return null;
    };
}

/// Queues consumer-owned work on the JavaScript runtime's async-work facility.
/// Native execution, result encoding and cleanup are callbacks so this helper
/// supports arbitrary consumer-defined structures without a type registry.
pub fn queuePromise(
    comptime Context: type,
    env: c.napi_env,
    name: [:0]const u8,
    context: Context,
    comptime execute: fn (*Context) anyerror!void,
    comptime complete: fn (c.napi_env, *Context) anyerror!c.napi_value,
    comptime deinit: fn (c.napi_env, *Context) void,
    failure_message: [:0]const u8,
) c.napi_value {
    const State = struct {
        work: c.napi_async_work = null,
        deferred: c.napi_deferred = null,
        context: Context,
        failed: bool = false,
        failure_message: [:0]const u8,

        fn destroy(state: *@This(), destroy_env: c.napi_env) void {
            deinit(destroy_env, &state.context);
            std.heap.c_allocator.destroy(state);
        }

        fn executeCallback(callback_env: c.napi_env, data: ?*anyopaque) callconv(.c) void {
            _ = callback_env;
            const state: *@This() = @ptrCast(@alignCast(data.?));
            execute(&state.context) catch {
                state.failed = true;
            };
        }

        fn completeCallback(callback_env: c.napi_env, status: c.napi_status, data: ?*anyopaque) callconv(.c) void {
            const state: *@This() = @ptrCast(@alignCast(data.?));
            defer state.destroy(callback_env);
            defer _ = c.napi_delete_async_work(callback_env, state.work);
            if (status != c.napi_ok or state.failed)
                return rejectPromise(callback_env, state.deferred, state.failure_message);
            const value = complete(callback_env, &state.context) catch
                return rejectPromise(callback_env, state.deferred, state.failure_message);
            _ = c.napi_resolve_deferred(callback_env, state.deferred, value);
        }
    };

    var owned_context = context;
    const state = std.heap.c_allocator.create(State) catch {
        deinit(env, &owned_context);
        throwError(env, "native allocation failed");
        return null;
    };
    state.* = .{ .context = owned_context, .failure_message = failure_message };
    const resource_name = makeString(env, name) catch {
        state.destroy(env);
        return null;
    };
    check(env, c.napi_create_async_work(env, null, resource_name, State.executeCallback, State.completeCallback, state, &state.work)) catch {
        state.destroy(env);
        return null;
    };
    var promise: c.napi_value = null;
    check(env, c.napi_create_promise(env, &state.deferred, &promise)) catch {
        _ = c.napi_delete_async_work(env, state.work);
        state.destroy(env);
        return null;
    };
    check(env, c.napi_queue_async_work(env, state.work)) catch {
        _ = c.napi_delete_async_work(env, state.work);
        rejectPromise(env, state.deferred, "could not queue native work");
        state.destroy(env);
        return promise;
    };
    return promise;
}

/// Promise binding for consumer work types that implement `executeWorker`,
/// `finish`, and `deinit`. This keeps Node-API callback trampolines reusable.
pub fn queueWorkPromise(
    comptime Work: type,
    env: c.napi_env,
    name: [:0]const u8,
    work: Work,
    failure_message: [:0]const u8,
) c.napi_value {
    const Binding = struct {
        fn execute(context: *Work) anyerror!void {
            context.executeWorker();
        }

        fn complete(complete_env: c.napi_env, context: *Work) anyerror!c.napi_value {
            return context.finish(complete_env);
        }

        fn deinit(_: c.napi_env, context: *Work) void {
            context.deinit();
        }
    };
    return queuePromise(Work, env, name, work, Binding.execute, Binding.complete, Binding.deinit, failure_message);
}

/// Native sync/Promise callbacks sharing a consumer's arbitrary argument decoder
/// and owned Work lifecycle. Preparation retains ownership on failure; on success
/// these callbacks take ownership. No schema or argument-count policy is imposed.
pub fn WorkBindings(
    comptime Work: type,
    comptime prepare: fn (c.napi_env, c.napi_callback_info) anyerror!Work,
    comptime name: [:0]const u8,
) type {
    return struct {
        pub fn sync(env: c.napi_env, info: c.napi_callback_info) callconv(.c) c.napi_value {
            var work = prepare(env, info) catch |err| return preparationFailure(env, err);
            defer work.deinit();
            work.executeWorker();
            return work.finish(env) catch |err| failure(env, err);
        }

        pub fn promise(env: c.napi_env, info: c.napi_callback_info) callconv(.c) c.napi_value {
            const work = prepare(env, info) catch |err| return preparationFailure(env, err);
            return queueWorkPromise(Work, env, name, work, name);
        }

        fn failure(env: c.napi_env, err: anyerror) c.napi_value {
            if (err != error.PendingException) throwError(env, @errorName(err));
            return null;
        }

        fn preparationFailure(env: c.napi_env, err: anyerror) c.napi_value {
            if (err != error.PendingException) throwTypeError(env, @errorName(err));
            return null;
        }
    };
}

pub fn rejectPromise(env: c.napi_env, deferred: c.napi_deferred, message: [:0]const u8) void {
    const text = makeString(env, message) catch return;
    var failure: c.napi_value = null;
    if (c.napi_create_error(env, null, text, &failure) == c.napi_ok)
        _ = c.napi_reject_deferred(env, deferred, failure);
}

/// Queues native work and delivers one standard error-first callback. Native
/// context, execution, encoding, and cleanup remain consumer-defined.
pub fn queueCallback(
    comptime Context: type,
    env: c.napi_env,
    name: [:0]const u8,
    callback: c.napi_value,
    context: Context,
    comptime execute: fn (*Context) anyerror!void,
    comptime encode: fn (c.napi_env, *Context) anyerror!c.napi_value,
    comptime deinit: fn (*Context) void,
    failure_message: [:0]const u8,
) c.napi_value {
    const State = struct {
        work: c.napi_async_work = null,
        callback_ref: c.napi_ref = null,
        context: Context,
        failed: bool = false,
        failure_message: [:0]const u8,

        fn destroy(state: *@This()) void {
            deinit(&state.context);
            std.heap.c_allocator.destroy(state);
        }

        fn executeWork(callback_env: c.napi_env, data: ?*anyopaque) callconv(.c) void {
            _ = callback_env;
            const state: *@This() = @ptrCast(@alignCast(data.?));
            execute(&state.context) catch {
                state.failed = true;
            };
        }

        fn completeWork(callback_env: c.napi_env, status: c.napi_status, data: ?*anyopaque) callconv(.c) void {
            const state: *@This() = @ptrCast(@alignCast(data.?));
            defer state.destroy();
            defer _ = c.napi_delete_reference(callback_env, state.callback_ref);
            defer _ = c.napi_delete_async_work(callback_env, state.work);
            var function: c.napi_value = null;
            if (c.napi_get_reference_value(callback_env, state.callback_ref, &function) != c.napi_ok) return;
            if (status != c.napi_ok or state.failed)
                return callErrorFirst(callback_env, function, null, state.failure_message);
            const value = encode(callback_env, &state.context) catch
                return callErrorFirst(callback_env, function, null, state.failure_message);
            callErrorFirst(callback_env, function, value, state.failure_message);
        }
    };

    var owned_context = context;
    const state = std.heap.c_allocator.create(State) catch {
        deinit(&owned_context);
        throwError(env, "native allocation failed");
        return null;
    };
    state.* = .{ .context = owned_context, .failure_message = failure_message };
    check(env, c.napi_create_reference(env, callback, 1, &state.callback_ref)) catch {
        state.destroy();
        return null;
    };
    const resource_name = makeString(env, name) catch {
        _ = c.napi_delete_reference(env, state.callback_ref);
        state.destroy();
        return null;
    };
    check(env, c.napi_create_async_work(env, null, resource_name, State.executeWork, State.completeWork, state, &state.work)) catch {
        _ = c.napi_delete_reference(env, state.callback_ref);
        state.destroy();
        return null;
    };
    check(env, c.napi_queue_async_work(env, state.work)) catch {
        _ = c.napi_delete_async_work(env, state.work);
        _ = c.napi_delete_reference(env, state.callback_ref);
        state.destroy();
        throwError(env, "could not queue native callback work");
        return null;
    };
    return undefinedValue(env);
}

/// Runs native work on the shared `std.Io` executor and completes through a
/// Node-API thread-safe function. No JavaScript value crosses the native worker.
pub fn queueThreadsafeCallback(
    comptime Context: type,
    env: c.napi_env,
    name: [:0]const u8,
    callback: c.napi_value,
    context: Context,
    comptime execute: fn (*Context) anyerror!void,
    comptime encode: fn (c.napi_env, *Context) anyerror!c.napi_value,
    comptime deinit: fn (*Context) void,
    failure_message: [:0]const u8,
) c.napi_value {
    comptime if (c.NAPI_VERSION < 4)
        @compileError("thread-safe native completion requires consumer-selected Node-API version 4 or newer");
    const State = struct {
        function: c.napi_threadsafe_function = null,
        context: Context,
        failed: bool = false,
        failure_message: [:0]const u8,

        fn worker(state: *@This()) void {
            execute(&state.context) catch {
                state.failed = true;
            };
            _ = c.napi_call_threadsafe_function(state.function, state, c.napi_tsfn_blocking);
            _ = c.napi_release_threadsafe_function(state.function, c.napi_tsfn_release);
        }

        fn callJs(callback_env: c.napi_env, function: c.napi_value, callback_context: ?*anyopaque, data: ?*anyopaque) callconv(.c) void {
            _ = callback_context;
            const state: *@This() = @ptrCast(@alignCast(data orelse return));
            if (callback_env == null or function == null) return;
            if (state.failed) return callErrorFirst(callback_env, function, null, state.failure_message);
            const value = encode(callback_env, &state.context) catch
                return callErrorFirst(callback_env, function, null, state.failure_message);
            callErrorFirst(callback_env, function, value, state.failure_message);
        }

        fn finalize(callback_env: c.napi_env, data: ?*anyopaque, hint: ?*anyopaque) callconv(.c) void {
            _ = callback_env;
            _ = hint;
            const state: *@This() = @ptrCast(@alignCast(data orelse return));
            deinit(&state.context);
            std.heap.c_allocator.destroy(state);
        }
    };

    var owned_context = context;
    const state = std.heap.c_allocator.create(State) catch {
        deinit(&owned_context);
        throwError(env, "native allocation failed");
        return null;
    };
    state.* = .{ .context = owned_context, .failure_message = failure_message };
    const resource_name = makeString(env, name) catch {
        deinit(&state.context);
        std.heap.c_allocator.destroy(state);
        return null;
    };
    check(env, c.napi_create_threadsafe_function(env, callback, null, resource_name, 1, 1, state, State.finalize, null, State.callJs, &state.function)) catch {
        deinit(&state.context);
        std.heap.c_allocator.destroy(state);
        return null;
    };
    execution.submit(State.worker, .{state}) catch {
        _ = c.napi_release_threadsafe_function(state.function, c.napi_tsfn_abort);
        throwError(env, "could not submit native callback work");
        return null;
    };
    return undefinedValue(env);
}

fn callErrorFirst(env: c.napi_env, function: c.napi_value, result: c.napi_value, failure_message: [:0]const u8) void {
    var receiver: c.napi_value = null;
    if (c.napi_get_undefined(env, &receiver) != c.napi_ok) return;
    var nil: c.napi_value = null;
    if (c.napi_get_null(env, &nil) != c.napi_ok) return;
    var values = [2]c.napi_value{ nil, result orelse nil };
    if (result == null) {
        const text = makeString(env, failure_message) catch return;
        if (c.napi_create_error(env, null, text, &values[0]) != c.napi_ok) return;
    }
    _ = c.napi_call_function(env, receiver, function, values.len, &values, null);
}

pub fn check(env: c.napi_env, status: c.napi_status) Error!void {
    if (status == c.napi_ok) return;
    var pending = false;
    if (c.napi_is_exception_pending(env, &pending) == c.napi_ok and pending)
        return error.PendingException;
    return error.NodeApiFailure;
}

pub fn throwTypeError(env: c.napi_env, message: [:0]const u8) void {
    _ = c.napi_throw_type_error(env, null, message.ptr);
}

pub fn throwError(env: c.napi_env, message: [:0]const u8) void {
    _ = c.napi_throw_error(env, null, message.ptr);
}

pub fn undefinedValue(env: c.napi_env) c.napi_value {
    var value: c.napi_value = null;
    check(env, c.napi_get_undefined(env, &value)) catch return null;
    return value;
}

pub fn arguments(env: c.napi_env, info: c.napi_callback_info, storage: []c.napi_value) Error![]c.napi_value {
    var count = storage.len;
    try check(env, c.napi_get_cb_info(env, info, &count, storage.ptr, null, null));
    return storage[0..count];
}

pub fn setFunction(env: c.napi_env, exports: c.napi_value, name: [:0]const u8, callback: c.napi_callback) Error!void {
    var function: c.napi_value = null;
    try check(env, c.napi_create_function(env, name.ptr, name.len, callback, null, &function));
    try check(env, c.napi_set_named_property(env, exports, name.ptr, function));
}

pub fn getU64(env: c.napi_env, value: c.napi_value) Error!u64 {
    var result: u64 = 0;
    var lossless = false;
    try check(env, c.napi_get_value_bigint_uint64(env, value, &result, &lossless));
    if (!lossless) return error.OutOfRange;
    return result;
}

pub fn makeU64(env: c.napi_env, value: u64) Error!c.napi_value {
    var result: c.napi_value = null;
    try check(env, c.napi_create_bigint_uint64(env, value, &result));
    return result;
}

pub fn getBool(env: c.napi_env, value: c.napi_value) Error!bool {
    var result = false;
    try check(env, c.napi_get_value_bool(env, value, &result));
    return result;
}

pub fn makeBool(env: c.napi_env, value: bool) Error!c.napi_value {
    var result: c.napi_value = null;
    try check(env, c.napi_get_boolean(env, value, &result));
    return result;
}

pub const OwnedString = struct {
    allocation: []u8,
    value: []u8,

    pub fn deinit(self: OwnedString, allocator: std.mem.Allocator) void {
        allocator.free(self.allocation);
    }
};

/// Owned byte storage for consumer-defined strings, buffers, and ABI views.
/// The allocator travels with the allocation so cleanup uses its origin.
pub const OwnedBytes = struct {
    allocator: std.mem.Allocator = std.heap.c_allocator,
    allocation: ?[]u8 = null,
    length: usize = 0,

    pub fn deinit(self: *OwnedBytes) void {
        if (self.allocation) |memory| self.allocator.free(memory);
        self.* = .{};
    }

    pub fn bytes(self: *const OwnedBytes) []const u8 {
        const memory = self.allocation orelse return &.{};
        return memory[0..self.length];
    }

    pub fn mutableView(self: *OwnedBytes, comptime View: type) View {
        const memory = self.allocation orelse return .{ .data = null, .length = 0, .capacity = 0 };
        return c_abi.initMutableView(View, memory);
    }
};

pub fn getBytesFromStringOwned(allocator: std.mem.Allocator, env: c.napi_env, value: c.napi_value) !OwnedBytes {
    const owned = try getStringOwned(allocator, env, value);
    return .{ .allocator = allocator, .allocation = owned.allocation, .length = owned.value.len };
}

pub fn getBufferOwned(allocator: std.mem.Allocator, env: c.napi_env, value: c.napi_value) !OwnedBytes {
    var is_buffer = false;
    try check(env, c.napi_is_buffer(env, value, &is_buffer));
    if (!is_buffer) return error.BufferRequired;
    var pointer: ?*anyopaque = null;
    var length: usize = 0;
    try check(env, c.napi_get_buffer_info(env, value, &pointer, &length));
    if (length == 0) return .{ .allocator = allocator };
    const memory = try allocator.alloc(u8, length);
    errdefer allocator.free(memory);
    @memcpy(memory, @as([*]const u8, @ptrCast(pointer.?))[0..length]);
    return .{ .allocator = allocator, .allocation = memory, .length = length };
}

pub fn getStringOwned(allocator: std.mem.Allocator, env: c.napi_env, value: c.napi_value) !OwnedString {
    var length: usize = 0;
    try check(env, c.napi_get_value_string_utf8(env, value, null, 0, &length));
    const allocation = try allocator.alloc(u8, length + 1);
    errdefer allocator.free(allocation);
    var written: usize = 0;
    try check(env, c.napi_get_value_string_utf8(env, value, allocation.ptr, allocation.len, &written));
    return .{ .allocation = allocation, .value = allocation[0..written] };
}

pub fn makeString(env: c.napi_env, value: []const u8) Error!c.napi_value {
    var result: c.napi_value = null;
    try check(env, c.napi_create_string_utf8(env, value.ptr, value.len, &result));
    return result;
}

pub fn namedProperty(env: c.napi_env, source: c.napi_value, name: [:0]const u8) Error!c.napi_value {
    var result: c.napi_value = null;
    try check(env, c.napi_get_named_property(env, source, name.ptr, &result));
    return result;
}

pub fn setNamedProperty(env: c.napi_env, destination: c.napi_value, name: [:0]const u8, value: c.napi_value) Error!void {
    try check(env, c.napi_set_named_property(env, destination, name.ptr, value));
}

pub fn object(env: c.napi_env) Error!c.napi_value {
    var result: c.napi_value = null;
    try check(env, c.napi_create_object(env, &result));
    return result;
}

pub fn array(env: c.napi_env, length: usize) Error!c.napi_value {
    var result: c.napi_value = null;
    const count = std.math.cast(usize, length) orelse return error.OutOfRange;
    try check(env, c.napi_create_array_with_length(env, count, &result));
    return result;
}

pub fn setElement(env: c.napi_env, value: c.napi_value, index: u32, element: c.napi_value) Error!void {
    try check(env, c.napi_set_element(env, value, index, element));
}

pub fn getElement(env: c.napi_env, value: c.napi_value, index: u32) Error!c.napi_value {
    var result: c.napi_value = null;
    try check(env, c.napi_get_element(env, value, index, &result));
    return result;
}

pub fn arrayLength(env: c.napi_env, value: c.napi_value) Error!u32 {
    var result: u32 = 0;
    try check(env, c.napi_get_array_length(env, value, &result));
    return result;
}

test "consumer converters accept unrelated arbitrary types" {
    const A = extern struct { id: u32, values: [*]const f64, length: usize };
    const B = extern union { bytes: [8]u8, handle: usize };
    _ = Converter.Type(A, void);
    _ = Converter.Type(B, *anyopaque);
}

/// Native callback completion with cancellation and environment-owned teardown.
pub const AsyncBindings = @import("async.zig").Bindings;

/// Read an optional AbortSignal without retaining it. The async binding retains
/// it before native submission; omitted, undefined and null mean no signal.
pub fn optionalAbortSignal(env: c.napi_env, object_value: c.napi_value) Error!c.napi_value {
    var present = false;
    try check(env, c.napi_has_named_property(env, object_value, "signal", &present));
    if (!present) return null;
    const signal = try namedProperty(env, object_value, "signal");
    var kind: c.napi_valuetype = undefined;
    try check(env, c.napi_typeof(env, signal, &kind));
    return if (kind == c.napi_null or kind == c.napi_undefined) null else signal;
}

pub const AsyncResource = @import("resource.zig").Resource;
