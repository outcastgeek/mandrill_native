//! Observable local state with an atomic initial-value/subscription boundary.
//! State changes are authoritative only within this scoped process-local ref.
const effects = @import("effect.zig");
const outcomes = @import("outcome.zig");
const pubsub = @import("pub_sub.zig");

/// Specialize a reference that publishes changes to its stored value type.
/// Parameters:
/// - `T: comptime type`: Value type stored or transported by the returned component.
/// Returns (`type`): synchronized reference that publishes committed changes to subscribers.
pub fn SubscriptionRef(comptime T: type) type {
    return struct {
        const Self = @This();
        // One retained publication is the current state; older history is
        // governed by caller capacity and may be reported as an explicit gap.
        const current_value_replay: usize = 1;
        /// Typed PubSub bus used by this subscription reference.
        pub const Bus = pubsub.PubSub(T);
        /// Subscription snapshot type emitted by this reference.
        pub const Observation = Bus.SnapshotSubscription;
        /// PubSub failure type propagated by subscriptions on this reference.
        pub const Error = pubsub.Error;
        bus: Bus,

        /// The current value always occupies the latest ring slot. A slow
        /// subscriber receives an explicit gap, while state publication never
        /// silently drops a successful set operation.
        ///
        /// Parameters:
        /// - `env: anytype`: Borrowed caller environment supplying allocator, I/O, and selected
        ///   capabilities.
        /// - `initial: T`: Initial published value, owned by the created reference.
        /// - `capacity: usize`: explicit publication buffer size for change notices.
        /// Returns (`outcomes.Exit(Self, Error)`): Carries `Self` on `.success` or `Error` on `.fail`; `.die`, `.interrupt`, and `.cleanup` preserve other terminal causes.
        pub fn init(env: anytype, initial: T, capacity: usize) outcomes.Exit(Self, Error) {
            const prepared = Bus.init(env, capacity, current_value_replay, .drop_oldest);
            var bus = switch (prepared) {
                .success => |value| value,
                else => return outcomes.castFailure(Self, prepared),
            };
            const installed = bus.publishImmediate(initial);
            switch (installed) {
                .success => return .{ .success = .{ .bus = bus } },
                else => {
                    bus.shutdown();
                    bus.deinit();
                    return outcomes.castFailure(Self, installed);
                },
            }
        }

        /// Look up the selected value.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// Returns (`outcomes.Exit(T, Error)`): Carries `T` on `.success` or `Error` on `.fail`; `.die`, `.interrupt`, and `.cleanup` preserve other terminal causes.
        pub fn get(self: *Self) outcomes.Exit(T, Error) {
            return self.bus.latest();
        }

        /// Read the current observable state.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// Returns (`outcomes.Exit(*Observation, Error)`): Carries `*Observation` on `.success` or `Error` on `.fail`; `.die`, `.interrupt`, and `.cleanup` preserve other terminal causes.
        pub fn observe(self: *Self) outcomes.Exit(*Observation, Error) {
            return self.bus.snapshotAndSubscribe();
        }

        /// Replace the current value.
        /// Parameters:
        /// - `R: comptime type`: Caller-defined environment type supplying the dependencies needed by this operation.
        /// - `select_ref: comptime fn (*const R) *Self`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_value: comptime fn (*const R) T`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// Returns (`effects.Effect(pubsub.PublishResult, Error, R)`): unit-valued committed update that also publishes a change notice.
        pub fn set(comptime R: type, comptime select_ref: fn (*const R) *Self, comptime select_value: fn (*const R) T) effects.Effect(pubsub.PublishResult, Error, R) {
            return Bus.publish(R, struct {
                fn select(env: *const R) *Bus {
                    return &select_ref(env).bus;
                }
            }.select, select_value);
        }

        /// Run a body while observing subscription changes.
        /// Parameters:
        /// - `R: comptime type`: Caller-defined environment type supplying the dependencies needed by this operation.
        /// - `A: comptime type`: Value type produced by a successful computation.
        /// - `E: comptime type`: Expected application-failure type; defects and interruption remain separate outcomes.
        /// - `select_ref: comptime fn (*const R) *Self`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `map_error: comptime fn (Error) E`: Compile-time typed function selected at comptime; no runtime callback
        ///   lookup is required.
        /// - `use: comptime fn (*Observation) effects.Effect(A, E, R)`: Compile-time typed function selected at comptime; no runtime callback lookup
        ///   is required.
        /// Returns (`effects.Effect(A, E, R)`): body result while observing this reference under a subscription scope.
        pub fn withObservation(
            comptime R: type,
            comptime A: type,
            comptime E: type,
            comptime select_ref: fn (*const R) *Self,
            comptime map_error: fn (Error) E,
            comptime use: fn (*Observation) effects.Effect(A, E, R),
        ) effects.Effect(A, E, R) {
            const Acquire = effects.Effect(*Observation, Error, R);
            const acquire = Acquire.fromOutcome(struct {
                fn run(env: *const R) outcomes.Exit(*Observation, Error) {
                    return select_ref(env).observe();
                }
            }.run).mapError(map_error);
            return acquire.acquireRelease(use, struct {
                fn finish(observation: *Observation, _: outcomes.PrimaryOutcome(E)) effects.Effect(void, E, R) {
                    observation.deinit();
                    return effects.Effect(void, E, R).succeed({});
                }
            }.finish);
        }

        /// Stop the capability and release its retained resources.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// Returns (`void`): no value.
        pub fn shutdown(self: *Self) void {
            self.bus.shutdown();
        }

        /// Release resources owned by this value.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// Returns (`void`): no value.
        pub fn deinit(self: *Self) void {
            self.bus.shutdown();
            self.bus.deinit();
            self.* = undefined;
        }
    };
}
