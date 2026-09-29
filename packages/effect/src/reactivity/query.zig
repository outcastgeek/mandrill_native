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

/// Specialize the typed reactivity error channel from its compile-time error type.
/// Parameters:
/// - `E: comptime type`: Expected application-failure type; defects and interruption remain separate outcomes.
/// Returns (`type`): typed query failure union retaining provider and domain errors.
pub fn Error(comptime E: type) type {
    return union(enum) {
        const Self = @This();
        /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements `deinit`
        /// and must be released after use.
        pub const effect_owned_payload = outcomes.ownsPayload(E);
        evaluation: E,
        invalidation: invalidation.Error,
        publication: pubsub.Error,
        admission: semaphore.Error,
        disposed,
        refresh_required,

        /// Release resources owned by this value.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// Returns (`void`): no value.
        pub fn deinit(self: *Self) void {
            if (self.* == .evaluation) outcomes.discardPayload(&self.evaluation);
            self.* = undefined;
        }

        /// Duplicate this value; marked owning payloads are cloned, while ordinary
        /// borrowed fields retain their original lifetime.
        /// Parameters:
        /// - `self: *const Self`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
        /// Returns (`error{OutOfMemory}!Self`): copied value on success; `OutOfMemory` if owned storage could not be cloned. The source remains valid.
        pub fn clone(self: *const Self) error{OutOfMemory}!Self {
            return switch (self.*) {
                .evaluation => |value| .{ .evaluation = try outcomes.clonePayload(value) },
                else => self.*,
            };
        }
    };
}

/// Specialize the typed state outcome for reactivity operations.
/// Parameters:
/// - `A: comptime type`: Value type produced by a successful computation.
/// - `E: comptime type`: Expected application-failure type; defects and interruption remain separate outcomes.
/// Returns (`type`): observable query state distinguishing pending, success, and failure.
pub fn State(comptime A: type, comptime E: type) type {
    return struct {
        const Self = @This();
        /// Typed result channel described by `async_result.AsyncResult(A, E)`.
        pub const Result = async_result.AsyncResult(A, E);
        /// Typed failure channel combining domain and provider errors.
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
        ///
        /// Parameters:
        /// - `env: anytype`: Borrowed caller environment supplying allocator, I/O, and selected
        ///   capabilities.
        /// - `registry: *invalidation.Registry`: Caller-supplied capability or scoped state; no concrete backend is
        ///   selected here.
        /// - `key: []const u8`: Stable identity used for matching, deduplication, or replay.
        /// - `capacity: usize`: buffer capacity for observable query changes.
        /// - `interval: std.Io.Duration`: Requested interval between scheduled operations.
        /// Returns (`outcomes.Exit(Self, Failure)`): Carries `Self` on `.success` or `Failure` on `.fail`; `.die`, `.interrupt`, and `.cleanup` preserve other terminal causes.
        pub fn initWithRefreshInterval(env: anytype, registry: *invalidation.Registry, key: []const u8, capacity: usize, interval: std.Io.Duration) outcomes.Exit(Self, Failure) {
            if (interval.nanoseconds <= 0) return .{ .die = .{ .kind = .invalid_configuration, .message = diagnostics.query_refresh_interval_must_be_positive } };
            var prepared = init(env, registry, key, capacity);
            if (prepared == .success) prepared.success.refresh_interval = interval;
            return prepared;
        }

        /// Claim an expired refresh once per interval. Reading retained state
        /// never publishes an invalidation, so expiry cannot form a synchronous
        /// notification loop. A forced request bypasses the interval.
        ///
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// - `clock: services.Clock`: Caller-selected clock capability.
        /// - `force: bool`: Whether to bypass the normal freshness or cache check.
        /// Returns (`outcomes.Exit(bool, Failure)`): Carries `bool` on `.success` or `Failure` on `.fail`; `.die`, `.interrupt`, and `.cleanup` preserve other terminal causes.
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
        ///
        /// Parameters:
        /// - `env: anytype`: Borrowed caller environment supplying allocator, I/O, and selected
        ///   capabilities.
        /// - `registry: *invalidation.Registry`: Caller-supplied capability or scoped state; no concrete backend is
        ///   selected here.
        /// - `key: []const u8`: Stable identity used for matching, deduplication, or replay.
        /// - `capacity: usize`: buffer capacity for observable query changes.
        /// Returns (`outcomes.Exit(Self, Failure)`): Carries `Self` on `.success` or `Failure` on `.fail`; `.die`, `.interrupt`, and `.cleanup` preserve other terminal causes.
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

        /// Read through the selected capability.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// Returns (`outcomes.Exit(Result, Failure)`): Carries `Result` on `.success` or `Failure` on `.fail`; `.die`, `.interrupt`, and `.cleanup` preserve other terminal causes.
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

        /// Read the current observable state.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// Returns (`outcomes.Exit(*Ref.Observation, Failure)`): Carries `*Ref.Observation` on `.success` or `Failure` on `.fail`; `.die`, `.interrupt`, and `.cleanup` preserve other terminal causes.
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
        ///
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// Returns (`outcomes.Exit(void, Failure)`): Carries no value on `.success` or `Failure` on `.fail`; `.die`, `.interrupt`, and `.cleanup` preserve other terminal causes.
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

        /// Update reactive query state from one stream event.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// - `evaluated: outcomes.Exit(A, E)`: Terminal Exit produced by the evaluated Effect.
        /// Returns (`outcomes.Exit(bool, Failure)`): Carries `bool` on `.success` or `Failure` on `.fail`; `.die`, `.interrupt`, and `.cleanup` preserve other terminal causes.
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

        /// Stop the capability and release its retained resources.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// Returns (`void`): no value.
        pub fn shutdown(self: *Self) void {
            self.mutex.lockUncancelable(self.io);
            self.closed = true;
            self.mutex.unlock(self.io);
            self.permit.shutdown();
            self.values.shutdown();
        }

        /// Release resources owned by this value.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// Returns (`void`): no value.
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
///
/// Parameters:
/// - `A: comptime type`: Value type produced by a successful computation.
/// - `E: comptime type`: Expected application-failure type; defects and interruption remain separate outcomes.
/// - `R: comptime type`: Caller-defined environment type that supplies dependencies and runtime capabilities when work executes.
/// - `select_state: comptime fn (*const R) *State(A, E)`: Compile-time typed selector from the current environment; its result must
///   remain valid through use.
/// - `evaluate: comptime fn (*const R) effects.Effect(A, E, R)`: Compile-time typed function selected at comptime; no runtime callback lookup is
///   required.
/// Returns (`effects.Effect(void, Error(E), R)`): query result fetched and committed to observable state.
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
///
/// Parameters:
/// - `A: comptime type`: Value type produced by a successful computation.
/// - `E: comptime type`: Expected application-failure type; defects and interruption remain separate outcomes.
/// - `R: comptime type`: Caller-defined environment type that supplies dependencies and runtime capabilities when work executes.
/// - `select_state: comptime fn (*const R) *State(A, E)`: Compile-time typed selector from the current environment; its result must
///   remain valid through use.
/// - `select_clock: comptime fn (*const R) services.Clock`: Compile-time typed selector from the current environment; its result must
///   remain valid through use.
/// - `evaluate: comptime fn (*const R) effects.Effect(A, E, R)`: Compile-time typed function selected at comptime; no runtime callback lookup is
///   required.
/// - `force: comptime bool`: Compile-time choice to bypass normal freshness or reuse checks.
/// Returns (`effects.Effect(void, Error(E), R)`): query result fetched only when the stored state is stale.
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
///
/// Parameters:
/// - `A: comptime type`: Value type produced by a successful computation.
/// - `E: comptime type`: Expected application-failure type; defects and interruption remain separate outcomes.
/// - `R: comptime type`: Caller-defined environment type that supplies dependencies and runtime capabilities when work executes.
/// - `select_state: comptime fn (*const R) *State(A, E)`: Compile-time typed selector from the current environment; its result must
///   remain valid through use.
/// - `evaluate: comptime fn (*const R) effects.Effect(A, E, R)`: Compile-time typed function selected at comptime; no runtime callback lookup is
///   required.
/// Returns (`effects.Effect(void, Error(E), R)`): observable state after consuming the selected update stream.
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
