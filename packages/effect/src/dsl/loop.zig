//! Per-run loop state and checked field moves on the existing Effect kernel.
const std = @import("std");
const effects = @import("../effect.zig");
const outcomes = @import("../outcome.zig");
const rt = @import("../internal/runtime.zig");
const inference = @import("inference.zig");
const ownership = @import("ownership.zig");

pub fn Control(comptime A: type) type {
    return union(enum) {
        pub const Value = A;
        pub const effect_owned_payload = ownership.containsOwned(A);
        again,
        done: A,
        pub fn deinit(self: *@This()) void {
            if (self.* == .done) ownership.discard(&self.done);
        }
        pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
            return switch (self.*) {
                .again => .again,
                .done => |value| .{ .done = try ownership.cloneValue(value) },
            };
        }
    };
}
pub fn View(comptime S: type) type {
    return struct {
        const Token = opaque {};
        const Storage = struct { value: *S, active: *bool, fields: *[fieldCount(S)]bool, gate: std.atomic.Value(bool) = .init(false) };
        control: *Token,
        fn storage(self: *const @This()) *Storage {
            return @ptrCast(@alignCast(self.control));
        }
        fn lock(slot: *Storage) void {
            while (slot.gate.swap(true, .acquire)) std.atomic.spinLoopHint();
        }
        fn unlock(slot: *Storage) void {
            slot.gate.store(false, .release);
        }
        /// Native mutation remains an application contract. Transfer state is
        /// hidden and synchronized, so copying a view does not reset consumption.
        pub fn state(self: *const @This()) *S {
            return self.storage().value;
        }
        pub fn takeState(self: *@This()) error{Consumed}!S {
            const slot = self.storage();
            lock(slot);
            defer unlock(slot);
            if (!slot.active.*) return error.Consumed;
            for (slot.fields.*) |initialized| if (!initialized) return error.Consumed;
            slot.active.* = false;
            return slot.value.*;
        }
        pub fn take(self: *@This(), comptime name: []const u8) error{Consumed}!@FieldType(S, name) {
            if (comptime outcomes.ownsPayload(S)) @compileError("loop field transfer requires composite per-field ownership; use takeState for an opaque owning state");
            const index = comptime fieldIndex(S, name);
            const slot = self.storage();
            lock(slot);
            defer unlock(slot);
            if (!slot.active.* or !slot.fields[index]) return error.Consumed;
            slot.fields[index] = false;
            return @field(slot.value.*, name);
        }
    };
}

fn fieldCount(comptime S: type) usize {
    return if (@typeInfo(S) == .@"struct") @typeInfo(S).@"struct".field_names.len else 0;
}
fn fieldIndex(comptime S: type, comptime name: []const u8) usize {
    for (@typeInfo(S).@"struct".field_names, 0..) |field, index| if (std.mem.eql(u8, field, name)) return index;
    @compileError("unknown loop state field '" ++ name ++ "'");
}
fn State(comptime initialize: anytype, comptime Args: type) type {
    const T = inference.result(initialize, @typeInfo(Args).@"struct".field_types, "loop.initialize");
    return if (inference.isEffect(T)) T.Success else T;
}
fn Body(comptime initialize: anytype, comptime body: anytype, comptime Args: type, comptime E: type, comptime R: type) type {
    const Out = inferredBody(initialize, body, Args, R);
    @import("../dsl.zig").checkContext(Out, E, R, "loop.body");
    return Out;
}
fn inferredBody(comptime initialize: anytype, comptime body: anytype, comptime Args: type, comptime R: type) type {
    const S = State(initialize, Args);
    const Out = inference.result(body, &.{ *View(S), *const R }, "loop.body");
    inference.requireEffect(Out, "loop.body");
    const C = Out.Success;
    if (@typeInfo(C) != .@"union" or !@hasDecl(C, "Value") or C != Control(C.Value)) @compileError("loop.body requires exact Effect(LoopControl(Result),E,R)");
    return Out;
}
pub fn Node(comptime label: []const u8, comptime selected: anytype, comptime initialize: anytype, comptime condition: anytype, comptime body: anytype, comptime finish: anytype) type {
    return struct {
        pub const name = label;
        pub const inputs = selected;
        pub const deferred_inputs = true;
        pub const context_callbacks = .{ initialize, condition, body, finish };
        pub fn contextResult(comptime R: type, comptime Args: type) type {
            const B = inferredBody(initialize, body, Args, R);
            return effects.Effect(ownership.Moved(B.Success.Value), B.Error, B.Environment);
        }
        pub fn result(comptime E: type, comptime R: type, comptime Args: type) type {
            const S = State(initialize, Args);
            const Initial = inference.result(initialize, @typeInfo(Args).@"struct".field_types, "loop.initialize");
            if (inference.isEffect(Initial)) @import("../dsl.zig").checkContext(Initial, E, R, "loop.initialize");
            if (inference.result(condition, &.{ *const S, *const R }, "loop.condition") != bool) @compileError("loop.condition requires bool");
            const B = Body(initialize, body, Args, E, R);
            if (inference.result(finish, &.{ *View(S), *const R }, "loop.finish") != B.Success.Value) @compileError("loop.finish must match the body's done result");
            ownership.validate(S);
            return effects.Effect(ownership.Moved(B.Success.Value), E, R);
        }
        pub fn invoke(comptime E: type, comptime R: type, args: anytype) result(E, R, @typeInfo(@TypeOf(args)).pointer.child) {
            const Args = @typeInfo(@TypeOf(args)).pointer.child;
            const Out = result(E, R, Args);
            return Out.deferBorrowed(Args, args, struct {
                fn create(input: *const Args, env: *const R) Out {
                    var current = input.*;
                    current[current.len - 1] = env;
                    return buildFrame(E, R, current);
                }
            }.create);
        }
        fn buildFrame(comptime E: type, comptime R: type, args: anytype) result(E, R, @TypeOf(args)) {
            const S = State(initialize, @TypeOf(args));
            const Initial = inference.result(initialize, @typeInfo(@TypeOf(args)).@"struct".field_types, "loop.initialize");
            const Init = effects.Effect(S, E, R);
            const B = Body(initialize, body, @TypeOf(args), E, R);
            const Out = result(E, R, @TypeOf(args));
            const Frame = struct {
                const Self = @This();
                header: rt.DescriptionNode,
                parent: *Out.RuntimeAdapter.Job = undefined,
                initial: Init = undefined,
                initial_job: Init.RuntimeAdapter.Job = undefined,
                initial_exit: outcomes.Exit(S, E) = undefined,
                state: S = undefined,
                initialized: bool = false,
                fields: [fieldCount(S)]bool = @splat(true),
                view: View(S) = undefined,
                view_storage: View(S).Storage = undefined,
                child: B.RuntimeAdapter.Job = undefined,
                value: outcomes.Exit(B.Success, E) = undefined,
                fn destroy(header: *rt.DescriptionNode) void {
                    const self: *Self = @alignCast(@fieldParentPtr("header", header));
                    if (self.initialized) {
                        if (comptime @typeInfo(S) == .@"struct" and !outcomes.ownsPayload(S)) {
                            const names = @typeInfo(S).@"struct".field_names;
                            inline for (0..names.len) |i| {
                                const index = names.len - i - 1;
                                if (self.fields[index]) ownership.discard(&@field(self.state, names[index]));
                            }
                        } else ownership.discard(&self.state);
                    }
                    header.allocator.destroy(self);
                }
                fn origin(raw: *const anyopaque, _: *const anyopaque) rt.Services {
                    return rt.services(R, @ptrCast(@alignCast(raw)));
                }
                fn resolve(env: *const R, _: @FieldType(Out, "value")) rt.Services {
                    return rt.services(R, env);
                }
                fn start(active: *rt.State, parent: *Out.RuntimeAdapter.Job) rt.Step {
                    const frame: *Self = @ptrCast(@alignCast(@constCast(parent.program.value.borrowed)));
                    frame.parent = parent;
                    frame.view_storage = .{ .value = &frame.state, .active = &frame.initialized, .fields = &frame.fields };
                    frame.view = .{ .control = @ptrCast(&frame.view_storage) };
                    if (comptime inference.isEffect(Initial)) {
                        frame.initial_job = .{ .program = frame.initial, .env = parent.env, .inherited = active.services, .sink = .{ .output = &frame.initial_exit, .next_step = .{ .data = frame, .advance = initializedStep } } };
                        return frame.initial_job.adapterStep();
                    }
                    return .{ .data = frame, .advance = next };
                }
                fn initializedStep(active: *rt.State, data: ?*anyopaque) rt.Step {
                    const frame: *Self = @ptrCast(@alignCast(data.?));
                    const value = frame.initial_exit;
                    if (value != .success) return frame.parent.adapterFinish(outcomes.castFailure(Out.Success, value));
                    frame.state = value.success;
                    frame.initialized = true;
                    return next(active, frame);
                }
                fn next(active: *rt.State, data: ?*anyopaque) rt.Step {
                    const frame: *Self = @ptrCast(@alignCast(data.?));
                    active.committed_delivery = false;
                    if (active.cancelled()) return frame.parent.adapterFinish(.interrupted);
                    if (!frame.initialized) return frame.parent.adapterFinish(.{ .die = .{ .kind = .invalid_configuration, .message = "loop state was consumed before another iteration" } });
                    if (!condition(&frame.state, frame.parent.env)) return frame.parent.adapterFinish(.{ .success = ownership.move(finish(&frame.view, frame.parent.env)) });
                    const described = body(&frame.view, frame.parent.env);
                    frame.child = .{ .program = described, .owned = described.ownership, .env = frame.parent.env, .inherited = active.services, .sink = .{ .output = &frame.value, .next_step = .{ .data = frame, .advance = completed } } };
                    return frame.child.adapterStep();
                }
                fn completed(active: *rt.State, data: ?*anyopaque) rt.Step {
                    const frame: *Self = @ptrCast(@alignCast(data.?));
                    const value = active.publish(frame.value);
                    if (value != .success) return frame.parent.adapterFinish(outcomes.castFailure(Out.Success, value));
                    return switch (value.success) {
                        .again => .{ .data = frame, .advance = next },
                        .done => |answer| frame.parent.adapterFinish(.{ .success = ownership.move(answer) }),
                    };
                }
            };
            const env = args[args.len - 1];
            const frame = env.allocator.create(Frame) catch return Out.die(outcomes.Defect.out_of_memory);
            frame.* = .{ .header = .{ .previous = null, .allocator = env.allocator, .destroy = Frame.destroy, .origin = .{ .data = frame, .resolve = Frame.origin } } };
            if (comptime inference.isEffect(Initial)) {
                frame.initial = @call(.auto, initialize, args);
                frame.header.previous = frame.initial.ownership;
            } else {
                frame.state = @call(.auto, initialize, args);
                frame.initialized = true;
            }
            return .{ .value = .{ .borrowed = frame }, .ownership = &frame.header, .resolve = Frame.resolve, .start = Frame.start };
        }
    };
}
