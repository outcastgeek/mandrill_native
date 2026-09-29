//! A stream of committed transactional versions. The supplied version comes
//! from an atomic initial read; a commit between that read and this stream's
//! first pull is seen by the transactional retry boundary.
const std = @import("std");
const outcomes = @import("../outcome.zig");
const effects = @import("../effect.zig");
const tx = @import("transaction.zig");
const subscriptions = @import("tx_subscription_ref.zig");
const TxError = @import("transaction_error.zig").Error;

pub fn changesAfter(
    comptime T: type,
    comptime R: type,
    comptime select_capture: fn (*const R) *const subscriptions.SubscriptionRef(T).AwaitCapture,
    comptime select_domain: fn (*const R) *tx.Domain,
) effects.Stream(tx.Observed(T), TxError(void), R) {
    const Observed = tx.Observed(T);
    const Failure = TxError(void);
    const Stream = effects.Stream(Observed, Failure, R);
    const Pull = effects.Effect(effects.Chunk(Observed), Failure, R);
    const Unit = effects.Effect(void, Failure, R);
    const State = struct {
        allocator: std.mem.Allocator,
        io: std.Io,
        mutex: std.Io.Mutex = .init,
        reference: *subscriptions.SubscriptionRef(T),
        version: u64,
    };
    const Acquire = effects.Effect(*State, Failure, R);
    const Waiting = struct {
        pub const effect_owned_payload = true;
        allocator: std.mem.Allocator,
        state: *State,
        capture: *subscriptions.SubscriptionRef(T).AwaitCapture,

        pub fn deinit(self: *@This()) void {
            self.allocator.destroy(self.capture);
            self.* = undefined;
        }

        pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
            const copy = try self.allocator.create(subscriptions.SubscriptionRef(T).AwaitCapture);
            copy.* = self.capture.*;
            return .{ .allocator = self.allocator, .state = self.state, .capture = copy };
        }
    };
    const Adapter = struct {
        fn allocate(env: *const R) outcomes.Exit(*State, Failure) {
            const capture = select_capture(env);
            const state = env.allocator.create(State) catch return .{ .die = outcomes.Defect.out_of_memory };
            state.* = .{
                .allocator = env.allocator,
                .io = env.io,
                .reference = capture.reference,
                .version = capture.version,
            };
            return .{ .success = state };
        }

        fn received(waiting: *const Waiting, observed: Observed, env: *const R) Pull {
            const state = waiting.state;
            state.mutex.lockUncancelable(state.io);
            if (observed.version <= state.version) {
                state.mutex.unlock(state.io);
                return pullNow(state, env);
            }
            state.version = observed.version;
            state.mutex.unlock(state.io);
            const values = env.allocator.alloc(Observed, 1) catch return Pull.die(outcomes.Defect.out_of_memory);
            values[0] = observed;
            return Pull.fromOwnedSuccess(env, effects.Chunk(Observed).owned(.{ .items = values, .allocator = env.allocator }));
        }

        fn pullNow(state: *State, env: *const R) Pull {
            state.mutex.lockUncancelable(state.io);
            const version = state.version;
            state.mutex.unlock(state.io);
            const capture = env.allocator.create(subscriptions.SubscriptionRef(T).AwaitCapture) catch
                return Pull.die(outcomes.Defect.out_of_memory);
            capture.* = .{ .reference = state.reference, .version = version };
            const owner: Waiting = .{ .allocator = env.allocator, .state = state, .capture = capture };
            var awaiting = subscriptions.SubscriptionRef(T).awaitAfter(R, capture, select_domain);
            return awaiting.flatMapCapturedOwned(env, effects.Chunk(Observed), owner, received);
        }

        fn pull(state: *State) Pull {
            return Pull.deferBorrowed(State, state, struct {
                fn run(current: *const State, env: *const R) Pull {
                    return pullNow(@constCast(current), env);
                }
            }.run);
        }

        fn release(state: *State) Unit {
            state.allocator.destroy(state);
            return Unit.succeed({});
        }
    };
    return Stream.fromPull(comptime Acquire.fromOutcome(Adapter.allocate), Adapter.pull, Adapter.release);
}
