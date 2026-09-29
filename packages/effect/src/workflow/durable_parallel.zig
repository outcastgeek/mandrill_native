//! Stable branch identities and recorded durable fan-out/race decisions.
//! Worker execution remains with the caller's Effect parallel capability;
//! this journal only supplies replay authority for completed branches.
const std = @import("std");
const effects = @import("../effect.zig");
const outcomes = @import("../outcome.zig");
const types = @import("workflow_types.zig");
const schedules = @import("../schedule.zig");

/// Recorded identity and definition of one durable parallel branch.
pub const Branch = struct {
    execution: types.ExecutionId,
    name: []const u8,
    version: u32,

    /// Check whether this value satisfies its structural invariants.
    /// Parameters:
    /// - `self: Branch`: Receiver passed by value: the struct itself is copied, while any referenced storage keeps its original ownership and lifetime.
    /// Returns (`bool`): `true` for valid execution, nonempty UTF-8 name, and positive version.
    pub fn valid(self: Branch) bool {
        return self.execution.validate() == .ok and self.name.len > 0 and
            std.unicode.utf8ValidateSlice(self.name) and self.version > 0;
    }
};

/// Caller-selected bounds and failure behavior for durable parallel work.
pub const Policy = struct {
    concurrency: usize,
    max_attempts: u64,

    /// Check whether this value satisfies its structural invariants.
    /// Parameters:
    /// - `self: Policy`: Receiver passed by value: the struct itself is copied, while any referenced storage keeps its original ownership and lifetime.
    /// Returns (`bool`): `true` when concurrency and maximum attempts are both positive.
    pub fn valid(self: Policy) bool {
        return self.concurrency > 0 and self.max_attempts > 0;
    }
};

/// Stored terminal outcome of a branch.
pub const BranchResult = struct {
    /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements `deinit` and
    /// must be released after use.
    pub const effect_owned_payload = true;
    encoded_exit: types.Bytes,
    attempt: u64,

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
        return .{ .encoded_exit = try self.encoded_exit.clone(), .attempt = self.attempt };
    }
};

/// Durably selected race winner and its completed outcome.
pub const Winner = struct {
    /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements `deinit` and
    /// must be released after use.
    pub const effect_owned_payload = true;
    branch_name: types.Bytes,
    branch_version: u32,
    encoded_exit: types.Bytes,

    /// Release resources owned by this value.
    /// Parameters:
    /// - `self: *@This()`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
    /// Returns (`void`): no value.
    pub fn deinit(self: *@This()) void {
        self.branch_name.deinit();
        self.encoded_exit.deinit();
        self.* = undefined;
    }

    /// Duplicate this value; marked owning payloads are cloned, while ordinary
    /// borrowed fields retain their original lifetime.
    /// Parameters:
    /// - `self: *const @This()`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
    /// Returns (`error{OutOfMemory}!@This()`): copied value on success; `OutOfMemory` if owned storage could not be cloned. The source remains valid.
    pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
        var branch_name = try self.branch_name.clone();
        errdefer branch_name.deinit();
        return .{ .branch_name = branch_name, .branch_version = self.branch_version, .encoded_exit = try self.encoded_exit.clone() };
    }
};

/// `Commit` cases: `recorded`, `already_recorded`, `conflicting_result`, `stale_attempt`,
/// `possibly_recorded`.
pub const Commit = enum { recorded, already_recorded, conflicting_result, stale_attempt, possibly_recorded };
/// Durable branch/race failure from invalid identity, malformed result, unavailable provider, or conflicting winner.
pub const Error = union(enum) { invalid_branch, malformed_exit, unavailable, race_conflict };
/// Storage or provider durability promised by this capability. `recoverable`
/// permits compositions that must survive process restart; `volatile` does not.
pub const Guarantee = enum { @"volatile", recoverable };

/// The winning branch and its full Exit are committed atomically. A race may
/// be reported complete only after an acknowledged or read-back commit.
///
/// Parameters:
/// - `R: comptime type`: Caller-defined environment type that supplies dependencies and runtime capabilities when work executes.
/// - `ProviderError: comptime type`: Failure type reported by the selected provider and retained in the typed error channel.
/// - `guarantee: comptime Guarantee`: Compile-time volatility or recoverability promised by this provider; durable compositions check it before compiling.
/// Returns (`type`): provider contract for durable parallel branch and race records.
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
        /// Lazy Effect for lookup branch effect; success, error, and environment types are fixed by
        /// `effects.Effect(?BranchResult, Failure, R)`.
        pub const LookupBranchEffect = effects.Effect(?BranchResult, Failure, R);
        /// Lazy Effect for lookup winner effect; success, error, and environment types are fixed by
        /// `effects.Effect(?Winner, Failure, R)`.
        pub const LookupWinnerEffect = effects.Effect(?Winner, Failure, R);
        /// Lazy Effect for commit effect; success, error, and environment types are fixed by
        /// `effects.Effect(Commit, Failure, R)`.
        pub const CommitEffect = effects.Effect(Commit, Failure, R);

        userdata: ?*anyopaque,
        vtable: *const VTable,

        /// Typed function-pointer interface for this capability. Implement every
        /// hook with the exact signature below; keep the table valid while the
        /// capability or pending work can invoke it.
        pub const VTable = struct {
            /// Lookup branch through this provider.
            /// Parameters (callback order):
            /// - `1: ?*anyopaque`: opaque provider or handle context; keep it live until the hook and any retained work complete.
            /// - `2: *const R`: borrowed generic environment `R`, including caller-supplied runtime services.
            /// - `3: Branch`: typed domain value consumed by this hook.
            /// Returns (`LookupBranchEffect`): lazy typed Effect; run it with its declared environment to obtain Exit.
            lookup_branch: *const fn (?*anyopaque, *const R, Branch) LookupBranchEffect,
            /// Commit branch through this provider.
            /// Parameters (callback order):
            /// - `1: ?*anyopaque`: opaque provider or handle context; keep it live until the hook and any retained work complete.
            /// - `2: *const R`: borrowed generic environment `R`, including caller-supplied runtime services.
            /// - `3: Branch`: typed domain value consumed by this hook.
            /// - `4: u64`: typed counter or optional sequence value.
            /// - `5: []const u8`: borrowed input slice; keep it valid through deferred use or copy before retaining it.
            /// Returns (`CommitEffect`): lazy typed Effect; run it with its declared environment to obtain Exit.
            commit_branch: *const fn (?*anyopaque, *const R, Branch, u64, []const u8) CommitEffect,
            /// Lookup winner through this provider.
            /// Parameters (callback order):
            /// - `1: ?*anyopaque`: opaque provider or handle context; keep it live until the hook and any retained work complete.
            /// - `2: *const R`: borrowed generic environment `R`, including caller-supplied runtime services.
            /// - `3: types.ExecutionId`: stable typed identity for the selected operation.
            /// - `4: []const u8`: borrowed input slice; keep it valid through deferred use or copy before retaining it.
            /// Returns (`LookupWinnerEffect`): lazy typed Effect; run it with its declared environment to obtain Exit.
            lookup_winner: *const fn (?*anyopaque, *const R, types.ExecutionId, []const u8) LookupWinnerEffect,
            /// Commit winner through this provider.
            /// Parameters (callback order):
            /// - `1: ?*anyopaque`: opaque provider or handle context; keep it live until the hook and any retained work complete.
            /// - `2: *const R`: borrowed generic environment `R`, including caller-supplied runtime services.
            /// - `3: types.ExecutionId`: stable typed identity for the selected operation.
            /// - `4: []const u8`: borrowed input slice; keep it valid through deferred use or copy before retaining it.
            /// - `5: Branch`: typed domain value consumed by this hook.
            /// - `6: []const u8`: borrowed input slice; keep it valid through deferred use or copy before retaining it.
            /// Returns (`CommitEffect`): lazy typed Effect; run it with its declared environment to obtain Exit.
            commit_winner: *const fn (?*anyopaque, *const R, types.ExecutionId, []const u8, Branch, []const u8) CommitEffect,
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
    };
}

/// Native fan-out goes through the existing caller-supplied parallel engine.
/// Durability is a separate record/replay step through Journal above.
///
/// Parameters:
/// - `A: comptime type`: Value type produced by a successful computation.
/// - `E: comptime type`: Expected application-failure type; defects and interruption remain separate outcomes.
/// - `R: comptime type`: Caller-defined environment type that supplies dependencies and runtime capabilities when work executes.
/// - `policy: Policy`: Caller-selected, validated behavior or resource bounds; no hidden default is
///   installed.
/// - `programs: []const effects.Effect(A, E, R)`: Typed Effect descriptions to compose or run.
/// Returns (`effects.Effect(outcomes.OwnedSlice(A), E, R)`): durably recorded results of bounded parallel branches.
pub fn fanOut(
    comptime A: type,
    comptime E: type,
    comptime R: type,
    policy: Policy,
    programs: []const effects.Effect(A, E, R),
) effects.Effect(outcomes.OwnedSlice(A), E, R) {
    if (!policy.valid()) return effects.Effect(outcomes.OwnedSlice(A), E, R).die(.{
        .kind = .invalid_configuration,
        .message = "durable fan-out requires positive concurrency and attempts",
    });
    return effects.Effect(A, E, R).allConcurrent(policy.concurrency, programs);
}

/// A statically described branch may retry its typed failures under the
/// caller's injected Clock. The first execution counts as one attempt;
/// successful branch replay still comes from runRecorded's journal.
///
/// Parameters:
/// - `A: comptime type`: Value type produced by a successful computation.
/// - `E: comptime type`: Expected application-failure type; defects and interruption remain separate outcomes.
/// - `R: comptime type`: Caller-defined environment type that supplies dependencies and runtime capabilities when work executes.
/// - `policy: comptime Policy`: Compile-time caller-selected, validated behavior or resource bounds; no hidden
///   default is installed.
/// - `program: comptime effects.Effect(A, E, R)`: Lazy typed program to sequence, run, or transform.
/// Returns (`effects.Effect(A, E, R)`): typed outcome of retrying one recorded failed branch.
pub fn retryBranch(comptime A: type, comptime E: type, comptime R: type, comptime policy: Policy, comptime program: effects.Effect(A, E, R)) effects.Effect(A, E, R) {
    if (!policy.valid()) @compileError("durable branch retry policy must have positive concurrency and attempts");
    if (policy.max_attempts == 1) return program;
    const retries = std.math.cast(usize, policy.max_attempts - 1) orelse
        @compileError("durable branch retry count exceeds target usize");
    return program.retry(schedules.recurs(E, retries));
}

/// Specialize the workflow failure variants from their typed channels.
/// Parameters:
/// - `JournalError: comptime type`: Compile-time typed journal failure.
/// - `CodecError: comptime type`: Failure type returned by the selected encoder or decoder.
/// Returns (`type`): typed failure from durable parallel execution.
pub fn RunError(comptime JournalError: type, comptime CodecError: type) type {
    return union(enum) {
        const Self = @This();
        /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements `deinit`
        /// and must be released after use.
        pub const effect_owned_payload = outcomes.ownsPayload(JournalError) or
            outcomes.ownsPayload(CodecError);
        journal: JournalError,
        codec: CodecError,
        invalid_branch,
        conflicting_result,
        stale_attempt,
        uncertain_commit,

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

/// Specialize the typed recorded outcome for workflow operations.
/// Parameters:
/// - `A: comptime type`: Value type produced by a successful computation.
/// - `E: comptime type`: Expected application-failure type; defects and interruption remain separate outcomes.
/// Returns (`type`): recorded parallel branch outcome with replay identity.
pub fn Recorded(comptime A: type, comptime E: type) type {
    return struct {
        const Self = @This();
        /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements `deinit`
        /// and must be released after use.
        pub const effect_owned_payload = true;
        source: enum { executed, replayed },
        exit: outcomes.Exit(A, E),

        /// Release resources owned by this value.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// Returns (`void`): no value.
        pub fn deinit(self: *Self) void {
            self.exit.deinit();
            self.* = undefined;
        }

        /// Duplicate this value; marked owning payloads are cloned, while ordinary
        /// borrowed fields retain their original lifetime.
        /// Parameters:
        /// - `self: *const Self`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
        /// Returns (`error{OutOfMemory}!Self`): copied value on success; `OutOfMemory` if owned storage could not be cloned. The source remains valid.
        pub fn clone(self: *const Self) error{OutOfMemory}!Self {
            const copy = self.exit.clone();
            if (copy == .die and copy.die.kind == .out_of_memory and
                !(self.exit == .die and self.exit.die.kind == .out_of_memory))
                return error.OutOfMemory;
            return .{ .source = self.source, .exit = copy };
        }
    };
}

/// Replays a completed branch or executes it once and records its full Exit
/// before publishing. The codec must return an independently owned decoded
/// Exit; its representation/version policy is supplied by the caller. An
/// ambiguous commit remains typed uncertainty for explicit read-back.
///
/// Parameters:
/// - `A: comptime type`: Value type produced by a successful computation.
/// - `E: comptime type`: Expected application-failure type; defects and interruption remain separate outcomes.
/// - `R: comptime type`: Caller-defined environment type that supplies dependencies and runtime capabilities when work executes.
/// - `ProviderError: comptime type`: Failure type reported by the selected provider and retained in the typed error channel.
/// - `CodecError: comptime type`: Failure type returned by the selected encoder or decoder.
/// - `guarantee: comptime Guarantee`: Compile-time volatility or recoverability promised by this provider; durable compositions check it before compiling.
/// - `program: comptime effects.Effect(A, E, R)`: Lazy typed program to sequence, run, or transform.
/// - `select_journal: comptime fn (*const R) *const Journal(R, ProviderError, guarantee)`: Compile-time typed selector from the current environment; its result must
///   remain valid through use.
/// - `select_branch: comptime fn (*const R) Branch`: Compile-time typed selector from the current environment; its result must
///   remain valid through use.
/// - `select_attempt: comptime fn (*const R) u64`: Compile-time typed selector from the current environment; its result must
///   remain valid through use.
/// - `encode_exit: comptime fn (*const outcomes.Exit(A, E), *const R) outcomes.Exit(outcomes.OwnedSlice(u8), CodecError)`: Compile-time typed function selected at comptime; no runtime callback lookup
///   is required.
/// - `decode_exit: comptime fn ([]const u8, *const R) outcomes.Exit(outcomes.Exit(A, E), CodecError)`: Compile-time typed function selected at comptime; no runtime callback lookup
///   is required.
/// Returns (`effects.Effect(Recorded(A, E), RunError(Journal(R, ProviderError, guarantee).Failure, CodecError), R)`): recorded or newly executed parallel branch outcome.
pub fn runRecorded(
    comptime A: type,
    comptime E: type,
    comptime R: type,
    comptime ProviderError: type,
    comptime CodecError: type,
    comptime guarantee: Guarantee,
    comptime program: effects.Effect(A, E, R),
    comptime select_journal: fn (*const R) *const Journal(R, ProviderError, guarantee),
    comptime select_branch: fn (*const R) Branch,
    comptime select_attempt: fn (*const R) u64,
    comptime encode_exit: fn (*const outcomes.Exit(A, E), *const R) outcomes.Exit(outcomes.OwnedSlice(u8), CodecError),
    comptime decode_exit: fn ([]const u8, *const R) outcomes.Exit(outcomes.Exit(A, E), CodecError),
) effects.Effect(Recorded(A, E), RunError(Journal(R, ProviderError, guarantee).Failure, CodecError), R) {
    const Store = Journal(R, ProviderError, guarantee);
    const Failure = RunError(Store.Failure, CodecError);
    const Out = effects.Effect(Recorded(A, E), Failure, R);
    const Encoded = outcomes.OwnedSlice(u8);
    const Capture = struct {
        /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements `deinit`
        /// and must be released after use.
        pub const effect_owned_payload = true;
        outcome: outcomes.Exit(A, E),
        encoded: Encoded,

        /// Release resources owned by this value.
        /// Parameters:
        /// - `self: *@This()`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// Returns (`void`): no value.
        pub fn deinit(self: *@This()) void {
            self.outcome.deinit();
            self.encoded.deinit();
            self.* = undefined;
        }

        /// Duplicate this value; marked owning payloads are cloned, while ordinary
        /// borrowed fields retain their original lifetime.
        /// Parameters:
        /// - `self: *const @This()`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
        /// Returns (`error{OutOfMemory}!@This()`): copied value on success; `OutOfMemory` if owned storage could not be cloned. The source remains valid.
        pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
            const outcome = self.outcome.clone();
            if (outcome == .die and outcome.die.kind == .out_of_memory and
                !(self.outcome == .die and self.outcome.die.kind == .out_of_memory))
                return error.OutOfMemory;
            var owned = outcome;
            errdefer owned.deinit();
            return .{ .outcome = owned, .encoded = try self.encoded.clone() };
        }
    };
    const Adapter = struct {
        fn journalError(problem: Store.Failure) Failure {
            return .{ .journal = problem };
        }
        fn codecError(problem: CodecError) Failure {
            return .{ .codec = problem };
        }
        fn impossible(_: void) Failure {
            unreachable;
        }

        fn replay(record: BranchResult, env: *const R) Out {
            var owned = record;
            const decoded = decode_exit(owned.encoded_exit.items, env);
            owned.deinit();
            const mapped = outcomes.mapError(Failure, decoded, env.allocator, codecError);
            return switch (mapped) {
                .success => |exit| Out.fromOwnedSuccess(env, .{ .source = .replayed, .exit = exit }),
                else => Out.fromOwnedOutcome(env, outcomes.castFailure(Recorded(A, E), mapped)),
            };
        }

        fn committed(captured: *const Capture, status: Commit, env: *const R) Out {
            return switch (status) {
                .recorded, .already_recorded => blk: {
                    const copy = captured.outcome.clone();
                    if (copy == .die and copy.die.kind == .out_of_memory and
                        !(captured.outcome == .die and captured.outcome.die.kind == .out_of_memory))
                        break :blk Out.die(outcomes.Defect.out_of_memory);
                    break :blk Out.fromOwnedSuccess(env, .{ .source = .executed, .exit = copy });
                },
                .conflicting_result => Out.fail(.conflicting_result),
                .stale_attempt => Out.fail(.stale_attempt),
                .possibly_recorded => Out.fail(.uncertain_commit),
            };
        }

        fn executed(value: outcomes.Exit(A, E), env: *const R) Out {
            var owned = value;
            const encoded_result = encode_exit(&owned, env);
            const encoded = switch (encoded_result) {
                .success => |bytes| bytes,
                else => {
                    owned.deinit();
                    const mapped = outcomes.mapError(Failure, outcomes.castFailure(Encoded, encoded_result), env.allocator, codecError);
                    return Out.fromOwnedOutcome(env, outcomes.castFailure(Recorded(A, E), mapped));
                },
            };
            const capture: Capture = .{ .outcome = owned, .encoded = encoded };
            const branch = select_branch(env);
            const store = select_journal(env);
            var committing = store.vtable.commit_branch(store.userdata, env, branch, select_attempt(env), capture.encoded.items);
            var mapped = committing.mapErrorOwned(env, journalError);
            return mapped.flatMapCapturedOwned(env, Recorded(A, E), capture, committed);
        }

        fn loaded(record: ?BranchResult, env: *const R) Out {
            if (record) |existing| return replay(existing, env);
            var running = program;
            var settled = running.materializeOwned(env);
            var aligned = settled.mapErrorOwned(env, impossible);
            return aligned.flatMapOwned(env, executed);
        }
    };
    return Out.deferEffect(struct {
        fn start(env: *const R) Out {
            const branch = select_branch(env);
            if (!branch.valid() or select_attempt(env) == 0)
                return Out.fail(.invalid_branch);
            const store = select_journal(env);
            var reading = store.vtable.lookup_branch(store.userdata, env, branch);
            var mapped = reading.mapErrorOwned(env, Adapter.journalError);
            return mapped.flatMapOwned(env, Adapter.loaded);
        }
    }.start);
}

/// Specialize the workflow failure variants from their typed channels.
/// Parameters:
/// - `BranchError: comptime type`: Compile-time typed branch failure.
/// - `JournalError: comptime type`: Compile-time typed journal failure.
/// - `CodecError: comptime type`: Failure type returned by the selected encoder or decoder.
/// Returns (`type`): typed failure while arbitrating a durable race.
pub fn RaceError(comptime BranchError: type, comptime JournalError: type, comptime CodecError: type) type {
    return union(enum) {
        const Self = @This();
        /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements `deinit`
        /// and must be released after use.
        pub const effect_owned_payload = outcomes.ownsPayload(BranchError) or
            outcomes.ownsPayload(JournalError) or outcomes.ownsPayload(CodecError);
        branch: BranchError,
        journal: JournalError,
        codec: CodecError,
        invalid_race,
        conflicting_winner,
        uncertain_commit,

        /// Release resources owned by this value.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// Returns (`void`): no value.
        pub fn deinit(self: *Self) void {
            switch (self.*) {
                .branch => outcomes.discardPayload(&self.branch),
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
                .branch => |value| .{ .branch = try outcomes.clonePayload(value) },
                .journal => |value| .{ .journal = try outcomes.clonePayload(value) },
                .codec => |value| .{ .codec = try outcomes.clonePayload(value) },
                else => self.*,
            };
        }
    };
}

/// Specialize the typed race outcome outcome for workflow operations.
/// Parameters:
/// - `A: comptime type`: Value type produced by a successful computation.
/// - `E: comptime type`: Expected application-failure type; defects and interruption remain separate outcomes.
/// Returns (`type`): durable race result retaining the winner and its terminal `Exit`.
pub fn RaceOutcome(comptime A: type, comptime E: type) type {
    return struct {
        const Self = @This();
        /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements `deinit`
        /// and must be released after use.
        pub const effect_owned_payload = true;
        source: enum { executed, replayed },
        branch_name: outcomes.OwnedSlice(u8),
        branch_version: u32,
        exit: outcomes.Exit(A, E),

        /// Release resources owned by this value.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// Returns (`void`): no value.
        pub fn deinit(self: *Self) void {
            self.branch_name.deinit();
            self.exit.deinit();
            self.* = undefined;
        }

        /// Duplicate this value; marked owning payloads are cloned, while ordinary
        /// borrowed fields retain their original lifetime.
        /// Parameters:
        /// - `self: *const Self`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
        /// Returns (`error{OutOfMemory}!Self`): copied value on success; `OutOfMemory` if owned storage could not be cloned. The source remains valid.
        pub fn clone(self: *const Self) error{OutOfMemory}!Self {
            var name = try self.branch_name.clone();
            errdefer name.deinit();
            const exit = self.exit.clone();
            if (exit == .die and exit.die.kind == .out_of_memory and
                !(self.exit == .die and self.exit.die.kind == .out_of_memory))
                return error.OutOfMemory;
            return .{ .source = self.source, .branch_name = name, .branch_version = self.branch_version, .exit = exit };
        }
    };
}

/// Both branches are checked against the branch journal. The first completed
/// branch then competes for one immutable winner record. The full Exit is
/// returned only after the winner commit is acknowledged. Heterogeneous
/// errors require the caller's explicit common union or mapping before race.
///
/// Parameters:
/// - `A: comptime type`: Value type produced by a successful computation.
/// - `E: comptime type`: Expected application-failure type; defects and interruption remain separate outcomes.
/// - `R: comptime type`: Caller-defined environment type that supplies dependencies and runtime capabilities when work executes.
/// - `ProviderError: comptime type`: Failure type reported by the selected provider and retained in the typed error channel.
/// - `CodecError: comptime type`: Failure type returned by the selected encoder or decoder.
/// - `guarantee: comptime Guarantee`: Compile-time volatility or recoverability promised by this provider; durable compositions check it before compiling.
/// - `left: comptime effects.Effect(A, E, R)`: Left-hand typed description in this composition.
/// - `right: comptime effects.Effect(A, E, R)`: Right-hand typed description in this composition.
/// - `select_journal: comptime fn (*const R) *const Journal(R, ProviderError, guarantee)`: Compile-time typed selector from the current environment; its result must
///   remain valid through use.
/// - `select_execution: comptime fn (*const R) types.ExecutionId`: Compile-time typed selector from the current environment; its result must
///   remain valid through use.
/// - `select_race_name: comptime fn (*const R) []const u8`: Compile-time typed selector from the current environment; its result must
///   remain valid through use.
/// - `select_left: comptime fn (*const R) Branch`: Compile-time typed selector from the current environment; its result must
///   remain valid through use.
/// - `select_right: comptime fn (*const R) Branch`: Compile-time typed selector from the current environment; its result must
///   remain valid through use.
/// - `select_attempt: comptime fn (*const R) u64`: Compile-time typed selector from the current environment; its result must
///   remain valid through use.
/// - `encode_exit: comptime fn (*const outcomes.Exit(A, E), *const R) outcomes.Exit(outcomes.OwnedSlice(u8), CodecError)`: Compile-time typed function selected at comptime; no runtime callback lookup
///   is required.
/// - `decode_exit: comptime fn ([]const u8, *const R) outcomes.Exit(outcomes.Exit(A, E), CodecError)`: Compile-time typed function selected at comptime; no runtime callback lookup
///   is required.
/// Returns (`effects.Effect(RaceOutcome(A, E), RaceError(RunError(Journal(R, ProviderError, guarantee).Failure, CodecError), Journal(R, ProviderError, guarantee).Failure, CodecError), R)`): durably selected winner and its full terminal `Exit` after losers are drained.
pub fn durableRace(
    comptime A: type,
    comptime E: type,
    comptime R: type,
    comptime ProviderError: type,
    comptime CodecError: type,
    comptime guarantee: Guarantee,
    comptime left: effects.Effect(A, E, R),
    comptime right: effects.Effect(A, E, R),
    comptime select_journal: fn (*const R) *const Journal(R, ProviderError, guarantee),
    comptime select_execution: fn (*const R) types.ExecutionId,
    comptime select_race_name: fn (*const R) []const u8,
    comptime select_left: fn (*const R) Branch,
    comptime select_right: fn (*const R) Branch,
    comptime select_attempt: fn (*const R) u64,
    comptime encode_exit: fn (*const outcomes.Exit(A, E), *const R) outcomes.Exit(outcomes.OwnedSlice(u8), CodecError),
    comptime decode_exit: fn ([]const u8, *const R) outcomes.Exit(outcomes.Exit(A, E), CodecError),
) effects.Effect(RaceOutcome(A, E), RaceError(RunError(Journal(R, ProviderError, guarantee).Failure, CodecError), Journal(R, ProviderError, guarantee).Failure, CodecError), R) {
    const Store = Journal(R, ProviderError, guarantee);
    const BranchFailure = RunError(Store.Failure, CodecError);
    const Failure = RaceError(BranchFailure, Store.Failure, CodecError);
    const Out = effects.Effect(RaceOutcome(A, E), Failure, R);
    const Encoded = outcomes.OwnedSlice(u8);
    const Tagged = struct {
        const Self = @This();
        /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements `deinit`
        /// and must be released after use.
        pub const effect_owned_payload = true;
        side: enum { left, right },
        recorded: Recorded(A, E),

        /// Release resources owned by this value.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// Returns (`void`): no value.
        pub fn deinit(self: *Self) void {
            self.recorded.deinit();
            self.* = undefined;
        }

        /// Duplicate this value; marked owning payloads are cloned, while ordinary
        /// borrowed fields retain their original lifetime.
        /// Parameters:
        /// - `self: *const Self`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
        /// Returns (`error{OutOfMemory}!Self`): copied value on success; `OutOfMemory` if owned storage could not be cloned. The source remains valid.
        pub fn clone(self: *const Self) error{OutOfMemory}!Self {
            return .{ .side = self.side, .recorded = try self.recorded.clone() };
        }
    };
    const Capture = struct {
        const Self = @This();
        /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements `deinit`
        /// and must be released after use.
        pub const effect_owned_payload = true;
        tagged: Tagged,
        encoded: Encoded,

        /// Release resources owned by this value.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// Returns (`void`): no value.
        pub fn deinit(self: *Self) void {
            self.tagged.deinit();
            self.encoded.deinit();
            self.* = undefined;
        }

        /// Duplicate this value; marked owning payloads are cloned, while ordinary
        /// borrowed fields retain their original lifetime.
        /// Parameters:
        /// - `self: *const Self`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
        /// Returns (`error{OutOfMemory}!Self`): copied value on success; `OutOfMemory` if owned storage could not be cloned. The source remains valid.
        pub fn clone(self: *const Self) error{OutOfMemory}!Self {
            var tagged = try self.tagged.clone();
            errdefer tagged.deinit();
            return .{ .tagged = tagged, .encoded = try self.encoded.clone() };
        }
    };
    const left_program = comptime runRecorded(A, E, R, ProviderError, CodecError, guarantee, left, select_journal, select_left, select_attempt, encode_exit, decode_exit);
    const right_program = comptime runRecorded(A, E, R, ProviderError, CodecError, guarantee, right, select_journal, select_right, select_attempt, encode_exit, decode_exit);
    const left_tagged = comptime left_program.map(struct {
        fn tag(recorded: Recorded(A, E)) Tagged {
            return .{ .side = .left, .recorded = recorded };
        }
    }.tag);
    const right_tagged = comptime right_program.map(struct {
        fn tag(recorded: Recorded(A, E)) Tagged {
            return .{ .side = .right, .recorded = recorded };
        }
    }.tag);
    const competing = comptime left_tagged.race(right_tagged);
    const Adapter = struct {
        fn journalError(problem: Store.Failure) Failure {
            return .{ .journal = problem };
        }
        fn branchError(problem: BranchFailure) Failure {
            return .{ .branch = problem };
        }
        fn codecError(problem: CodecError) Failure {
            return .{ .codec = problem };
        }

        fn replay(winner: Winner, env: *const R) Out {
            var owned = winner;
            const left_branch = select_left(env);
            const right_branch = select_right(env);
            const matches_left = owned.branch_version == left_branch.version and
                std.mem.eql(u8, owned.branch_name.items, left_branch.name);
            const matches_right = owned.branch_version == right_branch.version and
                std.mem.eql(u8, owned.branch_name.items, right_branch.name);
            if (!matches_left and !matches_right) {
                owned.deinit();
                return Out.fail(.conflicting_winner);
            }
            const decoded = decode_exit(owned.encoded_exit.items, env);
            const mapped = outcomes.mapError(Failure, decoded, env.allocator, codecError);
            if (mapped != .success) {
                owned.deinit();
                return Out.fromOwnedOutcome(env, outcomes.castFailure(RaceOutcome(A, E), mapped));
            }
            const branch_name = owned.branch_name;
            owned.encoded_exit.deinit();
            return Out.fromOwnedSuccess(env, .{ .source = .replayed, .branch_name = branch_name, .branch_version = owned.branch_version, .exit = mapped.success });
        }

        fn committed(capture: *const Capture, status: Commit, env: *const R) Out {
            return switch (status) {
                .recorded, .already_recorded => blk: {
                    const branch = if (capture.tagged.side == .left)
                        select_left(env)
                    else
                        select_right(env);
                    const name = env.allocator.dupe(u8, branch.name) catch
                        break :blk Out.die(outcomes.Defect.out_of_memory);
                    const exit = capture.tagged.recorded.exit.clone();
                    if (exit == .die and exit.die.kind == .out_of_memory and
                        !(capture.tagged.recorded.exit == .die and
                            capture.tagged.recorded.exit.die.kind == .out_of_memory))
                    {
                        env.allocator.free(name);
                        break :blk Out.die(outcomes.Defect.out_of_memory);
                    }
                    break :blk Out.fromOwnedSuccess(env, .{
                        .source = .executed,
                        .branch_name = .{ .items = name, .allocator = env.allocator },
                        .branch_version = branch.version,
                        .exit = exit,
                    });
                },
                .conflicting_result => Out.fail(.conflicting_winner),
                .possibly_recorded => Out.fail(.uncertain_commit),
                .stale_attempt => Out.fail(.conflicting_winner),
            };
        }

        fn competed(tagged: Tagged, env: *const R) Out {
            var owned = tagged;
            const encoded_result = encode_exit(&owned.recorded.exit, env);
            const encoded = switch (encoded_result) {
                .success => |bytes| bytes,
                else => {
                    owned.deinit();
                    const mapped = outcomes.mapError(Failure, outcomes.castFailure(Encoded, encoded_result), env.allocator, codecError);
                    return Out.fromOwnedOutcome(env, outcomes.castFailure(RaceOutcome(A, E), mapped));
                },
            };
            const capture: Capture = .{ .tagged = owned, .encoded = encoded };
            const branch = if (capture.tagged.side == .left)
                select_left(env)
            else
                select_right(env);
            const store = select_journal(env);
            var writing = store.vtable.commit_winner(store.userdata, env, select_execution(env), select_race_name(env), branch, capture.encoded.items);
            var mapped = writing.mapErrorOwned(env, journalError);
            return mapped.flatMapCapturedOwned(env, RaceOutcome(A, E), capture, committed);
        }

        fn loaded(existing: ?Winner, env: *const R) Out {
            if (existing) |winner| return replay(winner, env);
            var racing = competing;
            var mapped = racing.mapErrorOwned(env, branchError);
            return mapped.flatMapOwned(env, competed);
        }
    };
    return Out.deferEffect(struct {
        fn start(env: *const R) Out {
            const execution = select_execution(env);
            const race_name = select_race_name(env);
            const left_branch = select_left(env);
            const right_branch = select_right(env);
            if (execution.validate() == .err or race_name.len == 0 or
                !std.unicode.utf8ValidateSlice(race_name) or
                !left_branch.valid() or !right_branch.valid() or
                !std.mem.eql(u8, left_branch.execution.encoded, execution.encoded) or
                !std.mem.eql(u8, right_branch.execution.encoded, execution.encoded) or
                (left_branch.version == right_branch.version and
                    std.mem.eql(u8, left_branch.name, right_branch.name)) or
                select_attempt(env) == 0) return Out.fail(.invalid_race);
            const store = select_journal(env);
            var finding = store.vtable.lookup_winner(store.userdata, env, execution, race_name);
            var mapped = finding.mapErrorOwned(env, Adapter.journalError);
            return mapped.flatMapOwned(env, Adapter.loaded);
        }
    }.start);
}
