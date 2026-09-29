const std = @import("std");
const outcomes = @import("outcome.zig");
const rt = @import("internal/runtime.zig");
const diagnostics = @import("internal/diagnostics.zig");
const scopes = @import("scope.zig");
const providers = @import("services.zig");
const execution = @import("executor.zig");
const asynchronous = @import("async.zig");
const primitives = @import("primitives.zig");

const EffectTypeMember = enum { Success, Error, Environment };

fn functionInfo(comptime callback: anytype, comptime operation: diagnostics.Operation) std.builtin.Type.Fn {
    if (@typeInfo(@TypeOf(callback)) != .@"fn")
        @compileError(operation.label() ++ diagnostics.expected_known_function);
    const info = @typeInfo(@TypeOf(callback)).@"fn";
    if (info.is_generic or info.return_type == null)
        @compileError(operation.label() ++ diagnostics.expected_typed_callback);
    return info;
}

fn unaryResult(comptime callback: anytype, comptime Input: type, comptime operation: diagnostics.Operation) type {
    const info = functionInfo(callback, operation);
    const expected = @typeInfo(fn (Input) void).@"fn";
    if (info.params.len != expected.params.len)
        @compileError(operation.label() ++ diagnostics.expected_callback_input ++ @typeName(Input) ++ diagnostics.explicit_callback_conversion);
    inline for (info.params, expected.params) |actual, required| {
        if (actual.type != required.type)
            @compileError(operation.label() ++ diagnostics.expected_callback_input ++ @typeName(Input) ++ diagnostics.explicit_callback_conversion);
    }
    return info.return_type.?;
}

fn continuationResult(comptime callback: anytype, comptime Input: type, comptime R: type, comptime operation: diagnostics.Operation) type {
    const info = functionInfo(callback, operation);
    const expected = @typeInfo(fn (Input, *const R) void).@"fn";
    if (info.params.len != expected.params.len) return unaryResult(callback, Input, operation);
    inline for (info.params, expected.params) |actual, required| {
        if (actual.type != required.type)
            @compileError(operation.label() ++ diagnostics.expected_callback_input ++ @typeName(Input) ++ diagnostics.continuation_environment ++ @typeName(R));
    }
    return info.return_type.?;
}

inline fn callContinuation(comptime callback: anytype, value: anytype, env: anytype) @typeInfo(@TypeOf(callback)).@"fn".return_type.? {
    const expected = @typeInfo(fn (@TypeOf(value), @TypeOf(env)) void).@"fn";
    if (comptime @typeInfo(@TypeOf(callback)).@"fn".params.len == expected.params.len)
        return @call(.always_inline, callback, .{ value, env });
    return @call(.always_inline, callback, .{value});
}

fn requireEffect(comptime T: type, comptime E: type, comptime R: type, comptime operation: diagnostics.Operation) void {
    if (@typeInfo(T) != .@"struct")
        @compileError(operation.label() ++ diagnostics.expected_effect);
    inline for (std.meta.tags(EffectTypeMember)) |member| {
        if (!@hasDecl(T, @tagName(member)))
            @compileError(operation.label() ++ diagnostics.expected_effect);
    }
    if (T.Error != E)
        @compileError(operation.label() ++ diagnostics.error_type_mismatch ++ @typeName(E) ++ diagnostics.actual_type ++ @typeName(T.Error) ++ diagnostics.explicit_error_mapping);
    if (T.Environment != R)
        @compileError(operation.label() ++ diagnostics.environment_mismatch ++ @typeName(R) ++ diagnostics.actual_type ++ @typeName(T.Environment) ++ diagnostics.explicit_provision);
    if (T != Effect(T.Success, E, R))
        @compileError(operation.label() ++ diagnostics.expected_effect_contract);
}

pub fn Effect(comptime A: type, comptime E: type, comptime R: type) type {
    return struct {
        const Self = @This();
        pub const Success = A;
        pub const Error = E;
        pub const Environment = R;

        const Sequence = struct { items: *const anyopaque, len: usize, concurrency: ?usize = null };
        const Value = union(enum(u8)) { none, success: A, failure: E, defect: outcomes.Defect, borrowed: *const anyopaque, sequence: Sequence, duration: std.Io.Duration, annotations: []const providers.Annotation, terminal: outcomes.Exit(A, E) };
        const Ready = *const fn (*const R, Value, *rt.State) outcomes.Exit(A, E);
        const Start = *const fn (*rt.State, *Job) rt.Step;
        const Resolve = *const fn (*const R, Value) rt.Services;
        const Reduce = *const fn (*const R, Value, *rt.State) Self;
        const EntryFunction = *const fn (*const R, Value) outcomes.Exit(A, E);

        // Static construction is allocation-free. Known combinators generate
        // code; runtime leaf values live inline. Owned runtime composition
        // allocates typed nodes explicitly through R, without a capture buffer.
        value: Value = .none,
        ready: ?Ready = null,
        start: ?Start = null,
        resolve: ?Resolve = null,
        reduce: ?Reduce = null,
        pure_reduce: ?Reduce = null,
        entry: ?EntryFunction = null,
        delivery: rt.Delivery = .ordinary,
        ownership: ?*rt.DescriptionNode = null,

        const Job = struct {
            program: Self,
            inherited: rt.Services,
            // The initial description is borrowed from the consumer. Only
            // descriptions returned by a running factory/continuation transfer
            // ownership to this job, and must be reclaimed before the next one.
            owned: ?*rt.DescriptionNode = null,
            env: *const R,
            sink: rt.Sink(A, E),

            fn replace(self: *Job, produced: Self) void {
                rt.DescriptionNode.deinit(self.owned);
                self.owned = produced.ownership;
                self.program = produced;
            }

            fn finish(self: *Job, evaluated: outcomes.Exit(A, E)) rt.Step {
                rt.DescriptionNode.deinit(self.owned);
                self.owned = null;
                return self.sink.deliver(evaluated);
            }

            fn step(self: *Job) rt.Step {
                return .{ .data = self, .advance = advance };
            }

            fn tail(self: *Job, state: *rt.State) rt.Step {
                if (self.program.ready != null) return advance(state, self);
                return self.step();
            }

            fn advance(state: *rt.State, data: ?*anyopaque) rt.Step {
                const self: *Job = @ptrCast(@alignCast(data.?));
                state.services = self.program.services(self.env).inherit(self.inherited);
                state.protected_failure = false;
                if (self.program.delivery == .ordinary and state.cancelled()) return self.finish(.interrupted);
                const io = state.services.ioValue();
                const old = if (state.protection != 0 and io != null) io.?.swapCancelProtection(.blocked) else null;
                defer if (old) |protection| {
                    _ = io.?.swapCancelProtection(protection);
                };
                if (self.program.reduce) |reduce| {
                    self.replace(reduce(self.env, self.program.value, state));
                    return self.tail(state);
                }
                if (self.program.ready) |ready| {
                    return self.finish(ready(self.env, self.program.value, state));
                }
                return self.program.start.?(state, self);
            }
        };

        inline fn services(self: Self, env: *const R) rt.Services {
            if (self.resolve) |resolve| return resolve(env, self.value);
            return rt.services(R, env);
        }

        fn withEntry(comptime program: Self) Self {
            // Preserve the known composition at the root without introducing
            // another evaluator. Application callbacks remain lazy.
            var described = program;
            described.entry = struct {
                fn enter(env: *const R, _: Value) outcomes.Exit(A, E) {
                    return runDriver(true, env, program);
                }
            }.enter;
            return described;
        }

        pub fn succeed(value: A) Self {
            if (comptime outcomes.ownsPayload(A)) @compileError(diagnostics.reusable_owned_success_requires_a_lazy_sync_fromoutcome_factory_or_a_scoped);
            return succeedMoved(value);
        }

        inline fn succeedMoved(value: A) Self {
            var described = terminalExit(.{ .success = value });
            described.delivery = .ordinary;
            return described;
        }

        pub fn fail(err: E) Self {
            if (comptime outcomes.ownsPayload(E)) @compileError(diagnostics.reusable_owned_failure_requires_a_lazy_fromoutcome_factory);
            var described = terminalExit(.{ .fail = err });
            described.delivery = .ordinary;
            return described;
        }

        pub fn die(defect: outcomes.Defect) Self {
            var described = terminalExit(.{ .die = defect });
            described.delivery = .ordinary;
            return described;
        }

        pub fn interrupted() Self {
            var described = terminalExit(.interrupted);
            described.delivery = .ordinary;
            return described;
        }

        pub inline fn service(comptime project: anytype) Self {
            if (unaryResult(project, *const R, .service) != A)
                @compileError(diagnostics.service_result ++ @typeName(A));
            // The callback runs later; its adapter and description are already
            // completely known. Materialize that description at comptime even
            // when construction occurs inside a runtime continuation.
            return comptime .{ .ready = struct {
                fn ready(env: *const R, _: Value, _: *rt.State) outcomes.Exit(A, E) {
                    return .{ .success = project(env) };
                }
            }.ready };
        }

        /// The callback runs inline when reached, and anew on each run.
        pub inline fn sync(comptime callback: anytype) Self {
            if (unaryResult(callback, *const R, .sync) != A)
                @compileError(diagnostics.sync_result ++ @typeName(A));
            return comptime service(callback);
        }

        pub fn fromBorrowed(comptime Capture: type, capture: *const Capture, comptime callback: fn (*const Capture, *const R) outcomes.Exit(A, E)) Self {
            return .{ .value = .{ .borrowed = capture }, .ready = struct {
                fn ready(env: *const R, stored: Value, _: *rt.State) outcomes.Exit(A, E) {
                    const borrowed: *const Capture = @ptrCast(@alignCast(stored.borrowed));
                    return callback(borrowed, env);
                }
            }.ready };
        }

        /// Supplies the active logical token without introducing an execution
        /// context or duplicating allocator/I/O arguments outside R.
        pub fn fromCancellableBorrowed(comptime Capture: type, capture: *const Capture, comptime callback: fn (*const Capture, *const R, ?*const rt.Cancellation) outcomes.Exit(A, E)) Self {
            return .{ .value = .{ .borrowed = capture }, .ready = struct {
                fn ready(env: *const R, stored: Value, state: *rt.State) outcomes.Exit(A, E) {
                    const borrowed: *const Capture = @ptrCast(@alignCast(stored.borrowed));
                    return callback(borrowed, env, if (state.protection != 0) null else state.branch_cancellation orelse state.origin_cancellation orelse state.services.cancellation);
                }
            }.ready };
        }

        pub fn fromNativeBorrowed(comptime Capture: type, capture: *const Capture, comptime callback: fn (*const Capture, *const R) outcomes.Exit(A, E)) Self {
            comptime requireNativeEnvironment();
            return fromAsyncBorrowed(Capture, capture, struct {
                fn start(borrowed: *const Capture, env: *const R) asynchronous.Async(A, E).Start {
                    return asynchronous.Async(A, E).nativeBorrowed(env, borrowed, callback);
                }
            }.start);
        }

        pub fn fromNativeCancellableBorrowed(comptime Capture: type, capture: *const Capture, comptime callback: fn (*const Capture, *const R, ?*const rt.Cancellation) outcomes.Exit(A, E)) Self {
            comptime requireNativeEnvironment();
            return fromAsyncBorrowed(Capture, capture, struct {
                fn start(borrowed: *const Capture, env: *const R) asynchronous.Async(A, E).Start {
                    return asynchronous.Async(A, E).nativeCancellable(env, borrowed, callback);
                }
            }.start);
        }

        fn requireNativeEnvironment() void {
            if (R == void or !@hasField(R, "allocator") or @FieldType(R, "allocator") != std.mem.Allocator)
                @compileError(diagnostics.native_allocator_capability);
            if (R == void or !@hasField(R, "io") or @FieldType(R, "io") != std.Io)
                @compileError(diagnostics.native_future_io_capability);
        }

        // Finalizers already own their resource and continuation storage.
        // Suspend on the shared kernel without allocating another native task;
        // cleanup must still run when the allocator has been exhausted.
        fn cleanupBorrowed(comptime Capture: type, capture: *const Capture, comptime callback: fn (*const Capture, *const R) outcomes.Exit(A, E)) Self {
            const Adapter = struct {
                fn start(_: *rt.State, parent: *Job) rt.Step {
                    return .{ .data = parent, .advance = null, .suspended = cleanup };
                }
                fn cleanup(state: *rt.State, data: ?*anyopaque) rt.Step {
                    const parent: *Job = @ptrCast(@alignCast(data.?));
                    const borrowed: *const Capture = @ptrCast(@alignCast(parent.program.value.borrowed));
                    const io = state.services.ioValue();
                    const previous = if (io != null and state.protection != 0) io.?.swapCancelProtection(.blocked) else null;
                    defer if (previous) |protection| {
                        _ = io.?.swapCancelProtection(protection);
                    };
                    return parent.finish(callback(borrowed, parent.env));
                }
            };
            return .{ .value = .{ .borrowed = @ptrCast(capture) }, .start = Adapter.start };
        }

        fn captureOwned(allocator: std.mem.Allocator, capture: anytype, comptime suspended: bool, comptime callback: fn (*const @TypeOf(capture), *const R, ?*const rt.Cancellation) outcomes.Exit(A, E)) Self {
            const Capture = @TypeOf(capture);
            const Node = struct {
                header: rt.DescriptionNode,
                capture: Capture,
                fn destroy(header: *rt.DescriptionNode) void {
                    const node: *@This() = @alignCast(@fieldParentPtr("header", header));
                    outcomes.discardPayload(&node.capture);
                    header.allocator.destroy(node);
                }
                fn origin(environment: *const anyopaque, _: *const anyopaque) rt.Services {
                    return rt.services(R, @ptrCast(@alignCast(environment)));
                }
                fn ready(env: *const R, value: Value, state: *rt.State) outcomes.Exit(A, E) {
                    const node: *const @This() = @ptrCast(@alignCast(value.borrowed));
                    return callback(&node.capture, env, if (state.protection != 0) null else state.branch_cancellation orelse state.origin_cancellation orelse state.services.cancellation);
                }
                fn start(state: *rt.State, parent: *Job) rt.Step {
                    const node: *const @This() = @ptrCast(@alignCast(@constCast(parent.program.value.borrowed)));
                    const program = fromNativeCancellableBorrowed(Capture, &node.capture, callback);
                    parent.program.value = program.value;
                    return program.start.?(state, parent);
                }
            };
            const node = allocator.create(Node) catch {
                var abandoned = capture;
                outcomes.discardPayload(&abandoned);
                return die(outcomes.Defect.out_of_memory);
            };
            node.* = .{ .header = .{ .previous = null, .allocator = allocator, .destroy = Node.destroy, .origin = .{ .data = node, .resolve = Node.origin } }, .capture = capture };
            return .{ .value = .{ .borrowed = node }, .ready = if (suspended) null else Node.ready, .start = if (suspended) Node.start else null, .ownership = &node.header };
        }

        /// Copies capture storage into an owning, reusable description. Pointer
        /// fields remain borrowed; this operation does not invent a destructor
        /// for application data. Move/deinit the description exactly once.
        pub fn fromOwned(env: *const R, capture: anytype, comptime callback: fn (*const @TypeOf(capture), *const R) outcomes.Exit(A, E)) Self {
            const Capture = @TypeOf(capture);
            const Node = struct {
                header: rt.DescriptionNode,
                capture: Capture,

                fn destroy(header: *rt.DescriptionNode) void {
                    const node: *@This() = @alignCast(@fieldParentPtr("header", header));
                    outcomes.discardPayload(&node.capture);
                    header.allocator.destroy(node);
                }

                fn origin(env_data: *const anyopaque, _: *const anyopaque) rt.Services {
                    return rt.services(R, @ptrCast(@alignCast(env_data)));
                }

                fn ready(supplied: *const R, stored: Value, _: *rt.State) outcomes.Exit(A, E) {
                    const node: *const @This() = @ptrCast(@alignCast(stored.borrowed));
                    return callback(&node.capture, supplied);
                }
            };
            const allocator = rt.services(R, env).allocatorValue() orelse {
                var abandoned = capture;
                outcomes.discardPayload(&abandoned);
                return die(.{ .kind = .invalid_configuration, .message = diagnostics.owned_binding_allocator });
            };
            const node = allocator.create(Node) catch {
                var abandoned = capture;
                outcomes.discardPayload(&abandoned);
                return die(outcomes.Defect.out_of_memory);
            };
            node.* = .{
                .header = .{ .previous = null, .allocator = allocator, .destroy = Node.destroy, .origin = .{ .data = node, .resolve = Node.origin } },
                .capture = capture,
            };
            return .{ .value = .{ .borrowed = node }, .ready = Node.ready, .ownership = &node.header };
        }

        /// Owns a runtime capture while exposing the active branch token to
        /// a cancellation-aware synchronous provider operation.
        pub fn fromOwnedCancellable(
            env: *const R,
            capture: anytype,
            comptime callback: fn (*const @TypeOf(capture), *const R, ?*const rt.Cancellation) outcomes.Exit(A, E),
        ) Self {
            const allocator = rt.services(R, env).allocatorValue() orelse {
                var abandoned = capture;
                outcomes.discardPayload(&abandoned);
                return die(.{ .kind = .invalid_configuration, .message = diagnostics.owned_binding_allocator });
            };
            return captureOwned(allocator, capture, false, callback);
        }

        /// A runtime continuation may create an owning result after the
        /// program starts. This description transfers that Exit exactly once.
        /// If cancellation or disposal prevents execution, its owner is
        /// released with the description. No allocator or I/O argument is
        /// duplicated outside the caller's R.
        pub fn fromOwnedOutcome(env: *const R, supplied: outcomes.Exit(A, E)) Self {
            const Node = struct {
                header: rt.DescriptionNode,
                outcome: ?outcomes.Exit(A, E),
                claimed: std.atomic.Value(bool) = .init(false),

                fn destroy(header: *rt.DescriptionNode) void {
                    const node: *@This() = @alignCast(@fieldParentPtr("header", header));
                    if (node.outcome) |*remaining| remaining.deinit();
                    header.allocator.destroy(node);
                }

                fn origin(env_data: *const anyopaque, _: *const anyopaque) rt.Services {
                    return rt.services(R, @ptrCast(@alignCast(env_data)));
                }

                fn ready(_: *const R, stored: Value, _: *rt.State) outcomes.Exit(A, E) {
                    const node: *@This() = @ptrCast(@alignCast(@constCast(stored.borrowed)));
                    if (node.claimed.cmpxchgStrong(false, true, .acq_rel, .acquire) != null)
                        return .{ .die = .{ .kind = .invalid_configuration, .message = diagnostics.owned_effect_outcome_can_run_only_once } };
                    const resolved = node.outcome.?;
                    node.outcome = null;
                    return resolved;
                }
            };
            const allocator = rt.services(R, env).allocatorValue() orelse {
                var abandoned = supplied;
                abandoned.deinit();
                return die(.{ .kind = .invalid_configuration, .message = diagnostics.owned_binding_allocator });
            };
            const node = allocator.create(Node) catch {
                var abandoned = supplied;
                abandoned.deinit();
                return die(outcomes.Defect.out_of_memory);
            };
            node.* = .{ .header = .{
                .previous = null,
                .allocator = allocator,
                .destroy = Node.destroy,
                .origin = .{ .data = node, .resolve = Node.origin },
            }, .outcome = supplied };
            return .{ .value = .{ .borrowed = node }, .ready = Node.ready, .ownership = &node.header };
        }

        pub fn fromOwnedSuccess(env: *const R, value: A) Self {
            return fromOwnedOutcome(env, .{ .success = value });
        }

        fn afterDynamic(self: *Self, allocator: std.mem.Allocator, comptime complete: fn (outcomes.Exit(A, E), *const R, *rt.State, *outcomes.CleanupStorage(E)) outcomes.Exit(A, E)) Self {
            const Holder = struct {
                header: rt.DescriptionNode,
                source: Self,
                fn destroy(header: *rt.DescriptionNode) void {
                    const holder: *@This() = @alignCast(@fieldParentPtr("header", header));
                    header.allocator.destroy(holder);
                }
                fn origin(env: *const anyopaque, data: *const anyopaque) rt.Services {
                    const holder: *const @This() = @ptrCast(@alignCast(data));
                    return holder.source.services(@ptrCast(@alignCast(env)));
                }
                fn resolve(env: *const R, value: Value) rt.Services {
                    const holder: *const @This() = @ptrCast(@alignCast(value.borrowed));
                    return holder.source.services(env);
                }
                const Frame = struct {
                    parent: *Job,
                    allocator: std.mem.Allocator,
                    child: Job = undefined,
                    value: outcomes.Exit(A, E) = undefined,
                    storage: outcomes.CleanupStorage(E),
                    fn destroy(data: *anyopaque) void {
                        const frame: *@This() = @ptrCast(@alignCast(data));
                        frame.allocator.destroy(frame);
                    }
                    fn done(state: *rt.State, data: ?*anyopaque) rt.Step {
                        const frame: *@This() = @ptrCast(@alignCast(data.?));
                        const value = complete(frame.value, frame.parent.env, state, &frame.storage);
                        const parent = frame.parent;
                        if (!frame.storage.retained()) frame.allocator.destroy(frame);
                        return parent.finish(value);
                    }
                };
                fn start(state: *rt.State, parent: *Job) rt.Step {
                    const holder: *const @This() = @ptrCast(@alignCast(@constCast(parent.program.value.borrowed)));
                    const memory = state.services.allocatorValue() orelse return parent.finish(.{ .die = outcomes.Defect.out_of_memory });
                    const frame = memory.create(Frame) catch return parent.finish(.{ .die = outcomes.Defect.out_of_memory });
                    frame.* = .{ .parent = parent, .allocator = memory, .storage = .{ .owner = frame, .release = Frame.destroy, .allocator = memory } };
                    frame.child = .{ .program = holder.source.borrow(), .env = parent.env, .inherited = state.services, .sink = .{ .output = &frame.value, .next_step = .{ .data = frame, .advance = Frame.done } } };
                    return frame.child.step();
                }
            };
            const holder = allocator.create(Holder) catch {
                self.deinit();
                return die(outcomes.Defect.out_of_memory);
            };
            holder.* = .{ .source = self.*, .header = .{ .previous = self.ownership, .allocator = allocator, .destroy = Holder.destroy, .origin = .{ .data = holder, .resolve = Holder.origin } } };
            self.* = undefined;
            return .{ .value = .{ .borrowed = holder }, .ownership = &holder.header, .resolve = Holder.resolve, .start = Holder.start };
        }

        pub fn fromResult(value: outcomes.Result(A, E)) Self {
            return switch (value) {
                .ok => |a| succeed(a),
                .err => |e| fail(e),
            };
        }

        pub fn fromOption(value: ?A, comptime on_none: fn () E) Self {
            if (value) |a| return succeed(a);
            return fromOutcome(struct {
                fn outcome(_: *const R) outcomes.Exit(A, E) {
                    return .{ .fail = on_none() };
                }
            }.outcome);
        }

        /// A fresh typed outcome is produced at execution, never construction.
        pub fn fromOutcome(comptime callback: fn (*const R) outcomes.Exit(A, E)) Self {
            return .{ .ready = struct {
                fn ready(env: *const R, _: Value, _: *rt.State) outcomes.Exit(A, E) {
                    return callback(env);
                }
            }.ready };
        }

        /// A native error union is mapped inside the Effect boundary. Native
        /// cancellation remains logical interruption after acknowledgement.
        pub fn trySync(comptime callback: anytype, comptime on_error: anytype) Self {
            const Return = unaryResult(callback, *const R, .tryIo);
            if (@typeInfo(Return) != .error_union) @compileError(diagnostics.native_error_union);
            const native = @typeInfo(Return).error_union;
            if (native.payload != A or unaryResult(on_error, native.error_set, .tryIo_error_mapper) != E) @compileError(diagnostics.native_error_mapping);
            return fromOutcome(struct {
                fn invoke(env: *const R) outcomes.Exit(A, E) {
                    const value = callback(env) catch |err| {
                        if (@as(anyerror, err) == error.Canceled) return .interrupted;
                        return .{ .fail = on_error(err) };
                    };
                    return .{ .success = value };
                }
            }.invoke);
        }

        pub fn tryIo(comptime callback: anytype, comptime on_error: anytype) Self {
            const Return = unaryResult(callback, *const R, .tryIo);
            if (@typeInfo(Return) != .error_union)
                @compileError(diagnostics.native_error_union);
            const native = @typeInfo(Return).error_union;
            if (native.payload != A)
                @compileError(diagnostics.native_success_type);
            if (unaryResult(on_error, native.error_set, .tryIo_error_mapper) != E)
                @compileError(diagnostics.native_error_mapping);
            if (R == void or !@hasField(R, @tagName(rt.EnvironmentField.io)) or @FieldType(R, @tagName(rt.EnvironmentField.io)) != std.Io)
                @compileError(diagnostics.native_io_capability);
            return .{ .ready = struct {
                fn ready(env: *const R, _: Value, state: *rt.State) outcomes.Exit(A, E) {
                    const value = callback(env) catch |err| {
                        if (@as(anyerror, err) == error.Canceled) {
                            state.interruption = .requested;
                            return .interrupted;
                        }
                        return .{ .fail = on_error(err) };
                    };
                    return .{ .success = value };
                }
            }.ready };
        }

        pub fn tryNative(comptime callback: anytype, comptime on_error: anytype) Self {
            const Return = unaryResult(callback, *const R, .tryIo);
            if (@typeInfo(Return) != .error_union) @compileError(diagnostics.native_error_union);
            const native = @typeInfo(Return).error_union;
            if (native.payload != A or unaryResult(on_error, native.error_set, .tryIo_error_mapper) != E) @compileError(diagnostics.native_error_mapping);
            return fromNativeBorrowed(void, &{}, struct {
                fn invoke(_: *const void, env: *const R) outcomes.Exit(A, E) {
                    const value = callback(env) catch |err| {
                        if (@as(anyerror, err) == error.Canceled) return .interrupted;
                        return .{ .fail = on_error(err) };
                    };
                    return .{ .success = value };
                }
            }.invoke);
        }

        pub fn deferEffect(comptime callback: anytype) Self {
            if (unaryResult(callback, *const R, .deferEffect) != Self)
                @compileError(diagnostics.defer_result);
            return .{ .start = struct {
                fn start(_: *rt.State, job: *Job) rt.Step {
                    job.replace(callback(job.env));
                    return job.step();
                }
            }.start };
        }

        pub fn deferBorrowed(comptime Capture: type, capture: *const Capture, comptime factory: fn (*const Capture, *const R) Self) Self {
            return .{ .value = .{ .borrowed = capture }, .start = struct {
                fn start(_: *rt.State, parent: *Job) rt.Step {
                    const borrowed: *const Capture = @ptrCast(@alignCast(@constCast(parent.program.value.borrowed)));
                    parent.replace(factory(borrowed, parent.env));
                    return parent.step();
                }
            }.start };
        }

        pub fn fromAsync(comptime factory: fn (*const R) asynchronous.Async(A, E).Start) Self {
            return fromAsyncBorrowed(void, &{}, struct {
                fn start(_: *const void, env: *const R) asynchronous.Async(A, E).Start {
                    return factory(env);
                }
            }.start);
        }

        pub fn fromAsyncBorrowed(comptime Capture: type, capture: *const Capture, comptime factory: fn (*const Capture, *const R) asynchronous.Async(A, E).Start) Self {
            const Async = asynchronous.Async(A, E);
            const Adapter = struct {
                const Frame = struct {
                    parent: *Job,
                    allocator: std.mem.Allocator,
                    operation: Async.Operation = undefined,
                    registration: rt.Cancellation.Registration = .{ .data = null, .forward = request },
                    forwarding: ?outcomes.Defect = null,
                    storage: outcomes.CleanupStorage(E),

                    fn request(data: ?*anyopaque) void {
                        const frame: *@This() = @ptrCast(@alignCast(data.?));
                        frame.forwarding = frame.operation.request();
                    }

                    fn destroy(data: *anyopaque) void {
                        const frame: *@This() = @ptrCast(@alignCast(data));
                        frame.allocator.destroy(frame);
                    }

                    fn continueAsync(state: *rt.State, data: ?*anyopaque) rt.Step {
                        const frame: *@This() = @ptrCast(@alignCast(data.?));
                        const io = state.services.ioValue();
                        const old = if (state.protection != 0 and io != null) io.?.swapCancelProtection(.blocked) else null;
                        defer if (old) |protection| {
                            _ = io.?.swapCancelProtection(protection);
                        };
                        const joined = frame.operation.joinProtected(state.protection != 0);
                        // Unregistration synchronizes with in-flight requests
                        // before provider state or the adapter is destroyed.
                        frame.registration.unregister();
                        frame.operation.deinit();
                        if (joined.interrupted) state.interruption = .requested;
                        var result_value = joined.exit;
                        if (frame.forwarding) |defect| frame.storage.append(&result_value, .{ .release_defect = defect });
                        const parent = frame.parent;
                        if (!frame.storage.retained()) frame.allocator.destroy(frame);
                        return parent.finish(result_value);
                    }
                };

                fn start(state: *rt.State, parent: *Job) rt.Step {
                    const allocator = state.services.allocatorValue() orelse return parent.finish(.{ .die = .{ .kind = .invalid_configuration, .message = diagnostics.suspended_binding_allocator } });
                    const frame = allocator.create(Frame) catch return parent.finish(.{ .die = outcomes.Defect.out_of_memory });
                    frame.* = .{ .parent = parent, .allocator = allocator, .storage = .{ .owner = frame, .release = Frame.destroy, .allocator = allocator } };
                    const borrowed: *const Capture = @ptrCast(@alignCast(@constCast(parent.program.value.borrowed)));
                    const started = factory(borrowed, parent.env);
                    if (started == .completed) {
                        allocator.destroy(frame);
                        return parent.finish(started.completed);
                    }
                    frame.operation = started.pending;
                    frame.registration.data = frame;
                    if (state.protection == 0) if (state.branch_cancellation orelse state.origin_cancellation) |token| token.register(&frame.registration);
                    return .{ .data = frame, .advance = null, .suspended = Frame.continueAsync };
                }
            };
            return .{ .value = .{ .borrowed = @ptrCast(capture) }, .start = Adapter.start };
        }

        pub fn awaitDeferred(deferred: *primitives.Deferred(A, E)) Self {
            return fromNativeCancellableBorrowed(primitives.Deferred(A, E), deferred, struct {
                fn wait(value: *const primitives.Deferred(A, E), _: *const R, token: ?*const rt.Cancellation) outcomes.Exit(A, E) {
                    return @constCast(value).waitToken(token);
                }
            }.wait);
        }

        pub fn takeQueue(queue: *primitives.Queue(A)) Effect(A, primitives.QueueError, R) {
            return Effect(A, primitives.QueueError, R).fromNativeCancellableBorrowed(primitives.Queue(A), queue, struct {
                fn take(value: *const primitives.Queue(A), _: *const R, token: ?*const rt.Cancellation) outcomes.Exit(A, primitives.QueueError) {
                    return @constCast(value).takeToken(token);
                }
            }.take);
        }

        pub fn fork(comptime self: Self, scope: *scopes.Scope(E)) Effect(*Fiber(A, E), E, R) {
            const Acquire = Effect(*Fiber(A, E), E, R);
            const Adapter = struct {
                fn ready(env: *const R, _: Acquire.Value, state: *rt.State) outcomes.Exit(*Fiber(A, E), E) {
                    return forkStart(env, self, state);
                }

                fn release(fiber: *Fiber(A, E)) Effect(void, E, R) {
                    return Effect(void, E, R).cleanupBorrowed(Fiber(A, E), fiber, struct {
                        fn cleanup(value: *const Fiber(A, E), _: *const R) outcomes.Exit(void, E) {
                            return @constCast(value).close();
                        }
                    }.cleanup);
                }
            };
            const acquisition: Acquire = .{ .ready = Adapter.ready };
            return acquisition.scopeAcquire(scope, Adapter.release);
        }

        pub fn joinFiber(fiber: *Fiber(A, E)) Self {
            return fromNativeCancellableBorrowed(Fiber(A, E), fiber, struct {
                fn wait(value: *const Fiber(A, E), _: *const R, token: ?*const rt.Cancellation) outcomes.Exit(A, E) {
                    const target = @constCast(value);
                    const observed = target.wait(token);
                    if (!observed.observer_interrupted and !observed.clone_failed) target.observed.store(true, .release);
                    return observed.exit;
                }
            }.wait);
        }

        pub fn awaitFiberExit(fiber: *Fiber(A, E)) Effect(outcomes.Exit(A, E), E, R) {
            return Effect(outcomes.Exit(A, E), E, R).fromNativeCancellableBorrowed(Fiber(A, E), fiber, struct {
                fn wait(value: *const Fiber(A, E), _: *const R, token: ?*const rt.Cancellation) outcomes.Exit(outcomes.Exit(A, E), E) {
                    const target = @constCast(value);
                    const observed = target.wait(token);
                    if (observed.observer_interrupted) {
                        var discarded = observed.exit;
                        discarded.deinit();
                        return .interrupted;
                    }
                    if (!observed.clone_failed) target.observed.store(true, .release);
                    return .{ .success = observed.exit };
                }
            }.wait);
        }

        pub fn interruptFiber(fiber: *Fiber(A, E)) Effect(outcomes.Exit(A, E), E, R) {
            const Out = Effect(outcomes.Exit(A, E), E, R);
            return Out.fromNativeCancellableBorrowed(Fiber(A, E), fiber, struct {
                fn interrupt(value: *const Fiber(A, E), _: *const R, token: ?*const rt.Cancellation) outcomes.Exit(outcomes.Exit(A, E), E) {
                    const target = @constCast(value);
                    const observed = target.interrupt();
                    if (if (token) |origin| origin.isRequested() else false) {
                        var discarded = observed.exit;
                        discarded.deinit();
                        return .interrupted;
                    }
                    if (!observed.clone_failed) target.observed.store(true, .release);
                    return .{ .success = observed.exit };
                }
            }.interrupt);
        }

        // Full-exit binding deliberately receives a late acquisition before a
        // cancellation check. Public success/error continuations check inside
        // their handlers before calling application code.
        fn bindExit(comptime self: Self, comptime Out: type, comptime next: fn (outcomes.Exit(A, E), *const R, *rt.State) Out) Out {
            const Reduction = struct {
                fn reduce(env: *const R, _: Out.Value, state: *rt.State) Out {
                    return @call(.always_inline, evaluate, .{ false, env, state });
                }

                fn evaluate(comptime empty_entry: bool, env: *const R, state: *rt.State) Out {
                    const capabilities = self.services(env).inherit(state.services);
                    state.services = capabilities;
                    if (comptime self.ready != null) {
                        const value = @call(.always_inline, self.ready.?, .{ env, self.value, state });
                        return @call(.always_inline, next, .{ value, env, state });
                    }
                    const produced = if (comptime empty_entry and self.pure_reduce != null)
                        if (state.isPristine())
                            @call(.always_inline, self.pure_reduce.?, .{ env, self.value, state })
                        else
                            self.reduce.?(env, self.value, state)
                    else
                        @call(.always_inline, self.reduce.?, .{ env, self.value, state });
                    // The common terminal constructor has no resolver or
                    // continuation. Retain its typed outcome without copying
                    // dispatch metadata across a runtime-selected branch.
                    if (comptime E != void) if (produced.value == .terminal) {
                        state.services = rt.services(R, env).inherit(capabilities);
                        const value = if (produced.delivery == .ordinary) state.publish(produced.value.terminal) else produced.value.terminal;
                        return @call(.always_inline, next, .{ value, env, state });
                    };
                    if (produced.ready) |ready| {
                        state.services = produced.services(env).inherit(capabilities);
                        if (produced.delivery == .ordinary and state.cancelled()) {
                            rt.DescriptionNode.deinit(produced.ownership);
                            return @call(.always_inline, next, .{ .interrupted, env, state });
                        }
                        const io = state.services.ioValue();
                        const old = if (state.protection != 0 and io != null) io.?.swapCancelProtection(.blocked) else null;
                        defer if (old) |protection| {
                            _ = io.?.swapCancelProtection(protection);
                        };
                        const value = ready(env, produced.value, state);
                        const evaluated = @call(.always_inline, next, .{ value, env, state });
                        rt.DescriptionNode.deinit(produced.ownership);
                        return evaluated;
                    }
                    // A pending, runtime-selected continuation needs retained
                    // storage through R, including projected/bound services.
                    var retained = produced;
                    return retained.bindExitOwned(env, Out, next, capabilities);
                }
                fn pure(env: *const R, _: Out.Value, state: *rt.State) Out {
                    // run checked the complete empty-state invariant. Exposing
                    // that invariant inside this generated adapter lets known
                    // reductions simplify before the indirect entry is inlined.
                    state.* = .{ .services = .{} };
                    return @call(.always_inline, evaluate, .{ true, env, state });
                }
            };
            return comptime Out.withEntry(.{
                .reduce = if (self.ready != null or self.reduce != null) Reduction.reduce else null,
                .pure_reduce = if (self.ready != null or self.reduce != null) Reduction.pure else null,
                .resolve = struct {
                    fn resolve(env: *const R, _: Out.Value) rt.Services {
                        return self.services(env);
                    }
                }.resolve,
                .start = struct {
                    fn start(state: *rt.State, parent: *Out.Job) rt.Step {
                        return startBinding(self, Out, next, state, parent);
                    }
                }.start,
            });
        }

        // Fold ready reductions before allocating a suspended continuation.
        // Runtime-selected descriptions retain typed frames and stepping; known
        // synchronous composition does not need a frame simply to hold its evaluated.
        inline fn startBinding(source: Self, comptime Out: type, comptime next: fn (outcomes.Exit(A, E), *const R, *rt.State) Out, state: *rt.State, parent: *Out.Job) rt.Step {
            const Frame = struct {
                child: Job,
                evaluated: outcomes.Exit(A, E) = undefined,
                parent: *Out.Job,
                allocator: std.mem.Allocator,

                fn complete(active: *rt.State, data: ?*anyopaque) rt.Step {
                    const frame: *@This() = @ptrCast(@alignCast(data.?));
                    const evaluated = frame.evaluated;
                    const target = frame.parent;
                    frame.allocator.destroy(frame);
                    target.replace(next(evaluated, target.env, active));
                    return target.tail(active);
                }
            };
            var current = source;
            const enclosing = state.services;
            var produced_owner: ?*rt.DescriptionNode = null;
            while (true) {
                state.services = current.services(parent.env).inherit(enclosing);
                if (current.delivery == .ordinary and state.cancelled()) {
                    rt.DescriptionNode.deinit(produced_owner);
                    parent.replace(next(.interrupted, parent.env, state));
                    return parent.tail(state);
                }
                if (current.reduce) |reduce| {
                    const produced = reduce(parent.env, current.value, state);
                    rt.DescriptionNode.deinit(produced_owner);
                    produced_owner = produced.ownership;
                    current = produced;
                    continue;
                }
                if (current.ready) |ready| {
                    const evaluated = ready(parent.env, current.value, state);
                    rt.DescriptionNode.deinit(produced_owner);
                    parent.replace(next(evaluated, parent.env, state));
                    return parent.tail(state);
                }
                break;
            }
            const allocator = state.services.allocatorValue() orelse {
                rt.DescriptionNode.deinit(produced_owner);
                return parent.finish(.{ .die = .{
                    .kind = .invalid_configuration,
                    .message = diagnostics.suspended_binding_allocator,
                } });
            };
            const frame = allocator.create(Frame) catch {
                rt.DescriptionNode.deinit(produced_owner);
                return parent.finish(.{ .die = outcomes.Defect.out_of_memory });
            };
            frame.* = .{ .parent = parent, .allocator = allocator, .child = .{
                .program = current,
                .inherited = state.services,
                .owned = produced_owner,
                .env = parent.env,
                .sink = .{
                    .output = &frame.evaluated,
                    .next_step = .{ .data = frame, .advance = Frame.complete },
                },
            } };
            return frame.child.step();
        }

        pub fn flatMap(comptime self: Self, comptime next: anytype) continuationResult(next, A, R, .flatMap) {
            const Out = continuationResult(next, A, R, .flatMap);
            comptime requireEffect(Out, E, R, .flatMap);
            return self.bindExit(Out, struct {
                fn handle(value: outcomes.Exit(A, E), env: *const R, state: *rt.State) Out {
                    return switch (value) {
                        .success => |evaluated| if (state.cancelled()) blk: {
                            var discarded = evaluated;
                            outcomes.discardPayload(&discarded);
                            break :blk Out.interrupted();
                        } else callContinuation(next, evaluated, env),
                        else => Out.terminalFailure(value),
                    };
                }
            }.handle);
        }

        /// Consumes a runtime-built description, allocating its typed node
        /// through the supplied R. On construction failure it still consumes
        /// and releases the source, and returns a lazy defect description.
        /// The returned owning description must be moved and deinitialized once.
        pub fn flatMapOwned(self: *Self, env: *const R, comptime next: anytype) continuationResult(next, A, R, .flatMapOwned) {
            const Out = continuationResult(next, A, R, .flatMapOwned);
            comptime requireEffect(Out, E, R, .flatMapOwned);
            return self.bindExitOwned(env, Out, struct {
                fn handle(evaluated: outcomes.Exit(A, E), supplied: *const R, state: *rt.State) Out {
                    return switch (evaluated) {
                        .success => |value| if (state.cancelled()) blk: {
                            var discarded = value;
                            outcomes.discardPayload(&discarded);
                            break :blk Out.interrupted();
                        } else callContinuation(next, value, supplied),
                        else => Out.terminalFailure(evaluated),
                    };
                }
            }.handle, null);
        }

        /// Carries an owned runtime value through a suspended continuation.
        /// `next` may inspect the capture while constructing its returned
        /// Effect; any data needed after that return must be copied into the
        /// new description. The capture is released exactly once.
        pub fn flatMapCapturedOwned(
            self: *Self,
            env: *const R,
            comptime B: type,
            capture: anytype,
            comptime next: fn (*const @TypeOf(capture), A, *const R) Effect(B, E, R),
        ) Effect(B, E, R) {
            const Capture = @TypeOf(capture);
            const Out = Effect(B, E, R);
            const Node = struct {
                const BoundNode = @This();
                header: rt.DescriptionNode,
                source: Self,
                capture: Capture,

                fn destroy(header: *rt.DescriptionNode) void {
                    const node: *@This() = @alignCast(@fieldParentPtr("header", header));
                    outcomes.discardPayload(&node.capture);
                    header.allocator.destroy(node);
                }

                fn origin(env_data: *const anyopaque, source_data: *const anyopaque) rt.Services {
                    const node: *const @This() = @ptrCast(@alignCast(source_data));
                    const supplied: *const R = @ptrCast(@alignCast(env_data));
                    return node.source.services(supplied);
                }

                fn resolve(supplied: *const R, value: Out.Value) rt.Services {
                    const node: *const @This() = @ptrCast(@alignCast(value.borrowed));
                    return node.header.origin.services(supplied);
                }

                fn continueWith(node: *const @This(), result_value: outcomes.Exit(A, E), state: *rt.State, parent: *Out.Job) rt.Step {
                    const produced: Out = switch (result_value) {
                        .success => |value| if (state.cancelled()) blk: {
                            var discarded = value;
                            outcomes.discardPayload(&discarded);
                            break :blk Out.interrupted();
                        } else next(&node.capture, value, parent.env),
                        else => Out.terminalFailure(result_value),
                    };
                    parent.replace(produced);
                    return parent.tail(state);
                }

                fn start(state: *rt.State, parent: *Out.Job) rt.Step {
                    const node: *const @This() = @ptrCast(@alignCast(parent.program.value.borrowed));
                    const Frame = struct {
                        child: Job,
                        evaluated: outcomes.Exit(A, E) = undefined,
                        parent: *Out.Job,
                        node: *const BoundNode,
                        allocator: std.mem.Allocator,

                        fn complete(active: *rt.State, data: ?*anyopaque) rt.Step {
                            const frame: *@This() = @ptrCast(@alignCast(data.?));
                            const evaluated = frame.evaluated;
                            const target = frame.parent;
                            const captured = frame.node;
                            frame.allocator.destroy(frame);
                            return BoundNode.continueWith(captured, evaluated, active, target);
                        }
                    };
                    var current = node.source;
                    const enclosing = state.services;
                    var produced_owner: ?*rt.DescriptionNode = null;
                    while (true) {
                        state.services = current.services(parent.env).inherit(enclosing);
                        if (current.delivery == .ordinary and state.cancelled()) {
                            rt.DescriptionNode.deinit(produced_owner);
                            return node.continueWith(.interrupted, state, parent);
                        }
                        if (current.reduce) |reduce| {
                            const produced = reduce(parent.env, current.value, state);
                            rt.DescriptionNode.deinit(produced_owner);
                            produced_owner = produced.ownership;
                            current = produced;
                            continue;
                        }
                        if (current.ready) |ready| {
                            const evaluated = ready(parent.env, current.value, state);
                            rt.DescriptionNode.deinit(produced_owner);
                            return node.continueWith(evaluated, state, parent);
                        }
                        break;
                    }
                    const allocator = state.services.allocatorValue() orelse {
                        rt.DescriptionNode.deinit(produced_owner);
                        return parent.finish(.{ .die = .{
                            .kind = .invalid_configuration,
                            .message = diagnostics.suspended_binding_allocator,
                        } });
                    };
                    const frame = allocator.create(Frame) catch {
                        rt.DescriptionNode.deinit(produced_owner);
                        return parent.finish(.{ .die = outcomes.Defect.out_of_memory });
                    };
                    frame.* = .{ .parent = parent, .node = node, .allocator = allocator, .child = .{
                        .program = current,
                        .inherited = state.services,
                        .owned = produced_owner,
                        .env = parent.env,
                        .sink = .{ .output = &frame.evaluated, .next_step = .{ .data = frame, .advance = Frame.complete } },
                    } };
                    return frame.child.step();
                }
            };
            const allocator = self.services(env).allocatorValue() orelse {
                self.deinit();
                var abandoned = capture;
                outcomes.discardPayload(&abandoned);
                return Out.die(.{ .kind = .invalid_configuration, .message = diagnostics.owned_binding_allocator });
            };
            const node = allocator.create(Node) catch {
                self.deinit();
                var abandoned = capture;
                outcomes.discardPayload(&abandoned);
                return Out.die(outcomes.Defect.out_of_memory);
            };
            node.* = .{
                .header = .{ .previous = self.ownership, .allocator = allocator, .destroy = Node.destroy, .origin = .{ .data = node, .resolve = Node.origin } },
                .source = self.*,
                .capture = capture,
            };
            self.* = undefined;
            return .{ .value = .{ .borrowed = node }, .ownership = &node.header, .resolve = Node.resolve, .start = Node.start };
        }

        fn bindExitOwned(self: *Self, env: *const R, comptime Out: type, comptime next: fn (outcomes.Exit(A, E), *const R, *rt.State) Out, scoped_services: ?rt.Services) Out {
            const Node = struct {
                header: rt.DescriptionNode,
                source: Self,
                scoped_services: ?rt.Services,

                fn destroy(header: *rt.DescriptionNode) void {
                    const node: *@This() = @alignCast(@fieldParentPtr("header", header));
                    header.allocator.destroy(node);
                }

                fn origin(env_data: *const anyopaque, source_data: *const anyopaque) rt.Services {
                    const node: *const @This() = @ptrCast(@alignCast(source_data));
                    const supplied: *const R = @ptrCast(@alignCast(env_data));
                    if (node.source.resolve == null) {
                        if (node.scoped_services) |capabilities| return capabilities;
                    }
                    return node.source.services(supplied);
                }

                fn resolve(env_ptr: *const R, value: Out.Value) rt.Services {
                    const node: *const @This() = @ptrCast(@alignCast(value.borrowed));
                    return node.header.origin.services(env_ptr);
                }

                fn start(state: *rt.State, parent: *Out.Job) rt.Step {
                    const node: *const @This() = @ptrCast(@alignCast(@constCast(parent.program.value.borrowed)));
                    return startBinding(node.source, Out, next, state, parent);
                }
            };
            const capabilities = if (self.resolve == null) scoped_services orelse self.services(env) else self.services(env);
            const allocator = capabilities.allocatorValue() orelse {
                self.deinit();
                return Out.die(.{ .kind = .invalid_configuration, .message = diagnostics.owned_binding_allocator });
            };
            const node = allocator.create(Node) catch {
                self.deinit();
                return Out.die(outcomes.Defect.out_of_memory);
            };
            node.source = self.*;
            node.scoped_services = scoped_services;
            node.header = .{
                .previous = self.ownership,
                .allocator = allocator,
                .destroy = Node.destroy,
                .origin = if (scoped_services == null and self.ownership != null) self.ownership.?.origin else .{ .data = node, .resolve = Node.origin },
            };
            self.* = undefined;
            return .{ .value = .{ .borrowed = node }, .ownership = &node.header, .resolve = Node.resolve, .start = Node.start };
        }

        /// Frees owned description storage, not application success/error data.
        /// Borrowed/static descriptions require no allocation cleanup.
        pub fn deinit(self: *Self) void {
            rt.DescriptionNode.deinit(self.ownership);
            self.* = undefined;
        }

        /// Borrows an existing description without transferring its storage.
        /// The owner must outlive execution and joined cleanup of this value.
        pub fn borrow(self: *const Self) Self {
            var borrowed = self.*;
            borrowed.ownership = null;
            return borrowed;
        }

        // An owning Exit can only enter the private continuation path, where
        // its ownership moves exactly once. It is not a reusable constructor.
        fn terminalFailure(value: anytype) Self {
            return terminalExit(outcomes.castFailure(A, value));
        }

        pub fn map(comptime self: Self, comptime transform: anytype) Effect(unaryResult(transform, A, .map), E, R) {
            const B = unaryResult(transform, A, .map);
            const Out = Effect(B, E, R);
            if (comptime self.ready != null) return .{ .resolve = struct {
                fn resolve(env: *const R, _: Out.Value) rt.Services {
                    return self.services(env);
                }
            }.resolve, .ready = struct {
                fn ready(env: *const R, _: Out.Value, state: *rt.State) outcomes.Exit(B, E) {
                    const value = self.ready.?(env, self.value, state);
                    return switch (value) {
                        .success => |evaluated| if (state.cancelled()) blk: {
                            var discarded = evaluated;
                            outcomes.discardPayload(&discarded);
                            break :blk .interrupted;
                        } else .{ .success = transform(evaluated) },
                        else => outcomes.castFailure(B, value),
                    };
                }
            }.ready };
            return self.flatMap(struct {
                fn next(value: A) Out {
                    return Out.succeedMoved(transform(value));
                }
            }.next);
        }

        /// Consumes a runtime-built description while retaining the same
        /// typed environment and error. The mapped value is transferred to
        /// an owning continuation only when execution reaches this step.
        pub fn mapOwned(self: *Self, env: *const R, comptime transform: anytype) Effect(unaryResult(transform, A, .map), E, R) {
            const B = unaryResult(transform, A, .map);
            const Out = Effect(B, E, R);
            return self.flatMapOwned(env, struct {
                fn next(value: A, supplied: *const R) Out {
                    return Out.fromOwnedSuccess(supplied, transform(value));
                }
            }.next);
        }

        pub fn mapError(comptime self: Self, comptime transform: anytype) Effect(A, unaryResult(transform, E, .mapError), R) {
            const F = unaryResult(transform, E, .mapError);
            const Out = Effect(A, F, R);
            return self.bindExit(Out, struct {
                fn handle(value: outcomes.Exit(A, E), _: *const R, state: *rt.State) Out {
                    return Out.terminalExit(outcomes.mapError(F, value, state.services.allocatorValue(), transform));
                }
            }.handle);
        }

        /// Maps a runtime-built owning description without discarding its
        /// capture or requiring that its value be known at comptime.
        pub fn mapErrorOwned(self: *Self, env: *const R, comptime transform: anytype) Effect(A, unaryResult(transform, E, .mapError), R) {
            const F = unaryResult(transform, E, .mapError);
            const Out = Effect(A, F, R);
            return self.bindExitOwned(env, Out, struct {
                fn handle(value: outcomes.Exit(A, E), _: *const R, state: *rt.State) Out {
                    return Out.terminalExit(outcomes.mapError(F, value, state.services.allocatorValue(), transform));
                }
            }.handle, null);
        }

        inline fn terminalExit(value: outcomes.Exit(A, E)) Self {
            return .{ .delivery = .terminal, .value = .{ .terminal = value }, .ready = terminalReady };
        }

        fn terminalReady(_: *const R, stored: Value, _: *rt.State) outcomes.Exit(A, E) {
            return stored.terminal;
        }

        pub fn either(comptime self: Self) Effect(outcomes.Result(A, E), E, R) {
            const Out = Effect(outcomes.Result(A, E), E, R);
            return self.bindExit(Out, struct {
                fn handle(value: outcomes.Exit(A, E), _: *const R, _: *rt.State) Out {
                    return switch (value) {
                        .success => |a| Out.succeedMoved(.{ .ok = a }),
                        .fail => |e| Out.succeedMoved(.{ .err = e }),
                        else => Out.terminalFailure(value),
                    };
                }
            }.handle);
        }

        pub fn result(comptime self: Self) Effect(outcomes.Result(A, E), E, R) {
            return self.either();
        }

        /// Reify the entire terminal outcome as an owning success value.
        /// Domain failure, interruption, defect and failed cleanup remain
        /// distinct; callers can observe them without an error-union catch.
        pub fn exit(comptime self: Self) Effect(outcomes.Exit(A, E), E, R) {
            const Out = Effect(outcomes.Exit(A, E), E, R);
            return self.bindExit(Out, struct {
                fn handle(value: outcomes.Exit(A, E), _: *const R, _: *rt.State) Out {
                    return Out.succeedMoved(value);
                }
            }.handle);
        }

        /// Reifies a runtime-built description's full Exit. This consumes
        /// the description exactly once; its owner remains with the returned
        /// Effect until execution or disposal.
        pub fn exitOwned(self: *Self, env: *const R) Effect(outcomes.Exit(A, E), E, R) {
            const Out = Effect(outcomes.Exit(A, E), E, R);
            return self.bindExitOwned(env, Out, struct {
                fn handle(value: outcomes.Exit(A, E), _: *const R, _: *rt.State) Out {
                    return Out.succeedMoved(value);
                }
            }.handle, null);
        }

        /// Reifies every terminal path, including typed failure and cleanup,
        /// into an owned Exit value. The resulting Effect has no domain-error
        /// value because the original error is inside that Exit.
        pub fn materializeOwned(self: *Self, env: *const R) Effect(outcomes.Exit(A, E), void, R) {
            const Out = Effect(outcomes.Exit(A, E), void, R);
            return self.bindExitOwned(env, Out, struct {
                fn handle(value: outcomes.Exit(A, E), _: *const R, _: *rt.State) Out {
                    return Out.succeedMoved(value);
                }
            }.handle, null);
        }

        pub fn option(comptime self: Self) Effect(?A, E, R) {
            const Out = Effect(?A, E, R);
            return self.bindExit(Out, struct {
                fn handle(value: outcomes.Exit(A, E), _: *const R, _: *rt.State) Out {
                    return switch (value) {
                        .success => |a| Out.succeedMoved(a),
                        .fail => |err| blk: {
                            var discarded = err;
                            outcomes.discardPayload(&discarded);
                            break :blk Out.succeedMoved(null);
                        },
                        else => Out.terminalFailure(value),
                    };
                }
            }.handle);
        }

        pub fn sandbox(comptime self: Self, comptime on_defect: anytype) Self {
            if (unaryResult(on_defect, outcomes.Defect, .sandbox) != E)
                @compileError(diagnostics.sandbox_mapper_must_return_the_declared_effect_error_type);
            return self.bindExit(Self, struct {
                fn handle(value: outcomes.Exit(A, E), _: *const R, _: *rt.State) Self {
                    return switch (value) {
                        .die => |defect| fail(on_defect(defect)),
                        else => terminalExit(value),
                    };
                }
            }.handle);
        }

        pub fn zip(comptime self: Self, comptime right: anytype) Effect(outcomes.Pair(A, @TypeOf(right).Success), E, R) {
            const Right = @TypeOf(right);
            comptime requireEffect(Right, E, R, .zip);
            const Pair = outcomes.Pair(A, Right.Success);
            const Out = Effect(Pair, E, R);
            const Adapter = struct {
                fn resolve(env: *const R, _: Out.Value) rt.Services {
                    return self.services(env);
                }

                fn ready(env: *const R, _: Out.Value, state: *rt.State) outcomes.Exit(Pair, E) {
                    const left_value = self.ready.?(env, self.value, state);
                    if (left_value != .success) return outcomes.castFailure(Pair, left_value);
                    if (state.cancelled()) {
                        var discarded = left_value;
                        discarded.deinit();
                        return .interrupted;
                    }
                    state.services = right.services(env).inherit(state.services);
                    const right_value = right.ready.?(env, right.value, state);
                    if (right_value != .success) {
                        var discarded = left_value;
                        discarded.deinit();
                        return outcomes.castFailure(Pair, right_value);
                    }
                    return .{ .success = .{ .left = left_value.success, .right = right_value.success } };
                }

                const Frame = struct {
                    parent: *Out.Job,
                    allocator: std.mem.Allocator,
                    left_job: Job,
                    left_value: outcomes.Exit(A, E) = undefined,
                    right_job: Right.Job = undefined,
                    right_value: outcomes.Exit(Right.Success, E) = undefined,

                    fn leftDone(state: *rt.State, data: ?*anyopaque) rt.Step {
                        const frame: *@This() = @ptrCast(@alignCast(data.?));
                        frame.left_value = state.publish(frame.left_value);
                        if (frame.left_value != .success) {
                            const value = outcomes.castFailure(Pair, frame.left_value);
                            const parent = frame.parent;
                            frame.allocator.destroy(frame);
                            return parent.finish(value);
                        }
                        frame.right_job = .{ .program = right, .inherited = state.services, .env = frame.parent.env, .sink = .{
                            .output = &frame.right_value,
                            .next_step = .{ .data = frame, .advance = rightDone },
                        } };
                        return frame.right_job.step();
                    }

                    fn rightDone(_: *rt.State, data: ?*anyopaque) rt.Step {
                        const frame: *@This() = @ptrCast(@alignCast(data.?));
                        const value: outcomes.Exit(Pair, E) = if (frame.right_value == .success)
                            .{ .success = .{ .left = frame.left_value.success, .right = frame.right_value.success } }
                        else
                            outcomes.castFailure(Pair, frame.right_value);
                        if (frame.right_value != .success) outcomes.discardPayload(&frame.left_value.success);
                        const parent = frame.parent;
                        frame.allocator.destroy(frame);
                        return parent.finish(value);
                    }
                };

                fn start(state: *rt.State, parent: *Out.Job) rt.Step {
                    const allocator = state.services.allocatorValue() orelse return parent.finish(.{ .die = .{ .kind = .invalid_configuration, .message = diagnostics.suspended_binding_allocator } });
                    const frame = allocator.create(Frame) catch return parent.finish(.{ .die = outcomes.Defect.out_of_memory });
                    frame.* = .{ .parent = parent, .allocator = allocator, .left_job = .{
                        .program = self,
                        .inherited = state.services,
                        .env = parent.env,
                        .sink = .{ .output = &frame.left_value, .next_step = .{ .data = frame, .advance = Frame.leftDone } },
                    } };
                    return frame.left_job.step();
                }
            };
            return .{ .resolve = Adapter.resolve, .ready = if (self.ready != null and right.ready != null) Adapter.ready else null, .start = Adapter.start };
        }

        pub fn uninterruptible(comptime self: Self) Self {
            const Adapter = struct {
                fn resolve(env: *const R, _: Value) rt.Services {
                    return self.services(env);
                }

                fn ready(env: *const R, _: Value, state: *rt.State) outcomes.Exit(A, E) {
                    state.protection += 1;
                    const io = state.services.ioValue();
                    const old = if (io) |active| active.swapCancelProtection(.blocked) else null;
                    defer if (old) |protection| {
                        _ = io.?.swapCancelProtection(protection);
                    };
                    const value = self.ready.?(env, self.value, state);
                    state.protection -= 1;
                    state.protected_failure = value == .fail;
                    return value;
                }

                const Frame = struct {
                    parent: *Job,
                    child: Job,
                    allocator: std.mem.Allocator,
                    evaluated: outcomes.Exit(A, E) = undefined,

                    fn complete(state: *rt.State, data: ?*anyopaque) rt.Step {
                        const frame: *@This() = @ptrCast(@alignCast(data.?));
                        state.protection -= 1;
                        state.protected_failure = frame.evaluated == .fail;
                        const parent = frame.parent;
                        const value = frame.evaluated;
                        frame.allocator.destroy(frame);
                        return parent.finish(value);
                    }
                };

                fn start(state: *rt.State, parent: *Job) rt.Step {
                    const allocator = state.services.allocatorValue() orelse return parent.finish(.{ .die = .{ .kind = .invalid_configuration, .message = diagnostics.protected_allocator } });
                    const frame = allocator.create(Frame) catch return parent.finish(.{ .die = outcomes.Defect.out_of_memory });
                    frame.* = .{ .parent = parent, .allocator = allocator, .child = .{
                        .program = self,
                        .inherited = state.services,
                        .env = parent.env,
                        .sink = .{ .output = &frame.evaluated, .next_step = .{ .data = frame, .advance = Frame.complete } },
                    } };
                    state.protection += 1;
                    return frame.child.step();
                }
            };
            return .{ .resolve = Adapter.resolve, .ready = if (self.ready != null) Adapter.ready else null, .start = Adapter.start };
        }

        pub fn onInterrupt(comptime self: Self, comptime finalizer: Effect(void, E, R)) Self {
            const Unit = Effect(void, E, R);
            return (comptime Unit.succeed({})).acquireRelease(struct {
                fn body(_: void) Self {
                    return self;
                }
            }.body, struct {
                fn release(_: void, primary: outcomes.PrimaryOutcome(E)) Unit {
                    return if (primary == .was_interrupted) finalizer else Unit.succeed({});
                }
            }.release);
        }

        /// Borrows the descriptions until execution and cleanup have finished.
        /// Returned array storage is owned; elements keep their declared
        /// ownership. Use scoped acquisitions for resources in a traversal.
        pub fn all(effects: []const Self) Effect(outcomes.OwnedSlice(A), E, R) {
            return traverse(Self, effects, struct {
                fn identity(program: Self) Self {
                    return program.borrow();
                }
            }.identity);
        }

        pub fn forEach(comptime Input: type, values: []const Input, comptime transform: fn (Input) Self) Effect(outcomes.OwnedSlice(A), E, R) {
            return traverse(Input, values, transform);
        }

        pub fn forEachDiscard(comptime Input: type, values: []const Input, comptime transform: fn (Input) Effect(void, E, R)) Effect(void, E, R) {
            const Unit = Effect(void, E, R);
            const Adapter = struct {
                const Frame = struct {
                    parent: *Unit.Job,
                    allocator: std.mem.Allocator,
                    values: []const Input,
                    index: usize = 0,
                    child: Unit.Job = undefined,
                    result_value: outcomes.Exit(void, E) = undefined,
                    fn finish(frame: *@This(), value: outcomes.Exit(void, E)) rt.Step {
                        const parent = frame.parent;
                        frame.allocator.destroy(frame);
                        return parent.finish(value);
                    }
                    fn next(state: *rt.State, data: ?*anyopaque) rt.Step {
                        const frame: *@This() = @ptrCast(@alignCast(data.?));
                        if (state.cancelled()) return frame.finish(.interrupted);
                        if (frame.index == frame.values.len) return frame.finish(.{ .success = {} });
                        const value = outcomes.clonePayload(frame.values[frame.index]) catch return frame.finish(.{ .die = outcomes.Defect.out_of_memory });
                        const program = transform(value);
                        frame.child = .{ .program = program, .owned = program.ownership, .env = frame.parent.env, .inherited = state.services, .sink = .{ .output = &frame.result_value, .next_step = .{ .data = frame, .advance = complete } } };
                        return frame.child.step();
                    }
                    fn complete(state: *rt.State, data: ?*anyopaque) rt.Step {
                        const frame: *@This() = @ptrCast(@alignCast(data.?));
                        const value = state.publish(frame.result_value);
                        if (value != .success) return frame.finish(value);
                        frame.index += 1;
                        return .{ .data = frame, .advance = next };
                    }
                };
                fn start(state: *rt.State, parent: *Unit.Job) rt.Step {
                    const sequence = parent.program.value.sequence;
                    if (sequence.len == 0) return parent.finish(.{ .success = {} });
                    const allocator = state.services.allocatorValue() orelse return parent.finish(.{ .die = outcomes.Defect.out_of_memory });
                    const frame = allocator.create(Frame) catch return parent.finish(.{ .die = outcomes.Defect.out_of_memory });
                    const pointer: [*]const Input = @ptrCast(@alignCast(sequence.items));
                    frame.* = .{ .parent = parent, .allocator = allocator, .values = pointer[0..sequence.len] };
                    return .{ .data = frame, .advance = Frame.next };
                }
            };
            return .{ .value = .{ .sequence = .{ .items = @ptrCast(values.ptr), .len = values.len } }, .start = Adapter.start };
        }

        pub fn onCpu(comptime callback: fn (*const R, *const rt.Cancellation) outcomes.Exit(A, E)) Self {
            const program: Self = .{ .ready = struct {
                fn ready(env: *const R, _: Value, state: *rt.State) outcomes.Exit(A, E) {
                    return callback(env, state.branch_cancellation.?);
                }
            }.ready };
            return program.dispatch(.cpu);
        }

        pub fn onBlocking(comptime callback: fn (*const R, *const rt.Cancellation) outcomes.Exit(A, E)) Self {
            const program: Self = .{ .ready = struct {
                fn ready(env: *const R, _: Value, state: *rt.State) outcomes.Exit(A, E) {
                    return callback(env, state.branch_cancellation.?);
                }
            }.ready };
            return program.dispatch(.blocking);
        }

        /// Runtime captures stay borrowed until the selected engine's work has
        /// joined. Only callback/type metadata is part of specialization.
        pub fn onCpuBorrowed(comptime Capture: type, capture: *const Capture, comptime callback: fn (*const Capture, *const R, *const rt.Cancellation) outcomes.Exit(A, E)) Self {
            return onEngineBorrowed(Capture, capture, callback, .cpu);
        }

        pub fn onBlockingBorrowed(comptime Capture: type, capture: *const Capture, comptime callback: fn (*const Capture, *const R, *const rt.Cancellation) outcomes.Exit(A, E)) Self {
            return onEngineBorrowed(Capture, capture, callback, .blocking);
        }

        fn onEngineBorrowed(comptime Capture: type, capture: *const Capture, comptime callback: fn (*const Capture, *const R, *const rt.Cancellation) outcomes.Exit(A, E), comptime kind: execution.Kind) Self {
            const program: Self = .{ .ready = struct {
                fn ready(env: *const R, value: Value, state: *rt.State) outcomes.Exit(A, E) {
                    const borrowed: *const Capture = @ptrCast(@alignCast(value.borrowed));
                    return callback(borrowed, env, state.branch_cancellation.?);
                }
            }.ready };
            return program.dispatchValue(.{ .borrowed = capture }, kind);
        }

        fn executor(env: *const R, comptime kind: execution.Kind) execution.Executor {
            const Field = enum { async_engine, parallel_engine, blocking_engine };
            const field: Field = switch (kind) {
                .async_io, .independent => .async_engine,
                .cpu => .parallel_engine,
                .blocking => .blocking_engine,
            };
            if (R == void or !@hasField(R, @tagName(field)) or @FieldType(R, @tagName(field)) != execution.Executor)
                @compileError(diagnostics.effect_execution_requires_environment ++ @tagName(field) ++ ": effect.Executor");
            return @field(env, @tagName(field));
        }

        pub fn dispatch(comptime self: Self, comptime kind: execution.Kind) Self {
            return self.dispatchValue(self.value, kind);
        }

        fn dispatchValue(comptime self: Self, value: Value, comptime kind: execution.Kind) Self {
            const Adapter = struct {
                const Frame = struct {
                    parent: *Job,
                    allocator: std.mem.Allocator,
                    state: rt.State,
                    cancellation: rt.Cancellation = .{},
                    origin: rt.Cancellation.Registration = .{ .data = null, .forward = cancel },
                    handle: execution.Registered = undefined,
                    evaluated: outcomes.Exit(A, E) = undefined,
                    storage: outcomes.CleanupStorage(E),

                    fn cancel(data: ?*anyopaque) void {
                        const frame: *@This() = @ptrCast(@alignCast(data.?));
                        frame.cancellation.request();
                    }

                    fn runWork(data: *anyopaque) void {
                        const frame: *@This() = @ptrCast(@alignCast(data));
                        frame.state.branch_cancellation = &frame.cancellation;
                        var source = self;
                        source.value = frame.parent.program.value;
                        frame.evaluated = evaluateBorrowed(frame.parent.env, source, &frame.state);
                    }

                    fn continueAsync(state: *rt.State, data: ?*anyopaque) rt.Step {
                        const frame: *@This() = @ptrCast(@alignCast(data.?));
                        const parent = frame.parent;
                        const allocator = frame.allocator;
                        const joined = frame.handle.joinProtected(state.protection != 0);
                        if (joined.interrupted) state.interruption = .requested;
                        if (joined.forwarding) |defect| frame.storage.append(&frame.evaluated, .{ .release_defect = defect });
                        if (joined.joining) |defect| frame.storage.append(&frame.evaluated, .{ .release_defect = defect });
                        frame.origin.unregister();
                        state.protected_failure = frame.state.protected_failure;
                        const result_value = frame.evaluated;
                        if (!frame.storage.retained()) allocator.destroy(frame);
                        return parent.finish(result_value);
                    }

                    fn destroy(data: *anyopaque) void {
                        const frame: *@This() = @ptrCast(@alignCast(data));
                        frame.allocator.destroy(frame);
                    }
                };

                fn resolve(env: *const R, captured: Value) rt.Services {
                    var source = self;
                    source.value = captured;
                    return source.services(env);
                }

                fn start(state: *rt.State, parent: *Job) rt.Step {
                    const allocator = state.services.allocatorValue() orelse return parent.finish(.{ .die = .{ .kind = .invalid_configuration, .message = diagnostics.suspended_binding_allocator } });
                    const frame = allocator.create(Frame) catch return parent.finish(.{ .die = outcomes.Defect.out_of_memory });
                    frame.* = .{ .parent = parent, .allocator = allocator, .state = state.*, .storage = .{ .owner = frame, .release = Frame.destroy, .allocator = allocator } };
                    frame.origin.data = frame;
                    if (state.protection == 0) if (state.branch_cancellation orelse state.origin_cancellation) |origin| origin.register(&frame.origin);
                    const submission = executor(parent.env, kind).submit(kind, .{ .data = frame, .run = Frame.runWork, .cancellation = &frame.cancellation });
                    if (submission == .rejected) {
                        frame.origin.unregister();
                        const defect = submission.rejected;
                        allocator.destroy(frame);
                        return parent.finish(.{ .die = defect });
                    }
                    frame.handle.init(submission.accepted, &frame.cancellation);
                    return .{ .data = frame, .advance = if (frame.handle.handle.isComplete()) Frame.continueAsync else null, .suspended = Frame.continueAsync };
                }
            };
            return .{ .value = value, .resolve = Adapter.resolve, .start = Adapter.start };
        }

        pub fn race(comptime self: Self, comptime right: Self) Self {
            const Adapter = struct {
                const Side = enum { left, right };
                const Competitors = std.meta.Tuple(&.{ Self, Self });
                const count = @typeInfo(Competitors).@"struct".fields.len;
                const Worker = struct {
                    frame: *Frame,
                    side: Side,
                    cancellation: rt.Cancellation = .{},
                    origin: rt.Cancellation.Registration = .{ .data = null, .forward = cancel },
                    handle: execution.Registered = undefined,
                    admitted: bool = false,
                    evaluated: outcomes.Exit(A, E) = undefined,
                    protected_failure: bool = false,
                    forwarding: ?outcomes.Defect = null,
                    joining: ?outcomes.Defect = null,
                    cleanup_storage: outcomes.CleanupStorage(E),
                    handle_storage: outcomes.CleanupStorage(E),

                    fn cancel(data: ?*anyopaque) void {
                        const worker: *@This() = @ptrCast(@alignCast(data.?));
                        worker.cancellation.request();
                    }

                    fn runWork(data: *anyopaque) void {
                        const worker: *@This() = @ptrCast(@alignCast(data));
                        var child_state = worker.frame.initial;
                        child_state.branch_cancellation = &worker.cancellation;
                        const program = switch (worker.side) {
                            .left => self,
                            .right => right,
                        };
                        worker.evaluated = evaluateBorrowed(worker.frame.parent.env, program, &child_state);
                        worker.protected_failure = child_state.protected_failure;
                        worker.frame.mutex.lockUncancelable(worker.frame.io);
                        if (worker.frame.winner == null) worker.frame.winner = worker.side;
                        worker.frame.mutex.unlock(worker.frame.io);
                        worker.frame.selected.set(worker.frame.io);
                    }
                };
                const Frame = struct {
                    parent: *Job,
                    allocator: std.mem.Allocator,
                    io: std.Io,
                    initial: rt.State,
                    mutex: std.Io.Mutex = .init,
                    selected: std.Io.Event = .unset,
                    workers: [count]Worker = undefined,
                    winner: ?Side = null,
                    retained: std.atomic.Value(usize) = .init(0),

                    fn continueAsync(state: *rt.State, data: ?*anyopaque) rt.Step {
                        const frame: *@This() = @ptrCast(@alignCast(data.?));
                        const parent = frame.parent;
                        const allocator = frame.allocator;
                        const io = frame.io;
                        const previous = if (state.protection != 0) io.swapCancelProtection(.blocked) else null;
                        defer if (previous) |value| {
                            _ = io.swapCancelProtection(value);
                        };
                        frame.selected.wait(io) catch {
                            state.interruption = .requested;
                            for (&frame.workers) |*worker| worker.cancellation.request();
                        };
                        frame.mutex.lockUncancelable(io);
                        const winner = frame.winner;
                        frame.mutex.unlock(io);
                        if (winner) |chosen| for (&frame.workers) |*worker| {
                            if (worker.side != chosen) worker.cancellation.request();
                        };
                        for (&frame.workers) |*worker| {
                            if (worker.admitted) {
                                const joined = worker.handle.joinProtected(state.protection != 0);
                                worker.forwarding = joined.forwarding;
                                worker.joining = joined.joining;
                                if (joined.interrupted) state.interruption = .requested;
                            }
                            worker.origin.unregister();
                        }
                        const chosen = frame.winner.?;
                        state.protected_failure = frame.workers[@intFromEnum(chosen)].protected_failure;
                        var result_value = frame.workers[@intFromEnum(chosen)].evaluated;
                        for (&frame.workers) |*worker| {
                            if (worker.side != chosen) {
                                if (worker.evaluated == .success) outcomes.discardPayload(&worker.evaluated.success);
                                worker.cleanup_storage.appendFailures(&result_value, worker.evaluated);
                            }
                            if (worker.forwarding) |defect| worker.handle_storage.append(&result_value, .{ .release_defect = defect });
                            if (worker.joining) |defect| worker.handle_storage.append(&result_value, .{ .release_defect = defect });
                            if (worker.cleanup_storage.retained()) _ = frame.retained.fetchAdd(1, .monotonic);
                            if (worker.handle_storage.retained()) _ = frame.retained.fetchAdd(1, .monotonic);
                        }
                        if (frame.retained.load(.acquire) == 0) allocator.destroy(frame);
                        return parent.finish(result_value);
                    }

                    fn releaseStorage(data: *anyopaque) void {
                        const frame: *@This() = @ptrCast(@alignCast(data));
                        if (frame.retained.fetchSub(1, .acq_rel) == 1) frame.allocator.destroy(frame);
                    }
                };

                fn resolve(env: *const R, _: Value) rt.Services {
                    return self.services(env);
                }

                fn start(state: *rt.State, parent: *Job) rt.Step {
                    const allocator = state.services.allocatorValue() orelse return parent.finish(.{ .die = .{ .kind = .invalid_configuration, .message = diagnostics.suspended_binding_allocator } });
                    const io = state.services.ioValue() orelse return parent.finish(.{ .die = .{ .kind = .provider_unavailable, .message = diagnostics.race_requires_environment_i_o_for_synchronization } });
                    const frame = allocator.create(Frame) catch return parent.finish(.{ .die = outcomes.Defect.out_of_memory });
                    frame.* = .{ .parent = parent, .allocator = allocator, .io = io, .initial = state.* };
                    const provider = executor(parent.env, .independent);
                    for (&frame.workers, std.meta.tags(Side)) |*worker, side| {
                        worker.* = .{ .frame = frame, .side = side, .cleanup_storage = .{ .owner = frame, .release = Frame.releaseStorage, .allocator = allocator }, .handle_storage = .{ .owner = frame, .release = Frame.releaseStorage, .allocator = allocator } };
                    }
                    for (&frame.workers) |*worker| {
                        worker.origin.data = worker;
                        if (state.protection == 0) if (state.branch_cancellation orelse state.origin_cancellation) |origin| origin.register(&worker.origin);
                        const submitted = provider.submit(.independent, .{ .data = worker, .run = Worker.runWork, .cancellation = &worker.cancellation });
                        if (submitted == .rejected) {
                            worker.evaluated = .{ .die = submitted.rejected };
                            frame.mutex.lockUncancelable(io);
                            if (frame.winner == null) frame.winner = worker.side;
                            frame.mutex.unlock(io);
                            frame.selected.set(io);
                            continue;
                        }
                        worker.admitted = true;
                        worker.handle.init(submitted.accepted, &worker.cancellation);
                    }
                    var complete = true;
                    for (&frame.workers) |*worker| if (worker.admitted and !worker.handle.handle.isComplete()) {
                        complete = false;
                        break;
                    };
                    return .{ .data = frame, .advance = if (complete) Frame.continueAsync else null, .suspended = Frame.continueAsync };
                }
            };
            return .{ .resolve = Adapter.resolve, .start = Adapter.start };
        }

        pub fn timeout(comptime self: Self, comptime delay: std.Io.Duration, comptime on_timeout: fn () E) Self {
            const Unit = Effect(void, E, R);
            if (delay.nanoseconds < 0) return die(.{ .kind = .invalid_configuration, .message = diagnostics.timeout_duration_must_be_non_negative });
            if (delay.nanoseconds == 0) return fromOutcome(struct {
                fn value(_: *const R) outcomes.Exit(A, E) {
                    return .{ .fail = on_timeout() };
                }
            }.value);
            const timer = comptime Unit.sleep(delay).flatMap(struct {
                fn expired(_: void) Self {
                    return fail(on_timeout());
                }
            }.expired);
            return self.race(timer);
        }

        pub fn timeoutFrom(comptime self: Self, comptime select_delay: fn (*const R) std.Io.Duration, comptime on_timeout: fn () E) Self {
            const Adapter = struct {
                fn expired(_: *const void, env: *const R) outcomes.Exit(A, E) {
                    const duration = select_delay(env);
                    if (duration.nanoseconds < 0) return .{ .die = .{ .kind = .invalid_configuration, .message = diagnostics.timeout_duration_must_be_non_negative } };
                    if (comptime @hasField(R, "clock")) switch (env.clock.sleep(duration)) {
                        .done => {},
                        .interrupted => return .interrupted,
                        .defect => |defect| return .{ .die = defect },
                    } else env.io.sleep(duration, .awake) catch return .interrupted;
                    return .{ .fail = on_timeout() };
                }
                fn factory(env: *const R) Self {
                    const duration = select_delay(env);
                    if (duration.nanoseconds < 0) return die(.{ .kind = .invalid_configuration, .message = diagnostics.timeout_duration_must_be_non_negative });
                    if (duration.nanoseconds == 0) return fail(on_timeout());
                    const timer = comptime fromNativeBorrowed(void, &{}, expired);
                    return self.race(timer);
                }
            };
            return deferEffect(Adapter.factory);
        }

        pub fn allConcurrent(concurrency: usize, effects: []const Self) Effect(outcomes.OwnedSlice(A), E, R) {
            return traverseConcurrent(Self, effects, concurrency, struct {
                fn identity(program: Self) Self {
                    return program.borrow();
                }
            }.identity);
        }

        pub fn forEachConcurrent(comptime Input: type, concurrency: usize, values: []const Input, comptime transform: fn (Input) Self) Effect(outcomes.OwnedSlice(A), E, R) {
            return traverseConcurrent(Input, values, concurrency, transform);
        }

        fn traverseConcurrent(comptime Input: type, values: []const Input, concurrency: usize, comptime transform: fn (Input) Self) Effect(outcomes.OwnedSlice(A), E, R) {
            const Out = Effect(outcomes.OwnedSlice(A), E, R);
            const Adapter = struct {
                const Entry = struct {
                    evaluated: ?outcomes.Exit(A, E) = null,
                    protected_failure: bool = false,
                    storage: outcomes.CleanupStorage(E),
                };
                const Worker = struct {
                    frame: *Frame,
                    handle: execution.Registered = undefined,
                    admitted: bool = false,
                    forwarding: ?outcomes.Defect = null,
                    joining: ?outcomes.Defect = null,
                    storage: outcomes.CleanupStorage(E),

                    fn evaluateItem(frame: *Frame, index: usize, child_state: *rt.State) outcomes.Exit(A, E) {
                        const input_value = outcomes.clonePayload(frame.values[index]) catch return .{ .die = outcomes.Defect.out_of_memory };
                        var program = transform(input_value);
                        defer if (Input != Self) program.deinit();
                        return evaluateBorrowed(frame.parent.env, program, child_state);
                    }

                    fn runWork(data: *anyopaque) void {
                        const worker: *@This() = @ptrCast(@alignCast(data));
                        const frame = worker.frame;
                        while (true) {
                            frame.mutex.lockUncancelable(frame.io);
                            if (frame.terminal != null or frame.cancellation.isRequested() or frame.index == frame.values.len) {
                                frame.mutex.unlock(frame.io);
                                return;
                            }
                            const index = frame.index;
                            frame.index += 1;
                            frame.mutex.unlock(frame.io);
                            // Transform invocation belongs to admitted work.
                            var child_state = frame.initial;
                            child_state.branch_cancellation = &frame.cancellation;
                            const value = evaluateItem(frame, index, &child_state);
                            frame.mutex.lockUncancelable(frame.io);
                            frame.entries[index].evaluated = value;
                            frame.entries[index].protected_failure = child_state.protected_failure;
                            var cancel_siblings = false;
                            if (value == .success) frame.results[index] = value.success else if (frame.terminal == null) {
                                frame.terminal = .{ .input = index };
                                cancel_siblings = true;
                            }
                            frame.mutex.unlock(frame.io);
                            if (cancel_siblings) frame.cancellation.request();
                        }
                    }
                };
                const Frame = struct {
                    const Terminal = union(enum) { input: usize, rejection: outcomes.Defect };
                    parent: *Out.Job,
                    allocator: std.mem.Allocator,
                    io: std.Io,
                    initial: rt.State,
                    mutex: std.Io.Mutex = .init,
                    cancellation: rt.Cancellation = .{},
                    origin: rt.Cancellation.Registration = .{ .data = null, .forward = cancel },
                    values: []const Input,
                    results: []A,
                    entries: []Entry,
                    workers: []Worker,
                    index: usize = 0,
                    terminal: ?Terminal = null,
                    retained: std.atomic.Value(usize) = .init(0),
                    transferred_results: bool = false,

                    fn cancel(data: ?*anyopaque) void {
                        const frame: *@This() = @ptrCast(@alignCast(data.?));
                        frame.cancellation.request();
                    }

                    fn continueAsync(state: *rt.State, data: ?*anyopaque) rt.Step {
                        const frame: *@This() = @ptrCast(@alignCast(data.?));
                        const parent = frame.parent;
                        const allocator = frame.allocator;
                        const entries = frame.entries;
                        const workers = frame.workers;
                        const results = frame.results;
                        for (workers) |*worker| if (worker.admitted) {
                            const joined = worker.handle.joinProtected(state.protection != 0);
                            worker.forwarding = joined.forwarding;
                            worker.joining = joined.joining;
                            if (joined.interrupted) state.interruption = .requested;
                        };
                        frame.origin.unregister();
                        var result_value: outcomes.Exit(outcomes.OwnedSlice(A), E) = if (frame.terminal) |terminal| switch (terminal) {
                            .input => |index| outcomes.castFailure(outcomes.OwnedSlice(A), entries[index].evaluated.?),
                            .rejection => |defect| .{ .die = defect },
                        } else if (frame.cancellation.isRequested()) .interrupted else .{ .success = .{ .items = results, .allocator = allocator } };
                        if (frame.terminal) |terminal| if (terminal == .input) {
                            state.protected_failure = entries[terminal.input].protected_failure;
                        };
                        frame.transferred_results = result_value == .success;
                        for (entries, 0..) |*entry, index| {
                            if (entry.evaluated) |value| {
                                const primary = if (frame.terminal) |terminal| terminal == .input and terminal.input == index else false;
                                if (!primary) entry.storage.appendFailures(&result_value, value);
                                if (entry.storage.retained()) _ = frame.retained.fetchAdd(1, .monotonic);
                            }
                        }
                        for (workers) |*worker| {
                            if (worker.forwarding) |defect| worker.storage.append(&result_value, .{ .release_defect = defect });
                            if (worker.joining) |defect| worker.storage.append(&result_value, .{ .release_defect = defect });
                            if (worker.storage.retained()) _ = frame.retained.fetchAdd(1, .monotonic);
                        }
                        if (frame.retained.load(.acquire) == 0) frame.destroy();
                        return parent.finish(result_value);
                    }

                    fn releaseStorage(data: *anyopaque) void {
                        const frame: *@This() = @ptrCast(@alignCast(data));
                        if (frame.retained.fetchSub(1, .acq_rel) == 1) frame.destroy();
                    }

                    fn destroy(frame: *@This()) void {
                        if (!frame.transferred_results) {
                            for (frame.entries) |*entry| if (entry.evaluated) |*value| {
                                if (value.* == .success) outcomes.discardPayload(&value.success);
                            };
                            frame.allocator.free(frame.results);
                        }
                        frame.allocator.free(frame.entries);
                        frame.allocator.free(frame.workers);
                        frame.allocator.destroy(frame);
                    }
                };

                fn start(state: *rt.State, parent: *Out.Job) rt.Step {
                    const input = parent.program.value.sequence;
                    if (input.concurrency.? == 0) return parent.finish(.{ .die = .{ .kind = .invalid_configuration, .message = diagnostics.concurrency_must_be_positive } });
                    const allocator = state.services.allocatorValue() orelse return parent.finish(.{ .die = .{ .kind = .invalid_configuration, .message = diagnostics.traversal_allocator } });
                    const io = state.services.ioValue() orelse return parent.finish(.{ .die = .{ .kind = .provider_unavailable, .message = diagnostics.concurrent_traversal_requires_environment_i_o_for_synchronization } });
                    const results = allocator.alloc(A, input.len) catch return parent.finish(.{ .die = outcomes.Defect.out_of_memory });
                    const entries = allocator.alloc(Entry, input.len) catch {
                        allocator.free(results);
                        return parent.finish(.{ .die = outcomes.Defect.out_of_memory });
                    };
                    const workers = allocator.alloc(Worker, @min(input.concurrency.?, input.len)) catch {
                        allocator.free(entries);
                        allocator.free(results);
                        return parent.finish(.{ .die = outcomes.Defect.out_of_memory });
                    };
                    const frame = allocator.create(Frame) catch {
                        allocator.free(workers);
                        allocator.free(entries);
                        allocator.free(results);
                        return parent.finish(.{ .die = outcomes.Defect.out_of_memory });
                    };
                    const items: [*]const Input = @ptrCast(@alignCast(input.items));
                    frame.* = .{ .parent = parent, .allocator = allocator, .io = io, .initial = state.*, .values = items[0..input.len], .results = results, .entries = entries, .workers = workers };
                    for (entries) |*entry| entry.* = .{ .storage = .{ .owner = frame, .release = Frame.releaseStorage, .allocator = allocator } };
                    for (workers) |*worker| worker.* = .{ .frame = frame, .storage = .{ .owner = frame, .release = Frame.releaseStorage, .allocator = allocator } };
                    frame.origin.data = frame;
                    if (state.protection == 0) if (state.branch_cancellation orelse state.origin_cancellation) |origin| origin.register(&frame.origin);
                    const provider = executor(parent.env, .independent);
                    for (workers) |*worker| {
                        const submitted = provider.submit(.independent, .{ .data = worker, .run = Worker.runWork, .cancellation = &frame.cancellation });
                        if (submitted == .rejected) {
                            frame.mutex.lockUncancelable(io);
                            if (frame.terminal == null) frame.terminal = .{ .rejection = submitted.rejected };
                            frame.mutex.unlock(io);
                            frame.cancellation.request();
                            break;
                        }
                        worker.admitted = true;
                        worker.handle.init(submitted.accepted, &frame.cancellation);
                    }
                    var complete = true;
                    for (workers) |*worker| if (worker.admitted and !worker.handle.handle.isComplete()) {
                        complete = false;
                        break;
                    };
                    return .{ .data = frame, .advance = if (complete) Frame.continueAsync else null, .suspended = Frame.continueAsync };
                }
            };
            return .{ .value = .{ .sequence = .{ .items = @ptrCast(values.ptr), .len = values.len, .concurrency = concurrency } }, .start = Adapter.start };
        }

        fn traverse(comptime Input: type, values: []const Input, comptime transform: fn (Input) Self) Effect(outcomes.OwnedSlice(A), E, R) {
            const Out = Effect(outcomes.OwnedSlice(A), E, R);
            const Adapter = struct {
                const Frame = struct {
                    parent: *Out.Job,
                    allocator: std.mem.Allocator,
                    values: []const Input,
                    results: []A,
                    index: usize = 0,
                    child: Job = undefined,
                    outcome: outcomes.Exit(A, E) = undefined,
                    inherited: rt.Services,

                    fn next(state: *rt.State, data: ?*anyopaque) rt.Step {
                        const frame: *@This() = @ptrCast(@alignCast(data.?));
                        if (state.cancelled()) return frame.finish(.interrupted);
                        if (frame.index == frame.values.len) {
                            return frame.finish(.{ .success = .{ .items = frame.results, .allocator = frame.allocator } });
                        }
                        const input_value = outcomes.clonePayload(frame.values[frame.index]) catch return frame.finish(.{ .die = outcomes.Defect.out_of_memory });
                        const program = transform(input_value);
                        frame.child = .{
                            .program = program,
                            .inherited = frame.inherited,
                            .owned = if (Input == Self) null else program.ownership,
                            .env = frame.parent.env,
                            .sink = .{ .output = &frame.outcome, .next_step = .{ .data = frame, .advance = complete } },
                        };
                        return frame.child.step();
                    }

                    fn complete(state: *rt.State, data: ?*anyopaque) rt.Step {
                        const frame: *@This() = @ptrCast(@alignCast(data.?));
                        frame.outcome = state.publish(frame.outcome);
                        if (frame.outcome != .success) return frame.finish(outcomes.castFailure(outcomes.OwnedSlice(A), frame.outcome));
                        frame.results[frame.index] = frame.outcome.success;
                        frame.index += 1;
                        return .{ .data = frame, .advance = next };
                    }

                    fn finish(frame: *@This(), value: outcomes.Exit(outcomes.OwnedSlice(A), E)) rt.Step {
                        const parent = frame.parent;
                        if (value != .success) {
                            for (frame.results[0..frame.index]) |*item| outcomes.discardPayload(item);
                            frame.allocator.free(frame.results);
                        }
                        frame.allocator.destroy(frame);
                        return parent.finish(value);
                    }
                };

                fn start(state: *rt.State, parent: *Out.Job) rt.Step {
                    const allocator = state.services.allocatorValue() orelse return parent.finish(.{ .die = .{ .kind = .invalid_configuration, .message = diagnostics.traversal_allocator } });
                    const sequence = parent.program.value.sequence;
                    const results = allocator.alloc(A, sequence.len) catch return parent.finish(.{ .die = outcomes.Defect.out_of_memory });
                    const frame = allocator.create(Frame) catch {
                        allocator.free(results);
                        return parent.finish(.{ .die = outcomes.Defect.out_of_memory });
                    };
                    const input: [*]const Input = @ptrCast(@alignCast(sequence.items));
                    frame.* = .{ .parent = parent, .allocator = allocator, .values = input[0..sequence.len], .results = results, .inherited = state.services };
                    return .{ .data = frame, .advance = Frame.next };
                }
            };
            return .{ .value = .{ .sequence = .{ .items = @ptrCast(values.ptr), .len = values.len } }, .start = Adapter.start };
        }

        pub fn catchAll(comptime self: Self, comptime recover: anytype) Self {
            if (unaryResult(recover, E, .catchAll) != Self)
                @compileError(diagnostics.recovery_result);
            return self.bindExit(Self, struct {
                fn handle(value: outcomes.Exit(A, E), _: *const R, state: *rt.State) Self {
                    return switch (value) {
                        .success => |evaluated| succeedMoved(evaluated),
                        .fail => |err| if (state.cancelled()) blk: {
                            var discarded = err;
                            outcomes.discardPayload(&discarded);
                            break :blk interrupted();
                        } else @call(.always_inline, recover, .{err}),
                        else => terminalFailure(value),
                    };
                }
            }.handle);
        }

        pub fn scopeAcquire(comptime acquisition_description: Self, scope: *scopes.Scope(E), comptime release: fn (A) Effect(void, E, R)) Self {
            return scopeAcquisition(struct {
                const description: Self = acquisition_description;
                fn select(_: Value) Self {
                    return description;
                }
            }.select, struct {
                fn select(value: Value) *scopes.Scope(E) {
                    return @ptrCast(@alignCast(@constCast(value.borrowed)));
                }
            }.select, .{ .borrowed = scope }, release);
        }

        fn scopeAcquireDynamic(self: *Self, allocator: std.mem.Allocator, scope: *scopes.Scope(E), comptime release: fn (A) Effect(void, E, R)) Self {
            const Holder = struct {
                header: rt.DescriptionNode,
                source: Self,
                scope: *scopes.Scope(E),
                fn destroy(header: *rt.DescriptionNode) void {
                    const holder: *@This() = @alignCast(@fieldParentPtr("header", header));
                    header.allocator.destroy(holder);
                }
                fn origin(env: *const anyopaque, data: *const anyopaque) rt.Services {
                    const holder: *const @This() = @ptrCast(@alignCast(data));
                    return holder.source.services(@ptrCast(@alignCast(env)));
                }
                fn select(value: Value) Self {
                    const holder: *const @This() = @ptrCast(@alignCast(value.borrowed));
                    return holder.source;
                }
                fn selectedScope(value: Value) *scopes.Scope(E) {
                    const holder: *const @This() = @ptrCast(@alignCast(value.borrowed));
                    return holder.scope;
                }
            };
            const holder = allocator.create(Holder) catch {
                self.deinit();
                return die(outcomes.Defect.out_of_memory);
            };
            holder.* = .{ .header = .{ .previous = self.ownership, .allocator = allocator, .destroy = Holder.destroy, .origin = .{ .data = holder, .resolve = Holder.origin } }, .source = self.*, .scope = scope };
            self.* = undefined;
            var program = scopeAcquisition(Holder.select, Holder.selectedScope, .{ .borrowed = holder }, release);
            program.ownership = &holder.header;
            return program;
        }

        fn scopeAcquisition(comptime source: fn (Value) Self, comptime selected_scope: fn (Value) *scopes.Scope(E), stored: Value, comptime release: fn (A) Effect(void, E, R)) Self {
            if (comptime outcomes.ownsPayload(A)) @compileError(diagnostics.scoped_acquisitions_must_expose_borrowed_resource_handles_acquire_an_owning_evaluated_by);
            const Unit = Effect(void, E, R);
            const Scope = scopes.Scope(E);
            const Adapter = struct {
                const Node = struct {
                    header: Scope.Finalizer,
                    allocator: std.mem.Allocator,
                    scope: *Scope,
                    parent: *Job,
                    env: R,
                    inherited: rt.Services,
                    annotations: []providers.Annotation,
                    span: ?providers.SpanId,
                    previous_annotations: []const providers.Annotation = undefined,
                    previous_span: ?providers.SpanId = null,
                    finalizer_sink: rt.Sink(void, E) = undefined,
                    acquisition: Job = undefined,
                    acquired: outcomes.Exit(A, E) = undefined,
                    resource: A = undefined,
                    finalizer: Unit.Job = undefined,
                    cleanup: outcomes.Exit(void, E) = undefined,

                    fn destroy(header: *Scope.Finalizer) void {
                        const node: *@This() = @alignCast(@fieldParentPtr("header", header));
                        providers.freeAnnotations(node.allocator, node.annotations);
                        node.allocator.destroy(node);
                    }

                    fn releaseStorage(data: *anyopaque) void {
                        const node: *@This() = @ptrCast(@alignCast(data));
                        destroy(&node.header);
                    }

                    fn startFinalizer(header: *Scope.Finalizer, state: *rt.State, sink: rt.Sink(void, E)) rt.Step {
                        const node: *@This() = @alignCast(@fieldParentPtr("header", header));
                        const program = release(node.resource);
                        node.previous_annotations = state.annotations;
                        node.previous_span = state.span;
                        node.finalizer_sink = sink;
                        state.annotations = node.annotations;
                        state.span = node.span;
                        node.finalizer = .{ .program = program, .owned = program.ownership, .inherited = node.inherited, .env = &node.env, .sink = .{ .output = &node.cleanup, .next_step = .{ .data = node, .advance = restore } } };
                        state.services = node.inherited;
                        return node.finalizer.step();
                    }

                    fn restore(state: *rt.State, data: ?*anyopaque) rt.Step {
                        const node: *@This() = @ptrCast(@alignCast(data.?));
                        state.annotations = node.previous_annotations;
                        state.span = node.previous_span;
                        return node.finalizer_sink.deliver(node.cleanup);
                    }

                    fn acquiredStep(state: *rt.State, data: ?*anyopaque) rt.Step {
                        const node: *@This() = @ptrCast(@alignCast(data.?));
                        if (node.acquired != .success) {
                            const parent = node.parent;
                            const value = node.acquired;
                            destroy(&node.header);
                            return parent.finish(value);
                        }
                        node.resource = node.acquired.success;
                        // Ownership is installed before publishing cancellation.
                        if (node.scope.register(&node.header)) return node.parent.finish(.{ .success = node.resource });
                        state.protection += 1;
                        return startFinalizer(&node.header, state, .{ .output = &node.cleanup, .next_step = .{ .data = node, .advance = rejected } });
                    }

                    fn rejected(state: *rt.State, data: ?*anyopaque) rt.Step {
                        const node: *@This() = @ptrCast(@alignCast(data.?));
                        state.protection -= 1;
                        var evaluated: outcomes.Exit(A, E) = .{ .die = .{ .kind = .invalid_configuration, .message = diagnostics.cannot_register_a_resource_in_a_closing_or_closed_scope } };
                        node.header.storage.appendExit(&evaluated, node.cleanup);
                        const parent = node.parent;
                        if (!node.header.storage.retained()) destroy(&node.header);
                        return parent.finish(evaluated);
                    }
                };

                fn resolve(env: *const R, value: Value) rt.Services {
                    return source(value).services(env);
                }

                fn start(state: *rt.State, parent: *Job) rt.Step {
                    const allocator = state.services.allocatorValue() orelse return parent.finish(.{ .die = .{ .kind = .invalid_configuration, .message = diagnostics.resource_allocator } });
                    const node = allocator.create(Node) catch return parent.finish(.{ .die = outcomes.Defect.out_of_memory });
                    const annotations_copy = providers.cloneAnnotations(allocator, state.annotations) catch {
                        allocator.destroy(node);
                        return parent.finish(.{ .die = outcomes.Defect.out_of_memory });
                    };
                    node.* = .{
                        .allocator = allocator,
                        .scope = selected_scope(parent.program.value),
                        .parent = parent,
                        .env = parent.env.*,
                        .inherited = state.services,
                        .annotations = annotations_copy,
                        .span = state.span,
                        .header = .{ .start = Node.startFinalizer, .destroy = Node.destroy, .storage = .{ .owner = node, .release = Node.releaseStorage, .allocator = allocator } },
                    };
                    const acquisition = source(parent.program.value);
                    node.acquisition = .{ .program = acquisition.borrow(), .inherited = state.services, .env = parent.env, .sink = .{ .output = &node.acquired, .next_step = .{ .data = node, .advance = Node.acquiredStep } } };
                    return node.acquisition.step();
                }
            };
            return .{ .value = stored, .resolve = Adapter.resolve, .start = Adapter.start };
        }

        pub fn closeScope(scope: *scopes.Scope(E)) Effect(void, E, R) {
            return scopeCloser(scope, false);
        }

        fn scopeCloser(scope: *scopes.Scope(E), comptime destroy: bool) Effect(void, E, R) {
            const Unit = Effect(void, E, R);
            const Adapter = struct {
                fn complete(_: *rt.State, data: ?*anyopaque, value: outcomes.Exit(void, E)) rt.Step {
                    const parent: *Unit.Job = @ptrCast(@alignCast(data.?));
                    if (destroy) {
                        const owned: *scopes.Scope(E) = @ptrCast(@alignCast(@constCast(parent.program.value.borrowed)));
                        const allocator = owned.allocator;
                        owned.deinit();
                        allocator.destroy(owned);
                    }
                    return parent.finish(value);
                }

                fn start(state: *rt.State, parent: *Unit.Job) rt.Step {
                    const target: *scopes.Scope(E) = @ptrCast(@alignCast(@constCast(parent.program.value.borrowed)));
                    return switch (target.admitClose()) {
                        .completed => |value| complete(state, parent, value),
                        .owner => target.drain(state, parent, complete),
                        .pending => .{ .data = parent, .advance = null, .suspended = waitClosed },
                    };
                }
                fn waitClosed(state: *rt.State, data: ?*anyopaque) rt.Step {
                    const parent: *Unit.Job = @ptrCast(@alignCast(data.?));
                    const target: *scopes.Scope(E) = @ptrCast(@alignCast(@constCast(parent.program.value.borrowed)));
                    return complete(state, parent, target.beginClose().?);
                }
            };
            return .{ .value = .{ .borrowed = scope }, .start = Adapter.start };
        }

        pub fn scoped(comptime use_scope: fn (*scopes.Scope(E)) Self) Self {
            const Acquire = Effect(*scopes.Scope(E), E, R);
            const Factory = struct {
                fn create(env: *const R) outcomes.Exit(*scopes.Scope(E), E) {
                    const allocator = env.allocator;
                    const scope = allocator.create(scopes.Scope(E)) catch return .{ .die = outcomes.Defect.out_of_memory };
                    scope.* = scopes.Scope(E).init(env);
                    return .{ .success = scope };
                }

                fn release(scope: *scopes.Scope(E), _: outcomes.PrimaryOutcome(E)) Effect(void, E, R) {
                    return scopeCloser(scope, true);
                }
            };
            return (comptime Acquire.fromOutcome(Factory.create)).acquireRelease(use_scope, Factory.release);
        }

        pub fn sleep(delay: std.Io.Duration) Effect(void, E, R) {
            const Unit = Effect(void, E, R);
            const Adapter = struct {
                const Frame = struct {
                    parent: *Unit.Job,
                    allocator: std.mem.Allocator,
                    io: std.Io,
                    duration: std.Io.Duration,
                    future: std.Io.Future(void) = undefined,
                    result_value: outcomes.Exit(void, E) = .interrupted,
                    done: std.atomic.Value(bool) = .init(false),
                    requested: std.atomic.Value(bool) = .init(false),
                    wake: std.Io.Event = .unset,
                    registration: rt.Cancellation.Registration = .{ .data = null, .forward = request },
                    fn request(data: ?*anyopaque) void {
                        const frame: *@This() = @ptrCast(@alignCast(data.?));
                        frame.requested.store(true, .release);
                        frame.wake.set(frame.io);
                    }
                    fn work(frame: *@This()) void {
                        if (comptime @hasField(R, "clock")) frame.result_value = switch (frame.parent.env.clock.sleep(frame.duration)) {
                            .done => .{ .success = {} },
                            .interrupted => .interrupted,
                            .defect => |defect| .{ .die = defect },
                        } else {
                            frame.io.sleep(frame.duration, .awake) catch {
                                frame.done.store(true, .release);
                                frame.wake.set(frame.io);
                                return;
                            };
                            frame.result_value = .{ .success = {} };
                        }
                        frame.done.store(true, .release);
                        frame.wake.set(frame.io);
                    }
                    fn continueAsync(state: *rt.State, data: ?*anyopaque) rt.Step {
                        const frame: *@This() = @ptrCast(@alignCast(data.?));
                        const io = frame.io;
                        const previous = if (state.protection != 0) io.swapCancelProtection(.blocked) else null;
                        defer if (previous) |value| {
                            _ = io.swapCancelProtection(value);
                        };
                        frame.wake.wait(frame.io) catch {
                            state.interruption = .requested;
                            frame.requested.store(true, .release);
                        };
                        const protection = frame.io.swapCancelProtection(.blocked);
                        if (frame.requested.load(.acquire) and !frame.done.load(.acquire)) frame.future.cancel(frame.io) else frame.future.await(frame.io);
                        _ = frame.io.swapCancelProtection(protection);
                        frame.registration.unregister();
                        const parent = frame.parent;
                        const result_value = frame.result_value;
                        frame.allocator.destroy(frame);
                        return parent.finish(result_value);
                    }
                };
                fn start(state: *rt.State, parent: *Unit.Job) rt.Step {
                    const duration = parent.program.value.duration;
                    if (duration.nanoseconds < 0) return parent.finish(.{ .die = .{ .kind = .invalid_configuration, .message = diagnostics.sleep_duration_must_be_non_negative } });
                    if (duration.nanoseconds == 0) return parent.finish(.{ .success = {} });
                    const allocator = state.services.allocatorValue() orelse return parent.finish(.{ .die = .{ .kind = .invalid_configuration, .message = diagnostics.suspended_binding_allocator } });
                    const io = state.services.ioValue() orelse return parent.finish(.{ .die = .{ .kind = .provider_unavailable, .message = diagnostics.sleep_requires_i_o_through_r } });
                    const frame = allocator.create(Frame) catch return parent.finish(.{ .die = outcomes.Defect.out_of_memory });
                    frame.* = .{ .parent = parent, .allocator = allocator, .io = io, .duration = duration };
                    frame.registration.data = frame;
                    if (state.protection == 0) if (state.branch_cancellation orelse state.origin_cancellation orelse state.services.cancellation) |token| token.register(&frame.registration);
                    frame.future = io.concurrent(Frame.work, .{frame}) catch {
                        frame.registration.unregister();
                        allocator.destroy(frame);
                        return parent.finish(.{ .die = .{ .kind = .provider_unavailable, .message = diagnostics.sleep_requires_independent_native_progress } });
                    };
                    return .{ .data = frame, .advance = null, .suspended = Frame.continueAsync };
                }
            };
            return .{ .value = .{ .duration = delay }, .start = Adapter.start };
        }

        pub fn log(comptime level: providers.LogLevel, comptime message: []const u8) Effect(void, E, R) {
            const Unit = Effect(void, E, R);
            if (R == void or !@hasField(R, "logger") or @FieldType(R, "logger") != providers.Logger)
                @compileError(diagnostics.logging_requires_environment_logger_effect_logger);
            return .{ .ready = struct {
                fn ready(env: *const R, _: Unit.Value, state: *rt.State) outcomes.Exit(void, E) {
                    env.logger.write(level, message, state.annotations, state.span);
                    return .{ .success = {} };
                }
            }.ready };
        }

        pub fn logFrom(comptime level: providers.LogLevel, comptime message: fn (*const R) []const u8) Effect(void, E, R) {
            const Unit = Effect(void, E, R);
            if (@FieldType(R, "logger") != providers.Logger) @compileError(diagnostics.r_logger_must_be_effect_logger);
            return .{ .ready = struct {
                fn ready(env: *const R, _: Unit.Value, state: *rt.State) outcomes.Exit(void, E) {
                    env.logger.write(level, message(env), state.annotations, state.span);
                    return .{ .success = {} };
                }
            }.ready };
        }

        fn traceTime(env: *const R, state: *rt.State) ?std.Io.Timestamp {
            if (comptime R != void and @hasField(R, "clock")) return env.clock.now();
            if (state.services.ioValue()) |io| return .now(io, .awake);
            return null;
        }

        pub fn withSpan(comptime self: Self, comptime name: []const u8) Self {
            return self.withSpanFrom(struct {
                fn select(_: *const R) []const u8 {
                    return name;
                }
            }.select);
        }

        pub fn withSpanFrom(comptime self: Self, comptime select_name: fn (*const R) []const u8) Self {
            if (R == void or !@hasField(R, "tracer") or @FieldType(R, "tracer") != providers.Tracer)
                @compileError(diagnostics.tracing_requires_environment_tracer_effect_tracer);
            const Adapter = struct {
                fn resolve(env: *const R, _: Value) rt.Services {
                    return self.services(env);
                }

                fn ready(env: *const R, _: Value, state: *rt.State) outcomes.Exit(A, E) {
                    const tracer = env.tracer;
                    const parent = state.span;
                    const id = tracer.begin(parent, select_name(env), state.annotations, traceTime(env, state));
                    state.span = id orelse parent;
                    const result_value = state.publish(self.ready.?(env, self.value, state));
                    state.span = parent;
                    if (id) |span| tracer.end(span, result_value, traceTime(env, state));
                    return result_value;
                }

                const Frame = struct {
                    parent: *Job,
                    allocator: std.mem.Allocator,
                    tracer: providers.Tracer,
                    previous: ?providers.SpanId,
                    id: ?providers.SpanId,
                    child: Job = undefined,
                    evaluated: outcomes.Exit(A, E) = undefined,

                    fn complete(state: *rt.State, data: ?*anyopaque) rt.Step {
                        const frame: *@This() = @ptrCast(@alignCast(data.?));
                        const result_value = state.publish(frame.evaluated);
                        state.span = frame.previous;
                        if (frame.id) |id| frame.tracer.end(id, result_value, traceTime(frame.parent.env, state));
                        const parent = frame.parent;
                        frame.allocator.destroy(frame);
                        return parent.finish(result_value);
                    }
                };

                fn start(state: *rt.State, parent: *Job) rt.Step {
                    const tracer = parent.env.tracer;
                    const allocator = state.services.allocatorValue() orelse {
                        tracer.diagnostics.report(.{ .kind = .invalid_configuration, .message = diagnostics.suspended_binding_allocator });
                        parent.replace(self);
                        return parent.step();
                    };
                    const frame = allocator.create(Frame) catch {
                        tracer.diagnostics.report(outcomes.Defect.out_of_memory);
                        parent.replace(self);
                        return parent.step();
                    };
                    frame.* = .{ .parent = parent, .allocator = allocator, .tracer = tracer, .previous = state.span, .id = tracer.begin(state.span, select_name(parent.env), state.annotations, traceTime(parent.env, state)) };
                    state.span = frame.id orelse state.span;
                    frame.child = .{ .program = self, .inherited = state.services, .env = parent.env, .sink = .{ .output = &frame.evaluated, .next_step = .{ .data = frame, .advance = Frame.complete } } };
                    return frame.child.step();
                }
            };
            return .{ .resolve = Adapter.resolve, .ready = if (self.ready != null) Adapter.ready else null, .start = Adapter.start };
        }

        pub fn annotate(comptime self: Self, annotations: []const providers.Annotation) Self {
            const Adapter = struct {
                const Frame = struct {
                    parent: *Job,
                    allocator: std.mem.Allocator,
                    previous: []const providers.Annotation,
                    merged: []providers.Annotation,
                    count: usize,
                    child: Job = undefined,
                    evaluated: outcomes.Exit(A, E) = undefined,

                    fn complete(state: *rt.State, data: ?*anyopaque) rt.Step {
                        const frame: *@This() = @ptrCast(@alignCast(data.?));
                        state.annotations = frame.previous;
                        const parent = frame.parent;
                        const value = frame.evaluated;
                        frame.allocator.free(frame.merged);
                        frame.allocator.destroy(frame);
                        return parent.finish(value);
                    }
                };

                fn resolve(env: *const R, _: Value) rt.Services {
                    return self.services(env);
                }

                fn start(state: *rt.State, parent: *Job) rt.Step {
                    const allocator = state.services.allocatorValue() orelse return parent.finish(.{ .die = .{ .kind = .invalid_configuration, .message = diagnostics.suspended_binding_allocator } });
                    const additional = parent.program.value.annotations;
                    const capacity = std.math.add(usize, state.annotations.len, additional.len) catch return parent.finish(.{ .die = outcomes.Defect.out_of_memory });
                    const merged = allocator.alloc(providers.Annotation, capacity) catch return parent.finish(.{ .die = outcomes.Defect.out_of_memory });
                    const frame = allocator.create(Frame) catch {
                        allocator.free(merged);
                        return parent.finish(.{ .die = outcomes.Defect.out_of_memory });
                    };
                    @memcpy(merged[0..state.annotations.len], state.annotations);
                    var count = state.annotations.len;
                    for (additional) |annotation| {
                        var replaced = false;
                        for (merged[0..count]) |*existing| if (std.mem.eql(u8, existing.key, annotation.key)) {
                            existing.* = annotation;
                            replaced = true;
                            break;
                        };
                        if (!replaced) {
                            merged[count] = annotation;
                            count += 1;
                        }
                    }
                    frame.* = .{ .parent = parent, .allocator = allocator, .previous = state.annotations, .merged = merged, .count = count };
                    state.annotations = merged[0..count];
                    frame.child = .{ .program = self, .inherited = state.services, .env = parent.env, .sink = .{ .output = &frame.evaluated, .next_step = .{ .data = frame, .advance = Frame.complete } } };
                    return frame.child.step();
                }
            };
            return .{ .value = .{ .annotations = annotations }, .resolve = Adapter.resolve, .start = Adapter.start };
        }

        fn clockOutcome(value: providers.Clock.SleepResult, state: *rt.State) outcomes.Exit(void, E) {
            return switch (value) {
                .done => .{ .success = {} },
                .defect => |defect| .{ .die = defect },
                .interrupted => blk: {
                    state.interruption = .requested;
                    break :blk .interrupted;
                },
            };
        }

        pub fn retry(comptime self: Self, comptime policy: anytype) Self {
            return self.iterate(false, struct {
                fn select(_: *const R) @TypeOf(policy) {
                    return policy;
                }
            }.select);
        }

        pub fn repeat(comptime self: Self, comptime policy: anytype) Self {
            return self.iterate(true, struct {
                fn select(_: *const R) @TypeOf(policy) {
                    return policy;
                }
            }.select);
        }

        pub fn retryFrom(comptime self: Self, comptime policy: anytype) Self {
            return self.iterate(false, policy);
        }
        pub fn repeatFrom(comptime self: Self, comptime policy: anytype) Self {
            return self.iterate(true, policy);
        }

        fn iterate(comptime self: Self, comptime repeating: bool, comptime select_policy: anytype) Self {
            const Policy = unaryResult(select_policy, *const R, .service);
            const Input = if (repeating) A else E;
            if (Policy.Input != Input) @compileError(diagnostics.schedule_input_must_match_the_selected_effect_success_error_channel);
            if (Policy.uses_random and (R == void or !@hasField(R, "random") or @FieldType(R, "random") != providers.Random))
                @compileError(diagnostics.jittered_scheduling_requires_environment_random_effect_random);
            const Adapter = struct {
                const Frame = struct {
                    parent: *Job,
                    allocator: std.mem.Allocator,
                    inherited: rt.Services,
                    policy: Policy,
                    io: std.Io = undefined,
                    clock: providers.Clock,
                    started: std.Io.Timestamp,
                    child: Job = undefined,
                    outcome: outcomes.Exit(A, E) = undefined,
                    sleeping: Effect(void, E, R).Job = undefined,
                    slept: outcomes.Exit(void, E) = undefined,

                    fn attempt(_: *rt.State, data: ?*anyopaque) rt.Step {
                        const frame: *@This() = @ptrCast(@alignCast(data.?));
                        frame.child = .{ .program = self, .inherited = frame.inherited, .env = frame.parent.env, .sink = .{
                            .output = &frame.outcome,
                            .next_step = .{ .data = frame, .advance = complete },
                        } };
                        return frame.child.step();
                    }

                    fn complete(state: *rt.State, data: ?*anyopaque) rt.Step {
                        const frame: *@This() = @ptrCast(@alignCast(data.?));
                        frame.outcome = state.publish(frame.outcome);
                        const selected = if (repeating) frame.outcome == .success else frame.outcome == .fail;
                        if (!selected) return frame.finish(frame.outcome);
                        const input = if (repeating) frame.outcome.success else frame.outcome.fail;
                        const sample = if (Policy.uses_random) frame.parent.env.random.sample() else 0;
                        const decision = frame.policy.step(input, frame.started.durationTo(frame.clock.now()), sample);
                        switch (decision) {
                            .stop => return frame.finish(frame.outcome),
                            .invalid => |defect| {
                                frame.outcome.deinit();
                                return frame.finish(.{ .die = defect });
                            },
                            .continue_with => |next| {
                                frame.outcome.deinit();
                                if (state.cancelled()) return frame.finish(.interrupted);
                                const program = sleep(next.delay);
                                frame.sleeping = .{ .program = program, .env = frame.parent.env, .inherited = frame.inherited, .sink = .{ .output = &frame.slept, .next_step = .{ .data = frame, .advance = sleptStep } } };
                                return frame.sleeping.step();
                            },
                        }
                    }

                    fn sleptStep(state: *rt.State, data: ?*anyopaque) rt.Step {
                        const frame: *@This() = @ptrCast(@alignCast(data.?));
                        const value = state.publish(frame.slept);
                        if (value != .success) return frame.finish(outcomes.castFailure(A, value));
                        return .{ .data = frame, .advance = attempt };
                    }

                    fn finish(frame: *@This(), value: outcomes.Exit(A, E)) rt.Step {
                        const parent = frame.parent;
                        frame.allocator.destroy(frame);
                        return parent.finish(value);
                    }
                };

                fn resolve(env: *const R, _: Value) rt.Services {
                    return self.services(env);
                }

                fn start(state: *rt.State, parent: *Job) rt.Step {
                    const allocator = state.services.allocatorValue() orelse return parent.finish(.{ .die = .{ .kind = .invalid_configuration, .message = diagnostics.suspended_binding_allocator } });
                    const frame = allocator.create(Frame) catch return parent.finish(.{ .die = outcomes.Defect.out_of_memory });
                    frame.* = .{ .parent = parent, .allocator = allocator, .inherited = state.services, .policy = select_policy(parent.env), .clock = undefined, .started = undefined };
                    if (comptime R != void and @hasField(R, "clock")) {
                        frame.clock = parent.env.clock;
                    } else {
                        frame.io = state.services.ioValue() orelse {
                            allocator.destroy(frame);
                            return parent.finish(.{ .die = .{ .kind = .provider_unavailable, .message = diagnostics.scheduling_requires_a_clock_or_i_o_supplied_through_the_environment } });
                        };
                        frame.clock = providers.Clock.fromIo(&frame.io);
                    }
                    frame.started = frame.clock.now();
                    return .{ .data = frame, .advance = Frame.attempt };
                }
            };
            return .{ .resolve = Adapter.resolve, .start = Adapter.start };
        }

        pub fn acquireRelease(comptime self: Self, comptime use: anytype, comptime release: anytype) continuationResult(use, A, R, .acquireRelease_body) {
            const Out = continuationResult(use, A, R, .acquireRelease_body);
            if (self.resolve == null and (R == void or !@hasField(R, @tagName(rt.EnvironmentField.allocator))))
                @compileError(diagnostics.resource_allocator_capability);
            return acquireReleaseKernel(use, release, struct {
                fn select(_: *const R, _: Out.Value) Self {
                    return self;
                }
            }.select, @as(Out.Value, .none), null);
        }

        /// Consumes a runtime-owned acquisition while retaining the same
        /// bracket semantics and cleanup ordering as `acquireRelease`.
        pub fn acquireReleaseOwned(self: *Self, env: *const R, comptime use: anytype, comptime release: anytype) continuationResult(use, A, R, .acquireRelease_body) {
            const Out = continuationResult(use, A, R, .acquireRelease_body);
            if (R == void or !@hasField(R, @tagName(rt.EnvironmentField.allocator)))
                @compileError(diagnostics.resource_allocator_capability);
            const Node = struct {
                header: rt.DescriptionNode,
                source: Self,

                fn destroy(header: *rt.DescriptionNode) void {
                    const node: *@This() = @alignCast(@fieldParentPtr("header", header));
                    header.allocator.destroy(node);
                }

                fn origin(env_data: *const anyopaque, source_data: *const anyopaque) rt.Services {
                    const node: *const @This() = @ptrCast(@alignCast(source_data));
                    return node.source.services(@ptrCast(@alignCast(env_data)));
                }

                fn select(_: *const R, value: Out.Value) Self {
                    const node: *const @This() = @ptrCast(@alignCast(value.borrowed));
                    return node.source;
                }
            };
            const allocator = self.services(env).allocatorValue() orelse {
                self.deinit();
                return Out.die(.{ .kind = .invalid_configuration, .message = diagnostics.resource_allocator });
            };
            const node = allocator.create(Node) catch {
                self.deinit();
                return Out.die(outcomes.Defect.out_of_memory);
            };
            node.* = .{
                .header = .{ .previous = self.ownership, .allocator = allocator, .destroy = Node.destroy, .origin = .{ .data = node, .resolve = Node.origin } },
                .source = self.*,
            };
            self.* = undefined;
            return acquireReleaseKernel(use, release, Node.select, .{ .borrowed = node }, &node.header);
        }

        fn acquireReleaseKernel(
            comptime use: anytype,
            comptime release: anytype,
            comptime select: anytype,
            stored: continuationResult(use, A, R, .acquireRelease_body).Value,
            description_owner: ?*rt.DescriptionNode,
        ) continuationResult(use, A, R, .acquireRelease_body) {
            const Out = continuationResult(use, A, R, .acquireRelease_body);
            const Release = Effect(void, E, R);
            comptime requireEffect(Out, E, R, .acquireRelease_body);
            _ = functionInfo(release, .acquireRelease_finalizer);
            const Finalizer = fn (A, outcomes.PrimaryOutcome(E)) Release;
            const EnvironmentFinalizer = fn (A, outcomes.PrimaryOutcome(E), *const R) Release;
            if (@TypeOf(release) != Finalizer and @TypeOf(release) != EnvironmentFinalizer)
                @compileError(diagnostics.finalizer_contract);
            return .{
                .value = stored,
                .ownership = description_owner,
                .resolve = struct {
                    fn resolve(env: *const R, value: Out.Value) rt.Services {
                        return select(env, value).services(env);
                    }
                }.resolve,
                .start = struct {
                    const Frame = struct {
                        // A finalizer's structured failure can prepend its primary
                        // failure and its first cleanup record. All remaining
                        // records already own linked storage. Reserve exactly the
                        // tuple's field count, not a tuning constant.
                        const ReleasePrefix = std.meta.Tuple(&.{ outcomes.PrimaryOutcome(E), outcomes.CleanupFailure(E) });
                        const release_prefix_capacity = @typeInfo(ReleasePrefix).@"struct".fields.len;
                        parent: *Out.Job,
                        allocator: std.mem.Allocator,
                        source: Self,
                        acquisition: Job,
                        acquired: outcomes.Exit(A, E) = undefined,
                        body: Out.Job,
                        body_result: outcomes.Exit(Out.Success, E) = undefined,
                        body_protected_failure: bool = false,
                        finalizer: Release.Job,
                        cleanup: outcomes.Exit(void, E) = undefined,
                        resource: A = undefined,
                        retained_nodes: std.atomic.Value(usize) = .init(0),
                        nodes: [release_prefix_capacity]outcomes.CleanupNode(E) = undefined,

                        fn acquiredStep(state: *rt.State, data: ?*anyopaque) rt.Step {
                            const frame: *@This() = @ptrCast(@alignCast(data.?));
                            switch (frame.acquired) {
                                .success => |acquired_resource| frame.resource = acquired_resource,
                                else => {
                                    const parent = frame.parent;
                                    const failure = outcomes.castFailure(Out.Success, frame.acquired);
                                    frame.allocator.destroy(frame);
                                    return parent.finish(failure);
                                },
                            }
                            // The frame already owns the resource, even if the
                            // acquisition just requested cancellation.
                            if (state.cancelled()) {
                                frame.body_result = .interrupted;
                                return .{ .data = frame, .advance = bodyStep };
                            }
                            const body = callContinuation(use, frame.resource, frame.parent.env);
                            frame.body = .{ .program = body, .inherited = state.services, .owned = body.ownership, .env = frame.parent.env, .sink = .{
                                .output = &frame.body_result,
                                .next_step = .{ .data = frame, .advance = bodyStep },
                            } };
                            return frame.body.step();
                        }

                        fn bodyStep(state: *rt.State, data: ?*anyopaque) rt.Step {
                            const frame: *@This() = @ptrCast(@alignCast(data.?));
                            frame.body_protected_failure = state.protected_failure;
                            frame.body_result = state.publish(frame.body_result);
                            state.services = frame.source.services(frame.parent.env).inherit(frame.parent.inherited);
                            state.protection += 1;
                            const finalizer = if (comptime @TypeOf(release) == EnvironmentFinalizer)
                                release(frame.resource, frame.body_result.primary(), frame.parent.env)
                            else
                                release(frame.resource, frame.body_result.primary());
                            frame.finalizer = .{ .program = finalizer, .inherited = state.services, .owned = finalizer.ownership, .env = frame.parent.env, .sink = .{
                                .output = &frame.cleanup,
                                .next_step = .{ .data = frame, .advance = cleanupStep },
                            } };
                            return frame.finalizer.step();
                        }

                        fn append(frame: *@This(), evaluated: *outcomes.Exit(Out.Success, E), failure: outcomes.CleanupFailure(E)) void {
                            if (evaluated.* != .finalization_failed) {
                                const primary = evaluated.primary();
                                if (evaluated.* == .success) outcomes.discardPayload(&evaluated.success);
                                evaluated.* = .{ .finalization_failed = .{ .primary = primary, .first = failure } };
                                return;
                            }
                            const node = &frame.nodes[frame.retained_nodes.fetchAdd(1, .monotonic)];
                            node.* = .{ .failure = failure, .allocator = frame.allocator, .owner = frame, .release_owner = releaseNode };
                            appendChain(&evaluated.finalization_failed, node, node);
                        }

                        fn appendChain(details: *outcomes.Finalization(E), head: ?*outcomes.CleanupNode(E), tail: ?*outcomes.CleanupNode(E)) void {
                            if (head == null) return;
                            if (details.rest) |rest| {
                                var last = details.tail orelse rest;
                                // Legacy/hand-built metadata need not have a tail.
                                while (last.next) |next| last = next;
                                last.next = head;
                            } else details.rest = head;
                            details.tail = tail;
                        }

                        fn releaseNode(owner: *anyopaque) void {
                            const frame: *@This() = @ptrCast(@alignCast(owner));
                            if (frame.retained_nodes.fetchSub(1, .acq_rel) == 1) frame.allocator.destroy(frame);
                        }

                        fn cleanupStep(state: *rt.State, data: ?*anyopaque) rt.Step {
                            const frame: *@This() = @ptrCast(@alignCast(data.?));
                            state.protection -= 1;
                            state.protected_failure = frame.body_protected_failure;
                            state.services = frame.source.services(frame.parent.env).inherit(frame.parent.inherited);
                            var evaluated = state.publish(frame.body_result);
                            switch (frame.cleanup) {
                                .success => {},
                                .fail => |err| frame.append(&evaluated, .{ .release_failure = err }),
                                .die => |defect| frame.append(&evaluated, .{ .release_defect = defect }),
                                .interrupted => frame.append(&evaluated, .release_interrupted),
                                .finalization_failed => |details| {
                                    switch (details.primary) {
                                        .succeeded => {},
                                        .failed => |err| frame.append(&evaluated, .{ .release_failure = err }),
                                        .defected => |defect| frame.append(&evaluated, .{ .release_defect = defect }),
                                        .was_interrupted => frame.append(&evaluated, .release_interrupted),
                                    }
                                    frame.append(&evaluated, details.first);
                                    appendChain(&evaluated.finalization_failed, details.rest, details.tail);
                                },
                            }
                            const parent = frame.parent;
                            if (frame.retained_nodes.load(.acquire) == 0) frame.allocator.destroy(frame);
                            return parent.finish(evaluated);
                        }
                    };

                    fn start(state: *rt.State, parent: *Out.Job) rt.Step {
                        const allocator = state.services.allocatorValue() orelse return parent.finish(.{ .die = .{
                            .kind = .invalid_configuration,
                            .message = diagnostics.resource_allocator,
                        } });
                        // This single allocation reserves execution storage and
                        // all structurally required cleanup links BEFORE acquisition. Cleanup
                        // never allocates after a resource has been acquired.
                        const frame = allocator.create(Frame) catch return parent.finish(.{ .die = outcomes.Defect.out_of_memory });
                        const source = select(parent.env, parent.program.value);
                        frame.* = .{ .parent = parent, .allocator = allocator, .source = source, .acquisition = undefined, .body = undefined, .finalizer = undefined };
                        frame.acquisition = .{ .program = source, .inherited = state.services, .env = parent.env, .sink = .{
                            .output = &frame.acquired,
                            .next_step = .{ .data = frame, .advance = Frame.acquiredStep },
                        } };
                        return frame.acquisition.step();
                    }
                }.start,
            };
        }

        pub fn provide(comptime self: Self, env: *const R) Effect(A, E, void) {
            const Out = Effect(A, E, void);
            const Adapter = struct {
                fn project(_: *const void, value: Out.Value) *const R {
                    return @ptrCast(@alignCast(value.borrowed));
                }
            };
            var provided = self.bridge(void, Adapter.project);
            provided.value = .{ .borrowed = env };
            return provided;
        }

        pub fn provideSome(comptime self: Self, comptime Large: type, comptime project: anytype) Effect(A, E, Large) {
            if (unaryResult(project, *const Large, .provideSome) != *const R)
                @compileError(diagnostics.projection_contract);
            const Out = Effect(A, E, Large);
            return self.bridge(Large, struct {
                fn projectEnv(env: *const Large, _: Out.Value) *const R {
                    return project(env);
                }
            }.projectEnv);
        }

        /// Bridge a runtime-built description into a larger environment. The
        /// supplied source environment must remain alive until the returned
        /// Effect has joined or been disposed; callers can retain it in an
        /// owning continuation capture.
        pub fn provideOwned(self: *Self, source_env: *const R, comptime Large: type, outer_env: *const Large) Effect(A, E, Large) {
            const Out = Effect(A, E, Large);
            const Node = struct {
                header: rt.DescriptionNode,
                source: Self,
                source_env: *const R,

                fn destroy(header: *rt.DescriptionNode) void {
                    const node: *@This() = @alignCast(@fieldParentPtr("header", header));
                    header.allocator.destroy(node);
                }

                fn origin(_: *const anyopaque, data: *const anyopaque) rt.Services {
                    const node: *const @This() = @ptrCast(@alignCast(data));
                    return node.source.services(node.source_env);
                }

                fn resolve(_: *const Large, value: Out.Value) rt.Services {
                    const node: *const @This() = @ptrCast(@alignCast(value.borrowed));
                    return node.source.services(node.source_env);
                }

                const Frame = struct {
                    child: Job,
                    evaluated: outcomes.Exit(A, E) = undefined,
                    parent: *Out.Job,
                    allocator: std.mem.Allocator,

                    fn complete(_: *rt.State, data: ?*anyopaque) rt.Step {
                        const frame: *@This() = @ptrCast(@alignCast(data.?));
                        const evaluated = frame.evaluated;
                        const parent = frame.parent;
                        frame.allocator.destroy(frame);
                        return parent.finish(evaluated);
                    }
                };

                fn start(state: *rt.State, parent: *Out.Job) rt.Step {
                    const node: *const @This() = @ptrCast(@alignCast(parent.program.value.borrowed));
                    const allocator = state.services.allocatorValue() orelse return parent.finish(.{ .die = .{
                        .kind = .invalid_configuration,
                        .message = diagnostics.provision_allocator,
                    } });
                    const frame = allocator.create(Frame) catch return parent.finish(.{ .die = outcomes.Defect.out_of_memory });
                    frame.* = .{ .parent = parent, .allocator = allocator, .child = .{
                        .program = node.source,
                        .inherited = state.services,
                        .env = node.source_env,
                        .sink = .{ .output = &frame.evaluated, .next_step = .{ .data = frame, .advance = Frame.complete } },
                    } };
                    return frame.child.step();
                }
            };
            const allocator = rt.services(Large, outer_env).allocatorValue() orelse {
                self.deinit();
                return Out.die(.{ .kind = .invalid_configuration, .message = diagnostics.provision_allocator });
            };
            const node = allocator.create(Node) catch {
                self.deinit();
                return Out.die(outcomes.Defect.out_of_memory);
            };
            node.* = .{ .header = .{ .previous = self.ownership, .allocator = allocator, .destroy = Node.destroy, .origin = .{ .data = node, .resolve = Node.origin } }, .source = self.*, .source_env = source_env };
            self.* = undefined;
            return .{ .value = .{ .borrowed = node }, .ownership = &node.header, .resolve = Node.resolve, .start = Node.start };
        }

        fn bridge(comptime self: Self, comptime Large: type, comptime project: fn (*const Large, Effect(A, E, Large).Value) *const R) Effect(A, E, Large) {
            const Out = Effect(A, E, Large);
            const Adapter = struct {
                fn resolve(env: *const Large, value: Out.Value) rt.Services {
                    return self.services(project(env, value));
                }

                fn ready(env: *const Large, value: Out.Value, state: *rt.State) outcomes.Exit(A, E) {
                    return self.ready.?(project(env, value), self.value, state);
                }

                const Frame = struct {
                    child: Job,
                    evaluated: outcomes.Exit(A, E) = undefined,
                    parent: *Out.Job,
                    allocator: std.mem.Allocator,

                    fn complete(_: *rt.State, data: ?*anyopaque) rt.Step {
                        const frame: *@This() = @ptrCast(@alignCast(data.?));
                        const evaluated = frame.evaluated;
                        const parent = frame.parent;
                        frame.allocator.destroy(frame);
                        return parent.finish(evaluated);
                    }
                };

                fn start(state: *rt.State, parent: *Out.Job) rt.Step {
                    const allocator = state.services.allocatorValue() orelse return parent.finish(.{ .die = .{
                        .kind = .invalid_configuration,
                        .message = diagnostics.provision_allocator,
                    } });
                    const frame = allocator.create(Frame) catch return parent.finish(.{ .die = outcomes.Defect.out_of_memory });
                    frame.* = .{ .parent = parent, .allocator = allocator, .child = undefined };
                    frame.child = .{ .program = self, .inherited = state.services, .env = project(parent.env, parent.program.value), .sink = .{
                        .output = &frame.evaluated,
                        .next_step = .{ .data = frame, .advance = Frame.complete },
                    } };
                    return frame.child.step();
                }
            };
            return .{ .resolve = Adapter.resolve, .ready = if (self.ready != null) Adapter.ready else null, .start = if (self.ready == null) Adapter.start else null };
        }
    };
}

pub fn EffectBuilder(comptime E: type, comptime R: type) type {
    return struct {
        const Unit = Effect(void, E, R);
        pub fn succeed(value: anytype) Effect(@TypeOf(value), E, R) {
            return Effect(@TypeOf(value), E, R).succeed(value);
        }

        pub fn fail(comptime A: type, err: E) Effect(A, E, R) {
            return Effect(A, E, R).fail(err);
        }

        pub fn bind(comptime program: anytype, comptime next: anytype) BindingResult(@TypeOf(program), next) {
            if (comptime @hasDecl(@TypeOf(program), "Resource")) {
                const A = @TypeOf(program).Resource;
                const Out = continuationResult(next, ResourceValue(A), R, .EffectBuilder_bind);
                comptime requireEffect(Out, E, R, .EffectBuilder_bind);
                return program.acquire.acquireRelease(struct {
                    fn body(value: A, env: *const R) Out {
                        return callContinuation(next, ResourceValue(A){ .value = value }, env);
                    }
                }.body, struct {
                    fn releaseResource(value: A, primary: outcomes.PrimaryOutcome(E)) Unit {
                        return program.release(value, primary);
                    }
                }.releaseResource);
            }
            comptime requireEffect(@TypeOf(program), E, R, .EffectBuilder_bind);
            return program.flatMap(next);
        }

        fn BindingResult(comptime Binding: type, comptime next: anytype) type {
            if (@hasDecl(Binding, "AlreadyOwnedResource"))
                return unaryResult(next, Binding, .EffectBuilder_bind);
            if (@hasDecl(Binding, "Resource"))
                return continuationResult(next, ResourceValue(Binding.Resource), R, .EffectBuilder_bind);
            return continuationResult(next, Binding.Success, R, .EffectBuilder_bind);
        }

        pub fn zero() Unit {
            return Unit.succeed({});
        }

        pub fn returnFrom(program: anytype) @TypeOf(program) {
            comptime requireEffect(@TypeOf(program), E, R, .EffectBuilder_bind);
            return program;
        }

        pub fn delay(comptime factory: anytype) unaryResult(factory, *const R, .deferEffect) {
            const Out = unaryResult(factory, *const R, .deferEffect);
            comptime requireEffect(Out, E, R, .deferEffect);
            return Out.deferEffect(factory);
        }

        pub fn combine(comptime first: Unit, comptime second: anytype) @TypeOf(second) {
            comptime requireEffect(@TypeOf(second), E, R, .EffectBuilder_bind);
            return first.flatMap(struct {
                fn next(_: void) @TypeOf(second) {
                    return second;
                }
            }.next);
        }

        pub fn whileLoop(comptime guard: fn (*const R) bool, comptime body: Unit) Unit {
            const Loop = struct {
                fn factory(env: *const R) Unit {
                    if (!guard(env)) return zero();
                    return body.bindExit(Unit, complete);
                }

                fn complete(value: outcomes.Exit(void, E), _: *const R, state: *rt.State) Unit {
                    return switch (value) {
                        .success => if (state.cancelled()) Unit.interrupted() else Unit.deferEffect(factory),
                        else => Unit.terminalExit(value),
                    };
                }
            };
            return Unit.deferEffect(Loop.factory);
        }

        pub fn yieldEffect(comptime program: Unit) Unit {
            return program;
        }
        pub fn forLoop(comptime Input: type, values: []const Input, comptime body: fn (Input) Unit) Unit {
            return Unit.forEachDiscard(Input, values, body);
        }

        pub fn tryFinally(comptime body: anytype, comptime finalizer: fn (*const R) void) @TypeOf(body) {
            const Out = @TypeOf(body);
            comptime requireEffect(Out, E, R, .EffectBuilder_bind);
            return (comptime zero()).acquireRelease(struct {
                fn use(_: void) Out {
                    return body;
                }
            }.use, struct {
                fn release(_: void, _: outcomes.PrimaryOutcome(E)) Unit {
                    return Unit.sync(finalizer);
                }
            }.release);
        }

        pub inline fn using(binding: anytype, comptime body: anytype) BindingResult(@TypeOf(binding), body) {
            if (comptime @hasDecl(@TypeOf(binding), "AlreadyOwnedResource")) {
                const Out = BindingResult(@TypeOf(binding), body);
                comptime requireEffect(Out, E, R, .EffectBuilder_bind);
                return body(binding);
            }
            return bind(comptime binding, body);
        }
    };
}

pub fn ResourceValue(comptime A: type) type {
    return struct {
        pub const AlreadyOwnedResource = A;
        value: A,
    };
}

pub fn ResourceBinding(comptime A: type, comptime E: type, comptime R: type) type {
    return struct {
        pub const Resource = A;
        acquire: Effect(A, E, R),
        release: *const fn (A, outcomes.PrimaryOutcome(E)) Effect(void, E, R),
    };
}

pub fn resource(comptime acquisition: anytype, comptime release: anytype) ResourceBinding(@TypeOf(acquisition).Success, @TypeOf(acquisition).Error, @TypeOf(acquisition).Environment) {
    return .{ .acquire = acquisition, .release = release };
}

pub fn Layer(comptime Input: type, comptime E: type, comptime Output: type) type {
    if (outcomes.ownsPayload(Output)) @compileError(diagnostics.layer_services_must_be_borrowed_dependency_records_or_handles_install_owning_resources);
    return struct {
        const Self = @This();
        pub const Environment = Input;
        pub const Error = E;
        pub const Service = Output;
        const Scope = scopes.Scope(E);
        const Build = Effect(Output, E, Input);
        build_fn: *const fn (*Scope) Build,

        pub fn build(comptime self: Self, scope: *Scope) Build {
            return self.build_fn(scope);
        }

        pub fn succeed(comptime value: Output) Self {
            return .{ .build_fn = struct {
                fn build(_: *Scope) Build {
                    return Build.succeed(value);
                }
            }.build };
        }

        pub fn fromEffect(comptime program: Build) Self {
            return .{ .build_fn = struct {
                fn build(_: *Scope) Build {
                    return program;
                }
            }.build };
        }

        pub fn scoped(comptime acquire: Build, comptime release: fn (Output) Effect(void, E, Input)) Self {
            return .{ .build_fn = struct {
                fn build(scope: *Scope) Build {
                    return acquire.scopeAcquire(scope, release);
                }
            }.build };
        }

        pub fn merge(comptime self: Self, comptime right: anytype, comptime combine: anytype) Layer(Input, E, @typeInfo(@TypeOf(combine)).@"fn".return_type.?) {
            const Right = @TypeOf(right);
            if (Right.Environment != Input or Right.Error != E) @compileError(diagnostics.merged_layers_must_share_environment_and_error_types);
            const Merged = @typeInfo(@TypeOf(combine)).@"fn".return_type.?;
            const RightBuild = Effect(Right.Service, E, Input);
            const Out = Effect(Merged, E, Input);
            return .{ .build_fn = struct {
                const Frame = struct {
                    parent: *Out.Job,
                    allocator: std.mem.Allocator,
                    scope: *Scope,
                    left_job: Build.Job = undefined,
                    right_job: RightBuild.Job = undefined,
                    left: outcomes.Exit(Output, E) = undefined,
                    right_value: outcomes.Exit(Right.Service, E) = undefined,

                    fn leftDone(state: *rt.State, data: ?*anyopaque) rt.Step {
                        const frame: *@This() = @ptrCast(@alignCast(data.?));
                        frame.left = state.publish(frame.left);
                        if (frame.left != .success) return frame.finish(outcomes.castFailure(Merged, frame.left));
                        const program = right.build(frame.scope);
                        frame.right_job = .{ .program = program, .owned = program.ownership, .inherited = state.services, .env = frame.parent.env, .sink = .{ .output = &frame.right_value, .next_step = .{ .data = frame, .advance = rightDone } } };
                        return frame.right_job.step();
                    }

                    fn rightDone(state: *rt.State, data: ?*anyopaque) rt.Step {
                        const frame: *@This() = @ptrCast(@alignCast(data.?));
                        frame.right_value = state.publish(frame.right_value);
                        return frame.finish(if (frame.right_value == .success) .{ .success = combine(frame.left.success, frame.right_value.success) } else outcomes.castFailure(Merged, frame.right_value));
                    }

                    fn finish(frame: *@This(), value: outcomes.Exit(Merged, E)) rt.Step {
                        const parent = frame.parent;
                        frame.allocator.destroy(frame);
                        return parent.finish(value);
                    }
                };

                fn start(state: *rt.State, parent: *Out.Job) rt.Step {
                    const allocator = state.services.allocatorValue() orelse return parent.finish(.{ .die = .{ .kind = .invalid_configuration, .message = diagnostics.resource_allocator } });
                    const frame = allocator.create(Frame) catch return parent.finish(.{ .die = outcomes.Defect.out_of_memory });
                    const scope: *Scope = @ptrCast(@alignCast(@constCast(parent.program.value.borrowed)));
                    frame.* = .{ .parent = parent, .allocator = allocator, .scope = scope };
                    const program = self.build(scope);
                    frame.left_job = .{ .program = program, .owned = program.ownership, .inherited = state.services, .env = parent.env, .sink = .{ .output = &frame.left, .next_step = .{ .data = frame, .advance = Frame.leftDone } } };
                    return frame.left_job.step();
                }

                fn build(scope: *Scope) Out {
                    return .{ .value = .{ .borrowed = scope }, .start = start };
                }
            }.build };
        }

        pub fn provide(comptime self: Self, comptime second: anytype) Layer(Input, E, @TypeOf(second).Service) {
            const Second = @TypeOf(second);
            if (Second.Environment != Output or Second.Error != E) @compileError(diagnostics.provided_layer_input_must_match_the_first_layer_s_output_and_error);
            const Out = Effect(Second.Service, E, Input);
            const Next = Effect(Second.Service, E, Output);
            return .{ .build_fn = struct {
                const Frame = struct {
                    parent: *Out.Job,
                    allocator: std.mem.Allocator,
                    scope: *Scope,
                    first_job: Build.Job = undefined,
                    first: outcomes.Exit(Output, E) = undefined,
                    next_job: Next.Job = undefined,
                    next_value: outcomes.Exit(Second.Service, E) = undefined,

                    fn firstDone(state: *rt.State, data: ?*anyopaque) rt.Step {
                        const frame: *@This() = @ptrCast(@alignCast(data.?));
                        frame.first = state.publish(frame.first);
                        if (frame.first != .success) return frame.finish(outcomes.castFailure(Second.Service, frame.first));
                        const program = second.build(frame.scope);
                        frame.next_job = .{ .program = program, .owned = program.ownership, .inherited = state.services, .env = &frame.first.success, .sink = .{ .output = &frame.next_value, .next_step = .{ .data = frame, .advance = nextDone } } };
                        return frame.next_job.step();
                    }

                    fn nextDone(_: *rt.State, data: ?*anyopaque) rt.Step {
                        const frame: *@This() = @ptrCast(@alignCast(data.?));
                        return frame.finish(frame.next_value);
                    }

                    fn finish(frame: *@This(), value: outcomes.Exit(Second.Service, E)) rt.Step {
                        const parent = frame.parent;
                        frame.allocator.destroy(frame);
                        return parent.finish(value);
                    }
                };

                fn start(state: *rt.State, parent: *Out.Job) rt.Step {
                    const allocator = state.services.allocatorValue() orelse return parent.finish(.{ .die = .{ .kind = .invalid_configuration, .message = diagnostics.resource_allocator } });
                    const frame = allocator.create(Frame) catch return parent.finish(.{ .die = outcomes.Defect.out_of_memory });
                    const scope: *Scope = @ptrCast(@alignCast(@constCast(parent.program.value.borrowed)));
                    frame.* = .{ .parent = parent, .allocator = allocator, .scope = scope };
                    const program = self.build(scope);
                    frame.first_job = .{ .program = program, .owned = program.ownership, .inherited = state.services, .env = parent.env, .sink = .{ .output = &frame.first, .next_step = .{ .data = frame, .advance = Frame.firstDone } } };
                    return frame.first_job.step();
                }

                fn build(scope: *Scope) Out {
                    return .{ .value = .{ .borrowed = scope }, .start = start };
                }
            }.build };
        }

        pub fn provideTo(comptime self: Self, comptime program: anytype) Effect(@TypeOf(program).Success, E, Input) {
            const Body = @TypeOf(program);
            comptime requireEffect(Body, E, Output, .provideSome);
            const Out = Effect(Body.Success, E, Input);
            const Adapter = struct {
                const Frame = struct {
                    parent: *Out.Job,
                    allocator: std.mem.Allocator,
                    build_job: Build.Job = undefined,
                    built: outcomes.Exit(Output, E) = undefined,
                    body_job: Body.Job = undefined,
                    body: outcomes.Exit(Body.Success, E) = undefined,

                    fn builtStep(state: *rt.State, data: ?*anyopaque) rt.Step {
                        const frame: *@This() = @ptrCast(@alignCast(data.?));
                        frame.built = state.publish(frame.built);
                        if (frame.built != .success) return frame.finish(outcomes.castFailure(Body.Success, frame.built));
                        frame.body_job = .{ .program = program, .inherited = state.services, .env = &frame.built.success, .sink = .{ .output = &frame.body, .next_step = .{ .data = frame, .advance = bodyStep } } };
                        return frame.body_job.step();
                    }

                    fn bodyStep(_: *rt.State, data: ?*anyopaque) rt.Step {
                        const frame: *@This() = @ptrCast(@alignCast(data.?));
                        return frame.finish(frame.body);
                    }

                    fn finish(frame: *@This(), value: outcomes.Exit(Body.Success, E)) rt.Step {
                        const parent = frame.parent;
                        frame.allocator.destroy(frame);
                        return parent.finish(value);
                    }
                };

                fn start(state: *rt.State, parent: *Out.Job) rt.Step {
                    const allocator = state.services.allocatorValue() orelse return parent.finish(.{ .die = .{ .kind = .invalid_configuration, .message = diagnostics.resource_allocator } });
                    const frame = allocator.create(Frame) catch return parent.finish(.{ .die = outcomes.Defect.out_of_memory });
                    frame.* = .{ .parent = parent, .allocator = allocator };
                    const scope: *Scope = @ptrCast(@alignCast(@constCast(parent.program.value.borrowed)));
                    const building = self.build(scope);
                    frame.build_job = .{ .program = building, .owned = building.ownership, .inherited = state.services, .env = parent.env, .sink = .{ .output = &frame.built, .next_step = .{ .data = frame, .advance = Frame.builtStep } } };
                    return frame.build_job.step();
                }

                fn useScope(scope: *Scope) Out {
                    return .{ .value = .{ .borrowed = scope }, .start = start };
                }
            };
            return Out.scoped(Adapter.useScope);
        }
    };
}

pub fn Chunk(comptime A: type) type {
    return union(enum) {
        const Self = @This();
        pub const effect_owned_payload = true;
        end,
        items: struct { values: []const A, allocator: ?std.mem.Allocator },

        pub fn borrowed(values: []const A) Self {
            return .{ .items = .{ .values = values, .allocator = null } };
        }
        pub fn owned(values: outcomes.OwnedSlice(A)) Self {
            return .{ .items = .{ .values = values.items, .allocator = values.allocator } };
        }
        pub fn deinit(self: *Self) void {
            self.deinitFrom(0);
        }
        fn deinitFrom(self: *Self, consumed: usize) void {
            if (self.* == .items) if (self.items.allocator) |allocator| {
                for (@constCast(self.items.values[consumed..])) |*value| outcomes.discardPayload(value);
                allocator.free(self.items.values);
            };
            self.* = .end;
        }
        pub fn clone(self: *const Self) error{OutOfMemory}!Self {
            if (self.* == .end or self.items.allocator == null) return self.*;
            const values: outcomes.OwnedSlice(A) = .{ .items = @constCast(self.items.values), .allocator = self.items.allocator.? };
            return owned(try values.clone());
        }
    };
}

pub fn StreamReader(comptime A: type, comptime E: type, comptime R: type) type {
    return struct {
        pub const effect_owned_payload = true;
        const Description = struct {
            allocator: std.mem.Allocator,
            program: Effect(Chunk(A), E, R),
            references: std.atomic.Value(usize) = .init(1),
        };
        pull: Effect(Chunk(A), E, R),
        description: *Description,
        pub fn deinit(self: *@This()) void {
            if (self.description.references.fetchSub(1, .acq_rel) == 1) {
                self.description.program.deinit();
                self.description.allocator.destroy(self.description);
            }
            self.* = undefined;
        }
        pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
            _ = self.description.references.fetchAdd(1, .monotonic);
            return self.*;
        }
    };
}

pub fn Stream(comptime A: type, comptime E: type, comptime R: type) type {
    return struct {
        const Self = @This();
        const Scope = scopes.Scope(E);
        const Reader = StreamReader(A, E, R);
        const Open = Effect(Reader, E, R);
        const Pull = Effect(Chunk(A), E, R);
        const Unit = Effect(void, E, R);
        open_fn: ?*const fn (*Scope) Open = null,
        captured: ?*const anyopaque = null,
        captured_open_fn: ?*const fn (*const anyopaque, *Scope) Open = null,

        pub fn open(self: Self, scope: *Scope) Open {
            if (self.captured_open_fn) |open_captured| return open_captured(self.captured.?, scope);
            return self.open_fn.?(scope);
        }

        /// Borrow request-specific construction state without a heap node.
        /// The capture must outlive the opened reader and its scope.
        pub fn fromCaptured(
            comptime Capture: type,
            capture: *const Capture,
            comptime build: fn (*const Capture, *Scope) Open,
        ) Self {
            return .{ .captured = capture, .captured_open_fn = struct {
                fn invoke(raw: *const anyopaque, scope: *Scope) Open {
                    return build(@ptrCast(@alignCast(raw)), scope);
                }
            }.invoke };
        }

        /// Reinterprets expected stream failures while preserving the source
        /// scope, pull state, cancellation and finalizer outcomes.
        pub fn mapError(comptime self: Self, comptime transform: anytype) Stream(A, unaryResult(transform, E, .mapError), R) {
            const F = unaryResult(transform, E, .mapError);
            const Mapped = Stream(A, F, R);
            const Bridge = struct {
                allocator: std.mem.Allocator,
                scope: Scope,
                reader: ?Reader = null,
            };
            const Acquire = Effect(*Bridge, F, R);
            const MappedUnit = Effect(void, F, R);
            const Adapter = struct {
                fn allocate(env: *const R) outcomes.Exit(*Bridge, F) {
                    const bridge = env.allocator.create(Bridge) catch return .{ .die = outcomes.Defect.out_of_memory };
                    bridge.* = .{ .allocator = env.allocator, .scope = Scope.init(env) };
                    return .{ .success = bridge };
                }

                fn opened(bridge: *const *Bridge, reader: Reader, env: *const R) Acquire {
                    bridge.*.reader = reader;
                    return Acquire.fromOwnedSuccess(env, bridge.*);
                }

                fn use(bridge: *Bridge, env: *const R) Acquire {
                    var opening = self.open(&bridge.scope);
                    var mapped = opening.mapErrorOwned(env, transform);
                    return mapped.flatMapCapturedOwned(env, *Bridge, bridge, opened);
                }

                fn afterClose(bridge: *const *Bridge, result: outcomes.Exit(void, F), env: *const R) MappedUnit {
                    if (bridge.*.reader) |*reader| reader.deinit();
                    bridge.*.scope.deinit();
                    bridge.*.allocator.destroy(bridge.*);
                    return MappedUnit.fromOwnedOutcome(env, result);
                }

                fn close(bridge: *Bridge) MappedUnit {
                    return MappedUnit.deferBorrowed(Bridge, bridge, struct {
                        fn run(current: *const Bridge, env: *const R) MappedUnit {
                            const mutable = @constCast(current);
                            var closing = Effect(void, E, R).closeScope(&mutable.scope);
                            var mapped = closing.mapErrorOwned(env, transform);
                            var settled = mapped.materializeOwned(env);
                            var aligned = settled.mapErrorOwned(env, struct {
                                fn impossible(_: void) F {
                                    unreachable;
                                }
                            }.impossible);
                            return aligned.flatMapCapturedOwned(env, void, mutable, afterClose);
                        }
                    }.run);
                }

                fn cleanupFailed(bridge: *Bridge, primary: outcomes.PrimaryOutcome(F)) MappedUnit {
                    return if (primary == .succeeded) MappedUnit.succeed({}) else close(bridge);
                }

                fn pull(bridge: *Bridge) Effect(Chunk(A), F, R) {
                    const Out = Effect(Chunk(A), F, R);
                    return Out.deferBorrowed(Bridge, bridge, struct {
                        fn run(current: *const Bridge, env: *const R) Out {
                            var source = current.reader.?.pull.borrow();
                            return source.mapErrorOwned(env, transform);
                        }
                    }.run);
                }
            };
            const acquired = Acquire.fromOutcome(Adapter.allocate).acquireRelease(Adapter.use, Adapter.cleanupFailed);
            return Mapped.fromPull(acquired, Adapter.pull, Adapter.close);
        }

        pub fn fromPull(comptime acquisition: anytype, comptime pull: fn (@TypeOf(acquisition).Success) Pull, comptime release: fn (@TypeOf(acquisition).Success) Unit) Self {
            const Acquire = @TypeOf(acquisition);
            comptime requireEffect(Acquire, E, R, .acquireRelease_body);
            return .{ .open_fn = struct {
                const Frame = struct {
                    parent: *Open.Job,
                    allocator: std.mem.Allocator,
                    description: *Reader.Description,
                    acquired: outcomes.Exit(Acquire.Success, E) = undefined,
                    child: Acquire.Job = undefined,

                    fn complete(state: *rt.State, data: ?*anyopaque) rt.Step {
                        const frame: *@This() = @ptrCast(@alignCast(data.?));
                        frame.acquired = state.publish(frame.acquired);
                        var result_value: outcomes.Exit(Reader, E) = undefined;
                        if (frame.acquired == .success) {
                            const program = pull(frame.acquired.success);
                            frame.description.* = .{ .allocator = frame.allocator, .program = program };
                            result_value = .{ .success = .{ .pull = program.borrow(), .description = frame.description } };
                        } else {
                            frame.allocator.destroy(frame.description);
                            result_value = outcomes.castFailure(Reader, frame.acquired);
                        }
                        const parent = frame.parent;
                        frame.allocator.destroy(frame);
                        return parent.finish(result_value);
                    }
                };

                fn start(state: *rt.State, parent: *Open.Job) rt.Step {
                    const allocator = state.services.allocatorValue() orelse return parent.finish(.{ .die = .{ .kind = .invalid_configuration, .message = diagnostics.resource_allocator } });
                    const frame = allocator.create(Frame) catch return parent.finish(.{ .die = outcomes.Defect.out_of_memory });
                    const description = allocator.create(Reader.Description) catch {
                        allocator.destroy(frame);
                        return parent.finish(.{ .die = outcomes.Defect.out_of_memory });
                    };
                    const scope: *Scope = @ptrCast(@alignCast(@constCast(parent.program.value.borrowed)));
                    frame.* = .{ .parent = parent, .allocator = allocator, .description = description };
                    const program = acquisition.scopeAcquire(scope, release);
                    frame.child = .{ .program = program, .owned = program.ownership, .inherited = state.services, .env = parent.env, .sink = .{ .output = &frame.acquired, .next_step = .{ .data = frame, .advance = Frame.complete } } };
                    return frame.child.step();
                }

                fn open(scope: *Scope) Open {
                    return .{ .value = .{ .borrowed = scope }, .start = start };
                }
            }.open };
        }

        pub fn fromIterable(comptime Iterator: type, comptime factory: fn (*const R) outcomes.Exit(*Iterator, E), comptime next: fn (*Iterator, *const R) outcomes.Exit(?A, E), comptime release: fn (*Iterator) Unit) Self {
            const Acquire = Effect(*Iterator, E, R);
            return fromPull(comptime Acquire.fromOutcome(factory), struct {
                fn pull(iterator: *Iterator) Pull {
                    return Pull.fromBorrowed(Iterator, iterator, struct {
                        fn readNext(borrowed: *const Iterator, env: *const R) outcomes.Exit(Chunk(A), E) {
                            const result_value = next(@constCast(borrowed), env);
                            if (result_value != .success) return outcomes.castFailure(Chunk(A), result_value);
                            if (result_value.success) |value| {
                                const Single = std.meta.Tuple(&.{A});
                                const values = env.allocator.alloc(A, @typeInfo(Single).@"struct".fields.len) catch {
                                    var discarded = value;
                                    outcomes.discardPayload(&discarded);
                                    return .{ .die = outcomes.Defect.out_of_memory };
                                };
                                values[0] = value;
                                return .{ .success = Chunk(A).owned(.{ .items = values, .allocator = env.allocator }) };
                            }
                            return .{ .success = .end };
                        }
                    }.readNext);
                }
            }.pull, release);
        }

        pub fn fromAsyncIterable(comptime Iterator: type, comptime factory: fn (*const R) outcomes.Exit(*Iterator, E), comptime next: fn (*const Iterator, *const R) asynchronous.Async(?A, E).Start, comptime release: fn (*Iterator) Unit) Self {
            const Next = Effect(?A, E, R);
            return fromPull(comptime Effect(*Iterator, E, R).fromOutcome(factory), struct {
                const Frame = struct {
                    parent: *Pull.Job,
                    allocator: std.mem.Allocator,
                    child: Next.Job = undefined,
                    result_value: outcomes.Exit(?A, E) = undefined,
                    fn complete(state: *rt.State, data: ?*anyopaque) rt.Step {
                        const frame: *@This() = @ptrCast(@alignCast(data.?));
                        const value = state.publish(frame.result_value);
                        var result_value: outcomes.Exit(Chunk(A), E) = undefined;
                        if (value != .success) result_value = outcomes.castFailure(Chunk(A), value) else if (value.success) |item| {
                            const Single = std.meta.Tuple(&.{A});
                            const values = frame.allocator.alloc(A, @typeInfo(Single).@"struct".fields.len) catch {
                                var discarded = item;
                                outcomes.discardPayload(&discarded);
                                const parent = frame.parent;
                                frame.allocator.destroy(frame);
                                return parent.finish(.{ .die = outcomes.Defect.out_of_memory });
                            };
                            values[0] = item;
                            result_value = .{ .success = Chunk(A).owned(.{ .items = values, .allocator = frame.allocator }) };
                        } else result_value = .{ .success = .end };
                        const parent = frame.parent;
                        frame.allocator.destroy(frame);
                        return parent.finish(result_value);
                    }
                };
                fn start(state: *rt.State, parent: *Pull.Job) rt.Step {
                    const allocator = state.services.allocatorValue() orelse return parent.finish(.{ .die = outcomes.Defect.out_of_memory });
                    const frame = allocator.create(Frame) catch return parent.finish(.{ .die = outcomes.Defect.out_of_memory });
                    const iterator: *const Iterator = @ptrCast(@alignCast(@constCast(parent.program.value.borrowed)));
                    const program = Next.fromAsyncBorrowed(Iterator, iterator, next);
                    frame.* = .{ .parent = parent, .allocator = allocator };
                    frame.child = .{ .program = program, .env = parent.env, .inherited = state.services, .sink = .{ .output = &frame.result_value, .next_step = .{ .data = frame, .advance = Frame.complete } } };
                    return frame.child.step();
                }
                fn pull(iterator: *Iterator) Pull {
                    return .{ .value = .{ .borrowed = iterator }, .start = start };
                }
            }.pull, release);
        }

        pub fn mapEffect(comptime self: Self, comptime transform: anytype) Stream(unaryResult(transform, A, .flatMap).Success, E, R) {
            const Transform = unaryResult(transform, A, .flatMap);
            comptime requireEffect(Transform, E, R, .flatMap);
            const B = Transform.Success;
            const MappedReader = StreamReader(B, E, R);
            const MappedOpen = Effect(MappedReader, E, R);
            const MappedPull = Effect(Chunk(B), E, R);
            const Adapter = struct {
                const Capture = struct {
                    header: rt.DescriptionNode,
                    reader: Reader = undefined,
                    fn destroy(header: *rt.DescriptionNode) void {
                        const capture: *@This() = @alignCast(@fieldParentPtr("header", header));
                        capture.reader.deinit();
                        header.allocator.destroy(capture);
                    }
                    fn origin(env: *const anyopaque, _: *const anyopaque) rt.Services {
                        return rt.services(R, @ptrCast(@alignCast(env)));
                    }
                };
                const PullFrame = struct {
                    parent: *MappedPull.Job,
                    allocator: std.mem.Allocator,
                    pulling: Pull.Job = undefined,
                    chunk: outcomes.Exit(Chunk(A), E) = undefined,
                    child: Transform.Job = undefined,
                    transformed: outcomes.Exit(B, E) = undefined,
                    values: ?[]B = null,
                    count: usize = 0,
                    fn finish(frame: *@This(), value: outcomes.Exit(Chunk(B), E)) rt.Step {
                        if (frame.chunk == .success) frame.chunk.success.deinit();
                        if (frame.values) |values| {
                            for (values[0..frame.count]) |*item| outcomes.discardPayload(item);
                            frame.allocator.free(values);
                        }
                        const parent = frame.parent;
                        frame.allocator.destroy(frame);
                        return parent.finish(value);
                    }
                    fn pulled(state: *rt.State, data: ?*anyopaque) rt.Step {
                        const frame: *@This() = @ptrCast(@alignCast(data.?));
                        frame.chunk = state.publish(frame.chunk);
                        if (frame.chunk != .success) return frame.finish(outcomes.castFailure(Chunk(B), frame.chunk));
                        if (frame.chunk.success == .end) return frame.finish(.{ .success = .end });
                        frame.values = frame.allocator.alloc(B, frame.chunk.success.items.values.len) catch return frame.finish(.{ .die = outcomes.Defect.out_of_memory });
                        return .{ .data = frame, .advance = next };
                    }
                    fn next(state: *rt.State, data: ?*anyopaque) rt.Step {
                        const frame: *@This() = @ptrCast(@alignCast(data.?));
                        const values = frame.values.?;
                        if (frame.count == values.len) {
                            frame.values = null;
                            return frame.finish(.{ .success = Chunk(B).owned(.{ .items = values, .allocator = frame.allocator }) });
                        }
                        const value = outcomes.clonePayload(frame.chunk.success.items.values[frame.count]) catch return frame.finish(.{ .die = outcomes.Defect.out_of_memory });
                        const program = transform(value);
                        frame.child = .{ .program = program, .owned = program.ownership, .env = frame.parent.env, .inherited = state.services, .sink = .{ .output = &frame.transformed, .next_step = .{ .data = frame, .advance = transformedStep } } };
                        return frame.child.step();
                    }
                    fn transformedStep(state: *rt.State, data: ?*anyopaque) rt.Step {
                        const frame: *@This() = @ptrCast(@alignCast(data.?));
                        frame.transformed = state.publish(frame.transformed);
                        if (frame.transformed != .success) return frame.finish(outcomes.castFailure(Chunk(B), frame.transformed));
                        frame.values.?[frame.count] = frame.transformed.success;
                        frame.count += 1;
                        return .{ .data = frame, .advance = next };
                    }
                };
                fn pullStart(state: *rt.State, parent: *MappedPull.Job) rt.Step {
                    const allocator = state.services.allocatorValue() orelse return parent.finish(.{ .die = outcomes.Defect.out_of_memory });
                    const frame = allocator.create(PullFrame) catch return parent.finish(.{ .die = outcomes.Defect.out_of_memory });
                    const capture: *const Capture = @ptrCast(@alignCast(@constCast(parent.program.value.borrowed)));
                    frame.* = .{ .parent = parent, .allocator = allocator };
                    frame.pulling = .{ .program = capture.reader.pull.borrow(), .env = parent.env, .inherited = state.services, .sink = .{ .output = &frame.chunk, .next_step = .{ .data = frame, .advance = PullFrame.pulled } } };
                    return frame.pulling.step();
                }
                const OpenFrame = struct {
                    parent: *MappedOpen.Job,
                    allocator: std.mem.Allocator,
                    capture: *Capture,
                    description: *MappedReader.Description,
                    child: Open.Job = undefined,
                    opened: outcomes.Exit(Reader, E) = undefined,
                    fn complete(state: *rt.State, data: ?*anyopaque) rt.Step {
                        const frame: *@This() = @ptrCast(@alignCast(data.?));
                        frame.opened = state.publish(frame.opened);
                        var value: outcomes.Exit(MappedReader, E) = undefined;
                        if (frame.opened == .success) {
                            frame.capture.* = .{ .reader = frame.opened.success, .header = .{ .previous = null, .allocator = frame.allocator, .destroy = Capture.destroy, .origin = .{ .data = frame.capture, .resolve = Capture.origin } } };
                            const pull: MappedPull = .{ .value = .{ .borrowed = frame.capture }, .start = pullStart, .ownership = &frame.capture.header };
                            frame.description.* = .{ .allocator = frame.allocator, .program = pull };
                            value = .{ .success = .{ .pull = pull.borrow(), .description = frame.description } };
                        } else {
                            value = outcomes.castFailure(MappedReader, frame.opened);
                            frame.allocator.destroy(frame.capture);
                            frame.allocator.destroy(frame.description);
                        }
                        const parent = frame.parent;
                        frame.allocator.destroy(frame);
                        return parent.finish(value);
                    }
                };
                fn openStart(state: *rt.State, parent: *MappedOpen.Job) rt.Step {
                    const allocator = state.services.allocatorValue() orelse return parent.finish(.{ .die = outcomes.Defect.out_of_memory });
                    const frame = allocator.create(OpenFrame) catch return parent.finish(.{ .die = outcomes.Defect.out_of_memory });
                    const capture = allocator.create(Capture) catch {
                        allocator.destroy(frame);
                        return parent.finish(.{ .die = outcomes.Defect.out_of_memory });
                    };
                    const description = allocator.create(MappedReader.Description) catch {
                        allocator.destroy(capture);
                        allocator.destroy(frame);
                        return parent.finish(.{ .die = outcomes.Defect.out_of_memory });
                    };
                    frame.* = .{ .parent = parent, .allocator = allocator, .capture = capture, .description = description };
                    const scope: *Scope = @ptrCast(@alignCast(@constCast(parent.program.value.borrowed)));
                    const program = self.open(scope);
                    frame.child = .{ .program = program, .owned = program.ownership, .env = parent.env, .inherited = state.services, .sink = .{ .output = &frame.opened, .next_step = .{ .data = frame, .advance = OpenFrame.complete } } };
                    return frame.child.step();
                }
                fn open(scope: *Scope) MappedOpen {
                    return .{ .value = .{ .borrowed = scope }, .start = openStart };
                }
            };
            return .{ .open_fn = Adapter.open };
        }

        pub fn runCollect(comptime self: Self, comptime limit: ?usize) Effect(outcomes.OwnedSlice(A), E, R) {
            const Out = Effect(outcomes.OwnedSlice(A), E, R);
            const Adapter = struct {
                const Frame = struct {
                    parent: *Out.Job,
                    allocator: std.mem.Allocator,
                    opened: outcomes.Exit(Reader, E) = undefined,
                    opening: Open.Job = undefined,
                    pulling: Pull.Job = undefined,
                    chunk: outcomes.Exit(Chunk(A), E) = undefined,
                    buffer: std.ArrayList(A) = .empty,

                    fn openedStep(state: *rt.State, data: ?*anyopaque) rt.Step {
                        const frame: *@This() = @ptrCast(@alignCast(data.?));
                        frame.opened = state.publish(frame.opened);
                        if (frame.opened != .success) return frame.finish(outcomes.castFailure(outcomes.OwnedSlice(A), frame.opened));
                        return .{ .data = frame, .advance = next };
                    }

                    fn next(state: *rt.State, data: ?*anyopaque) rt.Step {
                        const frame: *@This() = @ptrCast(@alignCast(data.?));
                        frame.pulling = .{ .program = frame.opened.success.pull.borrow(), .inherited = state.services, .env = frame.parent.env, .sink = .{ .output = &frame.chunk, .next_step = .{ .data = frame, .advance = chunkStep } } };
                        return frame.pulling.step();
                    }

                    fn chunkStep(state: *rt.State, data: ?*anyopaque) rt.Step {
                        const frame: *@This() = @ptrCast(@alignCast(data.?));
                        frame.chunk = state.publish(frame.chunk);
                        if (frame.chunk != .success) return frame.finish(outcomes.castFailure(outcomes.OwnedSlice(A), frame.chunk));
                        if (frame.chunk.success == .end) {
                            const values = frame.buffer.toOwnedSlice(frame.allocator) catch return frame.finish(.{ .die = outcomes.Defect.out_of_memory });
                            return frame.finish(.{ .success = .{ .items = values, .allocator = frame.allocator } });
                        }
                        const incoming = frame.chunk.success.items;
                        const total = std.math.add(usize, frame.buffer.items.len, incoming.values.len) catch {
                            frame.chunk.success.deinit();
                            return frame.finish(.{ .die = .{ .kind = .invalid_configuration, .message = diagnostics.stream_collection_length_overflow } });
                        };
                        if (limit) |maximum| if (total > maximum) {
                            frame.chunk.success.deinit();
                            return frame.finish(.{ .die = .{ .kind = .invalid_configuration, .message = diagnostics.stream_exceeded_the_caller_s_collection_limit } });
                        };
                        frame.buffer.ensureTotalCapacity(frame.allocator, total) catch {
                            frame.chunk.success.deinit();
                            return frame.finish(.{ .die = outcomes.Defect.out_of_memory });
                        };
                        for (incoming.values, 0..) |value, index| {
                            const copied = if (incoming.allocator != null) value else outcomes.clonePayload(value) catch {
                                frame.chunk.success.deinitFrom(index);
                                return frame.finish(.{ .die = outcomes.Defect.out_of_memory });
                            };
                            frame.buffer.appendAssumeCapacity(copied);
                        }
                        frame.chunk.success.deinitFrom(incoming.values.len);
                        return .{ .data = frame, .advance = next };
                    }

                    fn finish(frame: *@This(), value: outcomes.Exit(outcomes.OwnedSlice(A), E)) rt.Step {
                        if (frame.opened == .success) frame.opened.success.deinit();
                        for (frame.buffer.items) |*item| outcomes.discardPayload(item);
                        frame.buffer.deinit(frame.allocator);
                        const parent = frame.parent;
                        frame.allocator.destroy(frame);
                        return parent.finish(value);
                    }
                };

                fn start(state: *rt.State, parent: *Out.Job) rt.Step {
                    const allocator = state.services.allocatorValue() orelse return parent.finish(.{ .die = .{ .kind = .invalid_configuration, .message = diagnostics.resource_allocator } });
                    const frame = allocator.create(Frame) catch return parent.finish(.{ .die = outcomes.Defect.out_of_memory });
                    frame.* = .{ .parent = parent, .allocator = allocator };
                    const scope: *Scope = @ptrCast(@alignCast(@constCast(parent.program.value.borrowed)));
                    const program = self.open(scope);
                    frame.opening = .{ .program = program, .owned = program.ownership, .inherited = state.services, .env = parent.env, .sink = .{ .output = &frame.opened, .next_step = .{ .data = frame, .advance = Frame.openedStep } } };
                    return frame.opening.step();
                }

                fn useScope(scope: *Scope) Out {
                    return .{ .value = .{ .borrowed = scope }, .start = start };
                }
            };
            return Out.scoped(Adapter.useScope);
        }

        pub fn runForEach(comptime self: Self, comptime consume: anytype) Unit {
            const Parameters = @typeInfo(@TypeOf(consume)).@"fn".params;
            comptime {
                if (Parameters.len != 1 and Parameters.len != 2)
                    @compileError("stream consumer accepts an item and optionally R");
                if (Parameters[0].type.? != A or
                    (Parameters.len == 2 and Parameters[1].type.? != *const R))
                    @compileError("stream consumer must accept the stream item and caller R");
            }
            const Adapter = struct {
                const Frame = struct {
                    parent: *Unit.Job,
                    allocator: std.mem.Allocator,
                    opening: Open.Job = undefined,
                    opened: outcomes.Exit(Reader, E) = undefined,
                    pulling: Pull.Job = undefined,
                    chunk: outcomes.Exit(Chunk(A), E) = undefined,
                    consuming: Unit.Job = undefined,
                    consumed: outcomes.Exit(void, E) = undefined,
                    index: usize = 0,

                    fn openedStep(state: *rt.State, data: ?*anyopaque) rt.Step {
                        const frame: *@This() = @ptrCast(@alignCast(data.?));
                        frame.opened = state.publish(frame.opened);
                        if (frame.opened != .success) return frame.finish(outcomes.castFailure(void, frame.opened));
                        return .{ .data = frame, .advance = next };
                    }
                    fn next(state: *rt.State, data: ?*anyopaque) rt.Step {
                        const frame: *@This() = @ptrCast(@alignCast(data.?));
                        frame.pulling = .{ .program = frame.opened.success.pull.borrow(), .inherited = state.services, .env = frame.parent.env, .sink = .{ .output = &frame.chunk, .next_step = .{ .data = frame, .advance = chunkStep } } };
                        return frame.pulling.step();
                    }
                    fn chunkStep(state: *rt.State, data: ?*anyopaque) rt.Step {
                        const frame: *@This() = @ptrCast(@alignCast(data.?));
                        frame.chunk = state.publish(frame.chunk);
                        if (frame.chunk != .success) return frame.finish(outcomes.castFailure(void, frame.chunk));
                        if (frame.chunk.success == .end) return frame.finish(.{ .success = {} });
                        frame.index = 0;
                        return .{ .data = frame, .advance = consumeStep };
                    }
                    fn consumeStep(state: *rt.State, data: ?*anyopaque) rt.Step {
                        const frame: *@This() = @ptrCast(@alignCast(data.?));
                        const values = frame.chunk.success.items.values;
                        if (frame.index == values.len) {
                            frame.chunk.success.deinit();
                            return .{ .data = frame, .advance = next };
                        }
                        const value = outcomes.clonePayload(values[frame.index]) catch {
                            frame.chunk.success.deinit();
                            return frame.finish(.{ .die = outcomes.Defect.out_of_memory });
                        };
                        const program = if (comptime Parameters.len == 2)
                            consume(value, frame.parent.env)
                        else
                            consume(value);
                        frame.consuming = .{ .program = program, .owned = program.ownership, .inherited = state.services, .env = frame.parent.env, .sink = .{ .output = &frame.consumed, .next_step = .{ .data = frame, .advance = consumedStep } } };
                        return frame.consuming.step();
                    }
                    fn consumedStep(state: *rt.State, data: ?*anyopaque) rt.Step {
                        const frame: *@This() = @ptrCast(@alignCast(data.?));
                        frame.consumed = state.publish(frame.consumed);
                        if (frame.consumed != .success) {
                            frame.chunk.success.deinit();
                            return frame.finish(frame.consumed);
                        }
                        frame.index += 1;
                        return .{ .data = frame, .advance = consumeStep };
                    }
                    fn finish(frame: *@This(), value: outcomes.Exit(void, E)) rt.Step {
                        if (frame.opened == .success) frame.opened.success.deinit();
                        const parent = frame.parent;
                        frame.allocator.destroy(frame);
                        return parent.finish(value);
                    }
                };
                fn start(state: *rt.State, parent: *Unit.Job) rt.Step {
                    const allocator = state.services.allocatorValue() orelse return parent.finish(.{ .die = .{ .kind = .invalid_configuration, .message = diagnostics.resource_allocator } });
                    const frame = allocator.create(Frame) catch return parent.finish(.{ .die = outcomes.Defect.out_of_memory });
                    frame.* = .{ .parent = parent, .allocator = allocator };
                    const scope: *Scope = @ptrCast(@alignCast(@constCast(parent.program.value.borrowed)));
                    const program = self.open(scope);
                    frame.opening = .{ .program = program, .owned = program.ownership, .inherited = state.services, .env = parent.env, .sink = .{ .output = &frame.opened, .next_step = .{ .data = frame, .advance = Frame.openedStep } } };
                    return frame.opening.step();
                }
                fn useScope(scope: *Scope) Unit {
                    return .{ .value = .{ .borrowed = scope }, .start = start };
                }
            };
            return Unit.scoped(Adapter.useScope);
        }
    };
}

/// Execute a description and return its full typed Exit.
/// Runtime services come from the caller's environment.
pub inline fn run(env: anytype, program: anytype) outcomes.Exit(@TypeOf(program).Success, @TypeOf(program).Error) {
    if (program.entry) |entry| return entry(env, program.value);
    return runDriver(false, env, program);
}

pub fn AsyncRun(comptime A: type, comptime E: type, comptime R: type) type {
    return union(enum) {
        const Self = @This();
        const Program = Effect(A, E, R);
        immediate: outcomes.Exit(A, E),
        pending: *Root,

        const Root = struct {
            allocator: std.mem.Allocator,
            io: std.Io,
            env: *const R,
            state: rt.State,
            owned_env: ?R = null,
            owned_annotations: ?[]providers.Annotation = null,
            completion: primitives.Deferred(A, E),
            job: Program.Job = undefined,
            next: rt.Step = .done,
            result: outcomes.Exit(A, E) = undefined,
            cancellation: rt.Cancellation = .{},
            origin: rt.Cancellation.Registration = .{ .data = null, .forward = cancel },
            handle: execution.Registered = undefined,
            has_handle: bool = false,
            mutex: std.Io.Mutex = .init,
            joined: bool = false,
            forwarding: ?outcomes.Defect = null,
            joining: ?outcomes.Defect = null,
            faults_augmented: bool = false,
            storage_reference: bool = false,
            references: std.atomic.Value(usize) = .init(1),
            storage: outcomes.CleanupStorage(E),

            fn cancel(data: ?*anyopaque) void {
                const root: *@This() = @ptrCast(@alignCast(data.?));
                root.cancellation.request();
            }

            fn work(data: *anyopaque) void {
                const root: *@This() = @ptrCast(@alignCast(data));
                drive(root.next, &root.state);
                root.result = root.state.publish(root.result);
                _ = root.completion.completeCopy(root.result);
            }

            fn release(data: *anyopaque) void {
                const root: *@This() = @ptrCast(@alignCast(data));
                if (root.references.fetchSub(1, .acq_rel) == 1) {
                    if (root.owned_annotations) |annotations| providers.freeAnnotations(root.allocator, annotations);
                    root.allocator.destroy(root);
                }
            }

            fn augment(root: *@This(), target: anytype) void {
                if (root.faults_augmented) return;
                if (root.forwarding) |defect| root.storage.append(target, .{ .release_defect = defect });
                if (root.joining) |defect| root.storage.append(target, .{ .release_defect = defect });
                if (root.storage.retained() and !root.storage_reference) {
                    _ = root.references.fetchAdd(1, .monotonic);
                    root.storage_reference = true;
                }
                root.faults_augmented = true;
            }

            fn join(root: *@This(), comptime augment_faults: bool, protected: bool) void {
                if (!root.joined) {
                    if (root.has_handle) {
                        const completed = root.handle.joinProtected(protected);
                        root.forwarding = completed.forwarding;
                        root.joining = completed.joining;
                        root.has_handle = false;
                    }
                    root.origin.unregister();
                    root.joined = true;
                }
                if (augment_faults) root.augment(&root.result);
            }
        };

        /// The handle is owned and must be moved/deinitialized once. Borrowed
        /// environment and description captures must outlive its joined work.
        pub fn join(self: *Self) outcomes.Exit(A, E) {
            switch (self.*) {
                .immediate => |value| return value.clone(),
                .pending => |root| {
                    root.mutex.lockUncancelable(root.io);
                    defer root.mutex.unlock(root.io);
                    root.join(true, false);
                    return root.result.clone();
                },
            }
        }

        pub fn request(self: *const Self) void {
            if (self.* == .pending) self.pending.cancellation.request();
        }

        /// Cancellation requests ownership-preserving shutdown, then joins it.
        /// Copies borrowed by observers must no longer be in use at destruction.
        pub fn deinit(self: *Self) void {
            switch (self.*) {
                .immediate => |*value| value.deinit(),
                .pending => |root| {
                    root.cancellation.request();
                    root.mutex.lockUncancelable(root.io);
                    root.join(true, true);
                    root.mutex.unlock(root.io);
                    root.result.deinit();
                    root.completion.deinit();
                    Root.release(root);
                },
            }
            self.* = undefined;
        }
    };
}

pub fn runAsync(env: anytype, program: anytype) AsyncRun(@TypeOf(program).Success, @TypeOf(program).Error, @TypeOf(program).Environment) {
    const Program = @TypeOf(program);
    const Handle = AsyncRun(Program.Success, Program.Error, Program.Environment);
    const Root = Handle.Root;
    const inherited = program.services(env);
    const allocator = inherited.allocatorValue() orelse return .{ .immediate = .{ .die = .{ .kind = .invalid_configuration, .message = diagnostics.suspended_binding_allocator } } };
    const io = inherited.ioValue() orelse return .{ .immediate = .{ .die = .{ .kind = .provider_unavailable, .message = diagnostics.asynchronous_execution_requires_environment_i_o_for_coordination } } };
    const root = allocator.create(Root) catch return .{ .immediate = .{ .die = outcomes.Defect.out_of_memory } };
    root.* = .{ .allocator = allocator, .io = io, .env = env, .state = .{ .services = inherited, .origin_cancellation = inherited.cancellation }, .completion = .{ .io = io }, .storage = .{ .owner = root, .release = Root.release, .allocator = allocator } };
    root.state.branch_cancellation = &root.cancellation;
    root.state.origin_cancellation = &root.cancellation;
    root.state.ignore_environment_cancellation = true;
    root.origin.data = root;
    if (inherited.cancellation) |token| token.register(&root.origin);
    root.job = .{ .program = program.borrow(), .inherited = inherited, .env = env, .sink = .{ .output = &root.result, .next_step = .done } };
    root.next = root.job.step();
    // Execute the synchronous prefix in the calling task. Only the suspended
    // continuation crosses the consumer-supplied asynchronous engine boundary.
    while (root.next.advance) |advance| root.next = advance(&root.state, root.next.data);
    if (root.next.suspended == null) {
        root.origin.unregister();
        const value = root.state.publish(root.result);
        root.completion.deinit();
        allocator.destroy(root);
        return .{ .immediate = value };
    }
    const submission = Program.executor(env, .async_io).submit(.async_io, .{ .data = root, .run = Root.work, .cancellation = &root.cancellation });
    if (submission == .rejected) {
        // Prefix work is already owned. Rejection cancels and drains it before
        // publishing the admission defect; cleanup metadata is retained.
        root.cancellation.request();
        drive(root.next, &root.state);
        var value: outcomes.Exit(Program.Success, Program.Error) = .{ .die = submission.rejected };
        if (root.result == .success) root.result.deinit() else root.storage.appendFailures(&value, root.result);
        root.result = value;
        _ = root.completion.complete(value.clone());
        root.origin.unregister();
        root.joined = true;
        root.faults_augmented = true;
        if (root.storage.retained()) {
            _ = root.references.fetchAdd(1, .monotonic);
            root.storage_reference = true;
        }
        return .{ .pending = root };
    }
    root.handle.init(submission.accepted, &root.cancellation);
    root.has_handle = true;
    return .{ .pending = root };
}

fn evaluateBorrowed(env: anytype, program: anytype, state: *rt.State) outcomes.Exit(@TypeOf(program).Success, @TypeOf(program).Error) {
    const Program = @TypeOf(program);
    var result: outcomes.Exit(Program.Success, Program.Error) = undefined;
    var job: Program.Job = .{ .program = program.borrow(), .inherited = state.services, .env = env, .sink = .{ .output = &result, .next_step = .done } };
    defer rt.DescriptionNode.deinit(job.owned);
    drive(job.step(), state);
    return state.publish(result);
}

pub fn Fiber(comptime A: type, comptime E: type) type {
    return struct {
        const Self = @This();
        userdata: *anyopaque,
        vtable: *const VTable,
        observed: std.atomic.Value(bool) = .init(false),

        const VTable = struct {
            wait: *const fn (*anyopaque, ?*const rt.Cancellation) primitives.Deferred(A, E).WaitResult,
            request: *const fn (*anyopaque) void,
            interrupt: *const fn (*anyopaque) primitives.Deferred(A, E).WaitResult,
            close: *const fn (*anyopaque, bool) outcomes.Exit(void, E),
        };

        fn wait(self: *Self, token: ?*const rt.Cancellation) primitives.Deferred(A, E).WaitResult {
            return self.vtable.wait(self.userdata, token);
        }
        pub fn request(self: *Self) void {
            self.vtable.request(self.userdata);
        }
        fn interrupt(self: *Self) primitives.Deferred(A, E).WaitResult {
            return self.vtable.interrupt(self.userdata);
        }
        fn close(self: *Self) outcomes.Exit(void, E) {
            return self.vtable.close(self.userdata, self.observed.load(.acquire));
        }
    };
}

fn forkStart(env: anytype, comptime program: anytype, initial: *rt.State) outcomes.Exit(*Fiber(@TypeOf(program).Success, @TypeOf(program).Error), @TypeOf(program).Error) {
    const Program = @TypeOf(program);
    const A = Program.Success;
    const E = Program.Error;
    const R = Program.Environment;
    const Handle = AsyncRun(A, E, R);
    const Root = Handle.Root;
    const Child = Fiber(A, E);
    const Controller = struct {
        allocator: std.mem.Allocator,
        fiber: Child,
        root: *Root,

        fn wait(data: *anyopaque, token: ?*const rt.Cancellation) primitives.Deferred(A, E).WaitResult {
            const controller: *@This() = @ptrCast(@alignCast(data));
            return controller.root.completion.waitObserved(token);
        }

        fn request(data: *anyopaque) void {
            const controller: *@This() = @ptrCast(@alignCast(data));
            controller.root.cancellation.request();
        }

        fn close(data: *anyopaque, observed: bool) outcomes.Exit(void, E) {
            const controller: *@This() = @ptrCast(@alignCast(data));
            const root = controller.root;
            root.cancellation.request();
            root.mutex.lockUncancelable(root.io);
            root.join(false, true);
            root.mutex.unlock(root.io);
            var result_value: outcomes.Exit(void, E) = .{ .success = {} };
            if (!observed and root.result != .success and root.result != .interrupted) {
                result_value = outcomes.castFailure(void, root.result);
                root.result = .interrupted;
            }
            root.augment(&result_value);
            root.result.deinit();
            root.completion.deinit();
            Root.release(root);
            controller.allocator.destroy(controller);
            return result_value;
        }

        fn interrupt(data: *anyopaque) primitives.Deferred(A, E).WaitResult {
            const controller: *@This() = @ptrCast(@alignCast(data));
            const root = controller.root;
            root.cancellation.request();
            root.mutex.lockUncancelable(root.io);
            defer root.mutex.unlock(root.io);
            root.join(true, true);
            const observation = root.result.observe();
            return .{ .exit = observation.exit, .clone_failed = !observation.copied };
        }
    };
    const allocator = initial.services.allocatorValue() orelse return .{ .die = .{ .kind = .invalid_configuration, .message = diagnostics.resource_allocator } };
    const io = initial.services.ioValue() orelse return .{ .die = .{ .kind = .provider_unavailable, .message = diagnostics.fork_requires_environment_i_o_for_observation } };
    const controller = allocator.create(Controller) catch return .{ .die = outcomes.Defect.out_of_memory };
    const root = allocator.create(Root) catch {
        allocator.destroy(controller);
        return .{ .die = outcomes.Defect.out_of_memory };
    };
    const annotations = providers.cloneAnnotations(allocator, initial.annotations) catch {
        allocator.destroy(root);
        allocator.destroy(controller);
        return .{ .die = outcomes.Defect.out_of_memory };
    };
    root.* = .{ .allocator = allocator, .io = io, .env = env, .owned_env = env.*, .owned_annotations = annotations, .completion = .{ .io = io }, .state = initial.*, .storage = .{ .owner = root, .release = Root.release, .allocator = allocator } };
    root.env = &root.owned_env.?;
    root.state.protection = 0;
    root.state.fiber = true;
    root.state.protected_failure = false;
    root.state.annotations = annotations;
    root.state.branch_cancellation = &root.cancellation;
    root.state.origin_cancellation = &root.cancellation;
    root.state.ignore_environment_cancellation = true;
    root.origin.data = root;
    if (initial.protection == 0) if (initial.branch_cancellation orelse initial.origin_cancellation) |origin| origin.register(&root.origin);
    root.job = .{ .program = program.borrow(), .inherited = initial.services, .env = root.env, .sink = .{ .output = &root.result, .next_step = .done } };
    root.next = root.job.step();
    const submitted = Program.executor(env, .independent).submit(.independent, .{ .data = root, .run = Root.work, .cancellation = &root.cancellation });
    if (submitted == .rejected) {
        root.origin.unregister();
        root.completion.deinit();
        Root.release(root);
        allocator.destroy(controller);
        return .{ .die = submitted.rejected };
    }
    root.handle.init(submitted.accepted, &root.cancellation);
    root.has_handle = true;
    controller.* = .{ .allocator = allocator, .root = root, .fiber = .{ .userdata = controller, .vtable = &.{ .wait = Controller.wait, .request = Controller.request, .interrupt = Controller.interrupt, .close = Controller.close } } };
    return .{ .success = &controller.fiber };
}

fn drive(initial: rt.Step, state: *rt.State) void {
    var step = initial;
    while (true) {
        if (step.advance) |advance| step = advance(state, step.data) else if (step.suspended) |continueAsync| step = continueAsync(state, step.data) else return;
    }
}

inline fn runDriver(comptime known_entry: bool, env: anytype, program: anytype) outcomes.Exit(@TypeOf(program).Success, @TypeOf(program).Error) {
    const Program = @TypeOf(program);
    const R = Program.Environment;
    const env_ptr: *const R = env;
    var current = program;
    const enclosing = current.services(env_ptr);
    var state: rt.State = .{ .services = enclosing };
    state.origin_cancellation = state.services.cancellation;
    if (state.cancelled()) return .interrupted;
    // Keep the known initial reduction outside the driver's loop. Folding it
    // into a mutable loop-carried descriptor needlessly erases its call target.
    var produced_owner: ?*rt.DescriptionNode = null;
    defer rt.DescriptionNode.deinit(produced_owner);
    if (program.reduce) |reduce| {
        current = if (comptime known_entry)
            if (program.pure_reduce != null and state.isPristine())
                @call(.always_inline, program.pure_reduce.?, .{ env_ptr, program.value, &state })
            else
                @call(.always_inline, reduce, .{ env_ptr, program.value, &state })
        else if (program.pure_reduce != null and state.isPristine())
            program.pure_reduce.?(env_ptr, program.value, &state)
        else
            reduce(env_ptr, program.value, &state);
        produced_owner = current.ownership;
    }
    if (comptime Program.Error != void) if (current.value == .terminal) {
        state.services = rt.services(R, env_ptr).inherit(enclosing);
        return state.publish(current.value.terminal);
    };
    state.services = current.services(env_ptr).inherit(enclosing);
    if (current.delivery == .ordinary and state.cancelled()) return .interrupted;
    if (current.ready) |ready| return state.publish(ready(env_ptr, current.value, &state));
    var result: outcomes.Exit(Program.Success, Program.Error) = undefined;
    var job: Program.Job = .{ .program = current, .inherited = enclosing, .owned = produced_owner, .env = env_ptr, .sink = .{ .output = &result, .next_step = .done } };
    produced_owner = null;
    defer rt.DescriptionNode.deinit(job.owned);
    drive(current.start.?(&state, &job), &state);
    return state.publish(result);
}

test "lazy typed binding and repeated independent environments" {
    const E = union(enum) { invalid: u16 };
    const R = struct { port: u16, calls: *usize };
    const P = Effect(u16, E, R);
    const Functions = struct {
        fn read(env: *const R) u16 {
            env.calls.* += 1;
            return env.port;
        }
        fn validate(port: u16) P {
            return if (port == 0) P.fail(.{ .invalid = port }) else P.succeed(port);
        }
    };
    const program = comptime P.service(Functions.read).flatMap(Functions.validate);
    var calls: usize = 0;
    const first: R = .{ .port = 0, .calls = &calls };
    const second: R = .{ .port = 8080, .calls = &calls };
    try std.testing.expectEqual(@as(usize, 0), calls);
    const rejected = run(&first, program);
    try std.testing.expectEqual(@as(u16, 0), rejected.fail.invalid);
    try std.testing.expectEqual(@as(u16, 8080), run(&second, program).success);
    try std.testing.expectEqual(@as(usize, 2), calls);
}

test "deferred tail recursion reuses the root job for 100000 steps" {
    const R = struct { remaining: *usize };
    const P = Effect(u32, void, R);
    const Factory = struct {
        fn loop(env: *const R) P {
            if (env.remaining.* == 0) return P.succeed(42);
            env.remaining.* -= 1;
            return P.deferEffect(loop);
        }
    };
    const program = comptime P.deferEffect(Factory.loop);
    // Depth specified by the plan and F# Core.fs recursion stress contract.
    const required_deferred_depth: usize = 100_000;
    var remaining: usize = required_deferred_depth;
    const env: R = .{ .remaining = &remaining };
    try std.testing.expectEqual(@as(u32, 42), run(&env, program).success);
}

test "pending cancellation skips ordinary continuation but defects survive" {
    const R = struct { cancellation: *rt.Cancellation };
    const P = Effect(void, u32, R);
    const F = struct {
        var continuation_called: bool = false;
        fn cancel(env: *const R) void {
            env.cancellation.request();
        }
        fn continuation(_: void) P {
            continuation_called = true;
            return P.succeed({});
        }
        fn defect(env: *const R) outcomes.Exit(void, u32) {
            env.cancellation.request();
            return .{ .die = outcomes.Defect.init("defect") };
        }
    };
    var token: rt.Cancellation = .{};
    F.continuation_called = false;
    const env: R = .{ .cancellation = &token };
    const program = comptime P.sync(F.cancel).flatMap(F.continuation);
    try std.testing.expect(run(&env, program) == .interrupted);
    try std.testing.expect(!F.continuation_called);
    token = .{};
    const defective = P.fromBorrowed(void, &{}, struct {
        fn ready(_: *const void, r: *const R) outcomes.Exit(void, u32) {
            return F.defect(r);
        }
    }.ready);
    try std.testing.expect(run(&env, defective) == .die);
}

/// Private provider callback execution borrows the one continuation driver.
/// Runtime facilities retain their environment/lexical metadata, not a public
/// execution context. Factory acquisition can request raw ownership delivery.
fn ManagedEnvironment(comptime R: type) type {
    return struct {
        env: R,
        state: rt.State,
        annotations: []providers.Annotation,
        fn create(env: *const R, state: *const rt.State) ?*@This() {
            const self = env.allocator.create(@This()) catch return null;
            const annotations = providers.cloneAnnotations(env.allocator, state.annotations) catch {
                env.allocator.destroy(self);
                return null;
            };
            self.* = .{ .env = env.*, .state = state.*, .annotations = annotations };
            self.state.annotations = annotations;
            return self;
        }
        fn destroy(self: *@This()) void {
            providers.freeAnnotations(self.env.allocator, self.annotations);
            self.env.allocator.destroy(self);
        }
        fn execute(self: *const @This(), produced: anytype, token: ?*const rt.Cancellation, comptime protected: bool, comptime publish: bool) outcomes.Exit(@TypeOf(produced).Success, @TypeOf(produced).Error) {
            const Program = @TypeOf(produced);
            var program = produced;
            defer program.deinit();
            var state = self.state;
            state.services = rt.services(R, &self.env);
            state.branch_cancellation = token;
            state.origin_cancellation = token;
            state.ignore_environment_cancellation = true;
            state.interruption = .clear;
            state.protection = if (protected) 1 else 0;
            state.fiber = false;
            state.protected_failure = false;
            var result_value: outcomes.Exit(Program.Success, Program.Error) = undefined;
            var job: Program.Job = .{ .program = program.borrow(), .env = &self.env, .inherited = state.services, .sink = .{ .output = &result_value, .next_step = .done } };
            defer rt.DescriptionNode.deinit(job.owned);
            drive(job.step(), &state);
            return if (publish) state.publish(result_value) else result_value;
        }
    };
}

pub fn EffectCache(comptime Key: type, comptime A: type, comptime E: type, comptime R: type) type {
    return struct {
        const Core = @import("cache.zig").Cache(Key, A, E);
        const Config = @import("cache.zig").Config(Key, E);
        const Out = Effect(*Core, E, R);
        const Unit = Effect(void, E, R);
        pub fn make(scope: *scopes.Scope(E), comptime select_config: fn (*const R) Config, comptime load: fn (Key) Effect(A, E, R)) Out {
            const Adapter = struct {
                fn ready(env: *const R, _: Out.Value, state: *rt.State) outcomes.Exit(*Core, E) {
                    const Context = ManagedEnvironment(R);
                    const context = Context.create(env, state) orelse return .{ .die = outcomes.Defect.out_of_memory };
                    const result_value = Core.create(env, select_config(&context.env), .{ .userdata = context, .run = struct {
                        fn run(data: *anyopaque, key: Key, token: *const rt.Cancellation) outcomes.Exit(A, E) {
                            const captured: *Context = @ptrCast(@alignCast(data));
                            return captured.execute(load(key), token, false, true);
                        }
                    }.run, .deinit = struct {
                        fn destroy(data: *anyopaque) void {
                            const captured: *Context = @ptrCast(@alignCast(data));
                            captured.destroy();
                        }
                    }.destroy });
                    if (result_value != .success) context.destroy();
                    return result_value;
                }
                fn release(cache: *Core) Unit {
                    return Unit.cleanupBorrowed(Core, cache, struct {
                        fn close(borrowed: *const Core, _: *const R) outcomes.Exit(void, E) {
                            const owned = @constCast(borrowed);
                            const result_value = owned.shutdown();
                            owned.deinit();
                            return result_value;
                        }
                    }.close);
                }
            };
            const acquire: Out = .{ .ready = Adapter.ready };
            return acquire.scopeAcquire(scope, Adapter.release);
        }
        pub fn get(cache: *Core, key: Key) Effect(A, E, R) {
            const Capture = struct { cache: *Core, key: Key };
            return Effect(A, E, R).captureOwned(cache.allocator, Capture{ .cache = cache, .key = key }, true, struct {
                fn get(value: *const Capture, _: *const R, token: ?*const rt.Cancellation) outcomes.Exit(A, E) {
                    return value.cache.getToken(value.key, token);
                }
            }.get);
        }
        pub fn invalidate(cache: *Core, key: Key) Unit {
            const Capture = struct { cache: *Core, key: Key };
            const program = Unit.captureOwned(cache.allocator, Capture{ .cache = cache, .key = key }, true, struct {
                fn invalidate(value: *const Capture, _: *const R, _: ?*const rt.Cancellation) outcomes.Exit(void, E) {
                    return value.cache.invalidate(value.key);
                }
            }.invalidate);
            return program;
        }
        pub fn shutdown(cache: *Core) Unit {
            return Unit.cleanupBorrowed(Core, cache, struct {
                fn close(value: *const Core, _: *const R) outcomes.Exit(void, E) {
                    return @constCast(value).shutdown();
                }
            }.close);
        }
    };
}

pub fn EffectPool(comptime A: type, comptime E: type, comptime R: type) type {
    return struct {
        const Core = @import("pool.zig").Pool(A, E);
        const Config = @import("pool.zig").Config;
        const Out = Effect(*Core, E, R);
        const Lease = Core.Lease;
        const Acquire = Effect(*Lease, E, R);
        const Unit = Effect(void, E, R);
        pub fn make(scope: *scopes.Scope(E), comptime select_config: fn (*const R) Config, comptime create: Effect(A, E, R), comptime validate: fn (A) Effect(bool, E, R), comptime dispose: fn (A) Unit) Out {
            const Adapter = struct {
                fn ready(env: *const R, _: Out.Value, state: *rt.State) outcomes.Exit(*Core, E) {
                    const Context = ManagedEnvironment(R);
                    const context = Context.create(env, state) orelse return .{ .die = outcomes.Defect.out_of_memory };
                    const result_value = Core.create(env, select_config(&context.env), .{
                        .userdata = context,
                        .create = struct {
                            fn run(data: *anyopaque, token: *const rt.Cancellation) outcomes.Exit(A, E) {
                                const captured: *Context = @ptrCast(@alignCast(data));
                                return captured.execute(create, token, false, false);
                            }
                        }.run,
                        .validate = struct {
                            fn run(data: *anyopaque, captured_value: A, token: *const rt.Cancellation) outcomes.Exit(bool, E) {
                                const captured: *Context = @ptrCast(@alignCast(data));
                                return captured.execute(validate(captured_value), token, false, true);
                            }
                        }.run,
                        .dispose = struct {
                            fn run(data: *anyopaque, captured_value: A) outcomes.Exit(void, E) {
                                const captured: *Context = @ptrCast(@alignCast(data));
                                return captured.execute(dispose(captured_value), null, true, false);
                            }
                        }.run,
                        .deinit = struct {
                            fn destroy(data: *anyopaque) void {
                                const captured: *Context = @ptrCast(@alignCast(data));
                                captured.destroy();
                            }
                        }.destroy,
                    });
                    if (result_value != .success) context.destroy();
                    return result_value;
                }
                fn release(pool: *Core) Unit {
                    return Unit.cleanupBorrowed(Core, pool, struct {
                        fn close(captured_value: *const Core, _: *const R) outcomes.Exit(void, E) {
                            const owned = @constCast(captured_value);
                            const result_value = owned.shutdown();
                            owned.deinit();
                            return result_value;
                        }
                    }.close);
                }
            };
            const program: Out = .{ .ready = Adapter.ready };
            return program.scopeAcquire(scope, Adapter.release);
        }
        pub fn acquire(scope: *scopes.Scope(E), pool: *Core) Acquire {
            var program = Acquire.fromNativeCancellableBorrowed(Core, pool, struct {
                fn acquire(captured_value: *const Core, _: *const R, token: ?*const rt.Cancellation) outcomes.Exit(*Lease, E) {
                    return @constCast(captured_value).acquireToken(token);
                }
            }.acquire);
            var owned = program.scopeAcquireDynamic(pool.allocator, scope, struct {
                fn release(lease: *Lease) Unit {
                    return Unit.cleanupBorrowed(Lease, lease, struct {
                        fn close(captured_value: *const Lease, _: *const R) outcomes.Exit(void, E) {
                            const owned = @constCast(captured_value);
                            const result_value = owned.end(false);
                            owned.deinit();
                            return result_value;
                        }
                    }.close);
                }
            }.release);
            return owned.afterDynamic(pool.allocator, struct {
                fn complete(captured_value: outcomes.Exit(*Lease, E), _: *const R, state: *rt.State, storage: *outcomes.CleanupStorage(E)) outcomes.Exit(*Lease, E) {
                    if (captured_value != .success) return captured_value;
                    const lease = captured_value.success;
                    lease.pool.mutex.lockUncancelable(lease.pool.io);
                    const closed = lease.pool.closed;
                    lease.pool.mutex.unlock(lease.pool.io);
                    if (!closed and !state.cancelled()) return captured_value;
                    var interrupted: outcomes.Exit(*Lease, E) = .interrupted;
                    state.protection += 1;
                    storage.appendExit(&interrupted, lease.end(true));
                    state.protection -= 1;
                    return interrupted;
                }
            }.complete);
        }
        pub fn value(lease: *const Lease) A {
            return lease.value();
        }
        pub fn release(lease: *Lease) Unit {
            return disposition(lease, false);
        }
        pub fn discard(lease: *Lease) Unit {
            return disposition(lease, true);
        }
        fn disposition(lease: *Lease, comptime retire: bool) Unit {
            return Unit.cleanupBorrowed(Lease, lease, struct {
                fn run(captured_value: *const Lease, _: *const R) outcomes.Exit(void, E) {
                    return @constCast(captured_value).end(retire);
                }
            }.run);
        }
        pub fn setCapacity(pool: *Core, capacity: usize) Unit {
            const Capture = struct { pool: *Core, capacity: usize };
            return Unit.captureOwned(pool.allocator, Capture{ .pool = pool, .capacity = capacity }, false, struct {
                fn run(captured_value: *const Capture, _: *const R, _: ?*const rt.Cancellation) outcomes.Exit(void, E) {
                    return captured_value.pool.setCapacity(captured_value.capacity);
                }
            }.run);
        }
        pub fn existsIdle(pool: *Core, comptime predicate: fn (A) bool) Effect(bool, E, R) {
            return Effect(bool, E, R).fromCancellableBorrowed(Core, pool, struct {
                fn run(captured_value: *const Core, _: *const R, token: ?*const rt.Cancellation) outcomes.Exit(bool, E) {
                    return @constCast(captured_value).existsIdle(token, predicate);
                }
            }.run);
        }
        pub fn ensureSize(pool: *Core, target: usize) Unit {
            const Capture = struct { pool: *Core, target: usize };
            return Unit.captureOwned(pool.allocator, Capture{ .pool = pool, .target = target }, true, struct {
                fn run(captured_value: *const Capture, _: *const R, token: ?*const rt.Cancellation) outcomes.Exit(void, E) {
                    return captured_value.pool.ensureSize(captured_value.target, token);
                }
            }.run);
        }
        pub fn shutdown(pool: *Core) Unit {
            return Unit.cleanupBorrowed(Core, pool, struct {
                fn close(captured_value: *const Core, _: *const R) outcomes.Exit(void, E) {
                    return @constCast(captured_value).shutdown();
                }
            }.close);
        }
    };
}

pub fn Fetch(comptime R: type) type {
    return struct {
        const http = @import("http.zig");
        const E = http.Failure;
        const Acquire = Effect(*http.Response, E, R);
        const Text = Effect(outcomes.OwnedSlice(u8), E, R);
        const Unit = Effect(void, E, R);
        pub fn request(scope: *scopes.Scope(E), comptime select_request: fn (*const R) http.Request, comptime accept: fn (std.http.Status) bool) Acquire {
            const Adapter = struct {
                const Frame = struct {
                    parent: *Acquire.Job,
                    allocator: std.mem.Allocator,
                    child: Acquire.Job = undefined,
                    result_value: outcomes.Exit(*http.Response, E) = undefined,
                    fn complete(state: *rt.State, data: ?*anyopaque) rt.Step {
                        const frame: *@This() = @ptrCast(@alignCast(data.?));
                        var value = state.publish(frame.result_value);
                        if (value == .success) {
                            const status = value.success.status();
                            if (!accept(status)) value = .{ .fail = .{ .http = status } };
                        }
                        const parent = frame.parent;
                        frame.allocator.destroy(frame);
                        return parent.finish(value);
                    }
                };
                fn acquire(_: *const void, env: *const R, token: ?*const rt.Cancellation) outcomes.Exit(*http.Response, E) {
                    if (@FieldType(R, "http_transport") != http.Transport) @compileError(diagnostics.r_http_transport_must_be_a_typed_http_transport);
                    return http.Response.acquire(env, env.http_transport, select_request(env), token);
                }
                fn release(response: *http.Response) Unit {
                    return Unit.fromBorrowed(http.Response, response, struct {
                        fn close(value: *const http.Response, _: *const R) outcomes.Exit(void, E) {
                            const owner = @constCast(value);
                            const result_value = owner.close();
                            owner.deinit();
                            return result_value;
                        }
                    }.close);
                }
                fn start(state: *rt.State, parent: *Acquire.Job) rt.Step {
                    const allocator = state.services.allocatorValue() orelse return parent.finish(.{ .die = outcomes.Defect.out_of_memory });
                    const frame = allocator.create(Frame) catch return parent.finish(.{ .die = outcomes.Defect.out_of_memory });
                    const target_scope: *scopes.Scope(E) = @ptrCast(@alignCast(@constCast(parent.program.value.borrowed)));
                    const native = comptime Acquire.fromNativeCancellableBorrowed(void, &{}, acquire);
                    const program = native.scopeAcquire(target_scope, release);
                    frame.* = .{ .parent = parent, .allocator = allocator };
                    frame.child = .{ .program = program, .env = parent.env, .inherited = state.services, .sink = .{ .output = &frame.result_value, .next_step = .{ .data = frame, .advance = Frame.complete } } };
                    return frame.child.step();
                }
            };
            return .{ .value = .{ .borrowed = scope }, .start = Adapter.start };
        }
        pub fn text(response: *http.Response) Text {
            return Text.fromNativeCancellableBorrowed(http.Response, response, struct {
                fn read(value: *const http.Response, _: *const R, token: ?*const rt.Cancellation) outcomes.Exit(outcomes.OwnedSlice(u8), E) {
                    return @constCast(value).readToken(token);
                }
            }.read);
        }
        pub fn fetchText(comptime select_request: fn (*const R) http.Request, comptime accept: fn (std.http.Status) bool) Text {
            const Adapter = struct {
                const Frame = struct {
                    parent: *Text.Job,
                    allocator: std.mem.Allocator,
                    acquired: outcomes.Exit(*http.Response, E) = undefined,
                    acquisition: Acquire.Job = undefined,
                    body: Text.Job = undefined,
                    fn acquiredStep(state: *rt.State, data: ?*anyopaque) rt.Step {
                        const frame: *@This() = @ptrCast(@alignCast(data.?));
                        frame.acquired = state.publish(frame.acquired);
                        if (frame.acquired != .success) {
                            const parent = frame.parent;
                            const failure = outcomes.castFailure(outcomes.OwnedSlice(u8), frame.acquired);
                            frame.allocator.destroy(frame);
                            return parent.finish(failure);
                        }
                        const program = text(frame.acquired.success);
                        frame.body = .{ .program = program, .env = frame.parent.env, .inherited = state.services, .sink = .{ .output = frame.parent.sink.output, .next_step = .{ .data = frame, .advance = done } } };
                        return frame.body.step();
                    }
                    fn done(_: *rt.State, data: ?*anyopaque) rt.Step {
                        const frame: *@This() = @ptrCast(@alignCast(data.?));
                        const parent = frame.parent;
                        frame.allocator.destroy(frame);
                        rt.DescriptionNode.deinit(parent.owned);
                        parent.owned = null;
                        return parent.sink.next_step;
                    }
                };
                fn start(state: *rt.State, parent: *Text.Job) rt.Step {
                    const allocator = state.services.allocatorValue() orelse return parent.finish(.{ .die = outcomes.Defect.out_of_memory });
                    const frame = allocator.create(Frame) catch return parent.finish(.{ .die = outcomes.Defect.out_of_memory });
                    const scope: *scopes.Scope(E) = @ptrCast(@alignCast(@constCast(parent.program.value.borrowed)));
                    const program = request(scope, select_request, accept);
                    frame.* = .{ .parent = parent, .allocator = allocator };
                    frame.acquisition = .{ .program = program, .env = parent.env, .inherited = state.services, .sink = .{ .output = &frame.acquired, .next_step = .{ .data = frame, .advance = Frame.acquiredStep } } };
                    return frame.acquisition.step();
                }
                fn scoped(scope: *scopes.Scope(E)) Text {
                    return .{ .value = .{ .borrowed = scope }, .start = start };
                }
            };
            return Text.scoped(Adapter.scoped);
        }
    };
}

pub fn EffectWebSocket(comptime R: type) type {
    return struct {
        const sockets = @import("websocket.zig");
        const NativeEffect = Effect(sockets.NativeSocket, sockets.Error, R);
        const Unit = Effect(void, sockets.Error, R);
        pub fn send(connection: *sockets.Connection, message: sockets.Message) Unit {
            const Capture = struct { connection: *sockets.Connection, message: sockets.Message };
            return Unit.captureOwned(connection.allocator, Capture{ .connection = connection, .message = message }, true, struct {
                fn run(value: *const Capture, _: *const R, token: ?*const rt.Cancellation) outcomes.Exit(void, sockets.Error) {
                    return value.connection.sendToken(value.message, token);
                }
            }.run);
        }
        pub fn receive(connection: *sockets.Connection) Effect(sockets.Receive, sockets.Error, R) {
            return Effect(sockets.Receive, sockets.Error, R).fromNativeCancellableBorrowed(sockets.Connection, connection, struct {
                fn run(value: *const sockets.Connection, _: *const R, token: ?*const rt.Cancellation) outcomes.Exit(sockets.Receive, sockets.Error) {
                    return @constCast(value).receiveToken(token);
                }
            }.run);
        }
        pub fn close(connection: *sockets.Connection) Unit {
            return Unit.fromNativeBorrowed(sockets.Connection, connection, struct {
                fn run(value: *const sockets.Connection, _: *const R) outcomes.Exit(void, sockets.Error) {
                    return @constCast(value).close();
                }
            }.run);
        }
        pub fn abort(connection: *sockets.Connection) Unit {
            return Unit.fromNativeBorrowed(sockets.Connection, connection, struct {
                fn run(value: *const sockets.Connection, _: *const R) outcomes.Exit(void, sockets.Error) {
                    return @constCast(value).abort();
                }
            }.run);
        }
        pub fn subProtocol(connection: *sockets.Connection) Effect(?[]const u8, sockets.Error, R) {
            return Effect(?[]const u8, sockets.Error, R).fromBorrowed(sockets.Connection, connection, struct {
                fn run(value: *const sockets.Connection, _: *const R) outcomes.Exit(?[]const u8, sockets.Error) {
                    return @constCast(value).subProtocol();
                }
            }.run);
        }
        fn identity(error_value: sockets.Error) sockets.Error {
            return error_value;
        }
        pub fn withSocket(comptime select_limits: fn (*const R) sockets.Limits, comptime acquisition: NativeEffect, comptime use_connection: anytype) @typeInfo(@TypeOf(use_connection)).@"fn".return_type.? {
            return withSocketMapError(identity, select_limits, acquisition, use_connection);
        }
        pub fn withSocketMapError(comptime map_error: anytype, comptime select_limits: fn (*const R) sockets.Limits, comptime acquisition: NativeEffect, comptime use_connection: anytype) @typeInfo(@TypeOf(use_connection)).@"fn".return_type.? {
            return withNative(map_error, struct {
                fn limits(env: *const R) sockets.Limits {
                    return select_limits(env);
                }
                fn acquire(_: *const R) NativeEffect {
                    return acquisition.borrow();
                }
            }, use_connection);
        }
        pub fn withConnection(comptime select_options: fn (*const R) sockets.ConnectOptions, comptime use_connection: anytype) @typeInfo(@TypeOf(use_connection)).@"fn".return_type.? {
            return withConnectionMapError(identity, select_options, use_connection);
        }
        pub fn withConnectionMapError(comptime map_error: anytype, comptime select_options: fn (*const R) sockets.ConnectOptions, comptime use_connection: anytype) @typeInfo(@TypeOf(use_connection)).@"fn".return_type.? {
            return withNative(map_error, struct {
                fn limits(env: *const R) sockets.Limits {
                    return select_options(env).limits;
                }
                fn acquire(env: *const R) NativeEffect {
                    return NativeEffect.captureOwned(env.allocator, select_options(env), true, struct {
                        fn connect(options: *const sockets.ConnectOptions, environment: *const R, token: ?*const rt.Cancellation) outcomes.Exit(sockets.NativeSocket, sockets.Error) {
                            const uri = std.Uri.parse(options.uri) catch return .{ .fail = .{ .invalid_options = diagnostics.an_absolute_ws_wss_uri_is_required } };
                            if (uri.host == null or (!std.ascii.eqlIgnoreCase(uri.scheme, "ws") and !std.ascii.eqlIgnoreCase(uri.scheme, "wss"))) return .{ .fail = .{ .invalid_options = diagnostics.an_absolute_ws_wss_uri_is_required } };
                            if (@FieldType(R, "websocket_transport") != sockets.Transport) @compileError(diagnostics.r_websocket_transport_must_be_a_typed_websocket_transport);
                            return environment.websocket_transport.connect(environment.websocket_transport.userdata, options.*, token);
                        }
                    }.connect);
                }
            }, use_connection);
        }
        fn withNative(comptime map_error: anytype, comptime Factory: type, comptime use_connection: anytype) @typeInfo(@TypeOf(use_connection)).@"fn".return_type.? {
            const Out = @typeInfo(@TypeOf(use_connection)).@"fn".return_type.?;
            const E = Out.Error;
            comptime requireEffect(Out, E, R, .acquireRelease_body);
            if (@TypeOf(use_connection) != fn (*sockets.Connection) Out) @compileError(diagnostics.websocket_callback_must_accept_a_borrowed_connection_pointer_and_return_an_effect);
            if (@TypeOf(map_error) != fn (sockets.Error) E) @compileError(diagnostics.websocket_error_mapper_must_return_the_callback_effect_s_error_type);
            const Adapter = struct {
                const Frame = struct {
                    const Cleanup = enum { close, abort, dispose };
                    parent: *Out.Job,
                    allocator: std.mem.Allocator,
                    limits: sockets.Limits,
                    acquisition: NativeEffect.Job = undefined,
                    acquired: outcomes.Exit(sockets.NativeSocket, sockets.Error) = undefined,
                    native: ?sockets.NativeSocket = null,
                    connection: ?*sockets.Connection = null,
                    body: Out.Job = undefined,
                    body_result: outcomes.Exit(Out.Success, E) = undefined,
                    storage: [@typeInfo(Cleanup).@"enum".fields.len]outcomes.CleanupStorage(E) = undefined,
                    references: std.atomic.Value(usize) = .init(1),
                    fn release(data: *anyopaque) void {
                        const frame: *@This() = @ptrCast(@alignCast(data));
                        if (frame.references.fetchSub(1, .acq_rel) == 1) frame.allocator.destroy(frame);
                    }
                    fn acquiredStep(state: *rt.State, data: ?*anyopaque) rt.Step {
                        const frame: *@This() = @ptrCast(@alignCast(data.?));
                        if (frame.acquired != .success) {
                            const parent = frame.parent;
                            const value = outcomes.mapError(E, outcomes.castFailure(Out.Success, frame.acquired), frame.allocator, map_error);
                            release(frame);
                            return parent.finish(value);
                        }
                        frame.native = frame.acquired.success;
                        const created = sockets.Connection.acquire(frame.parent.env, frame.native.?, frame.limits);
                        if (created != .success) {
                            frame.body_result = outcomes.mapError(E, outcomes.castFailure(Out.Success, created), frame.allocator, map_error);
                            return .{ .data = frame, .advance = bodyStep };
                        }
                        frame.connection = created.success;
                        if (state.cancelled()) {
                            frame.body_result = .interrupted;
                            return .{ .data = frame, .advance = bodyStep };
                        }
                        const program = use_connection(frame.connection.?);
                        frame.body = .{ .program = program, .owned = program.ownership, .env = frame.parent.env, .inherited = state.services, .sink = .{ .output = &frame.body_result, .next_step = .{ .data = frame, .advance = bodyStep } } };
                        return frame.body.step();
                    }
                    fn append(frame: *@This(), comptime operation: Cleanup, cleanup: outcomes.Exit(void, sockets.Error)) void {
                        const mapped = outcomes.mapError(E, cleanup, frame.allocator, map_error);
                        frame.storage[@intFromEnum(operation)].appendExit(&frame.body_result, mapped);
                        if (frame.storage[@intFromEnum(operation)].retained()) _ = frame.references.fetchAdd(1, .monotonic);
                    }
                    fn bodyStep(state: *rt.State, data: ?*anyopaque) rt.Step {
                        const frame: *@This() = @ptrCast(@alignCast(data.?));
                        frame.body_result = state.publish(frame.body_result);
                        state.protection += 1;
                        return .{ .data = frame, .advance = null, .suspended = cleanupStep };
                    }
                    fn cleanupStep(state: *rt.State, data: ?*anyopaque) rt.Step {
                        const frame: *@This() = @ptrCast(@alignCast(data.?));
                        const old = frame.parent.env.io.swapCancelProtection(.blocked);
                        if (frame.connection) |connection| {
                            if (frame.body_result.primary() == .succeeded) frame.append(.close, connection.close());
                            frame.append(.abort, connection.abort());
                            frame.append(.dispose, connection.dispose());
                        } else if (frame.native) |native| {
                            if (native.vtable.abort(native.userdata)) |defect| frame.append(.abort, .{ .fail = .{ .transport = .{ .operation = .abort, .cause = defect } } });
                            frame.append(.dispose, native.vtable.dispose(native.userdata));
                        }
                        _ = frame.parent.env.io.swapCancelProtection(old);
                        state.protection -= 1;
                        const parent = frame.parent;
                        const value = state.publish(frame.body_result);
                        release(frame);
                        return parent.finish(value);
                    }
                };
                fn start(state: *rt.State, parent: *Out.Job) rt.Step {
                    const limits = Factory.limits(parent.env);
                    const valid = sockets.validate(limits);
                    if (valid != .success) return parent.finish(outcomes.mapError(E, outcomes.castFailure(Out.Success, valid), state.services.allocatorValue(), map_error));
                    const allocator = state.services.allocatorValue() orelse return parent.finish(.{ .die = outcomes.Defect.out_of_memory });
                    const frame = allocator.create(Frame) catch return parent.finish(.{ .die = outcomes.Defect.out_of_memory });
                    frame.* = .{ .parent = parent, .allocator = allocator, .limits = limits };
                    for (&frame.storage) |*storage| storage.* = .{ .allocator = allocator, .owner = frame, .release = Frame.release };
                    const program = Factory.acquire(parent.env);
                    frame.acquisition = .{ .program = program, .owned = program.ownership, .env = parent.env, .inherited = state.services, .sink = .{ .output = &frame.acquired, .next_step = .{ .data = frame, .advance = Frame.acquiredStep } } };
                    return frame.acquisition.step();
                }
            };
            return .{ .start = Adapter.start };
        }
    };
}

pub fn EffectDeferred(comptime A: type, comptime E: type, comptime R: type) type {
    return struct {
        const Core = primitives.Deferred(A, E);
        const Acquire = Effect(*Core, E, R);
        const Unit = Effect(void, E, R);
        pub fn make(scope: *scopes.Scope(E)) Acquire {
            return (comptime Acquire.fromOutcome(struct {
                fn create(env: *const R) outcomes.Exit(*Core, E) {
                    const core = env.allocator.create(Core) catch return .{ .die = outcomes.Defect.out_of_memory };
                    core.* = Core.init(env);
                    return .{ .success = core };
                }
            }.create)).scopeAcquire(scope, struct {
                fn release(core: *Core) Unit {
                    return Unit.fromBorrowed(Core, core, struct {
                        fn dispose(value: *const Core, env: *const R) outcomes.Exit(void, E) {
                            const owned = @constCast(value);
                            owned.deinit();
                            env.allocator.destroy(owned);
                            return .{ .success = {} };
                        }
                    }.dispose);
                }
            }.release);
        }
        pub fn awaitValue(deferred: *Core) Effect(A, E, R) {
            return Effect(A, E, R).awaitDeferred(deferred);
        }
        pub fn poll(deferred: *Core) Effect(?outcomes.Exit(A, E), E, R) {
            return Effect(?outcomes.Exit(A, E), E, R).fromBorrowed(Core, deferred, struct {
                fn read(value: *const Core, _: *const R) outcomes.Exit(?outcomes.Exit(A, E), E) {
                    return .{ .success = @constCast(value).poll() };
                }
            }.read);
        }
        pub fn complete(deferred: *Core, comptime select_value: fn (*const R) outcomes.Exit(A, E)) Effect(bool, E, R) {
            return Effect(bool, E, R).fromBorrowed(Core, deferred, struct {
                fn complete(value: *const Core, env: *const R) outcomes.Exit(bool, E) {
                    var selected = select_value(env);
                    const accepted = @constCast(value).complete(selected);
                    if (!accepted) selected.deinit();
                    return .{ .success = accepted };
                }
            }.complete);
        }
    };
}

pub fn EffectLatch(comptime E: type, comptime R: type) type {
    return struct {
        const Deferred = EffectDeferred(void, E, R);
        pub const make = Deferred.make;
        pub const awaitValue = Deferred.awaitValue;
        pub fn open(latch: *primitives.Latch(E)) Effect(bool, E, R) {
            return Deferred.complete(latch, struct {
                fn success(_: *const R) outcomes.Exit(void, E) {
                    return .{ .success = {} };
                }
            }.success);
        }
    };
}

pub fn EffectRef(comptime A: type, comptime E: type, comptime R: type) type {
    return struct {
        const Core = primitives.Ref(A);
        const Acquire = Effect(*Core, E, R);
        const Unit = Effect(void, E, R);
        pub fn make(scope: *scopes.Scope(E), comptime initial: fn (*const R) A) Acquire {
            return (comptime Acquire.fromOutcome(struct {
                fn create(env: *const R) outcomes.Exit(*Core, E) {
                    const core = env.allocator.create(Core) catch return .{ .die = outcomes.Defect.out_of_memory };
                    core.* = Core.init(env, initial(env));
                    return .{ .success = core };
                }
            }.create)).scopeAcquire(scope, struct {
                fn release(core: *Core) Unit {
                    return Unit.fromBorrowed(Core, core, struct {
                        fn dispose(value: *const Core, env: *const R) outcomes.Exit(void, E) {
                            env.allocator.destroy(@constCast(value));
                            return .{ .success = {} };
                        }
                    }.dispose);
                }
            }.release);
        }
        pub fn get(ref: *Core) Effect(A, E, R) {
            return Effect(A, E, R).fromBorrowed(Core, ref, struct {
                fn read(value: *const Core, _: *const R) outcomes.Exit(A, E) {
                    return .{ .success = @constCast(value).get() };
                }
            }.read);
        }
        pub fn set(ref: *Core, comptime select_value: fn (*const R) A) Unit {
            return Unit.fromBorrowed(Core, ref, struct {
                fn write(value: *const Core, env: *const R) outcomes.Exit(void, E) {
                    @constCast(value).set(select_value(env));
                    return .{ .success = {} };
                }
            }.write);
        }
        pub fn update(ref: *Core, comptime transform: fn (A) A) Unit {
            return Unit.fromBorrowed(Core, ref, struct {
                fn change(value: *const Core, _: *const R) outcomes.Exit(void, E) {
                    @constCast(value).update(transform);
                    return .{ .success = {} };
                }
            }.change);
        }
        pub fn modify(ref: *Core, comptime transform: anytype) Effect(primitives.modificationResult(A, transform), E, R) {
            const Output = primitives.modificationResult(A, transform);
            return Effect(Output, E, R).fromBorrowed(Core, ref, struct {
                fn change(value: *const Core, _: *const R) outcomes.Exit(Output, E) {
                    return .{ .success = @constCast(value).modify(transform) };
                }
            }.change);
        }
    };
}

pub fn EffectQueue(comptime A: type, comptime E: type, comptime R: type) type {
    return struct {
        const Core = primitives.Queue(A);
        const Acquire = Effect(*Core, E, R);
        const Unit = Effect(void, E, R);
        const NativeUnit = Effect(void, primitives.QueueError, R);
        pub fn make(scope: *scopes.Scope(E), comptime select_capacity: fn (*const R) ?usize) Acquire {
            return (comptime Acquire.fromOutcome(struct {
                fn create(env: *const R) outcomes.Exit(*Core, E) {
                    const core = env.allocator.create(Core) catch return .{ .die = outcomes.Defect.out_of_memory };
                    const initialized = Core.init(env, select_capacity(env));
                    if (initialized != .success) {
                        env.allocator.destroy(core);
                        return .{ .die = initialized.die };
                    }
                    core.* = initialized.success;
                    return .{ .success = core };
                }
            }.create)).scopeAcquire(scope, struct {
                fn release(core: *Core) Unit {
                    return Unit.fromBorrowed(Core, core, struct {
                        fn dispose(value: *const Core, env: *const R) outcomes.Exit(void, E) {
                            const owned = @constCast(value);
                            owned.shutdown(false);
                            owned.deinit();
                            env.allocator.destroy(owned);
                            return .{ .success = {} };
                        }
                    }.dispose);
                }
            }.release);
        }
        pub fn take(queue: *Core) Effect(A, primitives.QueueError, R) {
            return Effect(A, E, R).takeQueue(queue);
        }
        pub fn offer(queue: *Core, comptime select_value: fn (*const R) A) NativeUnit {
            return NativeUnit.fromNativeCancellableBorrowed(Core, queue, struct {
                fn offer(value: *const Core, env: *const R, token: ?*const rt.Cancellation) outcomes.Exit(void, primitives.QueueError) {
                    var selected = select_value(env);
                    const result_value = @constCast(value).offerToken(selected, token);
                    if (result_value != .success) outcomes.discardPayload(&selected);
                    return result_value;
                }
            }.offer);
        }
        pub fn offerValue(queue: *Core, value: A) NativeUnit {
            if (comptime outcomes.ownsPayload(A)) @compileError(diagnostics.a_reusable_queue_offer_must_produce_a_fresh_owning_value_through_offer);
            const Capture = struct { queue: *Core, value: A };
            return NativeUnit.captureOwned(queue.allocator, Capture{ .queue = queue, .value = value }, true, struct {
                fn offer(selected: *const Capture, _: *const R, token: ?*const rt.Cancellation) outcomes.Exit(void, primitives.QueueError) {
                    return selected.queue.offerToken(selected.value, token);
                }
            }.offer);
        }
        pub fn shutdown(queue: *Core, comptime drain: bool) Unit {
            return Unit.fromBorrowed(Core, queue, struct {
                fn shutdown(value: *const Core, _: *const R) outcomes.Exit(void, E) {
                    @constCast(value).shutdown(drain);
                    return .{ .success = {} };
                }
            }.shutdown);
        }
        pub fn size(queue: *Core) Effect(usize, E, R) {
            return Effect(usize, E, R).fromBorrowed(Core, queue, struct {
                fn read(value: *const Core, _: *const R) outcomes.Exit(usize, E) {
                    return .{ .success = @constCast(value).size() };
                }
            }.read);
        }
    };
}

pub fn EffectConsole(comptime R: type) type {
    return struct {
        const Out = Effect(void, outcomes.Defect, R);
        pub fn write(comptime message: fn (*const R) []const u8) Out {
            return Out.fromOutcome(struct {
                fn run(env: *const R) outcomes.Exit(void, outcomes.Defect) {
                    return env.console.write(message(env));
                }
            }.run);
        }
        pub fn err(comptime message: fn (*const R) []const u8) Out {
            return Out.fromOutcome(struct {
                fn run(env: *const R) outcomes.Exit(void, outcomes.Defect) {
                    return env.console.err(message(env));
                }
            }.run);
        }
    };
}
