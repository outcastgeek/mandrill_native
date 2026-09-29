//! Workflow queue composition over Persistence.PersistedQueue and a typed
//! durable-deferred result. This module owns no second queue or delivery
//! authority. An ambiguous offer remains explicit for caller reconciliation.
const outcomes = @import("../outcome.zig");
const effects = @import("../effect.zig");
const keys = @import("../primary_key.zig");
const queue = @import("../persistence/persisted_queue.zig");
const deferred = @import("durable_deferred.zig");

/// Specialize the typed workflow error channel from its compile-time error type.
/// Parameters:
/// - `QueueError: comptime type`: Compile-time typed queue failure.
/// - `DeferredError: comptime type`: Compile-time typed deferred failure.
/// Returns (`type`): typed durable-queue failure union preserving its provider error.
pub fn Error(comptime QueueError: type, comptime DeferredError: type) type {
    return union(enum) {
        const Self = @This();
        /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements `deinit`
        /// and must be released after use.
        pub const effect_owned_payload = outcomes.ownsPayload(QueueError) or
            outcomes.ownsPayload(DeferredError);
        invalid_identity,
        queue: QueueError,
        deferred: DeferredError,

        /// Release resources owned by this value.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// Returns (`void`): no value.
        pub fn deinit(self: *Self) void {
            switch (self.*) {
                .queue => outcomes.discardPayload(&self.queue),
                .deferred => outcomes.discardPayload(&self.deferred),
                .invalid_identity => {},
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
                .queue => |value| .{ .queue = try outcomes.clonePayload(value) },
                .deferred => |value| .{ .deferred = try outcomes.clonePayload(value) },
                .invalid_identity => .invalid_identity,
            };
        }
    };
}

/// Queue submission result distinguishing accepted work from a duplicate or refusal.
pub const Submission = union(enum) {
    const Self = @This();
    /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements `deinit` and
    /// must be released after use.
    pub const effect_owned_payload = true;
    observed: struct {
        offer: queue.OfferResult,
        result: deferred.Observed,
    },
    possibly_offered,

    /// Release resources owned by this value.
    /// Parameters:
    /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
    /// Returns (`void`): no value.
    pub fn deinit(self: *Self) void {
        if (self.* == .observed) self.observed.result.deinit();
        self.* = undefined;
    }

    /// Duplicate this value; marked owning payloads are cloned, while ordinary
    /// borrowed fields retain their original lifetime.
    /// Parameters:
    /// - `self: *const Self`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
    /// Returns (`error{OutOfMemory}!Self`): copied value on success; `OutOfMemory` if owned storage could not be cloned. The source remains valid.
    pub fn clone(self: *const Self) error{OutOfMemory}!Self {
        return switch (self.*) {
            .observed => |value| .{ .observed = .{ .offer = value.offer, .result = try value.result.clone() } },
            .possibly_offered => .possibly_offered,
        };
    }
};

/// Offer and observe to the selected queue or store.
/// Parameters:
/// - `R: comptime type`: Caller-defined environment type that supplies dependencies and runtime capabilities when work executes.
/// - `QueueProviderError: comptime type`: Compile-time error type returned by the queue provider.
/// - `DeferredProviderError: comptime type`: Compile-time error type returned by the deferred provider.
/// - `queue_guarantee: comptime queue.Guarantee`: Compile-time recoverability promised by the persisted queue.
/// - `deferred_guarantee: comptime deferred.Guarantee`: Compile-time the deferred guarantee input required by this typed
///   operation.
/// - `select_queue: comptime fn (*const R) *const queue.Store(R, QueueProviderError, queue_guarantee)`: Compile-time typed selector from the current environment; its result must
///   remain valid through use.
/// - `select_deferred: comptime fn (*const R) *const deferred.Store(R, DeferredProviderError, deferred_guarantee)`: Compile-time typed selector from the current environment; its result must
///   remain valid through use.
/// - `select_submission: comptime fn (*const R) queue.Submission`: Compile-time typed selector from the current environment; its result
///   must remain valid through use.
/// - `select_identity: comptime fn (*const R) deferred.Identity`: Compile-time typed selector from the current environment; its result must
///   remain valid through use.
/// Returns (`effects.Effect(Submission, Error(queue.Store(R, QueueProviderError, queue_guarantee).Failure, deferred.Store(R, DeferredProviderError, deferred_guarantee).Failure), R)`): durable queue submission and its observed outcome under one stable request identity.
pub fn offerAndObserve(
    comptime R: type,
    comptime QueueProviderError: type,
    comptime DeferredProviderError: type,
    comptime queue_guarantee: queue.Guarantee,
    comptime deferred_guarantee: deferred.Guarantee,
    comptime select_queue: fn (*const R) *const queue.Store(R, QueueProviderError, queue_guarantee),
    comptime select_deferred: fn (*const R) *const deferred.Store(R, DeferredProviderError, deferred_guarantee),
    comptime select_submission: fn (*const R) queue.Submission,
    comptime select_identity: fn (*const R) deferred.Identity,
) effects.Effect(Submission, Error(queue.Store(R, QueueProviderError, queue_guarantee).Failure, deferred.Store(R, DeferredProviderError, deferred_guarantee).Failure), R) {
    const Q = queue.Store(R, QueueProviderError, queue_guarantee);
    const D = deferred.Store(R, DeferredProviderError, deferred_guarantee);
    const Failure = Error(Q.Failure, D.Failure);
    const Out = effects.Effect(Submission, Failure, R);
    const Adapter = struct {
        fn queueError(problem: Q.Failure) Failure {
            return .{ .queue = problem };
        }

        fn deferredError(problem: D.Failure) Failure {
            return .{ .deferred = problem };
        }

        fn offered(identity: *const deferred.Identity, acknowledgement: queue.OfferResult, env: *const R) Out {
            if (acknowledgement == .possibly_offered)
                return Out.fromOwnedSuccess(env, .possibly_offered);
            const store = select_deferred(env);
            var observed = store.vtable.observe(store.userdata, env, identity.*);
            var mapped = observed.mapErrorOwned(env, deferredError);
            return mapped.flatMapCapturedOwned(env, Submission, acknowledgement, struct {
                fn finish(offer: *const queue.OfferResult, result: deferred.Observed, supplied: *const R) Out {
                    return Out.fromOwnedSuccess(supplied, .{ .observed = .{ .offer = offer.*, .result = result } });
                }
            }.finish);
        }
    };
    return Out.deferEffect(struct {
        fn start(env: *const R) Out {
            const submission = select_submission(env);
            const identity = select_identity(env);
            if (keys.inspectEncoded(submission.queue_key) == .err or
                keys.inspectEncoded(submission.offer_key) == .err or !identity.valid())
                return Out.fromOwnedOutcome(env, .{ .fail = .invalid_identity });
            const store = select_queue(env);
            var offered_effect = store.vtable.offer(store.userdata, env, submission);
            var mapped = offered_effect.mapErrorOwned(env, Adapter.queueError);
            return mapped.flatMapCapturedOwned(env, Submission, identity, Adapter.offered);
        }
    }.start);
}
