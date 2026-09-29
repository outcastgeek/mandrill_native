//! Durable compensation metadata and journal boundary. A compensation is a
//! recorded workflow action, never an automatic rollback of an external side
//! effect. Uncertain recordings require lookup before another attempt.
const std = @import("std");
const effects = @import("../effect.zig");
const outcomes = @import("../outcome.zig");
const types = @import("workflow_types.zig");

/// Stable identity for one workflow compensation sequence.
pub const Identity = struct {
    execution: types.ExecutionId,
    name: []const u8,
    version: u32,

    /// Check whether this value satisfies its structural invariants.
    /// Parameters:
    /// - `self: Identity`: Receiver passed by value: the struct itself is copied, while any referenced storage keeps its original ownership and lifetime.
    /// Returns (`bool`): `true` for valid execution, nonempty UTF-8 name, and positive version.
    pub fn valid(self: Identity) bool {
        return self.execution.validate() == .ok and self.name.len > 0 and
            std.unicode.utf8ValidateSlice(self.name) and self.version > 0;
    }
};

/// One recorded attempt to execute a compensation step.
pub const Attempt = struct {
    identity: Identity,
    ordinal: u64,
    attempt: u64,

    /// Check whether this value satisfies its structural invariants.
    /// Parameters:
    /// - `self: Attempt`: Receiver passed by value: the struct itself is copied, while any referenced storage keeps its original ownership and lifetime.
    /// Returns (`bool`): `true` for valid identity and positive ordinal and attempt.
    pub fn valid(self: Attempt) bool {
        return self.identity.valid() and self.ordinal > 0 and self.attempt > 0;
    }
};

/// Stored compensation definition and its progress.
pub const Recorded = struct {
    /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements `deinit` and
    /// must be released after use.
    pub const effect_owned_payload = true;
    ordinal: u64,
    attempt: u64,
    encoded_exit: types.Bytes,

    /// Release resources owned by this value.
    /// Parameters:
    /// - `self: *@This()`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
    /// Returns (`void`): no value.
    pub fn deinit(self: *@This()) void {
        self.encoded_exit.deinit();
        self.* = undefined;
    }

    /// Duplicate this value; marked owning payloads are cloned, while ordinary
    /// borrowed fields retain their original lifetime.
    /// Parameters:
    /// - `self: *const @This()`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
    /// Returns (`error{OutOfMemory}!@This()`): copied value on success; `OutOfMemory` if owned storage could not be cloned. The source remains valid.
    pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
        return .{ .ordinal = self.ordinal, .attempt = self.attempt, .encoded_exit = try self.encoded_exit.clone() };
    }
};

/// Compensation step awaiting execution or resolution.
pub const Pending = struct {
    /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements `deinit` and
    /// must be released after use.
    pub const effect_owned_payload = true;
    name: types.Bytes,
    version: u32,
    ordinal: u64,
    attempt: u64,

    /// Release resources owned by this value.
    /// Parameters:
    /// - `self: *@This()`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
    /// Returns (`void`): no value.
    pub fn deinit(self: *@This()) void {
        self.name.deinit();
        self.* = undefined;
    }

    /// Duplicate this value; marked owning payloads are cloned, while ordinary
    /// borrowed fields retain their original lifetime.
    /// Parameters:
    /// - `self: *const @This()`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
    /// Returns (`error{OutOfMemory}!@This()`): copied value on success; `OutOfMemory` if owned storage could not be cloned. The source remains valid.
    pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
        return .{ .name = try self.name.clone(), .version = self.version, .ordinal = self.ordinal, .attempt = self.attempt };
    }
};

/// Bounded ordered page of pending compensations.
pub const Page = struct {
    /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements `deinit` and
    /// must be released after use.
    pub const effect_owned_payload = true;
    items: outcomes.OwnedSlice(Pending),
    more: bool,

    /// Release resources owned by this value.
    /// Parameters:
    /// - `self: *@This()`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
    /// Returns (`void`): no value.
    pub fn deinit(self: *@This()) void {
        self.items.deinit();
        self.* = undefined;
    }

    /// Duplicate this value; marked owning payloads are cloned, while ordinary
    /// borrowed fields retain their original lifetime.
    /// Parameters:
    /// - `self: *const @This()`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
    /// Returns (`error{OutOfMemory}!@This()`): copied value on success; `OutOfMemory` if owned storage could not be cloned. The source remains valid.
    pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
        return .{ .items = try self.items.clone(), .more = self.more };
    }
};

/// Result of registering or deduplicating a compensation step.
pub const Registration = union(enum) {
    registered: u64,
    already_registered: u64,
    conflicting_definition,
    possibly_registered,
};

/// `Recording` cases: `recorded`, `already_recorded`, `conflicting_outcome`, `stale_attempt`,
/// `possibly_recorded`.
pub const Recording = enum {
    recorded,
    already_recorded,
    conflicting_outcome,
    stale_attempt,
    possibly_recorded,
};

/// Typed compensation registration, execution, or resolution failure.
pub const Error = union(enum) {
    invalid_identity,
    malformed_record,
    unavailable,
    ordinal_exhausted,
    stale_attempt,
};

/// Storage or provider durability promised by this capability. `recoverable`
/// permits compositions that must survive process restart; `volatile` does not.
pub const Guarantee = enum { @"volatile", recoverable };

/// `register` atomically allocates a per-execution ordinal, deduplicated by
/// identity. `pendingReverse` returns highest ordinals first; its cursor is
/// exclusive. `record` fences the attempt and stores a complete encoded Exit.
/// A provider must never report success after an uncertain write.
///
/// Parameters:
/// - `R: comptime type`: Caller-defined environment type that supplies dependencies and runtime capabilities when work executes.
/// - `ProviderError: comptime type`: Failure type reported by the selected provider and retained in the typed error channel.
/// - `guarantee: comptime Guarantee`: Compile-time volatility or recoverability promised by this provider; durable compositions check it before compiling.
/// Returns (`type`): provider contract for registering, claiming, and resolving durable compensation steps.
pub fn Journal(comptime R: type, comptime ProviderError: type, comptime guarantee: Guarantee) type {
    return struct {
        const Self = @This();
        /// Compile-time volatility or recoverability promised by this provider.
        pub const storage_guarantee = guarantee;
        /// Typed failure channel combining domain and provider errors.
        pub const Failure = union(enum) {
            const Own = @This();
            /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements
            /// `deinit` and must be released after use.
            pub const effect_owned_payload = outcomes.ownsPayload(ProviderError);
            operation: Error,
            provider: ProviderError,

            /// Release resources owned by this value.
            /// Parameters:
            /// - `self: *Own`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
            /// Returns (`void`): no value.
            pub fn deinit(self: *Own) void {
                if (self.* == .provider) outcomes.discardPayload(&self.provider);
                self.* = undefined;
            }

            /// Duplicate this value; marked owning payloads are cloned, while ordinary
            /// borrowed fields retain their original lifetime.
            /// Parameters:
            /// - `self: *const Own`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
            /// Returns (`error{OutOfMemory}!Own`): A `Own` on success; `OutOfMemory` if required allocation fails.
            pub fn clone(self: *const Own) error{OutOfMemory}!Own {
                return switch (self.*) {
                    .provider => |value| .{ .provider = try outcomes.clonePayload(value) },
                    else => self.*,
                };
            }
        };
        /// Lazy Effect for register effect; success, error, and environment types are fixed by
        /// `effects.Effect(Registration, Failure, R)`.
        pub const RegisterEffect = effects.Effect(Registration, Failure, R);
        /// Lazy Effect for lookup effect; success, error, and environment types are fixed by
        /// `effects.Effect(?Recorded, Failure, R)`.
        pub const LookupEffect = effects.Effect(?Recorded, Failure, R);
        /// Lazy Effect for record effect; success, error, and environment types are fixed by
        /// `effects.Effect(Recording, Failure, R)`.
        pub const RecordEffect = effects.Effect(Recording, Failure, R);
        /// Lazy Effect for pending effect; success, error, and environment types are fixed by
        /// `effects.Effect(Page, Failure, R)`.
        pub const PendingEffect = effects.Effect(Page, Failure, R);

        userdata: ?*anyopaque,
        vtable: *const VTable,

        /// Typed function-pointer interface for this capability. Implement every
        /// hook with the exact signature below; keep the table valid while the
        /// capability or pending work can invoke it.
        pub const VTable = struct {
            /// Register through this provider.
            /// Parameters (callback order):
            /// - `1: ?*anyopaque`: opaque provider or handle context; keep it live until the hook and any retained work complete.
            /// - `2: *const R`: borrowed generic environment `R`, including caller-supplied runtime services.
            /// - `3: Identity`: stable typed identity for the selected operation.
            /// Returns (`RegisterEffect`): lazy typed Effect; run it with its declared environment to obtain Exit.
            register: *const fn (?*anyopaque, *const R, Identity) RegisterEffect,
            /// Lookup through this provider.
            /// Parameters (callback order):
            /// - `1: ?*anyopaque`: opaque provider or handle context; keep it live until the hook and any retained work complete.
            /// - `2: *const R`: borrowed generic environment `R`, including caller-supplied runtime services.
            /// - `3: Attempt`: typed domain value consumed by this hook.
            /// Returns (`LookupEffect`): lazy typed Effect; run it with its declared environment to obtain Exit.
            lookup: *const fn (?*anyopaque, *const R, Attempt) LookupEffect,
            /// Record the supplied measurement.
            /// Parameters (callback order):
            /// - `1: ?*anyopaque`: opaque provider or handle context; keep it live until the hook and any retained work complete.
            /// - `2: *const R`: borrowed generic environment `R`, including caller-supplied runtime services.
            /// - `3: Attempt`: typed domain value consumed by this hook.
            /// - `4: []const u8`: borrowed input slice; keep it valid through deferred use or copy before retaining it.
            /// Returns (`RecordEffect`): lazy typed Effect; run it with its declared environment to obtain Exit.
            record: *const fn (?*anyopaque, *const R, Attempt, []const u8) RecordEffect,
            /// Pending reverse through this provider.
            /// Parameters (callback order):
            /// - `1: ?*anyopaque`: opaque provider or handle context; keep it live until the hook and any retained work complete.
            /// - `2: *const R`: borrowed generic environment `R`, including caller-supplied runtime services.
            /// - `3: types.ExecutionId`: stable typed identity for the selected operation.
            /// - `4: ?u64`: typed counter or optional sequence value.
            /// - `5: usize`: caller-selected count or bound.
            /// Returns (`PendingEffect`): lazy typed Effect; run it with its declared environment to obtain Exit.
            pending_reverse: *const fn (?*anyopaque, *const R, types.ExecutionId, ?u64, usize) PendingEffect,
        };

        /// Wrap a caller-supplied provider implementation.
        /// Parameters:
        /// - `data: ?*anyopaque`: Borrowed provider context passed unchanged to every hook.
        ///   Its owner keeps it alive through any pending effect or operation.
        /// - `vtable: *const VTable`: Borrowed table of exactly typed callbacks; keep it
        ///   alive for every use of this capability, including deferred execution.
        /// Returns (`Self`): A non-owning adapter referencing `data` and `vtable`; no
        ///   provider state is allocated or transferred by this constructor.
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

        /// Register a definition or subscription with its owner.
        /// Parameters:
        /// - `select_journal: comptime fn (*const R) *const Self`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// - `select_identity: comptime fn (*const R) Identity`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// Returns (`RegisterEffect`): durable compensation registration or matching previously registered step.
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

        /// Look up a stored value using its stable identity.
        /// Parameters:
        /// - `select_journal: comptime fn (*const R) *const Self`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// - `select_attempt: comptime fn (*const R) Attempt`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// Returns (`LookupEffect`): recorded compensation step for the supplied execution and ordinal.
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

        /// Record the selected metric or durable result.
        /// Parameters:
        /// - `select_journal: comptime fn (*const R) *const Self`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// - `select_attempt: comptime fn (*const R) Attempt`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// - `select_encoded_exit: comptime fn (*const R) []const u8`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// Returns (`RecordEffect`): typed acknowledgement after recording a compensation result.
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

        /// Read pending compensations in reverse execution order.
        /// Parameters:
        /// - `select_journal: comptime fn (*const R) *const Self`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// - `select_execution: comptime fn (*const R) types.ExecutionId`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// - `select_before: comptime fn (*const R) ?u64`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_limit: comptime fn (*const R) usize`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// Returns (`PendingEffect`): bounded page of pending compensations in reverse execution order.
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
///
/// Parameters:
/// - `OriginalSuccess: comptime type`: Compile-time success type before the transformation.
/// - `OriginalError: comptime type`: Compile-time error type before the transformation.
/// - `CompensationError: comptime type`: Compile-time typed compensation failure.
/// Returns (`type`): record of an application-defined compensation outcome.
pub fn Resolution(comptime OriginalSuccess: type, comptime OriginalError: type, comptime CompensationError: type) type {
    return struct {
        /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements `deinit`
        /// and must be released after use.
        pub const effect_owned_payload = true;
        original: outcomes.Exit(OriginalSuccess, OriginalError),
        compensation: outcomes.Exit(void, CompensationError),

        /// Release resources owned by this value.
        /// Parameters:
        /// - `self: *@This()`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// Returns (`void`): no value.
        pub fn deinit(self: *@This()) void {
            self.original.deinit();
            self.compensation.deinit();
            self.* = undefined;
        }

        /// Duplicate this value; marked owning payloads are cloned, while ordinary
        /// borrowed fields retain their original lifetime.
        /// Parameters:
        /// - `self: *const @This()`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
        /// Returns (`error{OutOfMemory}!@This()`): copied value on success; `OutOfMemory` if owned storage could not be cloned. The source remains valid.
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

/// Specialize the workflow failure variants from their typed channels.
/// Parameters:
/// - `JournalError: comptime type`: Compile-time typed journal failure.
/// - `CodecError: comptime type`: Failure type returned by the selected encoder or decoder.
/// Returns (`type`): typed failure when running a compensation step.
pub fn RunError(comptime JournalError: type, comptime CodecError: type) type {
    return union(enum) {
        const Self = @This();
        /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements `deinit`
        /// and must be released after use.
        pub const effect_owned_payload = outcomes.ownsPayload(JournalError) or outcomes.ownsPayload(CodecError);
        journal: JournalError,
        codec: CodecError,
        invalid_identity,
        conflicting_outcome,
        stale_attempt,
        uncertain_record,

        /// Release resources owned by this value.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// Returns (`void`): no value.
        pub fn deinit(self: *Self) void {
            switch (self.*) {
                .journal => outcomes.discardPayload(&self.journal),
                .codec => outcomes.discardPayload(&self.codec),
                else => {},
            }
            self.* = undefined;
        }

        /// Duplicate this value; marked owning payloads are cloned, while ordinary
        /// borrowed fields retain their original lifetime.
        /// Parameters:
        /// - `self: *const Self`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
        /// Returns (`error{OutOfMemory}!Self`): copied value on success; `OutOfMemory` if owned storage could not be cloned. The source remains valid.
        pub fn clone(self: *const Self) error{OutOfMemory}!Self {
            return switch (self.*) {
                .journal => |value| .{ .journal = try outcomes.clonePayload(value) },
                .codec => |value| .{ .codec = try outcomes.clonePayload(value) },
                else => self.*,
            };
        }
    };
}

/// Specialize the typed completed outcome for workflow operations.
/// Parameters:
/// - `E: comptime type`: Expected application-failure type; defects and interruption remain separate outcomes.
/// Returns (`type`): completed compensation outcome retaining its typed result.
pub fn Completed(comptime E: type) type {
    return struct {
        /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements `deinit`
        /// and must be released after use.
        pub const effect_owned_payload = true;
        source: enum { executed, replayed },
        exit: outcomes.Exit(void, E),

        /// Release resources owned by this value.
        /// Parameters:
        /// - `self: *@This()`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// Returns (`void`): no value.
        pub fn deinit(self: *@This()) void {
            self.exit.deinit();
            self.* = undefined;
        }

        /// Duplicate this value; marked owning payloads are cloned, while ordinary
        /// borrowed fields retain their original lifetime.
        /// Parameters:
        /// - `self: *const @This()`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
        /// Returns (`error{OutOfMemory}!@This()`): copied value on success; `OutOfMemory` if owned storage could not be cloned. The source remains valid.
        pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
            const copy = self.exit.clone();
            if (copy == .die and copy.die.kind == .out_of_memory and
                !(self.exit == .die and self.exit.die.kind == .out_of_memory))
                return error.OutOfMemory;
            return .{ .source = self.source, .exit = copy };
        }
    };
}

/// Specialize a recorded compensation step for reverse replay.
/// Parameters:
/// - `E: comptime type`: Expected application-failure type; defects and interruption remain separate outcomes.
/// Returns (`type`): recorded compensation waiting for reverse-order replay.
pub fn PendingStep(comptime E: type) type {
    return union(enum) {
        const Self = @This();
        /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements `deinit`
        /// and must be released after use.
        pub const effect_owned_payload = true;
        none,
        completed: struct { ordinal: u64, outcome: Completed(E) },

        /// Release resources owned by this value.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// Returns (`void`): no value.
        pub fn deinit(self: *Self) void {
            if (self.* == .completed) self.completed.outcome.deinit();
            self.* = undefined;
        }

        /// Duplicate this value; marked owning payloads are cloned, while ordinary
        /// borrowed fields retain their original lifetime.
        /// Parameters:
        /// - `self: *const Self`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
        /// Returns (`error{OutOfMemory}!Self`): copied value on success; `OutOfMemory` if owned storage could not be cloned. The source remains valid.
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
///
/// Parameters:
/// - `E: comptime type`: Expected application-failure type; defects and interruption remain separate outcomes.
/// - `R: comptime type`: Caller-defined environment type that supplies dependencies and runtime capabilities when work executes.
/// - `ProviderError: comptime type`: Failure type reported by the selected provider and retained in the typed error channel.
/// - `CodecError: comptime type`: Failure type returned by the selected encoder or decoder.
/// - `guarantee: comptime Guarantee`: Compile-time volatility or recoverability promised by this provider; durable compositions check it before compiling.
/// - `select_journal: comptime fn (*const R) *const Journal(R, ProviderError, guarantee)`: Compile-time typed selector from the current environment; its result must
///   remain valid through use.
/// - `select_execution: comptime fn (*const R) types.ExecutionId`: Compile-time typed selector from the current environment; its result must
///   remain valid through use.
/// - `dispatch: comptime fn (*const Pending, *const R) effects.Effect(Completed(E), RunError(Journal(R, ProviderError, guarantee).Failure, CodecError), R)`: Compile-time typed function selected at comptime; no runtime callback lookup is
///   required.
/// Returns (`effects.Effect(PendingStep(E), RunError(Journal(R, ProviderError, guarantee).Failure, CodecError), R)`): result of executing the next pending compensation step.
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
///
/// Parameters:
/// - `E: comptime type`: Expected application-failure type; defects and interruption remain separate outcomes.
/// - `R: comptime type`: Caller-defined environment type that supplies dependencies and runtime capabilities when work executes.
/// - `ProviderError: comptime type`: Failure type reported by the selected provider and retained in the typed error channel.
/// - `CodecError: comptime type`: Failure type returned by the selected encoder or decoder.
/// - `guarantee: comptime Guarantee`: Compile-time volatility or recoverability promised by this provider; durable compositions check it before compiling.
/// - `select_journal: comptime fn (*const R) *const Journal(R, ProviderError, guarantee)`: Compile-time typed selector from the current environment; its result must
///   remain valid through use.
/// - `select_execution: comptime fn (*const R) types.ExecutionId`: Compile-time typed selector from the current environment; its result must
///   remain valid through use.
/// - `dispatch: comptime fn (*const Pending, *const R) effects.Effect(Completed(E), RunError(Journal(R, ProviderError, guarantee).Failure, CodecError), R)`: Compile-time typed function selected at comptime; no runtime callback lookup is
///   required.
/// Returns (`effects.Effect(outcomes.Exit(void, E), RunError(Journal(R, ProviderError, guarantee).Failure, CodecError), R)`): A lazy Effect; running it with its declared environment yields a typed Exit.
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
///
/// Parameters:
/// - `E: comptime type`: Expected application-failure type; defects and interruption remain separate outcomes.
/// - `R: comptime type`: Caller-defined environment type that supplies dependencies and runtime capabilities when work executes.
/// - `ProviderError: comptime type`: Failure type reported by the selected provider and retained in the typed error channel.
/// - `CodecError: comptime type`: Failure type returned by the selected encoder or decoder.
/// - `guarantee: comptime Guarantee`: Compile-time volatility or recoverability promised by this provider; durable compositions check it before compiling.
/// - `program: comptime effects.Effect(void, E, R)`: Lazy typed program to sequence, run, or transform.
/// - `select_journal: comptime fn (*const R) *const Journal(R, ProviderError, guarantee)`: Compile-time typed selector from the current environment; its result must
///   remain valid through use.
/// - `select_attempt: comptime fn (*const R) Attempt`: Compile-time typed selector from the current environment; its result must
///   remain valid through use.
/// - `encode_exit: comptime fn (*const outcomes.Exit(void, E), *const R) outcomes.Exit(outcomes.OwnedSlice(u8), CodecError)`: Compile-time typed function selected at comptime; no runtime callback lookup
///   is required.
/// - `decode_exit: comptime fn ([]const u8, *const R) outcomes.Exit(outcomes.Exit(void, E), CodecError)`: Compile-time typed function selected at comptime; no runtime callback lookup
///   is required.
/// Returns (`effects.Effect(Completed(E), RunError(Journal(R, ProviderError, guarantee).Failure, CodecError), R)`): recorded or newly executed compensation outcome.
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
        /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements `deinit`
        /// and must be released after use.
        pub const effect_owned_payload = true;
        exit: outcomes.Exit(void, E),
        encoded: Encoded,

        /// Release resources owned by this value.
        /// Parameters:
        /// - `self: *@This()`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// Returns (`void`): no value.
        pub fn deinit(self: *@This()) void {
            self.exit.deinit();
            self.encoded.deinit();
            self.* = undefined;
        }

        /// Duplicate this value; marked owning payloads are cloned, while ordinary
        /// borrowed fields retain their original lifetime.
        /// Parameters:
        /// - `self: *const @This()`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
        /// Returns (`error{OutOfMemory}!@This()`): copied value on success; `OutOfMemory` if owned storage could not be cloned. The source remains valid.
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
