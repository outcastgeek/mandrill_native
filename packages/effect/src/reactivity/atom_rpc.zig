//! RPC-backed query and mutation composition. Network/session behavior remains
//! with the caller's RPC client; this module only connects typed observations
//! to the scoped atom registry after a successful mutation.
const effects = @import("../effect.zig");
const outcomes = @import("../outcome.zig");
const registry = @import("atom_registry.zig");
const query = @import("query.zig");
const services = @import("../services.zig");

/// Specialize the typed mutation report outcome for reactivity operations.
/// Parameters:
/// - `A: comptime type`: Value type produced by a successful computation.
/// Returns (`type`): typed result of a remote atom mutation and its invalidation status.
pub fn MutationReport(comptime A: type) type {
    return struct {
        /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements `deinit`
        /// and must be released after use.
        pub const effect_owned_payload = true;
        reply: A,
        invalidation: outcomes.Exit(@import("invalidation_registry.zig").InvalidateResult, registry.Error),

        /// Release resources owned by this value.
        /// Parameters:
        /// - `self: *@This()`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// Returns (`void`): no value.
        pub fn deinit(self: *@This()) void {
            outcomes.discardPayload(&self.reply);
            self.invalidation.deinit();
            self.* = undefined;
        }

        /// Duplicate this value; marked owning payloads are cloned, while ordinary
        /// borrowed fields retain their original lifetime.
        /// Parameters:
        /// - `self: *const @This()`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
        /// Returns (`error{OutOfMemory}!@This()`): copied value on success; `OutOfMemory` if owned storage could not be cloned. The source remains valid.
        pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
            var reply = try outcomes.clonePayload(self.reply);
            errdefer outcomes.discardPayload(&reply);
            const copied = self.invalidation.clone();
            if (copied == .die and copied.die.kind == .out_of_memory and
                !(self.invalidation == .die and self.invalidation.die.kind == .out_of_memory))
                return error.OutOfMemory;
            return .{ .reply = reply, .invalidation = copied };
        }
    };
}

/// The query state is registered before its first refresh. Its normal
/// generation and dirty-bit rules reject obsolete RPC completions.
///
/// Parameters:
/// - `Client: comptime type`: Compile-time typed RPC client interface.
/// - `R: comptime type`: Caller-defined environment type that supplies dependencies and runtime capabilities when work executes.
/// - `select_request: comptime fn (*const R) *const Client.Request`: Compile-time typed selector from the current environment; its result must
///   remain valid through use.
/// - `select_state: comptime fn (*const R) *query.State(Client.Success, Client.Failure)`: Compile-time typed selector from the current environment; its result must
///   remain valid through use.
/// Returns (`effects.Effect(void, query.Error(Client.Failure), R)`): atom value fetched over RPC and published locally.
pub fn refresh(
    comptime Client: type,
    comptime R: type,
    comptime select_request: fn (*const R) *const Client.Request,
    comptime select_state: fn (*const R) *query.State(Client.Success, Client.Failure),
) effects.Effect(void, query.Error(Client.Failure), R) {
    return query.refresh(Client.Success, Client.Failure, R, select_state, struct {
        fn evaluate(_: *const R) Client.Result {
            return Client.call(select_request);
        }
    }.evaluate);
}

/// Refresh only when the atom or query is stale.
/// Parameters:
/// - `Client: comptime type`: Compile-time typed RPC client interface.
/// - `R: comptime type`: Caller-defined environment type that supplies dependencies and runtime capabilities when work executes.
/// - `select_request: comptime fn (*const R) *const Client.Request`: Compile-time typed selector from the current environment; its result must
///   remain valid through use.
/// - `select_state: comptime fn (*const R) *query.State(Client.Success, Client.Failure)`: Compile-time typed selector from the current environment; its result must
///   remain valid through use.
/// - `select_clock: comptime fn (*const R) services.Clock`: Compile-time typed selector from the current environment; its result must
///   remain valid through use.
/// - `force: comptime bool`: Compile-time choice to bypass normal freshness or reuse checks.
/// Returns (`effects.Effect(void, query.Error(Client.Failure), R)`): atom refresh performed only when local state is stale.
pub fn refreshIfStale(
    comptime Client: type,
    comptime R: type,
    comptime select_request: fn (*const R) *const Client.Request,
    comptime select_state: fn (*const R) *query.State(Client.Success, Client.Failure),
    comptime select_clock: fn (*const R) services.Clock,
    comptime force: bool,
) effects.Effect(void, query.Error(Client.Failure), R) {
    return query.refreshIfStale(Client.Success, Client.Failure, R, select_state, select_clock, struct {
        fn evaluate(_: *const R) Client.Result {
            return Client.call(select_request);
        }
    }.evaluate, force);
}

/// `StreamRun` cases: `completed`, `refresh_required`.
pub const StreamRun = enum { completed, refresh_required };

/// Scoped RPC events publish through the query's generation gate. A change
/// received during the stream closes the stream and asks its owner to rerun.
///
/// Parameters:
/// - `Client: comptime type`: Compile-time typed RPC client interface.
/// - `R: comptime type`: Caller-defined environment type that supplies dependencies and runtime capabilities when work executes.
/// - `select_request: comptime fn (*const R) *const Client.Request`: Compile-time typed selector from the current environment; its result must
///   remain valid through use.
/// - `select_state: comptime fn (*const R) *query.State(Client.Event, Client.Failure)`: Compile-time typed selector from the current environment; its result must
///   remain valid through use.
/// Returns (`effects.Effect(StreamRun, query.Error(Client.Failure), R)`): query state updated from the remote atom stream.
pub fn streamIntoQuery(
    comptime Client: type,
    comptime R: type,
    comptime select_request: fn (*const R) *const Client.Request,
    comptime select_state: fn (*const R) *query.State(Client.Event, Client.Failure),
) effects.Effect(StreamRun, query.Error(Client.Failure), R) {
    const Failure = query.Error(Client.Failure);
    const Unit = effects.Effect(void, Failure, R);
    const Out = effects.Effect(StreamRun, Failure, R);
    return Out.deferEffect(struct {
        fn start(env: *const R) Out {
            var beginning = Unit.fromOutcome(struct {
                fn begin(current: *const R) outcomes.Exit(void, Failure) {
                    return select_state(current).beginStream();
                }
            }.begin);
            return beginning.flatMapOwned(env, struct {
                fn consume(_: void, current: *const R) Out {
                    const source = comptime Client.stream(select_request).mapError(struct {
                        fn rpc(problem: Client.Failure) Failure {
                            return .{ .evaluation = problem };
                        }
                    }.rpc);
                    var running = source.runForEach(struct {
                        fn event(value: Client.Event, supplied: *const R) Unit {
                            return Unit.fromOwned(supplied, value, struct {
                                fn publish(owned: *const Client.Event, environment: *const R) outcomes.Exit(void, Failure) {
                                    const copy = outcomes.clonePayload(owned.*) catch
                                        return .{ .die = outcomes.Defect.out_of_memory };
                                    const published = select_state(environment).acceptStreamEvent(.{ .success = copy });
                                    return switch (published) {
                                        .success => |obsolete| if (obsolete)
                                            .{ .fail = .refresh_required }
                                        else
                                            .{ .success = {} },
                                        else => outcomes.castFailure(void, published),
                                    };
                                }
                            }.publish);
                        }
                    }.event);
                    var settled = running.materializeOwned(current);
                    var aligned = settled.mapErrorOwned(current, struct {
                        fn impossible(_: void) Failure {
                            unreachable;
                        }
                    }.impossible);
                    return aligned.flatMapOwned(current, struct {
                        fn finished(result: outcomes.Exit(void, Failure), environment: *const R) Out {
                            return switch (result) {
                                .success => Out.fromOwnedSuccess(environment, .completed),
                                .fail => |problem| blk: {
                                    if (problem == .refresh_required)
                                        break :blk Out.fromOwnedSuccess(environment, .refresh_required);
                                    if (problem == .evaluation) {
                                        const copy = outcomes.clonePayload(problem.evaluation) catch
                                            return Out.die(outcomes.Defect.out_of_memory);
                                        var recorded = select_state(environment).acceptStreamEvent(.{ .fail = copy });
                                        recorded.deinit();
                                    }
                                    break :blk Out.fromOwnedOutcome(environment, .{ .fail = problem });
                                },
                                .die => |defect| blk: {
                                    var recorded = select_state(environment).acceptStreamEvent(.{ .die = defect });
                                    recorded.deinit();
                                    break :blk Out.fromOwnedOutcome(environment, .{ .die = defect });
                                },
                                .interrupted => blk: {
                                    var recorded = select_state(environment).acceptStreamEvent(.interrupted);
                                    recorded.deinit();
                                    break :blk Out.fromOwnedOutcome(environment, .interrupted);
                                },
                                .finalization_failed => Out.fromOwnedOutcome(environment, outcomes.castFailure(StreamRun, result)),
                            };
                        }
                    }.finished);
                }
            }.consume);
        }
    }.start);
}

/// A successful remote reply is kept even if local invalidation fails. The
/// caller can inspect the nested Exit and reconcile from authoritative data.
///
/// Parameters:
/// - `Client: comptime type`: Compile-time typed RPC client interface.
/// - `R: comptime type`: Caller-defined environment type that supplies dependencies and runtime capabilities when work executes.
/// - `select_request: comptime fn (*const R) *const Client.Request`: Compile-time typed selector from the current environment; its result must
///   remain valid through use.
/// - `select_registry: comptime fn (*const R) *registry.Registry`: Compile-time typed selector from the current environment; its result must
///   remain valid through use.
/// - `select_key: comptime fn (*const R) []const u8`: Compile-time typed selector from the current environment; its result must
///   remain valid through use.
/// Returns (`effects.Effect(MutationReport(Client.Success), Client.Failure, R)`): remote atom mutation acknowledgement and local invalidation status.
pub fn mutate(
    comptime Client: type,
    comptime R: type,
    comptime select_request: fn (*const R) *const Client.Request,
    comptime select_registry: fn (*const R) *registry.Registry,
    comptime select_key: fn (*const R) []const u8,
) effects.Effect(MutationReport(Client.Success), Client.Failure, R) {
    const Report = MutationReport(Client.Success);
    const Out = effects.Effect(Report, Client.Failure, R);
    return Out.deferEffect(struct {
        fn start(env: *const R) Out {
            var called = Client.call(select_request);
            return called.flatMapOwned(env, struct {
                fn after(reply: Client.Success, current: *const R) Out {
                    return Out.fromOwnedSuccess(current, .{
                        .reply = reply,
                        .invalidation = select_registry(current).invalidate(select_key(current)),
                    });
                }
            }.after);
        }
    }.start);
}
