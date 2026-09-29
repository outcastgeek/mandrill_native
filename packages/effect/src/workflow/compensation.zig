//! Durable compensation metadata and journal boundary. A compensation is a
//! recorded workflow action, never an automatic rollback of an external side
//! effect. Uncertain recordings require lookup before another attempt.
const std = @import("std");
const effects = @import("../effect.zig");
const outcomes = @import("../outcome.zig");
const types = @import("workflow_types.zig");

pub const Identity = struct {
    execution: types.ExecutionId,
    name: []const u8,
    version: u32,

    pub fn valid(self: Identity) bool {
        return self.execution.validate() == .ok and self.name.len > 0 and
            std.unicode.utf8ValidateSlice(self.name) and self.version > 0;
    }
};

pub const Attempt = struct {
    identity: Identity,
    ordinal: u64,
    attempt: u64,

    pub fn valid(self: Attempt) bool {
        return self.identity.valid() and self.ordinal > 0 and self.attempt > 0;
    }
};

pub const Recorded = struct {
    pub const effect_owned_payload = true;
    ordinal: u64,
    attempt: u64,
    encoded_exit: types.Bytes,

    pub fn deinit(self: *@This()) void {
        self.encoded_exit.deinit();
        self.* = undefined;
    }

    pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
        return .{ .ordinal = self.ordinal, .attempt = self.attempt, .encoded_exit = try self.encoded_exit.clone() };
    }
};

pub const Pending = struct {
    pub const effect_owned_payload = true;
    name: types.Bytes,
    version: u32,
    ordinal: u64,
    attempt: u64,

    pub fn deinit(self: *@This()) void {
        self.name.deinit();
        self.* = undefined;
    }

    pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
        return .{ .name = try self.name.clone(), .version = self.version, .ordinal = self.ordinal, .attempt = self.attempt };
    }
};

pub const Page = struct {
    pub const effect_owned_payload = true;
    items: outcomes.OwnedSlice(Pending),
    more: bool,

    pub fn deinit(self: *@This()) void {
        self.items.deinit();
        self.* = undefined;
    }

    pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
        return .{ .items = try self.items.clone(), .more = self.more };
    }
};

pub const Registration = union(enum) {
    registered: u64,
    already_registered: u64,
    conflicting_definition,
    possibly_registered,
};

pub const Recording = enum {
    recorded,
    already_recorded,
    conflicting_outcome,
    stale_attempt,
    possibly_recorded,
};

pub const Error = union(enum) {
    invalid_identity,
    malformed_record,
    unavailable,
    ordinal_exhausted,
    stale_attempt,
};

pub const Guarantee = enum { @"volatile", recoverable };

/// `register` atomically allocates a per-execution ordinal, deduplicated by
/// identity. `pendingReverse` returns highest ordinals first; its cursor is
/// exclusive. `record` fences the attempt and stores a complete encoded Exit.
/// A provider must never report success after an uncertain write.
pub fn Journal(comptime R: type, comptime ProviderError: type, comptime guarantee: Guarantee) type {
    return struct {
        const Self = @This();
        pub const storage_guarantee = guarantee;
        pub const Failure = union(enum) {
            const Own = @This();
            pub const effect_owned_payload = outcomes.ownsPayload(ProviderError);
            operation: Error,
            provider: ProviderError,

            pub fn deinit(self: *Own) void {
                if (self.* == .provider) outcomes.discardPayload(&self.provider);
                self.* = undefined;
            }

            pub fn clone(self: *const Own) error{OutOfMemory}!Own {
                return switch (self.*) {
                    .provider => |value| .{ .provider = try outcomes.clonePayload(value) },
                    else => self.*,
                };
            }
        };
        pub const RegisterEffect = effects.Effect(Registration, Failure, R);
        pub const LookupEffect = effects.Effect(?Recorded, Failure, R);
        pub const RecordEffect = effects.Effect(Recording, Failure, R);
        pub const PendingEffect = effects.Effect(Page, Failure, R);

        userdata: ?*anyopaque,
        vtable: *const VTable,

        pub const VTable = struct {
            register: *const fn (?*anyopaque, *const R, Identity) RegisterEffect,
            lookup: *const fn (?*anyopaque, *const R, Attempt) LookupEffect,
            record: *const fn (?*anyopaque, *const R, Attempt, []const u8) RecordEffect,
            pending_reverse: *const fn (?*anyopaque, *const R, types.ExecutionId, ?u64, usize) PendingEffect,
        };

        pub fn fromProvider(data: ?*anyopaque, vtable: *const VTable) Self {
            return .{ .userdata = data, .vtable = vtable };
        }

        fn invalid(comptime A: type) effects.Effect(A, Failure, R) {
            return effects.Effect(A, Failure, R).fromOutcome(struct {
                fn result(_: *const R) outcomes.Exit(A, Failure) {
                    return .{ .fail = .{ .operation = .invalid_identity } };
                }
            }.result);
        }

        pub fn register(
            comptime select_journal: fn (*const R) *const Self,
            comptime select_identity: fn (*const R) Identity,
        ) RegisterEffect {
            return RegisterEffect.deferEffect(struct {
                fn start(env: *const R) RegisterEffect {
                    const identity = select_identity(env);
                    if (!identity.valid())
                        return invalid(Registration);
                    const journal = select_journal(env);
                    return journal.vtable.register(journal.userdata, env, identity);
                }
            }.start);
        }

        pub fn lookup(
            comptime select_journal: fn (*const R) *const Self,
            comptime select_attempt: fn (*const R) Attempt,
        ) LookupEffect {
            return LookupEffect.deferEffect(struct {
                fn start(env: *const R) LookupEffect {
                    const attempt = select_attempt(env);
                    if (!attempt.valid()) return invalid(?Recorded);
                    const journal = select_journal(env);
                    return journal.vtable.lookup(journal.userdata, env, attempt);
                }
            }.start);
        }

        pub fn record(
            comptime select_journal: fn (*const R) *const Self,
            comptime select_attempt: fn (*const R) Attempt,
            comptime select_encoded_exit: fn (*const R) []const u8,
        ) RecordEffect {
            return RecordEffect.deferEffect(struct {
                fn start(env: *const R) RecordEffect {
                    const attempt = select_attempt(env);
                    const encoded = select_encoded_exit(env);
                    if (!attempt.valid() or encoded.len == 0) return invalid(Recording);
                    const journal = select_journal(env);
                    return journal.vtable.record(journal.userdata, env, attempt, encoded);
                }
            }.start);
        }

        pub fn pendingReverse(
            comptime select_journal: fn (*const R) *const Self,
            comptime select_execution: fn (*const R) types.ExecutionId,
            comptime select_before: fn (*const R) ?u64,
            comptime select_limit: fn (*const R) usize,
        ) PendingEffect {
            return PendingEffect.deferEffect(struct {
                fn start(env: *const R) PendingEffect {
                    const execution = select_execution(env);
                    const limit = select_limit(env);
                    if (execution.validate() == .err or limit == 0) return invalid(Page);
                    const journal = select_journal(env);
                    return journal.vtable.pending_reverse(journal.userdata, env, execution, select_before(env), limit);
                }
            }.start);
        }
    };
}

/// Both exits remain visible; a failed compensation must not replace the
/// original failure. The error types may differ and preserve their own tags.
pub fn Resolution(comptime OriginalSuccess: type, comptime OriginalError: type, comptime CompensationError: type) type {
    return struct {
        pub const effect_owned_payload = true;
        original: outcomes.Exit(OriginalSuccess, OriginalError),
        compensation: outcomes.Exit(void, CompensationError),

        pub fn deinit(self: *@This()) void {
            self.original.deinit();
            self.compensation.deinit();
            self.* = undefined;
        }

        pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
            var original = self.original.clone();
            if (original == .die and original.die.kind == .out_of_memory and
                !(self.original == .die and self.original.die.kind == .out_of_memory))
                return error.OutOfMemory;
            errdefer original.deinit();
            const compensation = self.compensation.clone();
            if (compensation == .die and compensation.die.kind == .out_of_memory and
                !(self.compensation == .die and self.compensation.die.kind == .out_of_memory))
                return error.OutOfMemory;
            return .{ .original = original, .compensation = compensation };
        }
    };
}

pub fn RunError(comptime JournalError: type, comptime CodecError: type) type {
    return union(enum) {
        const Self = @This();
        pub const effect_owned_payload = outcomes.ownsPayload(JournalError) or outcomes.ownsPayload(CodecError);
        journal: JournalError,
        codec: CodecError,
        invalid_identity,
        conflicting_outcome,
        stale_attempt,
        uncertain_record,

        pub fn deinit(self: *Self) void {
            switch (self.*) {
                .journal => outcomes.discardPayload(&self.journal),
                .codec => outcomes.discardPayload(&self.codec),
                else => {},
            }
            self.* = undefined;
        }

        pub fn clone(self: *const Self) error{OutOfMemory}!Self {
            return switch (self.*) {
                .journal => |value| .{ .journal = try outcomes.clonePayload(value) },
                .codec => |value| .{ .codec = try outcomes.clonePayload(value) },
                else => self.*,
            };
        }
    };
}

pub fn Completed(comptime E: type) type {
    return struct {
        pub const effect_owned_payload = true;
        source: enum { executed, replayed },
        exit: outcomes.Exit(void, E),

        pub fn deinit(self: *@This()) void {
            self.exit.deinit();
            self.* = undefined;
        }

        pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
            const copy = self.exit.clone();
            if (copy == .die and copy.die.kind == .out_of_memory and
                !(self.exit == .die and self.exit.die.kind == .out_of_memory))
                return error.OutOfMemory;
            return .{ .source = self.source, .exit = copy };
        }
    };
}

pub fn PendingStep(comptime E: type) type {
    return union(enum) {
        const Self = @This();
        pub const effect_owned_payload = true;
        none,
        completed: struct { ordinal: u64, outcome: Completed(E) },

        pub fn deinit(self: *Self) void {
            if (self.* == .completed) self.completed.outcome.deinit();
            self.* = undefined;
        }

        pub fn clone(self: *const Self) error{OutOfMemory}!Self {
            return switch (self.*) {
                .none => .none,
                .completed => |value| .{ .completed = .{ .ordinal = value.ordinal, .outcome = try value.outcome.clone() } },
            };
        }
    };
}

/// Run the highest-ordinal outstanding compensation. Repeated calls process
/// an execution in reverse registration order. The journal page remains owned
/// through the dispatched Effect, including cancellation and cleanup. A
/// failed compensation stays in the returned completed Exit for the caller
/// to combine with the original workflow failure; dispatch must record before
/// this step can report completion.
pub fn runNextPending(
    comptime E: type,
    comptime R: type,
    comptime ProviderError: type,
    comptime CodecError: type,
    comptime guarantee: Guarantee,
    comptime select_journal: fn (*const R) *const Journal(R, ProviderError, guarantee),
    comptime select_execution: fn (*const R) types.ExecutionId,
    comptime dispatch: fn (*const Pending, *const R) effects.Effect(Completed(E), RunError(Journal(R, ProviderError, guarantee).Failure, CodecError), R),
) effects.Effect(PendingStep(E), RunError(Journal(R, ProviderError, guarantee).Failure, CodecError), R) {
    const Store = Journal(R, ProviderError, guarantee);
    const Failure = RunError(Store.Failure, CodecError);
    const Out = effects.Effect(PendingStep(E), Failure, R);
    const Unit = effects.Effect(void, Failure, R);
    const first_pending: usize = 1;
    const Adapter = struct {
        fn journalError(problem: Store.Failure) Failure {
            return .{ .journal = problem };
        }

        fn load(env: *const R) Store.PendingEffect {
            const journal = select_journal(env);
            return journal.vtable.pending_reverse(journal.userdata, env, select_execution(env), null, first_pending);
        }

        fn use(page: Page, env: *const R) Out {
            if (page.items.items.len > first_pending or
                (page.items.items.len == 0 and page.more))
                return Out.fromOwnedOutcome(env, .{ .fail = .{ .journal = .{ .operation = .malformed_record } } });
            if (page.items.items.len == 0) return Out.fromOwnedSuccess(env, .none);
            var running = dispatch(&page.items.items[0], env);
            return running.flatMapCapturedOwned(env, PendingStep(E), page.items.items[0].ordinal, struct {
                fn finished(ordinal: *const u64, value: Completed(E), supplied: *const R) Out {
                    return Out.fromOwnedSuccess(supplied, .{ .completed = .{ .ordinal = ordinal.*, .outcome = value } });
                }
            }.finished);
        }

        fn release(page: Page, _: outcomes.PrimaryOutcome(Failure)) Unit {
            var owned = page;
            owned.deinit();
            return Unit.succeed({});
        }
    };
    const acquire = comptime Store.PendingEffect.deferEffect(Adapter.load).mapError(Adapter.journalError);
    return acquire.acquireRelease(Adapter.use, Adapter.release);
}

/// Drain registered compensations in reverse order. A failed, interrupted or
/// defected compensation is returned as an owned Exit so the original workflow
/// Exit can remain visible in `Resolution`. A dispatcher that reports success
/// without making journal progress is rejected instead of looping forever.
pub fn runAllPending(
    comptime E: type,
    comptime R: type,
    comptime ProviderError: type,
    comptime CodecError: type,
    comptime guarantee: Guarantee,
    comptime select_journal: fn (*const R) *const Journal(R, ProviderError, guarantee),
    comptime select_execution: fn (*const R) types.ExecutionId,
    comptime dispatch: fn (*const Pending, *const R) effects.Effect(Completed(E), RunError(Journal(R, ProviderError, guarantee).Failure, CodecError), R),
) effects.Effect(outcomes.Exit(void, E), RunError(Journal(R, ProviderError, guarantee).Failure, CodecError), R) {
    const Failure = RunError(Journal(R, ProviderError, guarantee).Failure, CodecError);
    const Out = effects.Effect(outcomes.Exit(void, E), Failure, R);
    const Adapter = struct {
        fn remaining(previous: ?u64, env: *const R) Out {
            var next_step = runNextPending(E, R, ProviderError, CodecError, guarantee, select_journal, select_execution, dispatch);
            return next_step.flatMapCapturedOwned(env, outcomes.Exit(void, E), previous, next);
        }

        fn next(previous: *const ?u64, observed: PendingStep(E), env: *const R) Out {
            return switch (observed) {
                .none => Out.fromOwnedSuccess(env, .{ .success = {} }),
                .completed => |completed| blk: {
                    if (previous.*) |last| if (completed.ordinal >= last) {
                        var discarded = completed.outcome;
                        discarded.deinit();
                        break :blk Out.fromOwnedOutcome(env, .{ .fail = .stale_attempt });
                    };
                    if (completed.outcome.exit == .success) {
                        var discarded = completed.outcome;
                        discarded.deinit();
                        break :blk remaining(completed.ordinal, env);
                    }
                    break :blk Out.fromOwnedSuccess(env, completed.outcome.exit);
                },
            };
        }

        fn start(env: *const R) Out {
            return remaining(null, env);
        }
    };
    return Out.deferEffect(Adapter.start);
}

/// Replay a completed attempt or execute and record its complete Exit before
/// returning it. An ambiguous record remains typed uncertainty; callers must
/// look it up before retrying a possibly completed external compensation.
/// The supplied program must use a stable external idempotency key derived
/// from the attempt identity whenever it performs an external side effect;
/// recording the Exit cannot make that side effect exactly once.
pub fn runRecorded(
    comptime E: type,
    comptime R: type,
    comptime ProviderError: type,
    comptime CodecError: type,
    comptime guarantee: Guarantee,
    comptime program: effects.Effect(void, E, R),
    comptime select_journal: fn (*const R) *const Journal(R, ProviderError, guarantee),
    comptime select_attempt: fn (*const R) Attempt,
    comptime encode_exit: fn (*const outcomes.Exit(void, E), *const R) outcomes.Exit(outcomes.OwnedSlice(u8), CodecError),
    comptime decode_exit: fn ([]const u8, *const R) outcomes.Exit(outcomes.Exit(void, E), CodecError),
) effects.Effect(Completed(E), RunError(Journal(R, ProviderError, guarantee).Failure, CodecError), R) {
    const Store = Journal(R, ProviderError, guarantee);
    const Failure = RunError(Store.Failure, CodecError);
    const Out = effects.Effect(Completed(E), Failure, R);
    const Encoded = outcomes.OwnedSlice(u8);
    const Capture = struct {
        pub const effect_owned_payload = true;
        exit: outcomes.Exit(void, E),
        encoded: Encoded,

        pub fn deinit(self: *@This()) void {
            self.exit.deinit();
            self.encoded.deinit();
            self.* = undefined;
        }

        pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
            const exit = self.exit.clone();
            if (exit == .die and exit.die.kind == .out_of_memory and
                !(self.exit == .die and self.exit.die.kind == .out_of_memory))
                return error.OutOfMemory;
            var owned = exit;
            errdefer owned.deinit();
            return .{ .exit = owned, .encoded = try self.encoded.clone() };
        }
    };
    const Adapter = struct {
        fn journalError(problem: Store.Failure) Failure {
            return switch (problem) {
                .operation => |operation| if (operation == .stale_attempt)
                    .stale_attempt
                else
                    .{ .journal = problem },
                .provider => .{ .journal = problem },
            };
        }
        fn codecError(problem: CodecError) Failure {
            return .{ .codec = problem };
        }
        fn impossible(_: void) Failure {
            unreachable;
        }

        fn replay(record: Recorded, env: *const R) Out {
            var owned = record;
            const decoded = decode_exit(owned.encoded_exit.items, env);
            owned.deinit();
            const mapped = outcomes.mapError(Failure, decoded, env.allocator, codecError);
            return switch (mapped) {
                .success => |exit| Out.fromOwnedSuccess(env, .{ .source = .replayed, .exit = exit }),
                else => Out.fromOwnedOutcome(env, outcomes.castFailure(Completed(E), mapped)),
            };
        }

        fn committed(captured: *const Capture, status: Recording, env: *const R) Out {
            return switch (status) {
                .recorded, .already_recorded => blk: {
                    const copy = captured.exit.clone();
                    if (copy == .die and copy.die.kind == .out_of_memory and
                        !(captured.exit == .die and captured.exit.die.kind == .out_of_memory))
                        break :blk Out.die(outcomes.Defect.out_of_memory);
                    break :blk Out.fromOwnedSuccess(env, .{ .source = .executed, .exit = copy });
                },
                .conflicting_outcome => Out.fromOwnedOutcome(env, .{ .fail = .conflicting_outcome }),
                .stale_attempt => Out.fromOwnedOutcome(env, .{ .fail = .stale_attempt }),
                .possibly_recorded => Out.fromOwnedOutcome(env, .{ .fail = .uncertain_record }),
            };
        }

        fn executed(exit: outcomes.Exit(void, E), env: *const R) Out {
            var owned = exit;
            const encoded_result = encode_exit(&owned, env);
            const encoded = switch (encoded_result) {
                .success => |bytes| bytes,
                else => {
                    owned.deinit();
                    const mapped = outcomes.mapError(Failure, outcomes.castFailure(Encoded, encoded_result), env.allocator, codecError);
                    return Out.fromOwnedOutcome(env, outcomes.castFailure(Completed(E), mapped));
                },
            };
            const capture: Capture = .{ .exit = owned, .encoded = encoded };
            const store = select_journal(env);
            var recording = store.vtable.record(store.userdata, env, select_attempt(env), capture.encoded.items);
            var mapped = recording.mapErrorOwned(env, journalError);
            return mapped.flatMapCapturedOwned(env, Completed(E), capture, committed);
        }

        fn loaded(record: ?Recorded, env: *const R) Out {
            if (record) |existing| return replay(existing, env);
            var running = program;
            var settled = running.materializeOwned(env);
            var aligned = settled.mapErrorOwned(env, impossible);
            return aligned.flatMapOwned(env, executed);
        }

        fn registered(status: Registration, env: *const R) Out {
            const ordinal = switch (status) {
                .registered => |value| value,
                .already_registered => |value| value,
                .conflicting_definition => return Out.fromOwnedOutcome(env, .{ .fail = .conflicting_outcome }),
                .possibly_registered => return Out.fromOwnedOutcome(env, .{ .fail = .uncertain_record }),
            };
            const attempt = select_attempt(env);
            if (ordinal != attempt.ordinal) return Out.fromOwnedOutcome(env, .{ .fail = .stale_attempt });
            const store = select_journal(env);
            var reading = store.vtable.lookup(store.userdata, env, attempt);
            var mapped = reading.mapErrorOwned(env, journalError);
            return mapped.flatMapOwned(env, loaded);
        }
    };
    return Out.deferEffect(struct {
        fn start(env: *const R) Out {
            const attempt = select_attempt(env);
            if (!attempt.identity.valid() or attempt.ordinal == 0 or attempt.attempt == 0)
                return Out.fromOwnedOutcome(env, .{ .fail = .invalid_identity });
            const store = select_journal(env);
            var registering = store.vtable.register(store.userdata, env, attempt.identity);
            var mapped = registering.mapErrorOwned(env, Adapter.journalError);
            return mapped.flatMapOwned(env, Adapter.registered);
        }
    }.start);
}
