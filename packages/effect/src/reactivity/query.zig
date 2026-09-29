//! Typed local query observation. Evaluation stays in Effect; the registry
//! supplies only scoped invalidation and never starts a second interpreter.
//! A single scoped permit serializes refreshes, while a dirty bit coalesces
//! changes received during an active evaluation into a required next pass.
const std = @import("std");
const outcomes = @import("../outcome.zig");
const effects = @import("../effect.zig");
const pubsub = @import("../pub_sub.zig");
const semaphore = @import("../semaphore.zig");
const subscription_ref = @import("../subscription_ref.zig");
const invalidation = @import("invalidation_registry.zig");
const async_result = @import("async_result.zig");
const services = @import("../services.zig");
const diagnostics = @import("../internal/diagnostics.zig");

pub fn Error(comptime E: type) type {
    return union(enum) {
        const Self = @This();
        pub const effect_owned_payload = outcomes.ownsPayload(E);
        evaluation: E,
        invalidation: invalidation.Error,
        publication: pubsub.Error,
        admission: semaphore.Error,
        disposed,
        refresh_required,

        pub fn deinit(self: *Self) void {
            if (self.* == .evaluation) outcomes.discardPayload(&self.evaluation);
            self.* = undefined;
        }

        pub fn clone(self: *const Self) error{OutOfMemory}!Self {
            return switch (self.*) {
                .evaluation => |value| .{ .evaluation = try outcomes.clonePayload(value) },
                else => self.*,
            };
        }
    };
}

pub fn State(comptime A: type, comptime E: type) type {
    return struct {
        const Self = @This();
        pub const Result = async_result.AsyncResult(A, E);
        pub const Failure = Error(E);
        const Ref = subscription_ref.SubscriptionRef(Result);
        const single_evaluation_per_query: usize = 1;

        io: std.Io,
        mutex: std.Io.Mutex = .init,
        values: Ref,
        watch: *invalidation.Registry.Watch,
        permit: semaphore.Semaphore,
        dirty: bool = false,
        evaluating_generation: u64 = 0,
        refresh_interval: ?std.Io.Duration = null,
        next_refresh_at: ?std.Io.Timestamp = null,
        closed: bool = false,

        /// A configured interval is a request policy, not a timer. A caller
        /// supplies its clock through R when requesting a refresh; no ambient
        /// clock or background worker is created by the query.
        pub fn initWithRefreshInterval(env: anytype, registry: *invalidation.Registry, key: []const u8, capacity: usize, interval: std.Io.Duration) outcomes.Exit(Self, Failure) {
            if (interval.nanoseconds <= 0) return .{ .die = .{ .kind = .invalid_configuration, .message = diagnostics.query_refresh_interval_must_be_positive } };
            var prepared = init(env, registry, key, capacity);
            if (prepared == .success) prepared.success.refresh_interval = interval;
            return prepared;
        }

        /// Claim an expired refresh once per interval. Reading retained state
        /// never publishes an invalidation, so expiry cannot form a synchronous
        /// notification loop. A forced request bypasses the interval.
        pub fn claimRefresh(self: *Self, clock: services.Clock, force: bool) outcomes.Exit(bool, Failure) {
            const now = clock.now();
            self.mutex.lockUncancelable(self.io);
            defer self.mutex.unlock(self.io);
            if (self.closed) return .{ .fail = .disposed };
            if (self.refresh_interval) |interval| {
                if (!force) {
                    if (self.next_refresh_at) |due| {
                        if (now.nanoseconds < due.nanoseconds) return .{ .success = false };
                    }
                }
                const due = std.math.add(i96, now.nanoseconds, interval.nanoseconds) catch
                    return .{ .die = .{ .kind = .invalid_configuration, .message = diagnostics.query_refresh_deadline_overflowed } };
                self.next_refresh_at = .{ .nanoseconds = due };
            } else if (!force) return .{ .success = false };
            return .{ .success = true };
        }

        /// The watch exists before the initial result is installed. The caller
        /// owns the returned state inside a scope and joins its pump before
        /// deinit; capacity is an explicit observation-retention policy.
        pub fn init(env: anytype, registry: *invalidation.Registry, key: []const u8, capacity: usize) outcomes.Exit(Self, Failure) {
            const watched = registry.watch(key);
            const watch = switch (watched) {
                .success => |value| value,
                .fail => |problem| return .{ .fail = .{ .invalidation = problem } },
                .die => |defect| return .{ .die = defect },
                .interrupted => return .interrupted,
                .finalization_failed => unreachable,
            };
            const prepared = Ref.init(env, .initial, capacity);
            const values = switch (prepared) {
                .success => |value| value,
                .fail => |problem| {
                    watch.deinit();
                    return .{ .fail = .{ .publication = problem } };
                },
                .die => |defect| {
                    watch.deinit();
                    return .{ .die = defect };
                },
                else => unreachable,
            };
            const permit = switch (semaphore.Semaphore.init(env, single_evaluation_per_query)) {
                .ok => |value| value,
                .err => |problem| {
                    var abandoned = values;
                    abandoned.deinit();
                    watch.deinit();
                    return .{ .fail = .{ .admission = problem } };
                },
            };
            return .{ .success = .{
                .io = env.io,
                .values = values,
                .watch = watch,
                .permit = permit,
            } };
        }

        pub fn read(self: *Self) outcomes.Exit(Result, Failure) {
            const observed = self.values.get();
            return switch (observed) {
                .success => |value| .{ .success = value },
                .fail => |problem| .{ .fail = .{ .publication = problem } },
                .die => |defect| .{ .die = defect },
                .interrupted => .interrupted,
                .finalization_failed => unreachable,
            };
        }

        pub fn observe(self: *Self) outcomes.Exit(*Ref.Observation, Failure) {
            const opened = self.values.observe();
            return switch (opened) {
                .success => |value| .{ .success = value },
                .fail => |problem| .{ .fail = .{ .publication = problem } },
                .die => |defect| .{ .die = defect },
                .interrupted => .interrupted,
                .finalization_failed => unreachable,
            };
        }

        fn previous(current: *const Result) error{OutOfMemory}!?A {
            return switch (current.*) {
                .initial => null,
                .waiting => |value| try outcomes.clonePayload(value),
                .success => |value| try outcomes.clonePayload(value),
                .refreshing => |value| try outcomes.clonePayload(value),
                .failure => |value| try outcomes.clonePayload(value.previous),
            };
        }

        fn begin(self: *Self) outcomes.Exit(void, Failure) {
            const observed_generation = self.watch.registry.generation(self.watch.key.items);
            const generation = switch (observed_generation) {
                .success => |value| value,
                .fail => |problem| return .{ .fail = .{ .invalidation = problem } },
                .die => |defect| return .{ .die = defect },
                .interrupted => return .interrupted,
                .finalization_failed => unreachable,
            };
            self.mutex.lockUncancelable(self.io);
            if (self.closed) {
                self.mutex.unlock(self.io);
                return .{ .fail = .disposed };
            }
            self.dirty = false;
            self.evaluating_generation = generation;
            self.mutex.unlock(self.io);
            var observed = self.read();
            const current = switch (observed) {
                .success => |*value| value,
                else => return outcomes.castFailure(void, observed),
            };
            const prior = previous(current) catch {
                observed.deinit();
                return .{ .die = outcomes.Defect.out_of_memory };
            };
            observed.deinit();
            var next: Result = if (prior) |value| .{ .refreshing = value } else .{ .waiting = null };
            defer next.deinit();
            const published = self.values.bus.publishImmediate(next);
            return switch (published) {
                .success => .{ .success = {} },
                .fail => |problem| .{ .fail = .{ .publication = problem } },
                .die => |defect| .{ .die = defect },
                .interrupted => .interrupted,
                .finalization_failed => unreachable,
            };
        }

        /// Streaming adapters register the active generation before the
        /// first event. A newer invalidation makes a later event obsolete.
        pub fn beginStream(self: *Self) outcomes.Exit(void, Failure) {
            return self.begin();
        }

        /// Consumes the evaluator's full Exit, including owning failures.
        /// The returned flag requests one more evaluation after invalidation.
        fn finish(self: *Self, evaluated: outcomes.Exit(A, E)) outcomes.Exit(bool, Failure) {
            const observed_generation = self.watch.registry.generation(self.watch.key.items);
            const generation = switch (observed_generation) {
                .success => |value| value,
                .fail => |problem| {
                    var abandoned = evaluated;
                    abandoned.deinit();
                    return .{ .fail = .{ .invalidation = problem } };
                },
                .die => |defect| {
                    var abandoned = evaluated;
                    abandoned.deinit();
                    return .{ .die = defect };
                },
                .interrupted => {
                    var abandoned = evaluated;
                    abandoned.deinit();
                    return .interrupted;
                },
                .finalization_failed => unreachable,
            };
            self.mutex.lockUncancelable(self.io);
            const obsolete = self.dirty or generation != self.evaluating_generation;
            self.mutex.unlock(self.io);
            if (obsolete) {
                var discarded = evaluated;
                discarded.deinit();
                return .{ .success = true };
            }
            var observed = self.read();
            const current = switch (observed) {
                .success => |*value| value,
                else => {
                    var abandoned = evaluated;
                    abandoned.deinit();
                    return outcomes.castFailure(bool, observed);
                },
            };
            const prior = previous(current) catch {
                observed.deinit();
                var abandoned = evaluated;
                abandoned.deinit();
                return .{ .die = outcomes.Defect.out_of_memory };
            };
            observed.deinit();
            var next = Result.fromExit(evaluated, prior);
            defer next.deinit();
            const published = self.values.bus.publishImmediate(next);
            switch (published) {
                .success => {},
                .fail => |problem| return .{ .fail = .{ .publication = problem } },
                .die => |defect| return .{ .die = defect },
                .interrupted => return .interrupted,
                .finalization_failed => unreachable,
            }
            self.mutex.lockUncancelable(self.io);
            const rerun = self.dirty and !self.closed;
            self.mutex.unlock(self.io);
            if (rerun) return .{ .success = true };
            const latest = self.watch.registry.generation(self.watch.key.items);
            return switch (latest) {
                .success => |value| .{ .success = value != generation },
                .fail => |problem| .{ .fail = .{ .invalidation = problem } },
                .die => |defect| .{ .die = defect },
                .interrupted => .interrupted,
                .finalization_failed => unreachable,
            };
        }

        pub fn acceptStreamEvent(self: *Self, evaluated: outcomes.Exit(A, E)) outcomes.Exit(bool, Failure) {
            return self.finish(evaluated);
        }

        fn markDirty(self: *Self) outcomes.Exit(void, Failure) {
            self.mutex.lockUncancelable(self.io);
            defer self.mutex.unlock(self.io);
            if (self.closed) return .{ .fail = .disposed };
            self.dirty = true;
            return .{ .success = {} };
        }

        pub fn shutdown(self: *Self) void {
            self.mutex.lockUncancelable(self.io);
            self.closed = true;
            self.mutex.unlock(self.io);
            self.permit.shutdown();
            self.values.shutdown();
        }

        pub fn deinit(self: *Self) void {
            self.shutdown();
            self.watch.deinit();
            self.values.deinit();
            self.permit.deinit();
            self.* = undefined;
        }
    };
}

/// A query is a compile-time pairing of its evaluator and selected state.
/// It uses the caller's R for all engines, allocator, I/O and services.
pub fn refresh(
    comptime A: type,
    comptime E: type,
    comptime R: type,
    comptime select_state: fn (*const R) *State(A, E),
    comptime evaluate: fn (*const R) effects.Effect(A, E, R),
) effects.Effect(void, Error(E), R) {
    const Failure = Error(E);
    const Out = effects.Effect(void, Failure, R);
    return semaphore.Semaphore.withPermit(R, void, Failure, struct {
        fn select(env: *const R) *semaphore.Semaphore {
            return &select_state(env).permit;
        }
    }.select, struct {
        fn map(problem: semaphore.Error) Failure {
            return .{ .admission = problem };
        }
    }.map, struct {
        fn use(_: void) Out {
            return cycle(A, E, R, select_state, evaluate);
        }
    }.use);
}

/// Retains the previous result while a claimed refresh runs. The injected
/// clock makes expiry deterministic, and the existing scoped permit makes a
/// refresh waiting behind another evaluation cancellable.
pub fn refreshIfStale(
    comptime A: type,
    comptime E: type,
    comptime R: type,
    comptime select_state: fn (*const R) *State(A, E),
    comptime select_clock: fn (*const R) services.Clock,
    comptime evaluate: fn (*const R) effects.Effect(A, E, R),
    comptime force: bool,
) effects.Effect(void, Error(E), R) {
    const Failure = Error(E);
    const Out = effects.Effect(void, Failure, R);
    return semaphore.Semaphore.withPermit(R, void, Failure, struct {
        fn select(env: *const R) *semaphore.Semaphore {
            return &select_state(env).permit;
        }
    }.select, struct {
        fn map(problem: semaphore.Error) Failure {
            return .{ .admission = problem };
        }
    }.map, struct {
        fn use(_: void) Out {
            return Out.fromOutcome(struct {
                fn claim(env: *const R) outcomes.Exit(bool, Failure) {
                    return select_state(env).claimRefresh(select_clock(env), force);
                }
            }.claim).flatMap(struct {
                fn run(claimed: bool) Out {
                    return if (claimed) cycle(A, E, R, select_state, evaluate) else Out.succeed({});
                }
            }.run);
        }
    }.use);
}

fn cycle(
    comptime A: type,
    comptime E: type,
    comptime R: type,
    comptime select_state: fn (*const R) *State(A, E),
    comptime evaluate: fn (*const R) effects.Effect(A, E, R),
) effects.Effect(void, Error(E), R) {
    const Failure = Error(E);
    const Out = effects.Effect(void, Failure, R);
    const begin = Out.fromOutcome(struct {
        fn run(env: *const R) outcomes.Exit(void, Failure) {
            return select_state(env).begin();
        }
    }.run);
    return begin.flatMap(struct {
        fn run(_: void) Out {
            const source = effects.Effect(A, E, R).deferEffect(struct {
                fn start(env: *const R) effects.Effect(A, E, R) {
                    return evaluate(env);
                }
            }.start);
            return source.exit().mapError(struct {
                fn map(problem: E) Failure {
                    return .{ .evaluation = problem };
                }
            }.map).flatMap(struct {
                fn complete(result: outcomes.Exit(A, E), env: *const R) Out {
                    const finished = select_state(env).finish(result);
                    return switch (finished) {
                        .success => |rerun| if (rerun) cycle(A, E, R, select_state, evaluate) else Out.succeed({}),
                        else => Out.fromOwnedOutcome(env, outcomes.castFailure(void, finished)),
                    };
                }
            }.complete);
        }
    }.run);
}

/// The caller runs this Effect in a scoped fiber and joins it on disposal.
/// Notifications received while evaluating are coalesced by refresh's dirty
/// bit; an event gap forces the same conservative refresh path.
pub fn pump(
    comptime A: type,
    comptime E: type,
    comptime R: type,
    comptime select_state: fn (*const R) *State(A, E),
    comptime evaluate: fn (*const R) effects.Effect(A, E, R),
) effects.Effect(void, Error(E), R) {
    const Failure = Error(E);
    const Out = effects.Effect(void, Failure, R);
    const wait = effects.Effect(invalidation.Notice, invalidation.Error, R).deferEffect(struct {
        fn start(env: *const R) effects.Effect(invalidation.Notice, invalidation.Error, R) {
            return select_state(env).watch.next(R);
        }
    }.start).mapError(struct {
        fn map(problem: invalidation.Error) Failure {
            return .{ .invalidation = problem };
        }
    }.map);
    return wait.flatMap(struct {
        fn observed(_: invalidation.Notice, env: *const R) Out {
            const marked = select_state(env).markDirty();
            return switch (marked) {
                .success => refresh(A, E, R, select_state, evaluate).flatMap(struct {
                    fn repeat(_: void) Out {
                        return pump(A, E, R, select_state, evaluate);
                    }
                }.repeat),
                else => Out.fromOwnedOutcome(env, outcomes.castFailure(void, marked)),
            };
        }
    }.observed);
}
