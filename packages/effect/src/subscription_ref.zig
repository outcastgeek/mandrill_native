//! Observable local state with an atomic initial-value/subscription boundary.
//! State changes are authoritative only within this scoped process-local ref.
const effects = @import("effect.zig");
const outcomes = @import("outcome.zig");
const pubsub = @import("pub_sub.zig");

pub fn SubscriptionRef(comptime T: type) type {
    return struct {
        const Self = @This();
        // One retained publication is the current state; older history is
        // governed by caller capacity and may be reported as an explicit gap.
        const current_value_replay: usize = 1;
        pub const Bus = pubsub.PubSub(T);
        pub const Observation = Bus.SnapshotSubscription;
        pub const Error = pubsub.Error;
        bus: Bus,

        /// The current value always occupies the latest ring slot. A slow
        /// subscriber receives an explicit gap, while state publication never
        /// silently drops a successful set operation.
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

        pub fn get(self: *Self) outcomes.Exit(T, Error) {
            return self.bus.latest();
        }

        pub fn observe(self: *Self) outcomes.Exit(*Observation, Error) {
            return self.bus.snapshotAndSubscribe();
        }

        pub fn set(comptime R: type, comptime select_ref: fn (*const R) *Self, comptime select_value: fn (*const R) T) effects.Effect(pubsub.PublishResult, Error, R) {
            return Bus.publish(R, struct {
                fn select(env: *const R) *Bus {
                    return &select_ref(env).bus;
                }
            }.select, select_value);
        }

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

        pub fn shutdown(self: *Self) void {
            self.bus.shutdown();
        }

        pub fn deinit(self: *Self) void {
            self.bus.shutdown();
            self.bus.deinit();
            self.* = undefined;
        }
    };
}
