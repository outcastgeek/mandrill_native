//! Typed handler-to-journal sequencing. A handler runs before append; its
//! external effects are never described as rolled back if append is rejected
//! or ambiguous. Only a committed journal acknowledgement is authoritative.
const outcomes = @import("../outcome.zig");
const effects = @import("../effect.zig");
const keys = @import("../primary_key.zig");
const std = @import("std");
const journal = @import("event_journal.zig");
const journal_error = @import("event_log_error.zig");
const Issue = @import("../schema/schema_issue.zig").Issue;

/// Specialize the eventlog failure variants from their typed channels.
/// Parameters:
/// - `DomainError: comptime type`: Application failure type produced by domain-level operations.
/// - `ProviderError: comptime type`: Failure type reported by the selected provider and retained in the typed error channel.
/// Returns (`type`): typed append failure distinguishing handler and journal errors.
pub fn AppendError(comptime DomainError: type, comptime ProviderError: type) type {
    return union(enum) {
        const Self = @This();
        /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements `deinit`
        /// and must be released after use.
        pub const effect_owned_payload = true;
        encoding: Issue,
        handler: DomainError,
        journal: journal_error.Error(ProviderError),

        /// Release resources owned by this value.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// Returns (`void`): no value.
        pub fn deinit(self: *Self) void {
            switch (self.*) {
                .encoding => self.encoding.deinit(),
                .handler => outcomes.discardPayload(&self.handler),
                .journal => outcomes.discardPayload(&self.journal),
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
                .encoding => |value| .{ .encoding = try value.clone() },
                .handler => |value| .{ .handler = try outcomes.clonePayload(value) },
                .journal => |value| .{ .journal = try outcomes.clonePayload(value) },
            };
        }
    };
}

/// Specialize the typed append result outcome for eventlog operations.
/// Parameters:
/// - `Success: comptime type`: Success payload type represented by the returned component.
/// Returns (`type`): typed append outcome with its committed event identity.
pub fn AppendResult(comptime Success: type) type {
    return union(enum) {
        const Self = @This();
        /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements `deinit`
        /// and must be released after use.
        pub const effect_owned_payload = outcomes.ownsPayload(Success);
        committed: struct { cursor: journal.Cursor, result: Success },
        accepted: Success,
        rejected: Success,
        uncertain: Success,

        /// Release resources owned by this value.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// Returns (`void`): no value.
        pub fn deinit(self: *Self) void {
            switch (self.*) {
                .committed => outcomes.discardPayload(&self.committed.result),
                .accepted => outcomes.discardPayload(&self.accepted),
                .rejected => outcomes.discardPayload(&self.rejected),
                .uncertain => outcomes.discardPayload(&self.uncertain),
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
                .committed => |value| .{ .committed = .{ .cursor = value.cursor, .result = try outcomes.clonePayload(value.result) } },
                .accepted => |value| .{ .accepted = try outcomes.clonePayload(value) },
                .rejected => |value| .{ .rejected = try outcomes.clonePayload(value) },
                .uncertain => |value| .{ .uncertain = try outcomes.clonePayload(value) },
            };
        }
    };
}

fn appendSuccessful(
    comptime Success: type,
    comptime DomainError: type,
    comptime R: type,
    comptime ProviderError: type,
    env: *const R,
    store: *const journal.Journal(R, ProviderError),
    proposal: journal.Proposal,
    successful: Success,
) effects.Effect(AppendResult(Success), AppendError(DomainError, ProviderError), R) {
    const Store = journal.Journal(R, ProviderError);
    const Failure = AppendError(DomainError, ProviderError);
    const Result = AppendResult(Success);
    const Out = effects.Effect(Result, Failure, R);
    var appended = store.vtable.append(store.userdata, env, proposal);
    var mapped = appended.mapErrorOwned(env, struct {
        fn translate(problem: Store.Failure) Failure {
            return .{ .journal = problem };
        }
    }.translate);
    return mapped.flatMapCapturedOwned(env, Result, successful, struct {
        fn finish(result: *const Success, acknowledgement: journal.AppendAcknowledgement, supplied: *const R) Out {
            const copied = outcomes.clonePayload(result.*) catch return Out.die(outcomes.Defect.out_of_memory);
            const acknowledged: Result = switch (acknowledgement) {
                .committed => |cursor| .{ .committed = .{ .cursor = cursor, .result = copied } },
                .accepted => .{ .accepted = copied },
                .rejected => .{ .rejected = copied },
                .uncertain => .{ .uncertain = copied },
            };
            return Out.fromOwnedSuccess(supplied, acknowledged);
        }
    }.finish);
}

/// Handle an event, append its result, and invalidate observers.
/// Parameters:
/// - `handler: comptime anytype`: Compile-time typed callback; its signature is checked at comptime where static.
/// - `R: comptime type`: Caller-defined environment type that supplies dependencies and runtime capabilities when work executes.
/// - `ProviderError: comptime type`: Failure type reported by the selected provider and retained in the typed error channel.
/// - `select_payload: comptime fn (*const R) *const @TypeOf(handler).EventDefinition.Payload`: Compile-time typed selector from the current environment; its result must
///   remain valid through use.
/// - `select_journal: comptime fn (*const R) *const journal.Journal(R, ProviderError)`: Compile-time typed selector from the current environment; its result must
///   remain valid through use.
/// - `select_proposal: comptime fn (*const R) journal.Proposal`: Compile-time typed selector from the current environment; its result must
///   remain valid through use.
/// Returns (`effects.Effect(AppendResult(@TypeOf(handler).EventDefinition.Success), AppendError(@TypeOf(handler).EventDefinition.DomainError, ProviderError), R)`): handler result appended under the event’s stable identity.
pub fn handleAndAppend(
    comptime handler: anytype,
    comptime R: type,
    comptime ProviderError: type,
    comptime select_payload: fn (*const R) *const @TypeOf(handler).EventDefinition.Payload,
    comptime select_journal: fn (*const R) *const journal.Journal(R, ProviderError),
    comptime select_proposal: fn (*const R) journal.Proposal,
) effects.Effect(AppendResult(@TypeOf(handler).EventDefinition.Success), AppendError(@TypeOf(handler).EventDefinition.DomainError, ProviderError), R) {
    const Definition = @TypeOf(handler).EventDefinition;
    const Success = Definition.Success;
    const DomainError = Definition.DomainError;
    const Failure = AppendError(DomainError, ProviderError);
    const Result = AppendResult(Success);
    const Out = effects.Effect(Result, Failure, R);

    const Adapter = struct {
        fn handlerError(problem: DomainError) Failure {
            return .{ .handler = problem };
        }

        fn accepted(value: outcomes.Exit(Success, DomainError), env: *const R) Out {
            const successful = switch (value) {
                .success => |result| result,
                else => return Out.fromOwnedOutcome(env, outcomes.mapError(Failure, outcomes.castFailure(Result, value), env.allocator, handlerError)),
            };
            const proposal = select_proposal(env);
            if (!std.mem.eql(u8, proposal.tag, handler.definition.tag) or
                proposal.schema_version != handler.definition.version or
                keys.inspectEncoded(proposal.event_id) == .err)
            {
                var discarded = successful;
                outcomes.discardPayload(&discarded);
                return Out.fromOwnedOutcome(env, .{ .fail = .{ .journal = .invalid_identity } });
            }
            const store = select_journal(env);
            return appendSuccessful(Success, DomainError, R, ProviderError, env, store, proposal, successful);
        }
    };
    return Out.deferEffect(struct {
        fn start(env: *const R) Out {
            const proposal = select_proposal(env);
            if (!std.mem.eql(u8, proposal.tag, handler.definition.tag) or
                proposal.schema_version != handler.definition.version or
                keys.inspectEncoded(proposal.event_id) == .err)
                return Out.fromOwnedOutcome(env, .{ .fail = .{ .journal = .invalid_identity } });
            var body = handler.handle(select_payload(env), env);
            var observed = body.exitOwned(env);
            var mapped = observed.mapErrorOwned(env, Adapter.handlerError);
            return mapped.flatMapOwned(env, Adapter.accepted);
        }
    }.start);
}

/// Encode the event payload before invoking the handler, then retain its
/// owned bytes through the journal acknowledgement. The adapter receives the
/// definition's checked payload codec and can serialize its representation
/// using the caller's R; no codec or I/O implementation is fixed by core.
///
/// Parameters:
/// - `handler: comptime anytype`: Compile-time typed callback; its signature is checked at comptime where static.
/// - `R: comptime type`: Caller-defined environment type that supplies dependencies and runtime capabilities when work executes.
/// - `ProviderError: comptime type`: Failure type reported by the selected provider and retained in the typed error channel.
/// - `select_payload: comptime fn (*const R) *const @TypeOf(handler).EventDefinition.Payload`: Compile-time typed selector from the current environment; its result must
///   remain valid through use.
/// - `select_journal: comptime fn (*const R) *const journal.Journal(R, ProviderError)`: Compile-time typed selector from the current environment; its result must
///   remain valid through use.
/// - `select_event_id: comptime fn (*const R) []const u8`: Compile-time typed selector from the current environment; its result must
///   remain valid through use.
/// - `encode_payload: comptime fn (*const @TypeOf(handler).EventDefinition.PayloadSchema, *const @TypeOf(handler).EventDefinition.Payload, *const R) effects.Effect(outcomes.OwnedSlice(u8), Issue, R)`: Compile-time typed function selected at comptime; no runtime callback
///   lookup is required.
/// Returns (`effects.Effect(AppendResult(@TypeOf(handler).EventDefinition.Success), AppendError(@TypeOf(handler).EventDefinition.DomainError, ProviderError), R)`): encoded handler result appended under the event’s stable identity.
pub fn handleAndAppendEncoded(
    comptime handler: anytype,
    comptime R: type,
    comptime ProviderError: type,
    comptime select_payload: fn (*const R) *const @TypeOf(handler).EventDefinition.Payload,
    comptime select_journal: fn (*const R) *const journal.Journal(R, ProviderError),
    comptime select_event_id: fn (*const R) []const u8,
    comptime encode_payload: fn (*const @TypeOf(handler).EventDefinition.PayloadSchema, *const @TypeOf(handler).EventDefinition.Payload, *const R) effects.Effect(outcomes.OwnedSlice(u8), Issue, R),
) effects.Effect(AppendResult(@TypeOf(handler).EventDefinition.Success), AppendError(@TypeOf(handler).EventDefinition.DomainError, ProviderError), R) {
    const Definition = @TypeOf(handler).EventDefinition;
    const Success = Definition.Success;
    const DomainError = Definition.DomainError;
    const Failure = AppendError(DomainError, ProviderError);
    const Result = AppendResult(Success);
    const Out = effects.Effect(Result, Failure, R);
    const Adapter = struct {
        fn encodingError(problem: Issue) Failure {
            return .{ .encoding = problem };
        }

        fn handlerError(problem: DomainError) Failure {
            return .{ .handler = problem };
        }

        fn release(bytes: outcomes.OwnedSlice(u8), _: outcomes.PrimaryOutcome(Failure)) effects.Effect(void, Failure, R) {
            var owned = bytes;
            owned.deinit();
            return effects.Effect(void, Failure, R).succeed({});
        }

        fn use(bytes: outcomes.OwnedSlice(u8), env: *const R) Out {
            const event_id = select_event_id(env);
            if (keys.inspectEncoded(event_id) == .err)
                return Out.fromOwnedOutcome(env, .{ .fail = .{ .journal = .invalid_identity } });
            const proposal: journal.Proposal = .{
                .tag = handler.definition.tag,
                .schema_version = handler.definition.version,
                .event_id = event_id,
                .encoded = bytes.items,
            };
            var handler_effect = handler.handle(select_payload(env), env);
            var invoked = handler_effect.exitOwned(env);
            var mapped = invoked.mapErrorOwned(env, handlerError);
            return mapped.flatMapCapturedOwned(env, Result, proposal, accepted);
        }

        fn accepted(proposal: *const journal.Proposal, value: outcomes.Exit(Success, DomainError), env: *const R) Out {
            const successful = switch (value) {
                .success => |result| result,
                else => return Out.fromOwnedOutcome(env, outcomes.mapError(Failure, outcomes.castFailure(Result, value), env.allocator, handlerError)),
            };
            const store = select_journal(env);
            return appendSuccessful(Success, DomainError, R, ProviderError, env, store, proposal.*, successful);
        }
    };
    return Out.deferEffect(struct {
        fn start(env: *const R) Out {
            if (keys.inspectEncoded(select_event_id(env)) == .err)
                return Out.fromOwnedOutcome(env, .{ .fail = .{ .journal = .invalid_identity } });
            var encoded = encode_payload(&handler.definition.payload, select_payload(env), env);
            var mapped = encoded.mapErrorOwned(env, Adapter.encodingError);
            return mapped.acquireReleaseOwned(env, Adapter.use, Adapter.release);
        }
    }.start);
}
