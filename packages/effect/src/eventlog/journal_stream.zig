//! Finite, bounded catch-up over an injected event journal. Notifications may
//! prompt a caller to open another stream, but only journal cursors advance
//! authoritative progress. Concurrent pulls reconcile against the latest
//! cursor instead of holding a lock across an Effectful provider read.
const std = @import("std");
const effects = @import("../effect.zig");
const outcomes = @import("../outcome.zig");
const journals = @import("event_journal.zig");

/// Replay journal entries before following live events.
/// Parameters:
/// - `R: comptime type`: Caller-defined environment type that supplies dependencies and runtime capabilities when work executes.
/// - `ProviderError: comptime type`: Failure type reported by the selected provider and retained in the typed error channel.
/// - `select_journal: comptime fn (*const R) *const journals.Journal(R, ProviderError)`: Compile-time typed selector from the current environment; its result must
///   remain valid through use.
/// - `select_cursor: comptime fn (*const R) journals.Cursor`: Compile-time typed selector from the current environment; its result must
///   remain valid through use.
/// - `select_policy: comptime fn (*const R) journals.ReadPolicy`: Compile-time typed selector from the current environment; its result must
///   remain valid through use.
/// Returns (`effects.Stream(journals.ReadPage, journals.Journal(R, ProviderError).Failure, R)`): A lazy typed Stream; the caller consumes it under its scope and handles terminal Exit.
pub fn catchUp(
    comptime R: type,
    comptime ProviderError: type,
    comptime select_journal: fn (*const R) *const journals.Journal(R, ProviderError),
    comptime select_cursor: fn (*const R) journals.Cursor,
    comptime select_policy: fn (*const R) journals.ReadPolicy,
) effects.Stream(journals.ReadPage, journals.Journal(R, ProviderError).Failure, R) {
    const Store = journals.Journal(R, ProviderError);
    const Failure = Store.Failure;
    const Page = journals.ReadPage;
    const Stream = effects.Stream(Page, Failure, R);
    const Pull = effects.Effect(effects.Chunk(Page), Failure, R);
    const Unit = effects.Effect(void, Failure, R);
    const State = struct {
        allocator: std.mem.Allocator,
        io: std.Io,
        mutex: std.Io.Mutex = .init,
        cursor: journals.Cursor,
        policy: journals.ReadPolicy,
        store: *const Store,
        finished: bool = false,
    };
    const Acquire = effects.Effect(*State, Failure, R);
    const Snapshot = struct { state: *State, cursor: journals.Cursor };
    const Adapter = struct {
        fn allocate(env: *const R) outcomes.Exit(*State, Failure) {
            const policy = select_policy(env);
            if (!policy.valid()) return .{ .fail = .invalid_policy };
            const state = env.allocator.create(State) catch return .{ .die = outcomes.Defect.out_of_memory };
            state.* = .{
                .allocator = env.allocator,
                .io = env.io,
                .cursor = select_cursor(env),
                .policy = policy,
                .store = select_journal(env),
            };
            return .{ .success = state };
        }

        fn valid(page: *const Page, after: journals.Cursor, policy: journals.ReadPolicy) bool {
            if (page.entries.items.len > policy.max_entries or
                (page.entries.items.len == 0 and page.more)) return false;
            var sequence = after.sequence;
            var encoded_bytes: usize = 0;
            for (page.entries.items) |entry| {
                sequence = std.math.add(u64, sequence, 1) catch return false;
                if (entry.cursor.sequence != sequence) return false;
                encoded_bytes = std.math.add(usize, encoded_bytes, entry.encoded.items.len) catch return false;
                if (encoded_bytes > policy.max_encoded_bytes) return false;
            }
            return page.next.sequence == sequence;
        }

        fn accepted(snapshot: *const Snapshot, page: Page, env: *const R) Pull {
            const state = snapshot.state;
            if (!valid(&page, snapshot.cursor, state.policy)) {
                var rejected = page;
                rejected.deinit();
                return Pull.fromOwnedOutcome(env, .{ .fail = .malformed_entry });
            }
            state.mutex.lockUncancelable(state.io);
            if (state.cursor.sequence != snapshot.cursor.sequence or state.finished) {
                state.mutex.unlock(state.io);
                var obsolete = page;
                obsolete.deinit();
                return nextPage(state, env);
            }
            if (page.entries.items.len == 0) {
                state.finished = true;
                state.mutex.unlock(state.io);
                var empty = page;
                empty.deinit();
                return Pull.fromOwnedSuccess(env, .end);
            }
            state.cursor = page.next;
            state.mutex.unlock(state.io);
            const values = env.allocator.alloc(Page, 1) catch {
                var abandoned = page;
                abandoned.deinit();
                return Pull.die(outcomes.Defect.out_of_memory);
            };
            values[0] = page;
            return Pull.fromOwnedSuccess(env, effects.Chunk(Page).owned(.{ .items = values, .allocator = env.allocator }));
        }

        fn nextPage(state: *State, env: *const R) Pull {
            state.mutex.lockUncancelable(state.io);
            if (state.finished) {
                state.mutex.unlock(state.io);
                return Pull.fromOwnedSuccess(env, .end);
            }
            const cursor = state.cursor;
            state.mutex.unlock(state.io);
            var reading = state.store.vtable.read_after(state.store.userdata, env, cursor, state.policy);
            return reading.flatMapCapturedOwned(env, effects.Chunk(Page), Snapshot{ .state = state, .cursor = cursor }, accepted);
        }

        fn pull(state: *State) Pull {
            return Pull.deferBorrowed(State, state, struct {
                fn read(current: *const State, env: *const R) Pull {
                    return nextPage(@constCast(current), env);
                }
            }.read);
        }

        fn release(state: *State) Unit {
            state.allocator.destroy(state);
            return Unit.succeed({});
        }
    };
    return Stream.fromPull(comptime Acquire.fromOutcome(Adapter.allocate), Adapter.pull, Adapter.release);
}
