//! Heterogeneous compile-time stage recursion; owned stages consume their
//! sources and use the ordinary Effect description ownership chain.
const std = @import("std");
const effects = @import("../effect.zig");
const inference = @import("inference.zig");
const ownership = @import("ownership.zig");

const Kind = enum { map, bind, recover, map_error };

fn Stage(comptime kind: Kind, comptime run: anytype, comptime label: []const u8) type {
    return struct {
        pub fn Output(comptime Source: type) type {
            return struct {
                const Type = outputType(Source);
            }.Type;
        }
        fn outputType(comptime Source: type) type {
            inference.requireEffect(Source, "pipeline " ++ label);
            const Input = if (kind == .recover or kind == .map_error) Source.Error else Source.Success;
            const CallbackResult = if (kind == .bind) inference.continuation(run, Input, Source.Environment, "pipeline " ++ label) else inference.unary(run, Input, "pipeline " ++ label);
            if (kind == .bind or kind == .recover) {
                inference.requireEffect(CallbackResult, "pipeline " ++ label);
                if (CallbackResult.Error != Source.Error) @compileError("pipeline " ++ label ++ " error type mismatch; use explicit mapError");
                if (CallbackResult.Environment != Source.Environment) @compileError("pipeline " ++ label ++ " environment mismatch; use explicit project/provide");
                if (kind == .recover and CallbackResult.Success != Source.Success) @compileError("pipeline " ++ label ++ " recovery success type mismatch");
                return CallbackResult;
            }
            return if (kind == .map) effects.Effect(CallbackResult, Source.Error, Source.Environment) else effects.Effect(Source.Success, CallbackResult, Source.Environment);
        }
        pub fn static(_: @This(), comptime source: anytype) Output(@TypeOf(source)) {
            return switch (kind) {
                .map => source.map(run),
                .bind => source.flatMap(run),
                .recover => source.catchAll(run),
                .map_error => source.mapError(run),
            };
        }
        pub fn owned(_: @This(), source: anytype, allocator: std.mem.Allocator) Output(@TypeOf(source.*)) {
            return switch (kind) {
                .map => source.mapAllocated(allocator, run),
                .bind => source.flatMapAllocated(allocator, run),
                .recover => source.catchAllAllocated(allocator, run),
                .map_error => source.mapErrorAllocated(allocator, run),
            };
        }
    };
}

pub fn map(comptime run: anytype) Stage(.map, run, "map") {
    return .{};
}
pub fn bind(comptime run: anytype) Stage(.bind, run, "bind") {
    return .{};
}
pub fn recover(comptime run: anytype) Stage(.recover, run, "recover") {
    return .{};
}
pub fn mapError(comptime run: anytype) Stage(.map_error, run, "mapError") {
    return .{};
}
pub fn labeled(comptime label: []const u8, comptime kind: Kind, comptime run: anytype) Stage(kind, run, label) {
    return .{};
}

pub fn project(comptime run: anytype) Project(run) {
    return .{};
}
fn Project(comptime run: anytype) type {
    const Large = inference.environment(run, "pipeline.project");
    return struct {
        pub fn Output(comptime Source: type) type {
            if (inference.unary(run, *const Large, "pipeline.project") != *const Source.Environment) @compileError("pipeline.project expected pointer to source environment");
            return effects.Effect(Source.Success, Source.Error, Large);
        }
        pub fn static(_: @This(), comptime source: anytype) Output(@TypeOf(source)) {
            return source.provideSome(Large, run);
        }
        pub fn owned(_: @This(), source: anytype, allocator: std.mem.Allocator) Output(@TypeOf(source.*)) {
            return source.provideSomeAllocated(allocator, Large, run);
        }
    };
}

pub fn captured(capture: anytype, comptime run: anytype) Captured(@TypeOf(capture), run) {
    return .{ .capture = capture };
}
fn Captured(comptime C: type, comptime run: anytype) type {
    return struct {
        capture: C,
        pub fn Output(comptime Source: type) type {
            const Out = inference.result(run, &.{ *const C, Source.Success, *const Source.Environment }, "pipeline.captured");
            @import("../dsl.zig").checkContext(Out, Source.Error, Source.Environment, "pipeline.captured");
            return Out;
        }
        pub fn owned(self: @This(), source: anytype, allocator: std.mem.Allocator) Output(@TypeOf(source.*)) {
            const Source = @TypeOf(source.*);
            const Out = Output(Source);
            const Stored = ownership.Capture(C);
            return source.flatMapCapturedAllocated(allocator, Out.Success, Stored{ .value = self.capture }, struct {
                fn next(value: *const Stored, input: Source.Success, current: *const Source.Environment) Out {
                    return run(&value.value, input, current);
                }
            }.next);
        }
    };
}

pub fn Result(comptime Source: type, comptime Stages: type, comptime index: usize) type {
    return struct {
        const Type = resultType(Source, Stages, index);
    }.Type;
}
fn resultType(comptime Source: type, comptime Stages: type, comptime index: usize) type {
    const fields = @typeInfo(Stages).@"struct".field_types;
    if (index == fields.len) return Source;
    const StageType = fields[index];
    if (!@hasDecl(StageType, "Output")) @compileError("pipeline stage " ++ std.fmt.comptimePrint("{d}", .{index}) ++ " requires a checked stage descriptor");
    return Result(StageType.Output(Source), Stages, index + 1);
}

pub fn pipe(comptime source: anytype, comptime stages: anytype) Result(@TypeOf(source), @TypeOf(stages), 0) {
    return staticAt(source, stages, 0);
}
fn staticAt(comptime source: anytype, comptime stages: anytype, comptime index: usize) Result(@TypeOf(source), @TypeOf(stages), index) {
    return struct {
        const value = staticValue(source, stages, index);
    }.value;
}
fn staticValue(comptime source: anytype, comptime stages: anytype, comptime index: usize) Result(@TypeOf(source), @TypeOf(stages), index) {
    if (index == stages.len) return source;
    return staticAt(stages[index].static(source), stages, index + 1);
}

pub fn owned(source: anytype, env: anytype, stages: anytype) Result(@TypeOf(source.*), @TypeOf(stages), 0) {
    if (@typeInfo(@TypeOf(source)).pointer.attrs.@"const") @compileError("owned pipeline requires a mutable consuming source");
    if (stages.len == 0) @compileError("empty consuming pipeline cannot copy an owner; use explicit transfer");
    return ownedAt(source, env.allocator, stages, 0);
}
fn ownedAt(source: anytype, allocator: std.mem.Allocator, stages: anytype, comptime index: usize) Result(@TypeOf(source.*), @TypeOf(stages), index) {
    if (index == stages.len) return transfer(source);
    var next = stages[index].owned(source, allocator);
    return ownedAt(&next, allocator, stages, index + 1);
}

/// Explicit move for an empty chain. Invalidates the original description.
pub fn transfer(source: anytype) @TypeOf(source.*) {
    if (@typeInfo(@TypeOf(source)).pointer.attrs.@"const") @compileError("transfer requires a mutable consuming source");
    const moved = source.*;
    source.* = undefined;
    return moved;
}
