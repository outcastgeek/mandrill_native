//! Reusable native Node-API stream construction, work, and backpressure.
//!
//! Consumers provide native producer/transform functions. This module creates
//! actual Node stream instances and installs native `_read`, `_write`,
//! `_transform`, and `_destroy` callbacks on them through Node-API.

const std = @import("std");
const napi = @import("node_api_runtime");
const execution = @import("native_execution");
const c = napi.c;

const allocator = std.heap.c_allocator;

pub const ByteChunk = struct {
    allocator: std.mem.Allocator,
    allocation: []u8,
    value: []u8,

    pub fn allocate(chunk_allocator: std.mem.Allocator, capacity: usize) !ByteChunk {
        const memory = try chunk_allocator.alloc(u8, capacity);
        return .{ .allocator = chunk_allocator, .allocation = memory, .value = memory };
    }

    pub fn deinit(self: *ByteChunk) void {
        self.allocator.free(self.allocation);
        self.* = undefined;
    }
};

pub const ByteSource = struct {
    context: ?*anyopaque = null,
    next: *const fn (?*anyopaque, std.mem.Allocator) anyerror!?ByteChunk,
    /// Nonblocking cancellation request for context-owned native work. May run
    /// concurrently with next/apply; context stays alive through native return.
    cancel: *const fn (?*anyopaque) void = ignoreContext,
    deinit: *const fn (?*anyopaque) void = ignoreContext,
};

pub const ByteTransform = struct {
    context: ?*anyopaque = null,
    apply: *const fn (?*anyopaque, std.mem.Allocator, []const u8) anyerror!ByteChunk,
    /// Nonblocking cancellation request for context-owned native work. May run
    /// concurrently with next/apply; context stays alive through native return.
    cancel: *const fn (?*anyopaque) void = ignoreContext,
    deinit: *const fn (?*anyopaque) void = ignoreContext,
};

fn ignoreContext(_: ?*anyopaque) void {}

const PromiseTransform = struct {
    input: []u8,
    output: ?ByteChunk = null,
    transform: ByteTransform,
};

/// Runs a consumer transform as a native Promise operation. The transform and
/// byte layout are consumer supplied; async-work ownership is shared here.
pub fn transformPromise(env: c.napi_env, info: c.napi_callback_info, transform: ByteTransform) c.napi_value {
    var storage: [1]c.napi_value = undefined;
    const args = napi.arguments(env, info, &storage) catch return failType(env, "could not read stream chunk");
    if (args.len != 1) return failType(env, "streamTransform requires one Buffer");
    const input = copyBuffer(env, args[0]) catch return failType(env, "streamTransform requires one Buffer");
    return napi.queuePromise(PromiseTransform, env, "native-stream-transform", .{
        .input = input,
        .transform = transform,
    }, executePromiseTransform, completePromiseTransform, deinitPromiseTransform, "native stream transform failed");
}

fn executePromiseTransform(context: *PromiseTransform) !void {
    context.output = try context.transform.apply(context.transform.context, allocator, context.input);
}

fn completePromiseTransform(env: c.napi_env, context: *PromiseTransform) !c.napi_value {
    return bufferValue(env, context.output.?.value);
}

fn deinitPromiseTransform(env: c.napi_env, context: *PromiseTransform) void {
    _ = env;
    if (context.output) |*output| output.deinit();
    context.transform.deinit(context.transform.context);
    allocator.free(context.input);
}

const StreamKind = enum { pull, push, writable, transform, duplex };

const NativeStream = struct {
    owners: std.atomic.Value(u32) = .init(1),
    kind: StreamKind,
    transform: ByteTransform,
    source: ?ByteSource = null,
    reading: std.atomic.Value(bool) = .init(false),
    ended: std.atomic.Value(bool) = .init(false),
    closed: std.atomic.Value(bool) = .init(false),
    function: c.napi_threadsafe_function = null,
    function_release_claimed: std.atomic.Value(bool) = .init(false),
    function_referenced: std.atomic.Value(bool) = .init(true),
    stream_ref: c.napi_ref = null,
    waiting: std.atomic.Value(bool) = .init(false),
    cleanup_hook_active: std.atomic.Value(bool) = .init(false),
    current: ?ByteChunk = null,
    producer_failed: bool = false,
};

fn cancelStream(state: *NativeStream) void {
    if (state.closed.swap(true, .acq_rel)) return;
    if (state.source) |source| source.cancel(source.context);
    state.transform.cancel(state.transform.context);
}

fn retainStream(state: *NativeStream) void {
    _ = state.owners.fetchAdd(1, .acq_rel);
}

fn releaseStream(state: *NativeStream) void {
    if (state.owners.fetchSub(1, .acq_rel) != 1) return;
    if (state.current) |*current| current.deinit();
    if (state.source) |source| source.deinit(source.context);
    state.transform.deinit(state.transform.context);
    allocator.destroy(state);
}

fn acknowledge(source: *NativeStream) void {
    if (source.closed.load(.acquire) or source.ended.load(.acquire)) {
        if (source.function != null) releasePushFunction(source, false);
        return;
    }
    if (source.current) |*current| current.deinit();
    source.current = null;
    source.reading.store(false, .release);
    if (source.function != null) resumePush(source);
}

fn resumePush(source: *NativeStream) void {
    if (source.closed.load(.acquire) or source.reading.swap(true, .acq_rel)) return;
    if (c.napi_acquire_threadsafe_function(source.function) != c.napi_ok) return;
    retainStream(source);
    execution.submit(pushWorker, .{source}) catch {
        releaseStream(source);
        _ = c.napi_release_threadsafe_function(source.function, c.napi_tsfn_release);
        source.producer_failed = true;
        _ = c.napi_call_threadsafe_function(source.function, source, c.napi_tsfn_nonblocking);
    };
}

fn callbackInfo(
    env: c.napi_env,
    info: c.napi_callback_info,
    storage: []c.napi_value,
    this_arg: *c.napi_value,
    data: *?*anyopaque,
) ![]c.napi_value {
    var count = storage.len;
    try napi.check(env, c.napi_get_cb_info(env, info, &count, storage.ptr, this_arg, data));
    return storage[0..count];
}

fn callbackValue(env: c.napi_env, name: [:0]const u8, callback: c.napi_callback, data: ?*anyopaque) !c.napi_value {
    var value: c.napi_value = null;
    try napi.check(env, c.napi_create_function(env, name.ptr, name.len, callback, data, &value));
    return value;
}

fn nullValue(env: c.napi_env) !c.napi_value {
    var value: c.napi_value = null;
    try napi.check(env, c.napi_get_null(env, &value));
    return value;
}

fn errorValue(env: c.napi_env, message: [:0]const u8) !c.napi_value {
    const text = try napi.makeString(env, message);
    var value: c.napi_value = null;
    try napi.check(env, c.napi_create_error(env, null, text, &value));
    return value;
}

fn copyBuffer(env: c.napi_env, value: c.napi_value) ![]u8 {
    var is_buffer = false;
    try napi.check(env, c.napi_is_buffer(env, value, &is_buffer));
    if (!is_buffer) return error.BufferRequired;
    var pointer: ?*anyopaque = null;
    var length: usize = 0;
    try napi.check(env, c.napi_get_buffer_info(env, value, &pointer, &length));
    const result = try allocator.alloc(u8, length);
    if (length != 0) @memcpy(result, @as([*]const u8, @ptrCast(pointer.?))[0..length]);
    return result;
}

fn bufferValue(env: c.napi_env, bytes: []const u8) !c.napi_value {
    var value: c.napi_value = null;
    try napi.check(env, c.napi_create_buffer_copy(env, bytes.len, if (bytes.len == 0) null else bytes.ptr, null, &value));
    return value;
}

fn callFunction(env: c.napi_env, function: c.napi_value, values: []const c.napi_value) !c.napi_value {
    var receiver: c.napi_value = null;
    try napi.check(env, c.napi_get_undefined(env, &receiver));
    var result: c.napi_value = null;
    try napi.check(env, c.napi_call_function(env, receiver, function, values.len, values.ptr, &result));
    return result;
}

fn callMethod(env: c.napi_env, receiver: c.napi_value, name: [:0]const u8, values: []const c.napi_value) !c.napi_value {
    const function = try napi.namedProperty(env, receiver, name);
    var result: c.napi_value = null;
    try napi.check(env, c.napi_call_function(env, receiver, function, values.len, values.ptr, &result));
    return result;
}

fn streamOptions(
    env: c.napi_env,
    state: *NativeStream,
    high_water_mark: ?c.napi_value,
    read: bool,
    write: bool,
    transform: bool,
) !c.napi_value {
    const options = try napi.object(env);
    if (high_water_mark) |value| try napi.setNamedProperty(env, options, "highWaterMark", value);
    if (read) try napi.setNamedProperty(env, options, "read", try callbackValue(env, "nativeRead", nativeRead, state));
    if (write) try napi.setNamedProperty(env, options, "write", try callbackValue(env, "nativeWrite", nativeWrite, state));
    if (transform) try napi.setNamedProperty(env, options, "transform", try callbackValue(env, "nativeTransform", nativeTransform, state));
    try napi.setNamedProperty(env, options, "destroy", try callbackValue(env, "nativeDestroy", nativeDestroy, state));
    return options;
}

fn construct(
    env: c.napi_env,
    constructor: c.napi_value,
    state: *NativeStream,
    high_water_mark: ?c.napi_value,
    read: bool,
    write: bool,
    transform: bool,
) !c.napi_value {
    const options = try streamOptions(env, state, high_water_mark, read, write, transform);
    var stream: c.napi_value = null;
    try napi.check(env, c.napi_new_instance(env, constructor, 1, &options, &stream));
    try napi.check(env, c.napi_wrap(env, stream, state, finalizeStream, null, null));
    return stream;
}

fn newState(kind: StreamKind, source: ?ByteSource, transform: ByteTransform) !*NativeStream {
    const state = try allocator.create(NativeStream);
    state.* = .{ .kind = kind, .source = source, .transform = transform };
    return state;
}

fn createSourceStream(
    env: c.napi_env,
    constructor: c.napi_value,
    high_water_mark: ?c.napi_value,
    kind: StreamKind,
    source: ByteSource,
    transform: ByteTransform,
) c.napi_value {
    const state = newState(kind, source, transform) catch {
        source.deinit(source.context);
        transform.deinit(transform.context);
        return failError(env, "native allocation failed");
    };
    const value = construct(env, constructor, state, high_water_mark, true, kind == .duplex, false) catch {
        releaseStream(state);
        return null;
    };
    if (kind == .push) startPush(env, value, state) catch {
        return failError(env, "could not start native push stream");
    };
    return value;
}

pub fn createPullReadable(env: c.napi_env, constructor: c.napi_value, high_water_mark: ?c.napi_value, source: ByteSource, transform: ByteTransform) c.napi_value {
    return createSourceStream(env, constructor, high_water_mark, .pull, source, transform);
}

pub fn createPushReadable(env: c.napi_env, constructor: c.napi_value, high_water_mark: ?c.napi_value, source: ByteSource, transform: ByteTransform) c.napi_value {
    comptime if (c.NAPI_VERSION < 4)
        @compileError("native push streaming requires consumer-selected Node-API version 4 or newer");
    return createSourceStream(env, constructor, high_water_mark, .push, source, transform);
}

fn createWorkStream(env: c.napi_env, constructor: c.napi_value, high_water_mark: ?c.napi_value, kind: StreamKind, transform: ByteTransform) c.napi_value {
    const state = newState(kind, null, transform) catch {
        transform.deinit(transform.context);
        return failError(env, "native allocation failed");
    };
    return construct(env, constructor, state, high_water_mark, false, kind == .writable, kind == .transform) catch {
        releaseStream(state);
        return null;
    };
}

pub fn createWritable(env: c.napi_env, constructor: c.napi_value, high_water_mark: ?c.napi_value, transform: ByteTransform) c.napi_value {
    return createWorkStream(env, constructor, high_water_mark, .writable, transform);
}

pub fn createTransform(env: c.napi_env, constructor: c.napi_value, high_water_mark: ?c.napi_value, transform: ByteTransform) c.napi_value {
    return createWorkStream(env, constructor, high_water_mark, .transform, transform);
}

pub fn createDuplex(env: c.napi_env, constructor: c.napi_value, high_water_mark: ?c.napi_value, source: ByteSource, transform: ByteTransform) c.napi_value {
    return createSourceStream(env, constructor, high_water_mark, .duplex, source, transform);
}

pub const SourceOptions = struct {
    constructor: c.napi_value,
    high_water_mark: ?c.napi_value,
    source: ByteSource,
};

pub const WorkOptions = struct {
    constructor: c.napi_value,
    high_water_mark: ?c.napi_value,
};

/// Generates the Node-API C callbacks for a consumer's source argument parser
/// and transform factory. The consumer keeps its public argument contract;
/// native stream construction and callback forwarding stay reusable here.
pub fn Bindings(comptime Hooks: type) type {
    return struct {
        pub fn promise(env: c.napi_env, info: c.napi_callback_info) callconv(.c) c.napi_value {
            return transformPromise(env, info, Hooks.transform());
        }

        pub fn pull(env: c.napi_env, info: c.napi_callback_info) callconv(.c) c.napi_value {
            const options = Hooks.source(env, info) catch return failType(env, "invalid native stream source arguments");
            return createPullReadable(env, options.constructor, options.high_water_mark, options.source, Hooks.transform());
        }

        pub fn push(env: c.napi_env, info: c.napi_callback_info) callconv(.c) c.napi_value {
            const options = Hooks.source(env, info) catch return failType(env, "invalid native stream source arguments");
            return createPushReadable(env, options.constructor, options.high_water_mark, options.source, Hooks.transform());
        }

        pub fn writable(env: c.napi_env, info: c.napi_callback_info) callconv(.c) c.napi_value {
            const options = Hooks.work(env, info) catch return failType(env, "invalid native stream arguments");
            return createWritable(env, options.constructor, options.high_water_mark, Hooks.transform());
        }

        pub fn transform(env: c.napi_env, info: c.napi_callback_info) callconv(.c) c.napi_value {
            const options = Hooks.work(env, info) catch return failType(env, "invalid native stream arguments");
            return createTransform(env, options.constructor, options.high_water_mark, Hooks.transform());
        }

        pub fn duplex(env: c.napi_env, info: c.napi_callback_info) callconv(.c) c.napi_value {
            const options = Hooks.source(env, info) catch return failType(env, "invalid native stream source arguments");
            return createDuplex(env, options.constructor, options.high_water_mark, options.source, Hooks.transform());
        }
    };
}

/// Creates an actual object-mode Transform whose chunk type and conversion are
/// wholly consumer-defined. The shared helper owns Node-API async work,
/// references, completion, and cleanup; it has no closed set of data shapes.
pub fn createObjectTransform(
    comptime Context: type,
    env: c.napi_env,
    info: c.napi_callback_info,
    comptime prepare: fn (c.napi_env, c.napi_value) anyerror!Context,
    comptime execute: fn (*Context) anyerror!void,
    comptime encode: fn (c.napi_env, *Context) anyerror!c.napi_value,
    comptime deinit: fn (*Context) void,
) c.napi_value {
    const Adapter = ObjectTransformAdapter(Context, prepare, execute, encode, deinit, true);
    var storage: [2]c.napi_value = undefined;
    var this_arg: c.napi_value = null;
    var data: ?*anyopaque = null;
    const args = callbackInfo(env, info, &storage, &this_arg, &data) catch return failType(env, "could not read object stream arguments");
    if (args.len < 1 or args.len > 2) return failType(env, "object stream factory requires constructor and optional highWaterMark");
    const options = napi.object(env) catch return null;
    napi.setNamedProperty(env, options, "objectMode", napi.makeBool(env, true) catch return null) catch return null;
    if (args.len == 2) napi.setNamedProperty(env, options, "highWaterMark", args[1]) catch return null;
    napi.setNamedProperty(env, options, "transform", callbackValue(env, "nativeObjectTransform", Adapter.transform, null) catch return null) catch return null;
    napi.setNamedProperty(env, options, "destroy", callbackValue(env, "nativeObjectDestroy", passthroughDestroy, null) catch return null) catch return null;
    var stream: c.napi_value = null;
    napi.check(env, c.napi_new_instance(env, args[0], 1, &options, &stream)) catch return null;
    return stream;
}

/// Generates an object-mode Transform factory for consumer work types with
/// `executeWorker`, `finish`, and `deinit` methods. Only decoding remains a
/// consumer concern, so signature-only adapters do not leak into artifacts.
pub fn ObjectBindings(
    comptime Work: type,
    comptime prepare: fn (c.napi_env, c.napi_value) anyerror!Work,
) type {
    const Methods = struct {
        fn execute(work: *Work) anyerror!void {
            work.executeWorker();
        }

        fn encode(env: c.napi_env, work: *Work) anyerror!c.napi_value {
            return work.finish(env);
        }

        fn deinit(work: *Work) void {
            work.deinit();
        }
    };
    return struct {
        pub fn transform(env: c.napi_env, info: c.napi_callback_info) callconv(.c) c.napi_value {
            return createObjectTransform(Work, env, info, prepare, Methods.execute, Methods.encode, Methods.deinit);
        }

        pub fn writable(env: c.napi_env, info: c.napi_callback_info) callconv(.c) c.napi_value {
            const Adapter = ObjectTransformAdapter(Work, prepare, Methods.execute, Methods.encode, Methods.deinit, false);
            var storage: [2]c.napi_value = undefined;
            var this_arg: c.napi_value = null;
            var data: ?*anyopaque = null;
            const args = callbackInfo(env, info, &storage, &this_arg, &data) catch return failType(env, "could not read object stream arguments");
            if (args.len < 1 or args.len > 2) return failType(env, "object stream factory requires constructor and optional highWaterMark");
            const options = napi.object(env) catch return null;
            napi.setNamedProperty(env, options, "objectMode", napi.makeBool(env, true) catch return null) catch return null;
            if (args.len == 2) napi.setNamedProperty(env, options, "highWaterMark", args[1]) catch return null;
            napi.setNamedProperty(env, options, "write", callbackValue(env, "nativeObjectWrite", Adapter.transform, null) catch return null) catch return null;
            napi.setNamedProperty(env, options, "destroy", callbackValue(env, "nativeObjectDestroy", passthroughDestroy, null) catch return null) catch return null;
            var writable_stream: c.napi_value = null;
            napi.check(env, c.napi_new_instance(env, args[0], 1, &options, &writable_stream)) catch return null;
            return writable_stream;
        }
    };
}

fn ObjectTransformAdapter(
    comptime Context: type,
    comptime prepare: fn (c.napi_env, c.napi_value) anyerror!Context,
    comptime execute: fn (*Context) anyerror!void,
    comptime encode: fn (c.napi_env, *Context) anyerror!c.napi_value,
    comptime deinit: fn (*Context) void,
    comptime return_output: bool,
) type {
    return struct {
        const Work = struct {
            work: c.napi_async_work = null,
            stream_ref: c.napi_ref = null,
            callback_ref: c.napi_ref = null,
            context: Context,
            failed: bool = false,
        };

        fn transform(env: c.napi_env, info: c.napi_callback_info) callconv(.c) c.napi_value {
            var storage: [3]c.napi_value = undefined;
            var stream: c.napi_value = null;
            var data: ?*anyopaque = null;
            const args = callbackInfo(env, info, &storage, &stream, &data) catch return null;
            if (args.len != 3) return failType(env, "object transform requires chunk, encoding, and callback");
            const context = prepare(env, args[0]) catch return callbackFailure(env, args[2], "invalid structured stream chunk");
            const state = allocator.create(Work) catch {
                var owned = context;
                deinit(&owned);
                return callbackFailure(env, args[2], "native allocation failed");
            };
            state.* = .{ .context = context };
            if (c.napi_create_reference(env, stream, 1, &state.stream_ref) != c.napi_ok or c.napi_create_reference(env, args[2], 1, &state.callback_ref) != c.napi_ok) {
                if (state.stream_ref != null) _ = c.napi_delete_reference(env, state.stream_ref);
                if (state.callback_ref != null) _ = c.napi_delete_reference(env, state.callback_ref);
                destroy(state);
                return null;
            }
            const name = napi.makeString(env, "native-object-stream-work") catch {
                _ = c.napi_delete_reference(env, state.callback_ref);
                _ = c.napi_delete_reference(env, state.stream_ref);
                destroy(state);
                return null;
            };
            if (c.napi_create_async_work(env, null, name, executeWork, completeWork, state, &state.work) != c.napi_ok or c.napi_queue_async_work(env, state.work) != c.napi_ok) {
                if (state.work != null) _ = c.napi_delete_async_work(env, state.work);
                _ = c.napi_delete_reference(env, state.callback_ref);
                _ = c.napi_delete_reference(env, state.stream_ref);
                destroy(state);
                return callbackFailure(env, args[2], "could not queue structured stream work");
            }
            return napi.undefinedValue(env);
        }

        fn executeWork(env: c.napi_env, data: ?*anyopaque) callconv(.c) void {
            _ = env;
            const state: *Work = @ptrCast(@alignCast(data.?));
            execute(&state.context) catch {
                state.failed = true;
            };
        }

        fn completeWork(env: c.napi_env, status: c.napi_status, data: ?*anyopaque) callconv(.c) void {
            const state: *Work = @ptrCast(@alignCast(data.?));
            defer destroy(state);
            defer _ = c.napi_delete_reference(env, state.callback_ref);
            defer _ = c.napi_delete_reference(env, state.stream_ref);
            defer _ = c.napi_delete_async_work(env, state.work);
            var callback: c.napi_value = null;
            if (c.napi_get_reference_value(env, state.callback_ref, &callback) != c.napi_ok) return;
            if (status != c.napi_ok or state.failed) {
                _ = callbackFailure(env, callback, "structured stream work failed");
                return;
            }
            const nil = nullValue(env) catch return;
            if (return_output) {
                const value = encode(env, &state.context) catch {
                    _ = callbackFailure(env, callback, "could not encode structured stream chunk");
                    return;
                };
                _ = callFunction(env, callback, &.{ nil, value }) catch {};
            } else {
                _ = callFunction(env, callback, &.{nil}) catch {};
            }
        }

        fn destroy(state: *Work) void {
            deinit(&state.context);
            allocator.destroy(state);
        }
    };
}

pub fn ObjectSourceOptions(comptime Context: type) type {
    return struct {
        constructor: c.napi_value,
        high_water_mark: ?c.napi_value,
        context: Context,
        cancel: *const fn (*Context) void = ignoreCancellation,
        fn ignoreCancellation(_: *Context) void {}
    };
}

/// Generates object-mode Readable factories for arbitrary consumer-owned
/// native item types. `next` chooses EOF, `encode` runs on the environment
/// thread, and both source/item cleanup policies remain consumer supplied.
pub fn ObjectSourceBindings(
    comptime Context: type,
    comptime Item: type,
    comptime prepare: fn (c.napi_env, c.napi_callback_info) anyerror!ObjectSourceOptions(Context),
    comptime next: fn (*Context) anyerror!?Item,
    comptime encode: fn (c.napi_env, *Item) anyerror!c.napi_value,
    comptime deinit_item: fn (*Item) void,
    comptime deinit_context: fn (*Context) void,
    comptime SinkWork: type,
    comptime prepare_sink: fn (c.napi_env, c.napi_value) anyerror!SinkWork,
) type {
    return struct {
        const Self = @This();
        const SinkMethods = struct {
            fn execute(work: *SinkWork) anyerror!void {
                work.executeWorker();
            }

            fn encodeResult(env: c.napi_env, work: *SinkWork) anyerror!c.napi_value {
                return work.finish(env);
            }

            fn deinit(work: *SinkWork) void {
                work.deinit();
            }
        };
        const SinkAdapter = ObjectTransformAdapter(SinkWork, prepare_sink, SinkMethods.execute, SinkMethods.encodeResult, SinkMethods.deinit, false);
        const State = struct {
            owners: std.atomic.Value(u32) = .init(1),
            context: Context,
            cancel_context: *const fn (*Context) void,
            push_mode: bool,
            reading: std.atomic.Value(bool) = .init(false),
            ended: std.atomic.Value(bool) = .init(false),
            closed: std.atomic.Value(bool) = .init(false),
            function: c.napi_threadsafe_function = null,
            function_release_claimed: std.atomic.Value(bool) = .init(false),
            function_referenced: std.atomic.Value(bool) = .init(true),
            cleanup_hook_active: std.atomic.Value(bool) = .init(false),
            stream_ref: c.napi_ref = null,
            waiting: std.atomic.Value(bool) = .init(false),
            current: ?Item = null,
            producer_failed: bool = false,
        };

        const Pull = struct {
            work: c.napi_async_work = null,
            stream_ref: c.napi_ref = null,
            source: *State,
            item: ?Item = null,
            eof: bool = false,
            failed: bool = false,
        };

        pub fn pull(env: c.napi_env, info: c.napi_callback_info) callconv(.c) c.napi_value {
            return create(env, info, false, false);
        }

        pub fn push(env: c.napi_env, info: c.napi_callback_info) callconv(.c) c.napi_value {
            comptime if (c.NAPI_VERSION < 4)
                @compileError("native object push streaming requires consumer-selected Node-API version 4 or newer");
            return create(env, info, true, false);
        }

        pub fn duplex(env: c.napi_env, info: c.napi_callback_info) callconv(.c) c.napi_value {
            return create(env, info, false, true);
        }

        pub fn pushDuplex(env: c.napi_env, info: c.napi_callback_info) callconv(.c) c.napi_value {
            comptime if (c.NAPI_VERSION < 4)
                @compileError("native object push streaming requires consumer-selected Node-API version 4 or newer");
            return create(env, info, true, true);
        }

        fn create(env: c.napi_env, info: c.napi_callback_info, push_mode: bool, duplex_mode: bool) c.napi_value {
            var source_options = prepare(env, info) catch return failType(env, "invalid object stream source arguments");
            const state = allocator.create(State) catch {
                deinit_context(&source_options.context);
                return failError(env, "native allocation failed");
            };
            state.* = .{ .context = source_options.context, .cancel_context = source_options.cancel, .push_mode = push_mode };
            const options = napi.object(env) catch {
                release(state);
                return null;
            };
            napi.setNamedProperty(env, options, "objectMode", napi.makeBool(env, true) catch {
                release(state);
                return null;
            }) catch {
                release(state);
                return null;
            };
            if (source_options.high_water_mark) |value| napi.setNamedProperty(env, options, "highWaterMark", value) catch {
                release(state);
                return null;
            };
            napi.setNamedProperty(env, options, "read", callbackValue(env, "nativeObjectRead", read, state) catch {
                release(state);
                return null;
            }) catch {
                release(state);
                return null;
            };
            if (duplex_mode) napi.setNamedProperty(env, options, "write", callbackValue(env, "nativeObjectWrite", SinkAdapter.transform, null) catch {
                release(state);
                return null;
            }) catch {
                release(state);
                return null;
            };
            napi.setNamedProperty(env, options, "destroy", callbackValue(env, "nativeObjectDestroy", destroy, state) catch {
                release(state);
                return null;
            }) catch {
                release(state);
                return null;
            };
            var readable: c.napi_value = null;
            napi.check(env, c.napi_new_instance(env, source_options.constructor, 1, &options, &readable)) catch {
                release(state);
                return null;
            };
            napi.check(env, c.napi_wrap(env, readable, state, finalize, null, null)) catch {
                release(state);
                return null;
            };
            if (push_mode) Self.startPush(env, readable, state) catch return failError(env, "could not start native object push stream");
            return readable;
        }

        fn retain(state: *State) void {
            _ = state.owners.fetchAdd(1, .acq_rel);
        }

        fn release(state: *State) void {
            if (state.owners.fetchSub(1, .acq_rel) != 1) return;
            if (state.current) |*item| deinit_item(item);
            deinit_context(&state.context);
            allocator.destroy(state);
        }

        fn acknowledgeObject(state: *State) void {
            if (state.closed.load(.acquire) or state.ended.load(.acquire)) {
                if (state.function != null) releaseFunction(state, false);
                return;
            }
            if (state.current) |*item| deinit_item(item);
            state.current = null;
            state.reading.store(false, .release);
            if (state.function != null) Self.resumePush(state);
        }

        fn resumePush(state: *State) void {
            if (state.closed.load(.acquire) or state.reading.swap(true, .acq_rel)) return;
            if (c.napi_acquire_threadsafe_function(state.function) != c.napi_ok) return;
            retain(state);
            execution.submit(Self.pushWorker, .{state}) catch {
                release(state);
                _ = c.napi_release_threadsafe_function(state.function, c.napi_tsfn_release);
                state.producer_failed = true;
                _ = c.napi_call_threadsafe_function(state.function, state, c.napi_tsfn_nonblocking);
            };
        }

        fn read(env: c.napi_env, info: c.napi_callback_info) callconv(.c) c.napi_value {
            var storage: [1]c.napi_value = undefined;
            var readable: c.napi_value = null;
            var context_ptr: ?*anyopaque = null;
            _ = callbackInfo(env, info, &storage, &readable, &context_ptr) catch return null;
            const state: *State = @ptrCast(@alignCast(context_ptr orelse return null));
            if (state.push_mode) {
                if (state.waiting.swap(false, .acq_rel)) {
                    if (!refFunction(env, state)) return destroyWithMessage(env, readable, "could not resume native object push stream");
                    acknowledgeObject(state);
                }
                return napi.undefinedValue(env);
            }
            if (state.closed.load(.acquire) or state.ended.load(.acquire)) return napi.undefinedValue(env);
            if (state.reading.cmpxchgStrong(false, true, .acq_rel, .acquire) != null) return napi.undefinedValue(env);
            const work = allocator.create(Pull) catch {
                state.reading.store(false, .release);
                return destroyWithMessage(env, readable, "native allocation failed");
            };
            work.* = .{ .source = state };
            if (c.napi_create_reference(env, readable, 1, &work.stream_ref) != c.napi_ok) {
                state.reading.store(false, .release);
                allocator.destroy(work);
                return null;
            }
            const name = napi.makeString(env, "native-object-pull") catch {
                state.reading.store(false, .release);
                _ = c.napi_delete_reference(env, work.stream_ref);
                allocator.destroy(work);
                return null;
            };
            if (c.napi_create_async_work(env, null, name, Self.executePull, Self.completePull, work, &work.work) != c.napi_ok or c.napi_queue_async_work(env, work.work) != c.napi_ok) {
                if (work.work != null) _ = c.napi_delete_async_work(env, work.work);
                _ = c.napi_delete_reference(env, work.stream_ref);
                state.reading.store(false, .release);
                allocator.destroy(work);
                return destroyWithMessage(env, readable, "could not queue native object pull");
            }
            return napi.undefinedValue(env);
        }

        fn executePull(_: c.napi_env, data: ?*anyopaque) callconv(.c) void {
            const work: *Pull = @ptrCast(@alignCast(data.?));
            work.item = next(&work.source.context) catch {
                work.failed = true;
                return;
            };
            work.eof = work.item == null;
        }

        fn completePull(env: c.napi_env, status: c.napi_status, data: ?*anyopaque) callconv(.c) void {
            const work: *Pull = @ptrCast(@alignCast(data.?));
            defer allocator.destroy(work);
            defer if (work.item) |*item| deinit_item(item);
            defer _ = c.napi_delete_reference(env, work.stream_ref);
            defer _ = c.napi_delete_async_work(env, work.work);
            defer work.source.reading.store(false, .release);
            var readable: c.napi_value = null;
            if (c.napi_get_reference_value(env, work.stream_ref, &readable) != c.napi_ok) return;
            if (work.source.closed.load(.acquire)) return;
            if (status != c.napi_ok or work.failed) {
                _ = destroyWithMessage(env, readable, "native object pull failed");
                return;
            }
            if (work.eof) {
                work.source.ended.store(true, .release);
                const nil = nullValue(env) catch return;
                _ = callMethod(env, readable, "push", &.{nil}) catch {};
                return;
            }
            const value = encode(env, &work.item.?) catch {
                _ = destroyWithMessage(env, readable, "could not encode native object chunk");
                return;
            };
            _ = callMethod(env, readable, "push", &.{value}) catch {};
        }

        fn destroy(env: c.napi_env, info: c.napi_callback_info) callconv(.c) c.napi_value {
            var storage: [2]c.napi_value = undefined;
            var this_arg: c.napi_value = null;
            var context_ptr: ?*anyopaque = null;
            const args = callbackInfo(env, info, &storage, &this_arg, &context_ptr) catch return null;
            const state: *State = @ptrCast(@alignCast(context_ptr orelse return null));
            requestCancellation(state);
            if (state.push_mode) unrefFunction(env, state);
            acknowledgeObject(state);
            if (args.len == 2) _ = callFunction(env, args[1], args[0..1]) catch return null;
            return napi.undefinedValue(env);
        }

        fn finalize(_: c.napi_env, data: ?*anyopaque, _: ?*anyopaque) callconv(.c) void {
            const state: *State = @ptrCast(@alignCast(data orelse return));
            requestCancellation(state);
            acknowledgeObject(state);
            release(state);
        }

        fn startPush(env: c.napi_env, readable: c.napi_value, state: *State) !void {
            try napi.check(env, c.napi_create_reference(env, readable, 1, &state.stream_ref));
            const name = try napi.makeString(env, "native-object-push");
            napi.check(env, c.napi_create_threadsafe_function(env, null, null, name, 1, 1, state, Self.finalizePush, state, Self.callPush, &state.function)) catch {
                _ = c.napi_delete_reference(env, state.stream_ref);
                return error.NodeApiFailure;
            };
            retain(state);
            napi.check(env, c.napi_add_env_cleanup_hook(env, environmentCleanup, state)) catch {
                releaseFunction(state, true);
                return error.NodeApiFailure;
            };
            state.cleanup_hook_active.store(true, .release);
            Self.resumePush(state);
        }

        fn pushWorker(state: *State) void {
            defer release(state);
            defer _ = c.napi_release_threadsafe_function(state.function, c.napi_tsfn_release);
            if (state.closed.load(.acquire)) return;
            state.current = next(&state.context) catch result: {
                state.producer_failed = true;
                break :result null;
            };
            if (state.current == null) state.ended.store(true, .release);
            // Only one chunk is outstanding. JS demand schedules the next job.
            _ = c.napi_call_threadsafe_function(state.function, state, c.napi_tsfn_nonblocking);
        }

        fn callPush(env: c.napi_env, _: c.napi_value, _: ?*anyopaque, data: ?*anyopaque) callconv(.c) void {
            const state: *State = @ptrCast(@alignCast(data orelse return));
            if (env == null) return cancel(state);
            var readable: c.napi_value = null;
            if (c.napi_get_reference_value(env, state.stream_ref, &readable) != c.napi_ok) return cancel(state);
            if (state.closed.load(.acquire)) return cancel(state);
            if (state.producer_failed) {
                _ = destroyWithMessage(env, readable, "native object push source failed");
                return cancel(state);
            }
            if (state.ended.load(.acquire)) {
                const nil = nullValue(env) catch return cancel(state);
                _ = callMethod(env, readable, "push", &.{nil}) catch return cancel(state);
                state.waiting.store(false, .release);
                acknowledgeObject(state);
                return;
            }
            const value = encode(env, &state.current.?) catch return cancel(state);
            const accepted = callMethod(env, readable, "push", &.{value}) catch return cancel(state);
            if (napi.getBool(env, accepted) catch return cancel(state)) {
                acknowledgeObject(state);
            } else {
                state.waiting.store(true, .release);
                unrefFunction(env, state);
            }
        }

        fn finalizePush(env: c.napi_env, data: ?*anyopaque, _: ?*anyopaque) callconv(.c) void {
            const state: *State = @ptrCast(@alignCast(data orelse return));
            if (env != null and state.cleanup_hook_active.swap(false, .acq_rel))
                _ = c.napi_remove_env_cleanup_hook(env, environmentCleanup, state);
            if (env != null and state.stream_ref != null) _ = c.napi_delete_reference(env, state.stream_ref);
            release(state);
        }

        fn environmentCleanup(data: ?*anyopaque) callconv(.c) void {
            const state: *State = @ptrCast(@alignCast(data orelse return));
            state.cleanup_hook_active.store(false, .release);
            cancel(state);
            releaseFunction(state, true);
        }

        fn releaseFunction(state: *State, abort: bool) void {
            retain(state);
            defer release(state);
            if (state.function_release_claimed.swap(true, .acq_rel)) return;
            state.function_referenced.store(false, .release);
            _ = c.napi_release_threadsafe_function(state.function, if (abort) c.napi_tsfn_abort else c.napi_tsfn_release);
        }

        fn refFunction(env: c.napi_env, state: *State) bool {
            if (state.function_release_claimed.load(.acquire)) return false;
            if (state.function_referenced.swap(true, .acq_rel)) return true;
            if (c.napi_ref_threadsafe_function(env, state.function) == c.napi_ok) return true;
            state.function_referenced.store(false, .release);
            return false;
        }

        fn unrefFunction(env: c.napi_env, state: *State) void {
            if (state.function_release_claimed.load(.acquire)) {
                state.function_referenced.store(false, .release);
                return;
            }
            if (!state.function_referenced.swap(false, .acq_rel)) return;
            _ = c.napi_unref_threadsafe_function(env, state.function);
        }

        fn requestCancellation(state: *State) void {
            if (!state.closed.swap(true, .acq_rel)) state.cancel_context(&state.context);
        }

        fn cancel(state: *State) void {
            requestCancellation(state);
            state.waiting.store(false, .release);
            acknowledgeObject(state);
        }
    };
}

fn passthroughDestroy(env: c.napi_env, info: c.napi_callback_info) callconv(.c) c.napi_value {
    var storage: [2]c.napi_value = undefined;
    var this_arg: c.napi_value = null;
    var data: ?*anyopaque = null;
    const args = callbackInfo(env, info, &storage, &this_arg, &data) catch return null;
    if (args.len == 2) _ = callFunction(env, args[1], args[0..1]) catch return null;
    return napi.undefinedValue(env);
}

fn getU32(env: c.napi_env, value: c.napi_value) !u32 {
    var result: u32 = 0;
    try napi.check(env, c.napi_get_value_uint32(env, value, &result));
    return result;
}

fn finalizeStream(env: c.napi_env, data: ?*anyopaque, hint: ?*anyopaque) callconv(.c) void {
    _ = env;
    _ = hint;
    const state: *NativeStream = @ptrCast(@alignCast(data orelse return));
    cancelStream(state);
    acknowledge(state);
    releaseStream(state);
}

fn nativeDestroy(env: c.napi_env, info: c.napi_callback_info) callconv(.c) c.napi_value {
    var storage: [2]c.napi_value = undefined;
    var this_arg: c.napi_value = null;
    var context_ptr: ?*anyopaque = null;
    const args = callbackInfo(env, info, &storage, &this_arg, &context_ptr) catch return null;
    const state: *NativeStream = @ptrCast(@alignCast(context_ptr orelse return null));
    cancelStream(state);
    if (state.kind == .push) unrefPushFunction(env, state);
    acknowledge(state);
    if (args.len == 2) _ = callFunction(env, args[1], args[0..1]) catch return null;
    return napi.undefinedValue(env);
}

const PullWork = struct {
    work: c.napi_async_work = null,
    stream_ref: c.napi_ref = null,
    source: *NativeStream,
    input: ?ByteChunk = null,
    output: ?ByteChunk = null,
    eof: bool = false,
    failed: bool = false,
};

fn nativeRead(env: c.napi_env, info: c.napi_callback_info) callconv(.c) c.napi_value {
    var storage: [1]c.napi_value = undefined;
    var stream: c.napi_value = null;
    var context_ptr: ?*anyopaque = null;
    _ = callbackInfo(env, info, &storage, &stream, &context_ptr) catch return null;
    const source: *NativeStream = @ptrCast(@alignCast(context_ptr orelse return null));
    if (source.kind == .push) {
        if (source.waiting.swap(false, .acq_rel)) {
            if (!refPushFunction(env, source)) return destroyWithMessage(env, stream, "could not resume native push stream");
            acknowledge(source);
        }
        return napi.undefinedValue(env);
    }
    if (source.closed.load(.acquire) or source.ended.load(.acquire)) return napi.undefinedValue(env);
    if (source.reading.cmpxchgStrong(false, true, .acq_rel, .acquire) != null) return napi.undefinedValue(env);
    const state = allocator.create(PullWork) catch {
        source.reading.store(false, .release);
        return destroyWithMessage(env, stream, "native allocation failed");
    };
    state.* = .{ .source = source };
    if (c.napi_create_reference(env, stream, 1, &state.stream_ref) != c.napi_ok) {
        source.reading.store(false, .release);
        allocator.destroy(state);
        return null;
    }
    const name = napi.makeString(env, "native-stream-pull") catch {
        source.reading.store(false, .release);
        _ = c.napi_delete_reference(env, state.stream_ref);
        allocator.destroy(state);
        return null;
    };
    if (c.napi_create_async_work(env, null, name, executePull, completePull, state, &state.work) != c.napi_ok or c.napi_queue_async_work(env, state.work) != c.napi_ok) {
        if (state.work != null) _ = c.napi_delete_async_work(env, state.work);
        _ = c.napi_delete_reference(env, state.stream_ref);
        source.reading.store(false, .release);
        allocator.destroy(state);
        return destroyWithMessage(env, stream, "could not queue native pull");
    }
    return napi.undefinedValue(env);
}

fn executePull(env: c.napi_env, data: ?*anyopaque) callconv(.c) void {
    _ = env;
    const state: *PullWork = @ptrCast(@alignCast(data.?));
    const input = state.source.source.?.next(state.source.source.?.context, allocator) catch {
        state.failed = true;
        return;
    };
    if (input == null) {
        state.eof = true;
        return;
    }
    state.input = input;
    state.output = state.source.transform.apply(state.source.transform.context, allocator, state.input.?.value) catch {
        state.failed = true;
        return;
    };
}

fn completePull(env: c.napi_env, status: c.napi_status, data: ?*anyopaque) callconv(.c) void {
    const state: *PullWork = @ptrCast(@alignCast(data.?));
    defer allocator.destroy(state);
    defer if (state.output) |*output| output.deinit();
    defer if (state.input) |*input| input.deinit();
    defer _ = c.napi_delete_reference(env, state.stream_ref);
    defer _ = c.napi_delete_async_work(env, state.work);
    defer state.source.reading.store(false, .release);
    var stream: c.napi_value = null;
    if (c.napi_get_reference_value(env, state.stream_ref, &stream) != c.napi_ok) return;
    if (state.source.closed.load(.acquire)) return;
    if (status != c.napi_ok or state.failed) {
        _ = destroyWithMessage(env, stream, "native pull failed");
        return;
    }
    if (state.eof) {
        state.source.ended.store(true, .release);
        const value = nullValue(env) catch return;
        _ = callMethod(env, stream, "push", &.{value}) catch {};
        return;
    }
    const value = bufferValue(env, state.output.?.value) catch {
        _ = destroyWithMessage(env, stream, "could not create native stream Buffer");
        return;
    };
    _ = callMethod(env, stream, "push", &.{value}) catch {};
}

const TransformWork = struct {
    work: c.napi_async_work = null,
    stream_ref: c.napi_ref = null,
    callback_ref: c.napi_ref = null,
    source: *NativeStream,
    input: []u8,
    output: ?ByteChunk = null,
    failed: bool = false,
    return_output: bool,
};

fn nativeWrite(env: c.napi_env, info: c.napi_callback_info) callconv(.c) c.napi_value {
    return queueTransformWork(env, info, false);
}

fn nativeTransform(env: c.napi_env, info: c.napi_callback_info) callconv(.c) c.napi_value {
    return queueTransformWork(env, info, true);
}

fn queueTransformWork(env: c.napi_env, info: c.napi_callback_info, return_output: bool) c.napi_value {
    var storage: [3]c.napi_value = undefined;
    var stream: c.napi_value = null;
    var context_ptr: ?*anyopaque = null;
    const args = callbackInfo(env, info, &storage, &stream, &context_ptr) catch return null;
    const source: *NativeStream = @ptrCast(@alignCast(context_ptr orelse return null));
    if (args.len != 3) return failType(env, "native stream work requires chunk, encoding, and callback");
    const input = copyBuffer(env, args[0]) catch return callbackFailure(env, args[2], "native stream chunks must be Buffers");
    const state = allocator.create(TransformWork) catch {
        allocator.free(input);
        return callbackFailure(env, args[2], "native allocation failed");
    };
    state.* = .{ .source = source, .input = input, .return_output = return_output };
    if (c.napi_create_reference(env, stream, 1, &state.stream_ref) != c.napi_ok or c.napi_create_reference(env, args[2], 1, &state.callback_ref) != c.napi_ok) {
        if (state.stream_ref != null) _ = c.napi_delete_reference(env, state.stream_ref);
        if (state.callback_ref != null) _ = c.napi_delete_reference(env, state.callback_ref);
        destroyTransformWork(state);
        return null;
    }
    const name = napi.makeString(env, "native-stream-work") catch {
        _ = c.napi_delete_reference(env, state.callback_ref);
        _ = c.napi_delete_reference(env, state.stream_ref);
        destroyTransformWork(state);
        return null;
    };
    if (c.napi_create_async_work(env, null, name, executeTransformWork, completeTransformWork, state, &state.work) != c.napi_ok or c.napi_queue_async_work(env, state.work) != c.napi_ok) {
        if (state.work != null) _ = c.napi_delete_async_work(env, state.work);
        _ = c.napi_delete_reference(env, state.callback_ref);
        _ = c.napi_delete_reference(env, state.stream_ref);
        destroyTransformWork(state);
        return callbackFailure(env, args[2], "could not queue native stream work");
    }
    return napi.undefinedValue(env);
}

fn executeTransformWork(env: c.napi_env, data: ?*anyopaque) callconv(.c) void {
    _ = env;
    const state: *TransformWork = @ptrCast(@alignCast(data.?));
    state.output = state.source.transform.apply(state.source.transform.context, allocator, state.input) catch {
        state.failed = true;
        return;
    };
}

fn completeTransformWork(env: c.napi_env, status: c.napi_status, data: ?*anyopaque) callconv(.c) void {
    const state: *TransformWork = @ptrCast(@alignCast(data.?));
    defer destroyTransformWork(state);
    defer _ = c.napi_delete_reference(env, state.callback_ref);
    defer _ = c.napi_delete_reference(env, state.stream_ref);
    defer _ = c.napi_delete_async_work(env, state.work);
    var callback: c.napi_value = null;
    if (c.napi_get_reference_value(env, state.callback_ref, &callback) != c.napi_ok) return;
    const nil = nullValue(env) catch return;
    if (status != c.napi_ok or state.failed) {
        const failure = errorValue(env, "native stream work failed") catch return;
        _ = callFunction(env, callback, &.{failure}) catch {};
        return;
    }
    if (state.return_output) {
        const value = bufferValue(env, state.output.?.value) catch return;
        _ = callFunction(env, callback, &.{ nil, value }) catch {};
    } else {
        _ = callFunction(env, callback, &.{nil}) catch {};
    }
}

fn destroyTransformWork(state: *TransformWork) void {
    if (state.output) |*output| output.deinit();
    allocator.free(state.input);
    allocator.destroy(state);
}

fn startPush(env: c.napi_env, stream: c.napi_value, source: *NativeStream) !void {
    try napi.check(env, c.napi_create_reference(env, stream, 1, &source.stream_ref));
    const name = try napi.makeString(env, "native-stream-push");
    napi.check(env, c.napi_create_threadsafe_function(env, null, null, name, 1, 1, source, finalizePush, source, callPushOnJs, &source.function)) catch {
        _ = c.napi_delete_reference(env, source.stream_ref);
        return error.NodeApiFailure;
    };
    retainStream(source);
    napi.check(env, c.napi_add_env_cleanup_hook(env, pushEnvironmentCleanup, source)) catch {
        releasePushFunction(source, true);
        return error.NodeApiFailure;
    };
    source.cleanup_hook_active.store(true, .release);
    resumePush(source);
}

fn pushWorker(source: *NativeStream) void {
    defer releaseStream(source);
    defer _ = c.napi_release_threadsafe_function(source.function, c.napi_tsfn_release);
    if (source.closed.load(.acquire)) return;
    var input = source.source.?.next(source.source.?.context, allocator) catch result: {
        source.producer_failed = true;
        break :result null;
    };
    if (input) |*chunk| {
        defer chunk.deinit();
        source.current = source.transform.apply(source.transform.context, allocator, chunk.value) catch result: {
            source.producer_failed = true;
            break :result null;
        };
    }
    if (source.current == null) source.ended.store(true, .release);
    // One reserved queue slot corresponds to this single outstanding chunk.
    _ = c.napi_call_threadsafe_function(source.function, source, c.napi_tsfn_nonblocking);
}

fn callPushOnJs(env: c.napi_env, callback: c.napi_value, context: ?*anyopaque, data: ?*anyopaque) callconv(.c) void {
    _ = callback;
    _ = context;
    const source: *NativeStream = @ptrCast(@alignCast(data orelse return));
    if (env == null) return cancelPush(source);
    var stream: c.napi_value = null;
    if (c.napi_get_reference_value(env, source.stream_ref, &stream) != c.napi_ok) return cancelPush(source);
    if (source.closed.load(.acquire)) return cancelPush(source);
    if (source.producer_failed) {
        _ = destroyWithMessage(env, stream, "native push source failed");
        return cancelPush(source);
    }
    if (source.ended.load(.acquire)) {
        const nil = nullValue(env) catch return cancelPush(source);
        _ = callMethod(env, stream, "push", &.{nil}) catch return cancelPush(source);
        source.waiting.store(false, .release);
        acknowledge(source);
        return;
    }
    const chunk = bufferValue(env, source.current.?.value) catch return cancelPush(source);
    const accepted = callMethod(env, stream, "push", &.{chunk}) catch return cancelPush(source);
    const keep_going = napi.getBool(env, accepted) catch return cancelPush(source);
    if (keep_going) {
        acknowledge(source);
    } else {
        source.waiting.store(true, .release);
        unrefPushFunction(env, source);
    }
}

fn finalizePush(env: c.napi_env, data: ?*anyopaque, hint: ?*anyopaque) callconv(.c) void {
    _ = hint;
    const source: *NativeStream = @ptrCast(@alignCast(data orelse return));
    if (env != null and source.cleanup_hook_active.swap(false, .acq_rel))
        _ = c.napi_remove_env_cleanup_hook(env, pushEnvironmentCleanup, source);
    if (env != null and source.stream_ref != null) _ = c.napi_delete_reference(env, source.stream_ref);
    releaseStream(source);
}

fn pushEnvironmentCleanup(data: ?*anyopaque) callconv(.c) void {
    const source: *NativeStream = @ptrCast(@alignCast(data orelse return));
    source.cleanup_hook_active.store(false, .release);
    cancelPush(source);
    releasePushFunction(source, true);
}

// Base TSFN ownership/ref changes are confined to environment callbacks. Each
// production job holds its own acquired TSFN reference until it returns.
fn releasePushFunction(source: *NativeStream, abort: bool) void {
    retainStream(source);
    defer releaseStream(source);
    if (source.function_release_claimed.swap(true, .acq_rel)) return;
    source.function_referenced.store(false, .release);
    _ = c.napi_release_threadsafe_function(source.function, if (abort) c.napi_tsfn_abort else c.napi_tsfn_release);
}

fn refPushFunction(env: c.napi_env, source: *NativeStream) bool {
    if (source.function_release_claimed.load(.acquire)) return false;
    if (source.function_referenced.swap(true, .acq_rel)) return true;
    if (c.napi_ref_threadsafe_function(env, source.function) == c.napi_ok) return true;
    source.function_referenced.store(false, .release);
    return false;
}

fn unrefPushFunction(env: c.napi_env, source: *NativeStream) void {
    if (source.function_release_claimed.load(.acquire)) {
        source.function_referenced.store(false, .release);
        return;
    }
    if (!source.function_referenced.swap(false, .acq_rel)) return;
    _ = c.napi_unref_threadsafe_function(env, source.function);
}

fn cancelPush(source: *NativeStream) void {
    cancelStream(source);
    source.waiting.store(false, .release);
    acknowledge(source);
}

fn callbackFailure(env: c.napi_env, callback: c.napi_value, message: [:0]const u8) c.napi_value {
    const failure = errorValue(env, message) catch return null;
    _ = callFunction(env, callback, &.{failure}) catch return null;
    return napi.undefinedValue(env);
}

fn destroyWithMessage(env: c.napi_env, stream: c.napi_value, message: [:0]const u8) c.napi_value {
    const failure = errorValue(env, message) catch return null;
    _ = callMethod(env, stream, "destroy", &.{failure}) catch return null;
    return napi.undefinedValue(env);
}

fn failType(env: c.napi_env, message: [:0]const u8) c.napi_value {
    napi.throwTypeError(env, message);
    return null;
}

fn failError(env: c.napi_env, message: [:0]const u8) c.napi_value {
    napi.throwError(env, message);
    return null;
}

// Retain Node's destroy callback and its error until native work is quiescent.
// All methods run on the environment thread; no worker waits for JavaScript.
const PendingDestroy = struct {
    reference: c.napi_ref = null,
    fn save(self: *PendingDestroy, env: c.napi_env, args: []const c.napi_value) !void {
        if (self.reference != null) return error.DestroyAlreadyPending;
        const values = try napi.array(env, 2);
        try napi.setElement(env, values, 0, args[0]);
        try napi.setElement(env, values, 1, args[1]);
        try napi.check(env, c.napi_create_reference(env, values, 1, &self.reference));
    }
    fn complete(self: *PendingDestroy, env: c.napi_env) !void {
        if (self.reference == null) return;
        var values: c.napi_value = null;
        try napi.check(env, c.napi_get_reference_value(env, self.reference, &values));
        self.clear(env);
        const failure = try napi.getElement(env, values, 0);
        const callback = try napi.getElement(env, values, 1);
        _ = try callFunction(env, callback, &.{failure});
    }
    fn clear(self: *PendingDestroy, env: c.napi_env) void {
        if (self.reference != null) _ = c.napi_delete_reference(env, self.reference);
        self.reference = null;
    }
};

/// Callback-completed native object chunks; no parked async-work worker and no
/// JavaScript Promise adapter. Work supplies start(token, done, context), finish,
/// and deinit. Native Stream destruction requests the active operation's token.
pub fn AsyncObjectBindings(comptime Work: type, comptime prepare: fn (c.napi_env, c.napi_value) anyerror!Work) type {
    return struct {
        const cancellation = execution.cancellation;
        const State = struct {
            owners: usize = 1,
            token: ?*const cancellation.Token = null,
            closed: bool = false,
            writing: bool = false,
            completion_ref: c.napi_ref = null,
            pending_destroy: PendingDestroy = .{},
        };
        fn Adapter(comptime output: bool) type {
            return struct {
                const Chunk = struct {
                    native: Work,
                    signal: c.napi_value = null,
                    keep_alive: c.napi_value,
                    pub fn start(self: *@This(), token: *const cancellation.Token, done: *const fn (?*anyopaque, u32) callconv(.c) void, context: ?*anyopaque) u32 {
                        return self.native.start(token, done, context);
                    }
                    pub fn finish(self: *@This(), env: c.napi_env) !c.napi_value {
                        const value = try self.native.finish(env);
                        return if (output) value else napi.undefinedValue(env);
                    }
                    pub fn deinit(self: *@This()) void {
                        self.native.deinit();
                    }
                };
                const Async = napi.AsyncBindings(Chunk, prepareChunk, "native async stream chunk", 16384);
                fn prepareChunk(env: c.napi_env, info: c.napi_callback_info) !Chunk {
                    var storage: [3]c.napi_value = undefined;
                    var this: c.napi_value = null;
                    var context: ?*anyopaque = null;
                    const args = try callbackInfo(env, info, &storage, &this, &context);
                    return .{ .native = try prepare(env, args[0]), .keep_alive = this };
                }
                fn chunk(env: c.napi_env, info: c.napi_callback_info) callconv(.c) c.napi_value {
                    var storage: [3]c.napi_value = undefined;
                    var this: c.napi_value = null;
                    var context: ?*anyopaque = null;
                    const args = callbackInfo(env, info, &storage, &this, &context) catch return null;
                    if (args.len != 3) return failType(env, "native stream chunk requires callback");
                    const state: *State = @ptrCast(@alignCast(context.?));
                    if (state.closed) return callbackFailure(env, args[2], "native stream is closed");
                    if (state.writing) return callbackFailure(env, args[2], "native stream chunk already active");
                    if (state.token) |token| cancellation.native_cancel_release(token);
                    state.token = cancellation.native_cancel_create();
                    const completion = retainedCallback(env, "nativeChunkCompleted", completed, state) catch return null;
                    if (c.napi_create_reference(env, args[2], 1, &state.completion_ref) != c.napi_ok) return null;
                    state.writing = true;
                    return Async.callback(env, info, completion, state.token);
                }
            };
        }
        pub fn transform(env: c.napi_env, info: c.napi_callback_info) callconv(.c) c.napi_value {
            return create(env, info, true);
        }
        pub fn writable(env: c.napi_env, info: c.napi_callback_info) callconv(.c) c.napi_value {
            return create(env, info, false);
        }
        fn create(env: c.napi_env, info: c.napi_callback_info, comptime output: bool) c.napi_value {
            return initialize(env, info, output) catch |err| {
                if (err != error.PendingException) napi.throwTypeError(env, @errorName(err));
                return null;
            };
        }
        fn initialize(env: c.napi_env, info: c.napi_callback_info, comptime output: bool) !c.napi_value {
            var storage: [3]c.napi_value = undefined;
            const args = try napi.arguments(env, info, &storage);
            if (args.len < 1 or args.len > 3) return error.InvalidArgument;
            const state = try allocator.create(State);
            state.* = .{};
            errdefer {
                requestCancel(state);
                release(state);
            }
            const options = try napi.object(env);
            try napi.setNamedProperty(env, options, "objectMode", try napi.makeBool(env, true));
            if (args.len >= 2) try napi.setNamedProperty(env, options, "highWaterMark", args[1]);
            if (args.len == 3) try napi.setNamedProperty(env, options, "signal", args[2]);
            try napi.setNamedProperty(env, options, if (output) "transform" else "write", try retainedCallback(env, "nativeAsyncChunk", Adapter(output).chunk, state));
            try napi.setNamedProperty(env, options, "destroy", try retainedCallback(env, "nativeAsyncDestroy", destroy, state));
            var result: c.napi_value = null;
            try napi.check(env, c.napi_new_instance(env, args[0], 1, &options, &result));
            try napi.check(env, c.napi_wrap(env, result, state, finalize, null, null));
            return result;
        }
        fn completed(env: c.napi_env, info: c.napi_callback_info) callconv(.c) c.napi_value {
            var storage: [2]c.napi_value = undefined;
            var this: c.napi_value = null;
            var context: ?*anyopaque = null;
            const args = callbackInfo(env, info, &storage, &this, &context) catch return null;
            const state: *State = @ptrCast(@alignCast(context.?));
            var callback: c.napi_value = null;
            if (c.napi_get_reference_value(env, state.completion_ref, &callback) != c.napi_ok) return null;
            _ = c.napi_delete_reference(env, state.completion_ref);
            state.completion_ref = null;
            state.writing = false;
            _ = callFunction(env, callback, args) catch return null;
            state.pending_destroy.complete(env) catch return null;
            return napi.undefinedValue(env);
        }
        fn requestCancel(state: *State) void {
            if (state.closed) return;
            state.closed = true;
            if (state.token) |token| _ = cancellation.native_cancel_request(token);
        }
        fn destroy(env: c.napi_env, info: c.napi_callback_info) callconv(.c) c.napi_value {
            var storage: [2]c.napi_value = undefined;
            var this: c.napi_value = null;
            var context: ?*anyopaque = null;
            const args = callbackInfo(env, info, &storage, &this, &context) catch return null;
            const state: *State = @ptrCast(@alignCast(context.?));
            if (args.len != 2) return failType(env, "native destroy requires callback");
            if (state.writing) {
                state.pending_destroy.save(env, args) catch return null;
                requestCancel(state);
                return napi.undefinedValue(env);
            }
            requestCancel(state);
            if (args.len == 2) _ = callFunction(env, args[1], args[0..1]) catch return null;
            return napi.undefinedValue(env);
        }
        fn finalize(env: c.napi_env, context: ?*anyopaque, _: ?*anyopaque) callconv(.c) void {
            const state: *State = @ptrCast(@alignCast(context.?));
            requestCancel(state);
            if (env != null) {
                state.pending_destroy.clear(env);
                if (state.completion_ref != null) _ = c.napi_delete_reference(env, state.completion_ref);
            }
            release(state);
        }
        fn retainedCallback(env: c.napi_env, name: [:0]const u8, function: c.napi_callback, state: *State) !c.napi_value {
            const value = try callbackValue(env, name, function, state);
            try napi.check(env, c.napi_add_finalizer(env, value, state, callbackFinalized, null, null));
            state.owners += 1;
            return value;
        }
        fn callbackFinalized(_: c.napi_env, context: ?*anyopaque, _: ?*anyopaque) callconv(.c) void {
            release(@ptrCast(@alignCast(context.?)));
        }
        fn release(state: *State) void {
            state.owners -= 1;
            if (state.owners != 0) return;
            if (state.token) |token| cancellation.native_cancel_release(token);
            allocator.destroy(state);
        }
    };
}

/// Demand-driven native asynchronous source. next prepares an owned Work or EOF;
/// Work.start completes through a native callback and Work.finish encodes a chunk.
/// One chunk is in flight; Work may itself use arbitrary native parallelism.
pub fn AsyncObjectSourceBindings(comptime Context: type, comptime Work: type, comptime prepare: fn (c.napi_env, c.napi_callback_info) anyerror!ObjectSourceOptions(Context), comptime next: fn (*Context) anyerror!?Work, comptime deinit_context: fn (*Context) void) type {
    return struct {
        const cancellation = execution.cancellation;
        const State = struct {
            owners: std.atomic.Value(usize) = .init(1),
            context: Context,
            cancel_context: *const fn (*Context) void,
            token: ?*const cancellation.Token = null,
            stream_ref: c.napi_ref = null,
            callback_ref: c.napi_ref = null,
            closed: bool = false,
            reading: bool = false,
            pending_destroy: PendingDestroy = .{},
            push_mode: bool,
        };
        const Chunk = struct {
            state: *State,
            native: ?Work,
            signal: c.napi_value = null,
            keep_alive: c.napi_value,
            pub fn start(self: *@This(), token: *const cancellation.Token, done: *const fn (?*anyopaque, u32) callconv(.c) void, context: ?*anyopaque) u32 {
                if (self.native) |*work| return work.start(token, done, context);
                done(context, 0);
                return 0;
            }
            pub fn finish(self: *@This(), env: c.napi_env) !c.napi_value {
                if (self.native) |*work| return work.finish(env);
                return nullValue(env);
            }
            pub fn deinit(self: *@This()) void {
                if (self.native) |*work| work.deinit();
                release(self.state);
            }
        };
        const Async = napi.AsyncBindings(Chunk, prepareChunk, "native async source", 16384);
        fn prepareChunk(env: c.napi_env, info: c.napi_callback_info) !Chunk {
            var context: ?*anyopaque = null;
            try napi.check(env, c.napi_get_cb_info(env, info, null, null, null, &context));
            const state: *State = @ptrCast(@alignCast(context.?));
            var stream: c.napi_value = null;
            try napi.check(env, c.napi_get_reference_value(env, state.stream_ref, &stream));
            const work = try next(&state.context);
            _ = state.owners.fetchAdd(1, .monotonic);
            return .{ .state = state, .native = work, .keep_alive = stream };
        }
        pub fn pull(env: c.napi_env, info: c.napi_callback_info) callconv(.c) c.napi_value {
            return create(env, info, false);
        }
        pub fn push(env: c.napi_env, info: c.napi_callback_info) callconv(.c) c.napi_value {
            return create(env, info, true);
        }
        fn create(env: c.napi_env, info: c.napi_callback_info, push_mode: bool) c.napi_value {
            return initialize(env, info, push_mode) catch |err| {
                if (err != error.PendingException) napi.throwTypeError(env, @errorName(err));
                return null;
            };
        }
        fn initialize(env: c.napi_env, info: c.napi_callback_info, push_mode: bool) !c.napi_value {
            var source = try prepare(env, info);
            const state = allocator.create(State) catch |err| {
                deinit_context(&source.context);
                return err;
            };
            state.* = .{ .context = source.context, .cancel_context = source.cancel, .push_mode = push_mode };
            errdefer {
                cancel(state);
                clearRefs(env, state);
                release(state);
            }
            const options = try napi.object(env);
            try napi.setNamedProperty(env, options, "objectMode", try napi.makeBool(env, true));
            if (source.high_water_mark) |watermark| try napi.setNamedProperty(env, options, "highWaterMark", watermark);
            try napi.setNamedProperty(env, options, "read", try retainedCallback(env, "nativeAsyncRead", read, state));
            try napi.setNamedProperty(env, options, "destroy", try retainedCallback(env, "nativeAsyncSourceDestroy", destroy, state));
            const callback = try retainedCallback(env, "nativeAsyncProduced", produced, state);
            try napi.check(env, c.napi_create_reference(env, callback, 1, &state.callback_ref));
            var result: c.napi_value = null;
            try napi.check(env, c.napi_new_instance(env, source.constructor, 1, &options, &result));
            // Weak between requests; each accepted Chunk retains its stream.
            try napi.check(env, c.napi_create_reference(env, result, 0, &state.stream_ref));
            try napi.check(env, c.napi_wrap(env, result, state, finalize, null, null));
            return result;
        }
        fn read(env: c.napi_env, info: c.napi_callback_info) callconv(.c) c.napi_value {
            var context: ?*anyopaque = null;
            if (c.napi_get_cb_info(env, info, null, null, null, &context) != c.napi_ok) return null;
            return resumeSource(env, info, @ptrCast(@alignCast(context.?)));
        }
        fn resumeSource(env: c.napi_env, info: c.napi_callback_info, state: *State) c.napi_value {
            if (state.closed or state.reading) return napi.undefinedValue(env);
            state.reading = true;
            if (state.token) |token| cancellation.native_cancel_release(token);
            state.token = cancellation.native_cancel_create();
            var callback: c.napi_value = null;
            if (c.napi_get_reference_value(env, state.callback_ref, &callback) != c.napi_ok) return null;
            return Async.callback(env, info, callback, state.token);
        }
        fn produced(env: c.napi_env, info: c.napi_callback_info) callconv(.c) c.napi_value {
            var storage: [2]c.napi_value = undefined;
            var this: c.napi_value = null;
            var context: ?*anyopaque = null;
            const args = callbackInfo(env, info, &storage, &this, &context) catch return null;
            const state: *State = @ptrCast(@alignCast(context.?));
            state.reading = false;
            if (state.closed) {
                state.pending_destroy.complete(env) catch return null;
                return napi.undefinedValue(env);
            }
            var stream: c.napi_value = null;
            if (c.napi_get_reference_value(env, state.stream_ref, &stream) != c.napi_ok or stream == null) return null;
            var kind: c.napi_valuetype = undefined;
            if (c.napi_typeof(env, args[0], &kind) != c.napi_ok) return null;
            if (kind != c.napi_null) return callMethod(env, stream, "destroy", args[0..1]) catch null;
            const accepted = callMethod(env, stream, "push", args[1..2]) catch return null;
            if (c.napi_typeof(env, args[1], &kind) != c.napi_ok) return null;
            if (kind == c.napi_null) {
                cancel(state);
                return napi.undefinedValue(env);
            }
            if (state.push_mode and (napi.getBool(env, accepted) catch false)) return resumeSource(env, info, state);
            return napi.undefinedValue(env);
        }
        fn cancel(state: *State) void {
            if (state.closed) return;
            state.closed = true;
            if (state.token) |token| _ = cancellation.native_cancel_request(token);
            state.cancel_context(&state.context);
        }
        fn destroy(env: c.napi_env, info: c.napi_callback_info) callconv(.c) c.napi_value {
            var storage: [2]c.napi_value = undefined;
            var this: c.napi_value = null;
            var context: ?*anyopaque = null;
            const args = callbackInfo(env, info, &storage, &this, &context) catch return null;
            const state: *State = @ptrCast(@alignCast(context.?));
            if (args.len != 2) return failType(env, "native destroy requires callback");
            if (state.reading) {
                state.pending_destroy.save(env, args) catch return null;
                cancel(state);
                return napi.undefinedValue(env);
            }
            cancel(state);
            if (args.len == 2) _ = callFunction(env, args[1], args[0..1]) catch return null;
            return napi.undefinedValue(env);
        }
        fn retainedCallback(env: c.napi_env, name: [:0]const u8, callback: c.napi_callback, state: *State) !c.napi_value {
            const value = try callbackValue(env, name, callback, state);
            try napi.check(env, c.napi_add_finalizer(env, value, state, callbackFinalized, null, null));
            _ = state.owners.fetchAdd(1, .monotonic);
            return value;
        }
        fn callbackFinalized(_: c.napi_env, context: ?*anyopaque, _: ?*anyopaque) callconv(.c) void {
            release(@ptrCast(@alignCast(context.?)));
        }
        fn clearRefs(env: c.napi_env, state: *State) void {
            state.pending_destroy.clear(env);
            if (state.stream_ref != null) _ = c.napi_delete_reference(env, state.stream_ref);
            if (state.callback_ref != null) _ = c.napi_delete_reference(env, state.callback_ref);
            state.stream_ref = null;
            state.callback_ref = null;
        }
        fn finalize(env: c.napi_env, context: ?*anyopaque, _: ?*anyopaque) callconv(.c) void {
            const state: *State = @ptrCast(@alignCast(context.?));
            cancel(state);
            if (env != null) clearRefs(env, state);
            release(state);
        }
        fn release(state: *State) void {
            if (state.owners.fetchSub(1, .acq_rel) != 1) return;
            if (state.token) |token| cancellation.native_cancel_release(token);
            deinit_context(&state.context);
            allocator.destroy(state);
        }
    };
}
