//! Comptime transaction descriptions. The only runner is Domain.atomically;
//! descriptions can read/write its journal but cannot bind an arbitrary Effect.
const effects = @import("../effect.zig");
const outcomes = @import("../outcome.zig");
const tx = @import("transaction.zig");
const TransactionError = @import("transaction_error.zig").Error;

fn requireDescription(comptime Candidate: type) void {
    if (@typeInfo(Candidate) != .@"struct" or
        !@hasDecl(Candidate, "Success") or
        !@hasDecl(Candidate, "Error") or
        !@hasDecl(Candidate, "evaluate"))
        @compileError("transaction composition accepts only a typed transaction description, not an Effect or native I/O");
    if (@TypeOf(Candidate.evaluate) != fn (*const Candidate, *tx.Domain.Journal) tx.Decision(Candidate.Success, Candidate.Error))
        @compileError("transaction evaluation must use the domain journal and return a typed decision");
}

pub fn Description(
    comptime A: type,
    comptime E: type,
    comptime Capture: type,
    comptime evaluate_capture: fn (*const Capture, *tx.Domain.Journal) tx.Decision(A, E),
) type {
    return struct {
        const Self = @This();
        pub const Success = A;
        pub const Error = E;
        capture: Capture,

        pub fn evaluate(self: *const Self, journal: *tx.Domain.Journal) tx.Decision(A, E) {
            return evaluate_capture(&self.capture, journal);
        }

        pub fn map(self: Self, comptime B: type, comptime transform: fn (A) B) Mapped(Self, B, transform) {
            if (!tx.immutableValue(A)) @compileError("transaction map intermediate must be immutable and pointer-free");
            return .{ .source = self };
        }

        pub fn flatMap(self: Self, comptime Next: type, comptime next: fn (A) Next) Bound(Self, Next, next) {
            if (!tx.immutableValue(A)) @compileError("transaction flatMap intermediate must be immutable and pointer-free");
            return .{ .source = self };
        }

        pub fn zip(self: Self, other: anytype) Zipped(Self, @TypeOf(other)) {
            return .{ .left = self, .right = other };
        }

        pub fn orElse(self: Self, other: anytype) OrElse(Self, @TypeOf(other)) {
            return .{ .left = self, .right = other };
        }
    };
}

pub fn describe(
    comptime A: type,
    comptime E: type,
    capture: anytype,
    comptime evaluate_capture: fn (*const @TypeOf(capture), *tx.Domain.Journal) tx.Decision(A, E),
) Description(A, E, @TypeOf(capture), evaluate_capture) {
    return .{ .capture = capture };
}

pub fn Mapped(comptime Source: type, comptime B: type, comptime transform: fn (Source.Success) B) type {
    requireDescription(Source);
    return struct {
        const Self = @This();
        pub const Success = B;
        pub const Error = Source.Error;
        source: Source,

        pub fn evaluate(self: *const Self, journal: *tx.Domain.Journal) tx.Decision(B, Error) {
            return switch (self.source.evaluate(journal)) {
                .commit => |value| .{ .commit = transform(value) },
                .fail => |failure| .{ .fail = failure },
                .retry => .retry,
            };
        }

        pub fn map(self: Self, comptime C: type, comptime f: fn (B) C) Mapped(Self, C, f) {
            if (!tx.immutableValue(B)) @compileError("transaction map intermediate must be immutable and pointer-free");
            return .{ .source = self };
        }

        pub fn flatMap(self: Self, comptime Next: type, comptime f: fn (B) Next) Bound(Self, Next, f) {
            if (!tx.immutableValue(B)) @compileError("transaction flatMap intermediate must be immutable and pointer-free");
            return .{ .source = self };
        }

        pub fn zip(self: Self, other: anytype) Zipped(Self, @TypeOf(other)) {
            return .{ .left = self, .right = other };
        }

        pub fn orElse(self: Self, other: anytype) OrElse(Self, @TypeOf(other)) {
            return .{ .left = self, .right = other };
        }
    };
}

pub fn Bound(comptime Source: type, comptime Next: type, comptime next: fn (Source.Success) Next) type {
    requireDescription(Source);
    requireDescription(Next);
    if (Source.Error != Next.Error) @compileError("transaction flatMap must retain the typed error channel");
    return struct {
        const Self = @This();
        pub const Success = Next.Success;
        pub const Error = Source.Error;
        source: Source,

        pub fn evaluate(self: *const Self, journal: *tx.Domain.Journal) tx.Decision(Success, Error) {
            return switch (self.source.evaluate(journal)) {
                .commit => |value| blk: {
                    const continuation = next(value);
                    break :blk continuation.evaluate(journal);
                },
                .fail => |failure| .{ .fail = failure },
                .retry => .retry,
            };
        }

        pub fn map(self: Self, comptime B: type, comptime f: fn (Success) B) Mapped(Self, B, f) {
            if (!tx.immutableValue(Success)) @compileError("transaction map intermediate must be immutable and pointer-free");
            return .{ .source = self };
        }

        pub fn flatMap(self: Self, comptime NextDescription: type, comptime f: fn (Success) NextDescription) Bound(Self, NextDescription, f) {
            if (!tx.immutableValue(Success)) @compileError("transaction flatMap intermediate must be immutable and pointer-free");
            return .{ .source = self };
        }

        pub fn zip(self: Self, other: anytype) Zipped(Self, @TypeOf(other)) {
            return .{ .left = self, .right = other };
        }

        pub fn orElse(self: Self, other: anytype) OrElse(Self, @TypeOf(other)) {
            return .{ .left = self, .right = other };
        }
    };
}

pub fn Zipped(comptime Left: type, comptime Right: type) type {
    requireDescription(Left);
    requireDescription(Right);
    if (Left.Error != Right.Error) @compileError("transaction zip requires one typed error channel");
    return struct {
        const Self = @This();
        pub const Success = outcomes.Pair(Left.Success, Right.Success);
        pub const Error = Left.Error;
        left: Left,
        right: Right,

        pub fn evaluate(self: *const Self, journal: *tx.Domain.Journal) tx.Decision(Success, Error) {
            const first = switch (self.left.evaluate(journal)) {
                .commit => |value| value,
                .fail => |failure| return .{ .fail = failure },
                .retry => return .retry,
            };
            return switch (self.right.evaluate(journal)) {
                .commit => |value| .{ .commit = .{ .left = first, .right = value } },
                .fail => |failure| blk: {
                    var discarded = first;
                    outcomes.discardPayload(&discarded);
                    break :blk .{ .fail = failure };
                },
                .retry => blk: {
                    var discarded = first;
                    outcomes.discardPayload(&discarded);
                    break :blk .retry;
                },
            };
        }

        pub fn map(self: Self, comptime B: type, comptime f: fn (Success) B) Mapped(Self, B, f) {
            if (!tx.immutableValue(Success)) @compileError("transaction map intermediate must be immutable and pointer-free");
            return .{ .source = self };
        }

        pub fn flatMap(self: Self, comptime Next: type, comptime f: fn (Success) Next) Bound(Self, Next, f) {
            if (!tx.immutableValue(Success)) @compileError("transaction flatMap intermediate must be immutable and pointer-free");
            return .{ .source = self };
        }

        pub fn zip(self: Self, other: anytype) Zipped(Self, @TypeOf(other)) {
            return .{ .left = self, .right = other };
        }

        pub fn orElse(self: Self, other: anytype) OrElse(Self, @TypeOf(other)) {
            return .{ .left = self, .right = other };
        }
    };
}

pub fn OrElse(comptime Left: type, comptime Right: type) type {
    requireDescription(Left);
    requireDescription(Right);
    if (Left.Success != Right.Success or Left.Error != Right.Error)
        @compileError("transaction alternatives need identical success and error types");
    return struct {
        const Self = @This();
        pub const Success = Left.Success;
        pub const Error = Left.Error;
        left: Left,
        right: Right,

        pub fn evaluate(self: *const Self, journal: *tx.Domain.Journal) tx.Decision(Success, Error) {
            var saved = journal.checkpoint() catch {
                journal.fault = .out_of_memory;
                return .retry;
            };
            defer saved.deinit();
            const first = self.left.evaluate(journal);
            if (first != .retry or journal.fault != null) return first;
            journal.restore(&saved);
            return self.right.evaluate(journal);
        }

        pub fn map(self: Self, comptime B: type, comptime f: fn (Success) B) Mapped(Self, B, f) {
            if (!tx.immutableValue(Success)) @compileError("transaction map intermediate must be immutable and pointer-free");
            return .{ .source = self };
        }

        pub fn flatMap(self: Self, comptime Next: type, comptime f: fn (Success) Next) Bound(Self, Next, f) {
            if (!tx.immutableValue(Success)) @compileError("transaction flatMap intermediate must be immutable and pointer-free");
            return .{ .source = self };
        }

        pub fn zip(self: Self, other: anytype) Zipped(Self, @TypeOf(other)) {
            return .{ .left = self, .right = other };
        }

        pub fn orElse(self: Self, other: anytype) OrElse(Self, @TypeOf(other)) {
            return .{ .left = self, .right = other };
        }
    };
}

fn SuccessDescription(comptime A: type, comptime E: type) type {
    if (!tx.immutableValue(A)) @compileError("retriable transaction constants must be immutable and pointer-free");
    const Eval = struct {
        fn run(source: *const A, _: *tx.Domain.Journal) tx.Decision(A, E) {
            return .{ .commit = source.* };
        }
    };
    return Description(A, E, A, Eval.run);
}

pub fn succeed(comptime A: type, comptime E: type, value: A) SuccessDescription(A, E) {
    return .{ .capture = value };
}

fn FailureDescription(comptime A: type, comptime E: type) type {
    if (!tx.immutableValue(E)) @compileError("retriable transaction failures must be immutable and pointer-free");
    const Eval = struct {
        fn run(source: *const E, _: *tx.Domain.Journal) tx.Decision(A, E) {
            return .{ .fail = source.* };
        }
    };
    return Description(A, E, E, Eval.run);
}

pub fn fail(comptime A: type, comptime E: type, failure: E) FailureDescription(A, E) {
    return .{ .capture = failure };
}

fn RetryDescription(comptime A: type, comptime E: type) type {
    const Eval = struct {
        fn run(_: *const void, _: *tx.Domain.Journal) tx.Decision(A, E) {
            return .retry;
        }
    };
    return Description(A, E, void, Eval.run);
}

pub fn retry(comptime A: type, comptime E: type) RetryDescription(A, E) {
    return .{ .capture = {} };
}

fn ReadDescription(comptime T: type, comptime E: type) type {
    const Capture = *tx.Ref(T);
    const Eval = struct {
        fn run(source: *const Capture, journal: *tx.Domain.Journal) tx.Decision(T, E) {
            return switch (journal.read(T, source.*)) {
                .ok => |value| .{ .commit = value },
                .err => .retry,
            };
        }
    };
    return Description(T, E, Capture, Eval.run);
}

pub fn read(comptime T: type, comptime E: type, reference: *tx.Ref(T)) ReadDescription(T, E) {
    return .{ .capture = reference };
}

fn WriteDescription(comptime T: type, comptime E: type) type {
    if (!tx.immutableValue(T)) @compileError("transactional writes require immutable values");
    const Capture = struct { reference: *tx.Ref(T), value: T };
    const Eval = struct {
        fn run(source: *const Capture, journal: *tx.Domain.Journal) tx.Decision(void, E) {
            journal.write(T, source.reference, source.value);
            return .{ .commit = {} };
        }
    };
    return Description(void, E, Capture, Eval.run);
}

pub fn write(comptime T: type, comptime E: type, reference: *tx.Ref(T), value: T) WriteDescription(T, E) {
    return .{ .capture = .{ .reference = reference, .value = value } };
}

pub fn atomically(
    comptime R: type,
    description: anytype,
    comptime select_domain: fn (*const R) *tx.Domain,
) effects.Effect(@TypeOf(description.*).Success, TransactionError(@TypeOf(description.*).Error), R) {
    const D = @TypeOf(description.*);
    return tx.Domain.atomically(D.Success, D.Error, R, D, description, select_domain, struct {
        fn evaluate(value: *const D, journal: *tx.Domain.Journal) tx.Decision(D.Success, D.Error) {
            return value.evaluate(journal);
        }
    }.evaluate);
}
