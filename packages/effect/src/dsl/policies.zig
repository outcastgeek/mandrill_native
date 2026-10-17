//! Source-driven scheduling roles lower to the existing recurrence authority.
const std = @import("std");
const effects = @import("../effect.zig");
const schedules = @import("../schedule.zig");
const outcomes = @import("../outcome.zig");
const inference = @import("inference.zig");
const roles = @import("roles.zig");
const ownership = @import("ownership.zig");
const rt = @import("../internal/runtime.zig");

fn Factory(comptime Source: type, comptime repeating: bool, comptime supplied: anytype) type {
    inference.requireEffect(Source, "schedule.source");
    roles.validate(roles.recordType(supplied), "schedule", &.{ "initialize", "step", "random" }, &.{"compose"});
    const E = Source.Error;
    const R = Source.Environment;
    const SelectedInput = if (repeating) Source.Success else E;
    const Initial = inference.unary(roles.get("initialize", supplied), *const R, "schedule.initialize");
    const is_effect = inference.isEffect(Initial);
    const is_exit = inference.isExit(Initial);
    const State = if (is_effect or is_exit) Initial.Success else Initial;
    ownership.validate(State);
    if (@typeInfo(State) == .error_union) @compileError("schedule.initialize native errors need an explicit Effect error mapper");
    if (is_effect and (Initial.Error != E or Initial.Environment != R)) @compileError("schedule.initialize Effect must match source E/R");
    if (is_exit and Initial.Error != E) @compileError("schedule.initialize Exit must match source E");
    if (@TypeOf(roles.get("random", supplied)) != bool) @compileError("schedule.random requires an explicit comptime bool");
    const with_env = inference.info(roles.get("step", supplied), "schedule.step").param_types.len == 5;
    const Decision = inference.result(roles.get("step", supplied), if (with_env) &.{ *State, SelectedInput, std.Io.Duration, f64, *const R } else &.{ *State, SelectedInput, std.Io.Duration, f64 }, "schedule.step");
    if (@typeInfo(Decision) != .@"union" or !@hasField(Decision, "stop") or Decision != schedules.Decision(@FieldType(Decision, "stop"))) @compileError("schedule.step requires exact Schedule.Decision(Output)");
    const TransitionOutput = @FieldType(Decision, "stop");
    const Stored = struct { value: State, environment: *const R };
    const Adapter = struct {
        fn step(stored: *Stored, input: SelectedInput, elapsed: std.Io.Duration, sample: f64) Decision {
            return if (comptime with_env) roles.get("step", supplied)(&stored.value, input, elapsed, sample, stored.environment) else roles.get("step", supplied)(&stored.value, input, elapsed, sample);
        }
    };
    const Base = schedules.Unfold(SelectedInput, TransitionOutput, Stored, roles.get("random", supplied), Adapter.step);
    const composed = roles.has(roles.recordType(supplied), "compose");
    const Composed = if (composed) inference.continuation(roles.get("compose", supplied), Base, R, "schedule.compose") else Base;
    if (@typeInfo(Composed) != .@"struct" or !@hasDecl(Composed, "Input") or !@hasDecl(Composed, "Output") or !@hasDecl(Composed, "uses_random") or !@hasDecl(Composed, "step")) @compileError("schedule.compose requires the existing schedule interface");
    if (Composed.Input != SelectedInput or inference.result(Composed.step, &.{ *Composed, SelectedInput, std.Io.Duration, f64 }, "schedule.compose.step") != schedules.Decision(Composed.Output)) @compileError("schedule.compose must preserve the selected source input channel");
    ownership.validate(Composed);
    const Policy = struct {
        pub const effect_owned_payload = ownership.containsOwned(Composed);
        pub const Input = Base.Input;
        pub const Output = Composed.Output;
        pub const uses_random = Composed.uses_random;
        value: Composed,
        pub fn deinit(self: *@This()) void {
            ownership.discard(&self.value);
        }
        pub fn step(self: *@This(), input: Input, elapsed: std.Io.Duration, sample: f64) schedules.Decision(Output) {
            return self.value.step(input, elapsed, sample);
        }
    };
    const Result = if (is_effect or is_exit) effects.Effect(Policy, E, R) else Policy;
    return struct {
        fn wrap(state: State, env: *const R) Policy {
            const base: Base = .{ .state = .{ .value = state, .environment = env } };
            return .{ .value = if (comptime composed) inference.call(roles.get("compose", supplied), base, env) else base };
        }
        fn select(env: *const R) Result {
            if (comptime is_effect) {
                const Initialize = effects.Effect(State, E, R);
                const Out = effects.Effect(Policy, E, R);
                const Initialization = struct {
                    const Frame = struct {
                        allocator: std.mem.Allocator,
                        parent: *Out.RuntimeAdapter.Job,
                        child: Initialize.RuntimeAdapter.Job = undefined,
                        result: outcomes.Exit(State, E) = undefined,
                        fn complete(_: *rt.State, data: ?*anyopaque) rt.Step {
                            const frame: *@This() = @ptrCast(@alignCast(data.?));
                            const parent = frame.parent;
                            const result = if (frame.result == .success)
                                outcomes.Exit(Policy, E){ .success = wrap(frame.result.success, parent.env) }
                            else
                                outcomes.castFailure(Policy, frame.result);
                            frame.allocator.destroy(frame);
                            // The iteration frame installs the owning Policy
                            // before cancellation can discard its initialization.
                            return parent.adapterFinish(result);
                        }
                    };
                    fn start(state: *rt.State, parent: *Out.RuntimeAdapter.Job) rt.Step {
                        const allocator = state.services.allocatorValue() orelse return parent.adapterFinish(.{ .die = .{ .kind = .invalid_configuration, .message = "schedule initialization requires its selected allocator" } });
                        const frame = allocator.create(Frame) catch return parent.adapterFinish(.{ .die = outcomes.Defect.out_of_memory });
                        frame.* = .{ .allocator = allocator, .parent = parent };
                        const initialized = roles.get("initialize", supplied)(parent.env);
                        frame.child = .{ .program = initialized, .owned = initialized.ownership, .inherited = state.services, .env = parent.env, .sink = .{ .output = &frame.result, .next_step = .{ .data = frame, .advance = Frame.complete } } };
                        return frame.child.adapterStep();
                    }
                };
                return .{ .start = Initialization.start };
            } else if (comptime is_exit) {
                return effects.Effect(Policy, E, R).fromOutcome(struct {
                    fn run(current: *const R) outcomes.Exit(Policy, E) {
                        const initialized = roles.get("initialize", supplied)(current);
                        return if (initialized == .success) .{ .success = wrap(initialized.success, current) } else outcomes.castFailure(Policy, initialized);
                    }
                }.run);
            } else return wrap(roles.get("initialize", supplied)(env), env);
        }
    };
}

pub fn retry(comptime source: anytype, comptime supplied: anytype) @TypeOf(source) {
    return source.retryFrom(Factory(@TypeOf(source), false, supplied).select);
}
pub fn repeat(comptime source: anytype, comptime supplied: anytype) @TypeOf(source) {
    return source.repeatFrom(Factory(@TypeOf(source), true, supplied).select);
}
pub fn retryOwned(source: anytype, env: anytype, comptime supplied: anytype) @TypeOf(source.*) {
    if (@typeInfo(@TypeOf(source)) != .pointer or @typeInfo(@TypeOf(source)).pointer.attrs.@"const") @compileError("retryOwned requires a mutable consuming Effect source");
    return source.retryFromOwned(env, Factory(@TypeOf(source.*), false, supplied).select);
}
pub fn repeatOwned(source: anytype, env: anytype, comptime supplied: anytype) @TypeOf(source.*) {
    if (@typeInfo(@TypeOf(source)) != .pointer or @typeInfo(@TypeOf(source)).pointer.attrs.@"const") @compileError("repeatOwned requires a mutable consuming Effect source");
    return source.repeatFromOwned(env, Factory(@TypeOf(source.*), true, supplied).select);
}

pub fn race(comptime left: anytype, comptime right: anytype) @TypeOf(left) {
    comptime inference.requireEffect(@TypeOf(left), "race.left");
    if (@TypeOf(left) != @TypeOf(right)) @compileError("race requires exact matching A/E/R; map or project explicitly");
    return left.race(right);
}
pub fn timeout(comptime source: anytype, comptime supplied: anytype) @TypeOf(source) {
    comptime roles.validate(roles.recordType(supplied), "timeout", &.{ "duration", "on_timeout" }, &.{});
    const Source = @TypeOf(source);
    comptime inference.requireEffect(Source, "timeout.source");
    if (inference.result(roles.get("on_timeout", supplied), &.{}, "timeout.on_timeout") != Source.Error) @compileError("timeout.on_timeout must return the exact source E");
    const Adapter = struct {
        fn timedOut() Source.Error {
            return roles.get("on_timeout", supplied)();
        }
    };
    if (comptime @TypeOf(roles.get("duration", supplied)) == std.Io.Duration) {
        if (comptime roles.get("duration", supplied).nanoseconds < 0) @compileError("timeout static duration must be non-negative");
        return source.timeout(roles.get("duration", supplied), Adapter.timedOut);
    }
    if (inference.unary(roles.get("duration", supplied), *const Source.Environment, "timeout.duration") != std.Io.Duration) @compileError("timeout.duration requires a Duration or a selector returning it");
    return source.timeoutFrom(struct {
        fn duration(env: *const Source.Environment) std.Io.Duration {
            return roles.get("duration", supplied)(env);
        }
    }.duration, Adapter.timedOut);
}
pub fn uninterruptible(comptime source: anytype) @TypeOf(source) {
    comptime inference.requireEffect(@TypeOf(source), "uninterruptible.source");
    return source.uninterruptible();
}
