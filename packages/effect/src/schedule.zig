//! Typed recurrence decisions and schedule composition over injected time.
const diagnostics = @import("internal/diagnostics.zig");
const std = @import("std");
const Defect = @import("outcome.zig").Defect;

/// Specialize the typed decision outcome for schedule operations.
/// Parameters:
/// - `Output: comptime type`: Service or value type produced by this composition.
/// Returns (`type`): schedule step outcome distinguishing continuation from completion.
pub fn Decision(comptime Output: type) type {
    return union(enum) {
        stop: Output,
        continue_with: struct { output: Output, delay: std.Io.Duration },
        invalid: Defect,
    };
}

fn invalid(message: []const u8) Defect {
    return .{ .kind = .invalid_configuration, .message = message };
}

const delay_contract = diagnostics.schedule_delay;
const factor_contract = diagnostics.schedule_factor;
const jitter_contract = diagnostics.schedule_jitter;
const counter_contract = diagnostics.schedule_counter;

/// Schedule a fixed number of recurrences.
/// Parameters:
/// - `InputType: comptime type`: Compile-time input value type accepted by the schedule or operation.
/// - `count: usize`: number of successful recurrences before the schedule stops.
/// Returns (`Recurs(InputType)`): A typed Schedule that computes its next delay and continuation decision.
pub fn recurs(comptime InputType: type, count: usize) Recurs(InputType) {
    return .{ .count = count };
}

/// Specialize a schedule that continues a fixed number of times.
/// Parameters:
/// - `InputType: comptime type`: Compile-time input value type accepted by the schedule or operation.
/// Returns (`type`): schedule allowing the caller-selected number of recurrences.
pub fn Recurs(comptime InputType: type) type {
    return struct {
        /// Input value type accepted by this Schedule.
        pub const Input = InputType;
        /// Typed result value produced by this definition.
        pub const Output = usize;
        /// Compile-time indication that this Schedule consumes the environment’s random source.
        pub const uses_random = false;
        count: usize,
        index: usize = 0,

        /// Advance the schedule or state machine by one decision.
        /// Parameters:
        /// - `self: *@This()`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// - `_: f64`: Unused argument required by the capability signature.
        /// - `_: f64`: Unused argument required by the capability signature.
        /// - `_: f64`: Unused argument required by the capability signature.
        /// Returns (`Decision(Output)`): A schedule decision to continue with an output and delay, or stop.
        pub fn step(self: *@This(), _: Input, _: std.Io.Duration, _: f64) Decision(Output) {
            const current = self.index;
            if (current == self.count) return .{ .stop = current };
            self.index += 1;
            return .{ .continue_with = .{ .output = current, .delay = .zero } };
        }
    };
}

/// Schedule repeated work at a fixed delay.
/// Parameters:
/// - `InputType: comptime type`: Compile-time input value type accepted by the schedule or operation.
/// - `delay: std.Io.Duration`: Requested duration before the next scheduled action.
/// Returns (`Spaced(InputType)`): A typed Schedule that computes its next delay and continuation decision.
pub fn spaced(comptime InputType: type, delay: std.Io.Duration) Spaced(InputType) {
    return .{ .delay = delay };
}

/// Specialize a schedule with a fixed delay between decisions.
/// Parameters:
/// - `InputType: comptime type`: Compile-time input value type accepted by the schedule or operation.
/// Returns (`type`): schedule waiting a fixed caller-selected delay between steps.
pub fn Spaced(comptime InputType: type) type {
    return struct {
        /// Input value type accepted by this Schedule.
        pub const Input = InputType;
        /// Typed result value produced by this definition.
        pub const Output = usize;
        /// Compile-time indication that this Schedule consumes the environment’s random source.
        pub const uses_random = false;
        delay: std.Io.Duration,
        index: usize = 0,

        /// Advance the schedule or state machine by one decision.
        /// Parameters:
        /// - `self: *@This()`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// - `_: f64`: Unused argument required by the capability signature.
        /// - `_: f64`: Unused argument required by the capability signature.
        /// - `_: f64`: Unused argument required by the capability signature.
        /// Returns (`Decision(Output)`): A schedule decision to continue with an output and delay, or stop.
        pub fn step(self: *@This(), _: Input, _: std.Io.Duration, _: f64) Decision(Output) {
            if (self.delay.nanoseconds < 0) return .{ .invalid = invalid(delay_contract) };
            if (self.index == std.math.maxInt(usize)) return .{ .invalid = invalid(counter_contract) };
            const current = self.index;
            self.index += 1;
            return .{ .continue_with = .{ .output = current, .delay = self.delay } };
        }
    };
}

/// Schedule repeated work with exponential backoff.
/// Parameters:
/// - `InputType: comptime type`: Compile-time input value type accepted by the schedule or operation.
/// - `initial: std.Io.Duration`: Delay of the first scheduled recurrence.
/// - `factor: f64`: Multiplier controlling exponential backoff.
/// Returns (`Exponential(InputType)`): A typed Schedule that computes its next delay and continuation decision.
pub fn exponential(comptime InputType: type, initial: std.Io.Duration, factor: f64) Exponential(InputType) {
    return .{ .delay = @floatFromInt(initial.nanoseconds), .factor = factor };
}

fn duration(nanoseconds: f64) ?std.Io.Duration {
    // Upper bound is exclusive because maxInt(i96) rounds upward in f64.
    if (!std.math.isFinite(nanoseconds) or nanoseconds < 0 or nanoseconds >= @as(f64, @floatFromInt(std.math.maxInt(i96)))) return null;
    return .{ .nanoseconds = @intFromFloat(nanoseconds) };
}

/// Specialize a schedule whose delay grows by the selected factor.
/// Parameters:
/// - `InputType: comptime type`: Compile-time input value type accepted by the schedule or operation.
/// Returns (`type`): schedule whose delay grows by the selected factor.
pub fn Exponential(comptime InputType: type) type {
    return struct {
        /// Input value type accepted by this Schedule.
        pub const Input = InputType;
        /// Typed result value produced by this definition.
        pub const Output = usize;
        /// Compile-time indication that this Schedule consumes the environment’s random source.
        pub const uses_random = false;
        delay: f64,
        factor: f64,
        index: usize = 0,

        /// Advance the schedule or state machine by one decision.
        /// Parameters:
        /// - `self: *@This()`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// - `_: f64`: Unused argument required by the capability signature.
        /// - `_: f64`: Unused argument required by the capability signature.
        /// - `_: f64`: Unused argument required by the capability signature.
        /// Returns (`Decision(Output)`): A schedule decision to continue with an output and delay, or stop.
        pub fn step(self: *@This(), _: Input, _: std.Io.Duration, _: f64) Decision(Output) {
            if (!std.math.isFinite(self.factor) or self.factor < 1) return .{ .invalid = invalid(factor_contract) };
            const current_delay = duration(self.delay) orelse return .{ .invalid = invalid(delay_contract) };
            if (self.index == std.math.maxInt(usize)) return .{ .invalid = invalid(counter_contract) };
            const current = self.index;
            self.index += 1;
            self.delay *= self.factor;
            return .{ .continue_with = .{ .output = current, .delay = current_delay } };
        }
    };
}

/// Stop a Schedule after the requested elapsed duration.
/// Parameters:
/// - `limit: std.Io.Duration`: elapsed-time bound after which the schedule stops.
/// - `schedule: anytype`: Statically inferred input checked when this operation is instantiated.
/// Returns (`UpTo(@TypeOf(schedule))`): A typed Schedule that computes its next delay and continuation decision.
pub fn upTo(limit: std.Io.Duration, schedule: anytype) UpTo(@TypeOf(schedule)) {
    return .{ .limit = limit, .inner = schedule };
}

/// Bound a schedule by elapsed time.
/// Parameters:
/// - `Inner: comptime type`: Compile-time inner wrapped type.
/// Returns (`type`): schedule stopping at the supplied elapsed-time bound.
pub fn UpTo(comptime Inner: type) type {
    return struct {
        /// Input value type accepted by this Schedule.
        pub const Input = Inner.Input;
        /// Typed result value produced by this definition.
        pub const Output = Inner.Output;
        /// Compile-time indication that this Schedule consumes the environment’s random source.
        pub const uses_random = Inner.uses_random;
        limit: std.Io.Duration,
        inner: Inner,

        /// Advance the schedule or state machine by one decision.
        /// Parameters:
        /// - `self: *@This()`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// - `input: Input`: typed value supplied to this schedule decision.
        /// - `elapsed: std.Io.Duration`: Elapsed duration considered by the scheduling decision.
        /// - `sample: f64`: Random sample used by jitter scheduling.
        /// Returns (`Decision(Output)`): A schedule decision to continue with an output and delay, or stop.
        pub fn step(self: *@This(), input: Input, elapsed: std.Io.Duration, sample: f64) Decision(Output) {
            if (self.limit.nanoseconds < 0) return .{ .invalid = invalid(delay_contract) };
            return switch (self.inner.step(input, elapsed, sample)) {
                .continue_with => |next| if (elapsed.nanoseconds <= self.limit.nanoseconds and next.delay.nanoseconds <= self.limit.nanoseconds - elapsed.nanoseconds)
                    .{ .continue_with = next }
                else
                    .{ .stop = next.output },
                .stop => |output| .{ .stop = output },
                .invalid => |defect| .{ .invalid = defect },
            };
        }
    };
}

/// Apply caller-bounded random jitter to a Schedule.
/// Parameters:
/// - `min_factor: f64`: Lower jitter multiplier bound.
/// - `max_factor: f64`: Upper jitter multiplier bound.
/// - `schedule: anytype`: Statically inferred input checked when this operation is instantiated.
/// Returns (`Jitter(@TypeOf(schedule))`): A typed Schedule that computes its next delay and continuation decision.
pub fn jitter(min_factor: f64, max_factor: f64, schedule: anytype) Jitter(@TypeOf(schedule)) {
    return .{ .min_factor = min_factor, .max_factor = max_factor, .inner = schedule };
}

/// Apply bounded random jitter to a schedule.
/// Parameters:
/// - `Inner: comptime type`: Compile-time inner wrapped type.
/// Returns (`type`): schedule applying bounded randomness to each selected delay.
pub fn Jitter(comptime Inner: type) type {
    return struct {
        /// Input value type accepted by this Schedule.
        pub const Input = Inner.Input;
        /// Typed result value produced by this definition.
        pub const Output = Inner.Output;
        /// Compile-time indication that this Schedule consumes the environment’s random source.
        pub const uses_random = true;
        min_factor: f64,
        max_factor: f64,
        inner: Inner,

        /// Advance the schedule or state machine by one decision.
        /// Parameters:
        /// - `self: *@This()`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// - `input: Input`: typed value supplied to this schedule decision.
        /// - `elapsed: std.Io.Duration`: Elapsed duration considered by the scheduling decision.
        /// - `sample: f64`: Random sample used by jitter scheduling.
        /// Returns (`Decision(Output)`): A schedule decision to continue with an output and delay, or stop.
        pub fn step(self: *@This(), input: Input, elapsed: std.Io.Duration, sample: f64) Decision(Output) {
            if (!std.math.isFinite(self.min_factor) or !std.math.isFinite(self.max_factor) or !std.math.isFinite(sample) or
                self.min_factor < 0 or self.max_factor < self.min_factor or sample < 0 or sample > 1)
                return .{ .invalid = invalid(jitter_contract) };
            return switch (self.inner.step(input, elapsed, sample)) {
                .continue_with => |next| blk: {
                    const nanos: f64 = @floatFromInt(next.delay.nanoseconds);
                    const delayed = duration(nanos * (self.min_factor + sample * (self.max_factor - self.min_factor))) orelse break :blk .{ .invalid = invalid(delay_contract) };
                    break :blk .{ .continue_with = .{ .output = next.output, .delay = delayed } };
                },
                .stop => |output| .{ .stop = output },
                .invalid => |defect| .{ .invalid = defect },
            };
        }
    };
}

/// Combine Schedules so both must continue.
/// Parameters:
/// - `left: anytype`: Statically inferred input checked when this operation is instantiated.
/// - `right: anytype`: Statically inferred input checked when this operation is instantiated.
/// Returns (`Intersection(@TypeOf(left), @TypeOf(right))`): A typed Schedule that computes its next delay and continuation decision.
pub fn intersect(left: anytype, right: anytype) Intersection(@TypeOf(left), @TypeOf(right)) {
    return .{ .left = left, .right = right };
}

/// Combine two schedules so both govern continuation.
/// Parameters:
/// - `Left: comptime type`: Compile-time left-hand description type.
/// - `Right: comptime type`: Compile-time right-hand description type.
/// Returns (`type`): schedule continuing only while both component schedules continue.
pub fn Intersection(comptime Left: type, comptime Right: type) type {
    if (Left.Input != Right.Input) @compileError(diagnostics.intersected_schedules_must_accept_the_same_input_type);
    return struct {
        /// Input value type accepted by this Schedule.
        pub const Input = Left.Input;
        /// Typed result value produced by this definition.
        pub const Output = @Tuple(&.{ Left.Output, Right.Output });
        /// Compile-time indication that this Schedule consumes the environment’s random source.
        pub const uses_random = Left.uses_random or Right.uses_random;
        left: Left,
        right: Right,

        /// Advance the schedule or state machine by one decision.
        /// Parameters:
        /// - `self: *@This()`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// - `input: Input`: typed value supplied to this schedule decision.
        /// - `elapsed: std.Io.Duration`: Elapsed duration considered by the scheduling decision.
        /// - `sample: f64`: Random sample used by jitter scheduling.
        /// Returns (`Decision(Output)`): A schedule decision to continue with an output and delay, or stop.
        pub fn step(self: *@This(), input: Input, elapsed: std.Io.Duration, sample: f64) Decision(Output) {
            const a = self.left.step(input, elapsed, sample);
            const b = self.right.step(input, elapsed, sample);
            if (a == .invalid) return .{ .invalid = a.invalid };
            if (b == .invalid) return .{ .invalid = b.invalid };
            const output: Output = .{ if (a == .stop) a.stop else a.continue_with.output, if (b == .stop) b.stop else b.continue_with.output };
            if (a == .stop or b == .stop) return .{ .stop = output };
            return .{ .continue_with = .{ .output = output, .delay = .{ .nanoseconds = @max(a.continue_with.delay.nanoseconds, b.continue_with.delay.nanoseconds) } } };
        }
    };
}

/// An arbitrary state machine keeps its concrete state/callbacks available to
/// comptime specialization instead of allocating an erased schedule node.
///
/// Parameters:
/// - `InputType: comptime type`: Compile-time input value type accepted by the schedule or operation.
/// - `OutputType: comptime type`: Compile-time output value type produced by the schedule or operation.
/// - `state: anytype`: Statically inferred input checked when this operation is instantiated.
/// - `random: comptime bool`: Compile-time flag selecting whether the schedule consumes randomness.
/// - `next: comptime fn (*@TypeOf(state), InputType, std.Io.Duration, f64) Decision(OutputType)`: Compile-time typed callback; its signature is checked at comptime where static.
/// Returns (`Unfold(InputType, OutputType, @TypeOf(state), random, next)`): A typed Schedule that computes its next delay and continuation decision.
pub fn unfold(comptime InputType: type, comptime OutputType: type, state: anytype, comptime random: bool, comptime next: fn (*@TypeOf(state), InputType, std.Io.Duration, f64) Decision(OutputType)) Unfold(InputType, OutputType, @TypeOf(state), random, next) {
    return .{ .state = state };
}

/// Build a stateful schedule from a typed transition callback.
/// Parameters:
/// - `InputType: comptime type`: Compile-time input value type accepted by the schedule or operation.
/// - `OutputType: comptime type`: Compile-time output value type produced by the schedule or operation.
/// - `State: comptime type`: State value type retained between operations.
/// - `random: comptime bool`: Compile-time flag selecting whether the schedule consumes randomness.
/// - `next: comptime fn (*State, InputType, std.Io.Duration, f64) Decision(OutputType)`: Compile-time typed callback; its signature is checked at comptime where static.
/// Returns (`type`): stateful schedule driven by the supplied typed transition callback.
pub fn Unfold(comptime InputType: type, comptime OutputType: type, comptime State: type, comptime random: bool, comptime next: fn (*State, InputType, std.Io.Duration, f64) Decision(OutputType)) type {
    return struct {
        /// Input value type accepted by this Schedule.
        pub const Input = InputType;
        /// Typed result value produced by this definition.
        pub const Output = OutputType;
        /// Compile-time indication that this Schedule consumes the environment’s random source.
        pub const uses_random = random;
        state: State,

        /// Advance the schedule or state machine by one decision.
        /// Parameters:
        /// - `self: *@This()`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// - `input: Input`: typed value supplied to this schedule decision.
        /// - `elapsed: std.Io.Duration`: Elapsed duration considered by the scheduling decision.
        /// - `sample: f64`: Random sample used by jitter scheduling.
        /// Returns (`Decision(Output)`): A schedule decision to continue with an output and delay, or stop.
        pub fn step(self: *@This(), input: Input, elapsed: std.Io.Duration, sample: f64) Decision(Output) {
            const decision = next(&self.state, input, elapsed, sample);
            if (decision == .continue_with and decision.continue_with.delay.nanoseconds < 0)
                return .{ .invalid = invalid(delay_contract) };
            return decision;
        }
    };
}
