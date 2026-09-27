const diagnostics = @import("internal/diagnostics.zig");
const std = @import("std");
const Defect = @import("outcome.zig").Defect;

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

pub fn recurs(comptime InputType: type, count: usize) Recurs(InputType) {
    return .{ .count = count };
}

pub fn Recurs(comptime InputType: type) type {
    return struct {
        pub const Input = InputType;
        pub const Output = usize;
        pub const uses_random = false;
        count: usize,
        index: usize = 0,

        pub fn step(self: *@This(), _: Input, _: std.Io.Duration, _: f64) Decision(Output) {
            const current = self.index;
            if (current == self.count) return .{ .stop = current };
            self.index += 1;
            return .{ .continue_with = .{ .output = current, .delay = .zero } };
        }
    };
}

pub fn spaced(comptime InputType: type, delay: std.Io.Duration) Spaced(InputType) {
    return .{ .delay = delay };
}

pub fn Spaced(comptime InputType: type) type {
    return struct {
        pub const Input = InputType;
        pub const Output = usize;
        pub const uses_random = false;
        delay: std.Io.Duration,
        index: usize = 0,

        pub fn step(self: *@This(), _: Input, _: std.Io.Duration, _: f64) Decision(Output) {
            if (self.delay.nanoseconds < 0) return .{ .invalid = invalid(delay_contract) };
            if (self.index == std.math.maxInt(usize)) return .{ .invalid = invalid(counter_contract) };
            const current = self.index;
            self.index += 1;
            return .{ .continue_with = .{ .output = current, .delay = self.delay } };
        }
    };
}

pub fn exponential(comptime InputType: type, initial: std.Io.Duration, factor: f64) Exponential(InputType) {
    return .{ .delay = @floatFromInt(initial.nanoseconds), .factor = factor };
}

fn duration(nanoseconds: f64) ?std.Io.Duration {
    // Upper bound is exclusive because maxInt(i96) rounds upward in f64.
    if (!std.math.isFinite(nanoseconds) or nanoseconds < 0 or nanoseconds >= @as(f64, @floatFromInt(std.math.maxInt(i96)))) return null;
    return .{ .nanoseconds = @intFromFloat(nanoseconds) };
}

pub fn Exponential(comptime InputType: type) type {
    return struct {
        pub const Input = InputType;
        pub const Output = usize;
        pub const uses_random = false;
        delay: f64,
        factor: f64,
        index: usize = 0,

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

pub fn upTo(limit: std.Io.Duration, schedule: anytype) UpTo(@TypeOf(schedule)) {
    return .{ .limit = limit, .inner = schedule };
}

pub fn UpTo(comptime Inner: type) type {
    return struct {
        pub const Input = Inner.Input;
        pub const Output = Inner.Output;
        pub const uses_random = Inner.uses_random;
        limit: std.Io.Duration,
        inner: Inner,

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

pub fn jitter(min_factor: f64, max_factor: f64, schedule: anytype) Jitter(@TypeOf(schedule)) {
    return .{ .min_factor = min_factor, .max_factor = max_factor, .inner = schedule };
}

pub fn Jitter(comptime Inner: type) type {
    return struct {
        pub const Input = Inner.Input;
        pub const Output = Inner.Output;
        pub const uses_random = true;
        min_factor: f64,
        max_factor: f64,
        inner: Inner,

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

pub fn intersect(left: anytype, right: anytype) Intersection(@TypeOf(left), @TypeOf(right)) {
    return .{ .left = left, .right = right };
}

pub fn Intersection(comptime Left: type, comptime Right: type) type {
    if (Left.Input != Right.Input) @compileError(diagnostics.intersected_schedules_must_accept_the_same_input_type);
    return struct {
        pub const Input = Left.Input;
        pub const Output = std.meta.Tuple(&.{ Left.Output, Right.Output });
        pub const uses_random = Left.uses_random or Right.uses_random;
        left: Left,
        right: Right,

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
pub fn unfold(comptime InputType: type, comptime OutputType: type, state: anytype, comptime random: bool, comptime next: fn (*@TypeOf(state), InputType, std.Io.Duration, f64) Decision(OutputType)) Unfold(InputType, OutputType, @TypeOf(state), random, next) {
    return .{ .state = state };
}

pub fn Unfold(comptime InputType: type, comptime OutputType: type, comptime State: type, comptime random: bool, comptime next: fn (*State, InputType, std.Io.Duration, f64) Decision(OutputType)) type {
    return struct {
        pub const Input = InputType;
        pub const Output = OutputType;
        pub const uses_random = random;
        state: State,

        pub fn step(self: *@This(), input: Input, elapsed: std.Io.Duration, sample: f64) Decision(Output) {
            const decision = next(&self.state, input, elapsed, sample);
            if (decision == .continue_with and decision.continue_with.delay.nanoseconds < 0)
                return .{ .invalid = invalid(delay_contract) };
            return decision;
        }
    };
}
