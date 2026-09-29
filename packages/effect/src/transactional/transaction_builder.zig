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

/// Restricted retryable transaction description evaluated against one domain
/// journal. Its callback must be synchronous and repeatable: external I/O and
/// arbitrary Effects cannot participate in the atomic commit.
///
/// Parameters:
/// - `A: comptime type`: Value type produced by a successful computation.
/// - `E: comptime type`: Expected application-failure type; defects and interruption remain separate outcomes.
/// - `Capture: comptime type`: Compile-time captured state type retained by the composed operation.
/// - `evaluate_capture: comptime fn (*const Capture, *tx.Domain.Journal) tx.Decision(A, E)`: Compile-time typed function selected at comptime; no runtime callback
///   lookup is required.
/// Returns (`type`): retryable transaction description type evaluated against one journal.
pub fn Description(
    comptime A: type,
    comptime E: type,
    comptime Capture: type,
    comptime evaluate_capture: fn (*const Capture, *tx.Domain.Journal) tx.Decision(A, E),
) type {
    return struct {
        const Self = @This();
        /// Value type carried by the successful channel.
        pub const Success = A;
        /// Expected transaction failure type `E` carried by this description.
        pub const Error = E;
        capture: Capture,

        /// Evaluates this description against one transaction domain journal.
        ///
        /// Parameters:
        /// - `self: *const Self`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
        /// - `journal: *tx.Domain.Journal`: Caller-supplied capability or scoped state; no concrete backend is
        ///   selected here.
        /// Returns (`tx.Decision(A, E)`): A transaction decision: success, typed failure, or retry.
        pub fn evaluate(self: *const Self, journal: *tx.Domain.Journal) tx.Decision(A, E) {
            return evaluate_capture(&self.capture, journal);
        }

        /// Transforms a committed value without running another transaction.
        ///
        /// Parameters:
        /// - `self: Self`: Receiver passed by value: the struct itself is copied, while any referenced storage keeps its original ownership and lifetime.
        /// - `B: comptime type`: Success value type after this operation transforms the original value.
        /// - `transform: comptime fn (A) B`: Compile-time typed callback; its signature is checked at comptime where
        ///   static.
        /// Returns (`Mapped(Self, B, transform)`): A composed description whose evaluation remains deferred.
        pub fn map(self: Self, comptime B: type, comptime transform: fn (A) B) Mapped(Self, B, transform) {
            if (!tx.immutableValue(A)) @compileError("transaction map intermediate must be immutable and pointer-free");
            return .{ .source = self };
        }

        /// Sequences a continuation returning another restricted transaction description.
        ///
        /// Parameters:
        /// - `self: Self`: Receiver passed by value: the struct itself is copied, while any referenced storage keeps its original ownership and lifetime.
        /// - `Next: comptime type`: Compile-time next typed description produced by the continuation.
        /// - `next: comptime fn (A) Next`: Compile-time typed callback; its signature is checked at comptime where
        ///   static.
        /// Returns (`Bound(Self, Next, next)`): A composed description whose evaluation remains deferred.
        pub fn flatMap(self: Self, comptime Next: type, comptime next: fn (A) Next) Bound(Self, Next, next) {
            if (!tx.immutableValue(A)) @compileError("transaction flatMap intermediate must be immutable and pointer-free");
            return .{ .source = self };
        }

        /// Combines two descriptions in the same atomic journal.
        ///
        /// Parameters:
        /// - `self: Self`: Receiver passed by value: the struct itself is copied, while any referenced storage keeps its original ownership and lifetime.
        /// - `other: anytype`: Statically inferred input checked when this operation is instantiated.
        /// Returns (`Zipped(Self, @TypeOf(other))`): A composed description whose evaluation remains deferred.
        pub fn zip(self: Self, other: anytype) Zipped(Self, @TypeOf(other)) {
            return .{ .left = self, .right = other };
        }

        /// Tries an alternative after restoring writes from a retrying branch.
        ///
        /// Parameters:
        /// - `self: Self`: Receiver passed by value: the struct itself is copied, while any referenced storage keeps its original ownership and lifetime.
        /// - `other: anytype`: Statically inferred input checked when this operation is instantiated.
        /// Returns (`OrElse(Self, @TypeOf(other))`): A composed description whose evaluation remains deferred.
        pub fn orElse(self: Self, other: anytype) OrElse(Self, @TypeOf(other)) {
            return .{ .left = self, .right = other };
        }
    };
}

/// Captures immutable inputs for a repeatable journal callback; arbitrary I/O is outside the transaction.
///
/// Parameters:
/// - `A: comptime type`: Value type produced by a successful computation.
/// - `E: comptime type`: Expected application-failure type; defects and interruption remain separate outcomes.
/// - `capture: anytype`: Statically inferred input checked when this operation is instantiated.
/// - `evaluate_capture: comptime fn (*const @TypeOf(capture), *tx.Domain.Journal) tx.Decision(A, E)`: Compile-time typed function selected at comptime; no runtime callback
///   lookup is required.
/// Returns (`Description(A, E, @TypeOf(capture), evaluate_capture)`): A typed transactional description evaluated later against a journal.
pub fn describe(
    comptime A: type,
    comptime E: type,
    capture: anytype,
    comptime evaluate_capture: fn (*const @TypeOf(capture), *tx.Domain.Journal) tx.Decision(A, E),
) Description(A, E, @TypeOf(capture), evaluate_capture) {
    return .{ .capture = capture };
}

/// Typed mapping node; only a committed value reaches the pure transform.
///
/// Parameters:
/// - `Source: comptime type`: Compile-time source description being adapted.
/// - `B: comptime type`: Success value type after this operation transforms the original value.
/// - `transform: comptime fn (Source.Success) B`: Compile-time typed callback; its signature is checked at comptime where static.
/// Returns (`type`): transaction description whose success value is transformed by the selected pure callback.
pub fn Mapped(comptime Source: type, comptime B: type, comptime transform: fn (Source.Success) B) type {
    requireDescription(Source);
    return struct {
        const Self = @This();
        /// Value type carried by the successful channel.
        pub const Success = B;
        /// Source transaction failure type preserved by this mapped description.
        pub const Error = Source.Error;
        source: Source,

        /// Evaluates this description against one transaction domain journal.
        ///
        /// Parameters:
        /// - `self: *const Self`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
        /// - `journal: *tx.Domain.Journal`: Caller-supplied capability or scoped state; no concrete backend is
        ///   selected here.
        /// Returns (`tx.Decision(B, Error)`): A transaction decision: success, typed failure, or retry.
        pub fn evaluate(self: *const Self, journal: *tx.Domain.Journal) tx.Decision(B, Error) {
            return switch (self.source.evaluate(journal)) {
                .commit => |value| .{ .commit = transform(value) },
                .fail => |failure| .{ .fail = failure },
                .retry => .retry,
            };
        }

        /// Transforms a committed value without running another transaction.
        ///
        /// Parameters:
        /// - `self: Self`: Receiver passed by value: the struct itself is copied, while any referenced storage keeps its original ownership and lifetime.
        /// - `C: comptime type`: Success value type produced by the next stage.
        /// - `f: comptime fn (B) C`: Compile-time typed callback; its signature is checked at comptime where static.
        /// Returns (`Mapped(Self, C, f)`): A composed description whose evaluation remains deferred.
        pub fn map(self: Self, comptime C: type, comptime f: fn (B) C) Mapped(Self, C, f) {
            if (!tx.immutableValue(B)) @compileError("transaction map intermediate must be immutable and pointer-free");
            return .{ .source = self };
        }

        /// Sequences a continuation returning another restricted transaction description.
        ///
        /// Parameters:
        /// - `self: Self`: Receiver passed by value: the struct itself is copied, while any referenced storage keeps its original ownership and lifetime.
        /// - `Next: comptime type`: Compile-time next typed description produced by the continuation.
        /// - `f: comptime fn (B) Next`: Compile-time typed callback; its signature is checked at comptime where static.
        /// Returns (`Bound(Self, Next, f)`): A composed description whose evaluation remains deferred.
        pub fn flatMap(self: Self, comptime Next: type, comptime f: fn (B) Next) Bound(Self, Next, f) {
            if (!tx.immutableValue(B)) @compileError("transaction flatMap intermediate must be immutable and pointer-free");
            return .{ .source = self };
        }

        /// Combines two descriptions in the same atomic journal.
        ///
        /// Parameters:
        /// - `self: Self`: Receiver passed by value: the struct itself is copied, while any referenced storage keeps its original ownership and lifetime.
        /// - `other: anytype`: Statically inferred input checked when this operation is instantiated.
        /// Returns (`Zipped(Self, @TypeOf(other))`): A composed description whose evaluation remains deferred.
        pub fn zip(self: Self, other: anytype) Zipped(Self, @TypeOf(other)) {
            return .{ .left = self, .right = other };
        }

        /// Tries an alternative after restoring writes from a retrying branch.
        ///
        /// Parameters:
        /// - `self: Self`: Receiver passed by value: the struct itself is copied, while any referenced storage keeps its original ownership and lifetime.
        /// - `other: anytype`: Statically inferred input checked when this operation is instantiated.
        /// Returns (`OrElse(Self, @TypeOf(other))`): A composed description whose evaluation remains deferred.
        pub fn orElse(self: Self, other: anytype) OrElse(Self, @TypeOf(other)) {
            return .{ .left = self, .right = other };
        }
    };
}

/// Typed sequencing node; continuations must return a transaction description.
///
/// Parameters:
/// - `Source: comptime type`: Compile-time source description being adapted.
/// - `Next: comptime type`: Compile-time next typed description produced by the continuation.
/// - `next: comptime fn (Source.Success) Next`: Compile-time typed callback; its signature is checked at comptime where static.
/// Returns (`type`): transaction description that chooses its next journal operation from a prior success.
pub fn Bound(comptime Source: type, comptime Next: type, comptime next: fn (Source.Success) Next) type {
    requireDescription(Source);
    requireDescription(Next);
    if (Source.Error != Next.Error) @compileError("transaction flatMap must retain the typed error channel");
    return struct {
        const Self = @This();
        /// Value type carried by the successful channel.
        pub const Success = Next.Success;
        /// Source transaction failure type preserved by this bound description.
        pub const Error = Source.Error;
        source: Source,

        /// Evaluates this description against one transaction domain journal.
        ///
        /// Parameters:
        /// - `self: *const Self`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
        /// - `journal: *tx.Domain.Journal`: Caller-supplied capability or scoped state; no concrete backend is
        ///   selected here.
        /// Returns (`tx.Decision(Success, Error)`): A transaction decision: success, typed failure, or retry.
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

        /// Transforms a committed value without running another transaction.
        ///
        /// Parameters:
        /// - `self: Self`: Receiver passed by value: the struct itself is copied, while any referenced storage keeps its original ownership and lifetime.
        /// - `B: comptime type`: Success value type after this operation transforms the original value.
        /// - `f: comptime fn (Success) B`: Compile-time typed callback; its signature is checked at comptime where static.
        /// Returns (`Mapped(Self, B, f)`): A composed description whose evaluation remains deferred.
        pub fn map(self: Self, comptime B: type, comptime f: fn (Success) B) Mapped(Self, B, f) {
            if (!tx.immutableValue(Success)) @compileError("transaction map intermediate must be immutable and pointer-free");
            return .{ .source = self };
        }

        /// Sequences a continuation returning another restricted transaction description.
        ///
        /// Parameters:
        /// - `self: Self`: Receiver passed by value: the struct itself is copied, while any referenced storage keeps its original ownership and lifetime.
        /// - `NextDescription: comptime type`: Compile-time next transaction description type.
        /// - `f: comptime fn (Success) NextDescription`: Compile-time typed callback; its signature is checked at comptime where static.
        /// Returns (`Bound(Self, NextDescription, f)`): A composed description whose evaluation remains deferred.
        pub fn flatMap(self: Self, comptime NextDescription: type, comptime f: fn (Success) NextDescription) Bound(Self, NextDescription, f) {
            if (!tx.immutableValue(Success)) @compileError("transaction flatMap intermediate must be immutable and pointer-free");
            return .{ .source = self };
        }

        /// Combines two descriptions in the same atomic journal.
        ///
        /// Parameters:
        /// - `self: Self`: Receiver passed by value: the struct itself is copied, while any referenced storage keeps its original ownership and lifetime.
        /// - `other: anytype`: Statically inferred input checked when this operation is instantiated.
        /// Returns (`Zipped(Self, @TypeOf(other))`): A composed description whose evaluation remains deferred.
        pub fn zip(self: Self, other: anytype) Zipped(Self, @TypeOf(other)) {
            return .{ .left = self, .right = other };
        }

        /// Tries an alternative after restoring writes from a retrying branch.
        ///
        /// Parameters:
        /// - `self: Self`: Receiver passed by value: the struct itself is copied, while any referenced storage keeps its original ownership and lifetime.
        /// - `other: anytype`: Statically inferred input checked when this operation is instantiated.
        /// Returns (`OrElse(Self, @TypeOf(other))`): A composed description whose evaluation remains deferred.
        pub fn orElse(self: Self, other: anytype) OrElse(Self, @TypeOf(other)) {
            return .{ .left = self, .right = other };
        }
    };
}

/// Runs two descriptions in one journal and commits only their combined result.
///
/// Parameters:
/// - `Left: comptime type`: Compile-time left-hand description type.
/// - `Right: comptime type`: Compile-time right-hand description type.
/// Returns (`type`): transaction description combining the results of two journal operations.
pub fn Zipped(comptime Left: type, comptime Right: type) type {
    requireDescription(Left);
    requireDescription(Right);
    if (Left.Error != Right.Error) @compileError("transaction zip requires one typed error channel");
    return struct {
        const Self = @This();
        /// Value type carried by the successful channel.
        pub const Success = outcomes.Pair(Left.Success, Right.Success);
        /// Left transaction failure type shared by both zipped branches.
        pub const Error = Left.Error;
        left: Left,
        right: Right,

        /// Evaluates this description against one transaction domain journal.
        ///
        /// Parameters:
        /// - `self: *const Self`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
        /// - `journal: *tx.Domain.Journal`: Caller-supplied capability or scoped state; no concrete backend is
        ///   selected here.
        /// Returns (`tx.Decision(Success, Error)`): A transaction decision: success, typed failure, or retry.
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

        /// Transforms a committed value without running another transaction.
        ///
        /// Parameters:
        /// - `self: Self`: Receiver passed by value: the struct itself is copied, while any referenced storage keeps its original ownership and lifetime.
        /// - `B: comptime type`: Success value type after this operation transforms the original value.
        /// - `f: comptime fn (Success) B`: Compile-time typed callback; its signature is checked at comptime where static.
        /// Returns (`Mapped(Self, B, f)`): A composed description whose evaluation remains deferred.
        pub fn map(self: Self, comptime B: type, comptime f: fn (Success) B) Mapped(Self, B, f) {
            if (!tx.immutableValue(Success)) @compileError("transaction map intermediate must be immutable and pointer-free");
            return .{ .source = self };
        }

        /// Sequences a continuation returning another restricted transaction description.
        ///
        /// Parameters:
        /// - `self: Self`: Receiver passed by value: the struct itself is copied, while any referenced storage keeps its original ownership and lifetime.
        /// - `Next: comptime type`: Compile-time next typed description produced by the continuation.
        /// - `f: comptime fn (Success) Next`: Compile-time typed callback; its signature is checked at comptime where static.
        /// Returns (`Bound(Self, Next, f)`): A composed description whose evaluation remains deferred.
        pub fn flatMap(self: Self, comptime Next: type, comptime f: fn (Success) Next) Bound(Self, Next, f) {
            if (!tx.immutableValue(Success)) @compileError("transaction flatMap intermediate must be immutable and pointer-free");
            return .{ .source = self };
        }

        /// Combines two descriptions in the same atomic journal.
        ///
        /// Parameters:
        /// - `self: Self`: Receiver passed by value: the struct itself is copied, while any referenced storage keeps its original ownership and lifetime.
        /// - `other: anytype`: Statically inferred input checked when this operation is instantiated.
        /// Returns (`Zipped(Self, @TypeOf(other))`): A composed description whose evaluation remains deferred.
        pub fn zip(self: Self, other: anytype) Zipped(Self, @TypeOf(other)) {
            return .{ .left = self, .right = other };
        }

        /// Tries an alternative after restoring writes from a retrying branch.
        ///
        /// Parameters:
        /// - `self: Self`: Receiver passed by value: the struct itself is copied, while any referenced storage keeps its original ownership and lifetime.
        /// - `other: anytype`: Statically inferred input checked when this operation is instantiated.
        /// Returns (`OrElse(Self, @TypeOf(other))`): A composed description whose evaluation remains deferred.
        pub fn orElse(self: Self, other: anytype) OrElse(Self, @TypeOf(other)) {
            return .{ .left = self, .right = other };
        }
    };
}

/// Restores the journal savepoint before evaluating an alternative after retry.
///
/// Parameters:
/// - `Left: comptime type`: Compile-time left-hand description type.
/// - `Right: comptime type`: Compile-time right-hand description type.
/// Returns (`type`): transaction description that tries its fallback when the first branch retries.
pub fn OrElse(comptime Left: type, comptime Right: type) type {
    requireDescription(Left);
    requireDescription(Right);
    if (Left.Success != Right.Success or Left.Error != Right.Error)
        @compileError("transaction alternatives need identical success and error types");
    return struct {
        const Self = @This();
        /// Value type carried by the successful channel.
        pub const Success = Left.Success;
        /// Left transaction failure type shared by both fallback branches.
        pub const Error = Left.Error;
        left: Left,
        right: Right,

        /// Evaluates this description against one transaction domain journal.
        ///
        /// Parameters:
        /// - `self: *const Self`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
        /// - `journal: *tx.Domain.Journal`: Caller-supplied capability or scoped state; no concrete backend is
        ///   selected here.
        /// Returns (`tx.Decision(Success, Error)`): A transaction decision: success, typed failure, or retry.
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

        /// Transforms a committed value without running another transaction.
        ///
        /// Parameters:
        /// - `self: Self`: Receiver passed by value: the struct itself is copied, while any referenced storage keeps its original ownership and lifetime.
        /// - `B: comptime type`: Success value type after this operation transforms the original value.
        /// - `f: comptime fn (Success) B`: Compile-time typed callback; its signature is checked at comptime where static.
        /// Returns (`Mapped(Self, B, f)`): A composed description whose evaluation remains deferred.
        pub fn map(self: Self, comptime B: type, comptime f: fn (Success) B) Mapped(Self, B, f) {
            if (!tx.immutableValue(Success)) @compileError("transaction map intermediate must be immutable and pointer-free");
            return .{ .source = self };
        }

        /// Sequences a continuation returning another restricted transaction description.
        ///
        /// Parameters:
        /// - `self: Self`: Receiver passed by value: the struct itself is copied, while any referenced storage keeps its original ownership and lifetime.
        /// - `Next: comptime type`: Compile-time next typed description produced by the continuation.
        /// - `f: comptime fn (Success) Next`: Compile-time typed callback; its signature is checked at comptime where static.
        /// Returns (`Bound(Self, Next, f)`): A composed description whose evaluation remains deferred.
        pub fn flatMap(self: Self, comptime Next: type, comptime f: fn (Success) Next) Bound(Self, Next, f) {
            if (!tx.immutableValue(Success)) @compileError("transaction flatMap intermediate must be immutable and pointer-free");
            return .{ .source = self };
        }

        /// Combines two descriptions in the same atomic journal.
        ///
        /// Parameters:
        /// - `self: Self`: Receiver passed by value: the struct itself is copied, while any referenced storage keeps its original ownership and lifetime.
        /// - `other: anytype`: Statically inferred input checked when this operation is instantiated.
        /// Returns (`Zipped(Self, @TypeOf(other))`): A composed description whose evaluation remains deferred.
        pub fn zip(self: Self, other: anytype) Zipped(Self, @TypeOf(other)) {
            return .{ .left = self, .right = other };
        }

        /// Tries an alternative after restoring writes from a retrying branch.
        ///
        /// Parameters:
        /// - `self: Self`: Receiver passed by value: the struct itself is copied, while any referenced storage keeps its original ownership and lifetime.
        /// - `other: anytype`: Statically inferred input checked when this operation is instantiated.
        /// Returns (`OrElse(Self, @TypeOf(other))`): A composed description whose evaluation remains deferred.
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

/// Creates a pure successful transaction description from an immutable value.
///
/// Parameters:
/// - `A: comptime type`: Value type produced by a successful computation.
/// - `E: comptime type`: Expected application-failure type; defects and interruption remain separate outcomes.
/// - `value: A`: pure success value reused by this retryable transaction description.
/// Returns (`SuccessDescription(A, E)`): A typed transactional description evaluated later against a journal.
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

/// Creates a typed failed transaction description without committing writes.
///
/// Parameters:
/// - `A: comptime type`: Value type produced by a successful computation.
/// - `E: comptime type`: Expected application-failure type; defects and interruption remain separate outcomes.
/// - `failure: E`: Typed failure value propagated through the error channel.
/// Returns (`FailureDescription(A, E)`): A typed transactional description evaluated later against a journal.
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

/// Requests suspension until an observed transactional read changes.
///
/// Parameters:
/// - `A: comptime type`: Value type produced by a successful computation.
/// - `E: comptime type`: Expected application-failure type; defects and interruption remain separate outcomes.
/// Returns (`RetryDescription(A, E)`): A typed transactional description evaluated later against a journal.
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

/// Reads a cell through the journal and records its version for validation.
///
/// Parameters:
/// - `T: comptime type`: Value type stored or transported by the returned component.
/// - `E: comptime type`: Expected application-failure type; defects and interruption remain separate outcomes.
/// - `reference: *tx.Ref(T)`: Reference cell used for transactional access.
/// Returns (`ReadDescription(T, E)`): A typed transactional description evaluated later against a journal.
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

/// Stages a cell replacement; publication occurs only at commit.
///
/// Parameters:
/// - `T: comptime type`: Value type stored or transported by the returned component.
/// - `E: comptime type`: Expected application-failure type; defects and interruption remain separate outcomes.
/// - `reference: *tx.Ref(T)`: Reference cell used for transactional access.
/// - `value: T`: immutable replacement staged in the journal during evaluation.
/// Returns (`WriteDescription(T, E)`): A typed transactional description evaluated later against a journal.
pub fn write(comptime T: type, comptime E: type, reference: *tx.Ref(T), value: T) WriteDescription(T, E) {
    return .{ .capture = .{ .reference = reference, .value = value } };
}

/// Executes a description through the caller-owned serializable transaction domain.
///
/// Parameters:
/// - `R: comptime type`: Caller-defined environment type that supplies dependencies and runtime capabilities when work executes.
/// - `description: anytype`: Statically inferred input checked when this operation is instantiated.
/// - `select_domain: comptime fn (*const R) *tx.Domain`: Compile-time typed selector from the current environment; its result must
///   remain valid through use.
/// Returns (`effects.Effect(@TypeOf(description.*).Success, TransactionError(@TypeOf(description.*).Error), R)`): typed result of repeatedly evaluating this pure description until commit or failure.
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
