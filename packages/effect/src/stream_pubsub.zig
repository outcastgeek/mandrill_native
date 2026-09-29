//! Scoped Stream/PubSub bridge. The caller owns the bounded bus and chooses
//! its capacity, replay window and overload policy in the same environment.
//! Subscribe before starting producers when every item must be observed:
//! replay is an explicit bus policy, not an implicit stream buffer. With
//! backpressure, publishers wait only while an active subscriber protects the
//! oldest item; cancellation interrupts that wait. A closed bus lets a reader
//! drain retained items before `end`. Publisher failures stay on the returned
//! producer Effect, so its owner must join that Effect as well as consuming
//! the stream; an end marker alone never means the producer succeeded.
const outcomes = @import("outcome.zig");
const effects = @import("effect.zig");
const pubsub = @import("pub_sub.zig");
const scopes = @import("scope.zig");
const Cancellation = @import("internal/runtime.zig").Cancellation;

/// Create a Stream from an existing PubSub subscription.
/// Parameters:
/// - `A: comptime type`: Value type produced by a successful computation.
/// - `E: comptime type`: Expected application-failure type; defects and interruption remain separate outcomes.
/// - `R: comptime type`: Caller-defined environment type that supplies dependencies and runtime capabilities when work executes.
/// - `select_bus: comptime fn (*const R) *pubsub.PubSub(A)`: Compile-time typed selector from the current environment; its result must
///   remain valid through use.
/// - `map_bus_error: comptime fn (pubsub.Error) E`: Compile-time typed function selected at comptime; no runtime callback lookup
///   is required.
/// - `map_gap: comptime fn (u64) E`: Compile-time typed function selected at comptime; no runtime callback lookup is
///   required.
/// Returns (`effects.Stream(A, E, R)`): A lazy typed Stream; the caller consumes it under its scope and handles terminal Exit.
pub fn fromSubscription(
    comptime A: type,
    comptime E: type,
    comptime R: type,
    comptime select_bus: fn (*const R) *pubsub.PubSub(A),
    comptime map_bus_error: fn (pubsub.Error) E,
    comptime map_gap: fn (u64) E,
) effects.Stream(A, E, R) {
    const Bus = pubsub.PubSub(A);
    const Sub = Bus.Subscription;
    const Pull = effects.Effect(effects.Chunk(A), E, R);
    const Unit = effects.Effect(void, E, R);
    const Acquire = effects.Effect(*Sub, pubsub.Error, R);
    const acquired = comptime Acquire.fromOutcome(struct {
        fn open(env: *const R) outcomes.Exit(*Sub, pubsub.Error) {
            return select_bus(env).subscribe();
        }
    }.open).mapError(map_bus_error);
    return effects.Stream(A, E, R).fromPull(acquired, struct {
        fn pull(subscription: *Sub) Pull {
            return Pull.deferBorrowed(Sub, subscription, struct {
                fn run(current: *const Sub, env: *const R) Pull {
                    var taken = @constCast(current).take(R);
                    var mapped = taken.mapErrorOwned(env, map_bus_error);
                    return mapped.flatMapOwned(env, struct {
                        fn convert(delivery: pubsub.Delivery(A), supplied: *const R) Pull {
                            return switch (delivery) {
                                .item => |item| blk: {
                                    const values = supplied.allocator.alloc(A, 1) catch {
                                        var abandoned = item;
                                        outcomes.discardPayload(&abandoned);
                                        break :blk Pull.die(outcomes.Defect.out_of_memory);
                                    };
                                    values[0] = item;
                                    break :blk Pull.fromOwnedSuccess(supplied, effects.Chunk(A).owned(.{ .items = values, .allocator = supplied.allocator }));
                                },
                                .gap => |count| Pull.fromOwnedOutcome(supplied, .{ .fail = map_gap(count) }),
                                .end => Pull.fromOwnedSuccess(supplied, .end),
                            };
                        }
                    }.convert);
                }
            }.run);
        }
    }.pull, struct {
        fn release(subscription: *Sub) Unit {
            return Unit.fromBorrowed(Sub, subscription, struct {
                fn close(current: *const Sub, _: *const R) outcomes.Exit(void, E) {
                    @constCast(current).deinit();
                    return .{ .success = {} };
                }
            }.close);
        }
    }.release);
}

/// The producer completes when its source ends; bus shutdown is a separate
/// caller-owned decision because other producers or subscribers may remain.
///
/// Parameters:
/// - `A: comptime type`: Value type produced by a successful computation.
/// - `E: comptime type`: Expected application-failure type; defects and interruption remain separate outcomes.
/// - `R: comptime type`: Caller-defined environment type that supplies dependencies and runtime capabilities when work executes.
/// - `source: comptime effects.Stream(A, E, R)`: Compile-time typed input value; move or clone owning payloads according to its type
///   contract.
/// - `select_bus: comptime fn (*const R) *pubsub.PubSub(A)`: Compile-time typed selector from the current environment; its result must
///   remain valid through use.
/// - `map_bus_error: comptime fn (pubsub.Error) E`: Compile-time typed function selected at comptime; no runtime callback lookup
///   is required.
/// - `on_drop: comptime fn () E`: Compile-time typed callback; its signature is checked at comptime where static.
/// Returns (`effects.Effect(void, E, R)`): producer effect that publishes this stream into the borrowed bus; the caller still owns bus shutdown.
pub fn publishTo(
    comptime A: type,
    comptime E: type,
    comptime R: type,
    comptime source: effects.Stream(A, E, R),
    comptime select_bus: fn (*const R) *pubsub.PubSub(A),
    comptime map_bus_error: fn (pubsub.Error) E,
    comptime on_drop: fn () E,
) effects.Effect(void, E, R) {
    const Unit = effects.Effect(void, E, R);
    return source.runForEach(struct {
        fn publish(value: A, env: *const R) Unit {
            return Unit.fromOwnedCancellable(env, value, struct {
                fn run(item: *const A, current: *const R, token: ?*const Cancellation) outcomes.Exit(void, E) {
                    return switch (select_bus(current).publishCancellable(item.*, token)) {
                        .success => |result| switch (result) {
                            .published => .{ .success = {} },
                            .dropped => .{ .fail = on_drop() },
                        },
                        .fail => |problem| .{ .fail = map_bus_error(problem) },
                        .die => |defect| .{ .die = defect },
                        .interrupted => .interrupted,
                        .finalization_failed => unreachable,
                    };
                }
            }.run);
        }
    }.publish);
}

/// Publish a single source into an exclusively owned bus, then signal its
/// subscribers after all retained items have become available to drain.
/// The publisher's typed failure remains on the returned Effect.
///
/// Parameters:
/// - `A: comptime type`: Value type produced by a successful computation.
/// - `E: comptime type`: Expected application-failure type; defects and interruption remain separate outcomes.
/// - `R: comptime type`: Caller-defined environment type that supplies dependencies and runtime capabilities when work executes.
/// - `source: comptime effects.Stream(A, E, R)`: Compile-time typed input value; move or clone owning payloads according to its type
///   contract.
/// - `select_bus: comptime fn (*const R) *pubsub.PubSub(A)`: Compile-time typed selector from the current environment; its result must
///   remain valid through use.
/// - `map_bus_error: comptime fn (pubsub.Error) E`: Compile-time typed function selected at comptime; no runtime callback lookup
///   is required.
/// - `on_drop: comptime fn () E`: Compile-time typed callback; its signature is checked at comptime where static.
/// Returns (`effects.Effect(void, E, R)`): producer effect that publishes this stream and closes its exclusively owned bus afterward.
pub fn publishToAndClose(
    comptime A: type,
    comptime E: type,
    comptime R: type,
    comptime source: effects.Stream(A, E, R),
    comptime select_bus: fn (*const R) *pubsub.PubSub(A),
    comptime map_bus_error: fn (pubsub.Error) E,
    comptime on_drop: fn () E,
) effects.Effect(void, E, R) {
    const Unit = effects.Effect(void, E, R);
    return (comptime Unit.succeed({})).acquireRelease(struct {
        fn produce(_: void) Unit {
            return publishTo(A, E, R, source, select_bus, map_bus_error, on_drop);
        }
    }.produce, struct {
        fn close(_: void, _: outcomes.PrimaryOutcome(E), env: *const R) Unit {
            select_bus(env).shutdown();
            return Unit.fromOwnedSuccess(env, {});
        }
    }.close);
}

/// Run both producers concurrently into the same bounded bus. Consumers may
/// subscribe before starting this Effect; the caller shuts the bus down only
/// after all publishers and observers using it have joined.
///
/// Parameters:
/// - `A: comptime type`: Value type produced by a successful computation.
/// - `E: comptime type`: Expected application-failure type; defects and interruption remain separate outcomes.
/// - `R: comptime type`: Caller-defined environment type that supplies dependencies and runtime capabilities when work executes.
/// - `left: comptime effects.Stream(A, E, R)`: Left-hand typed description in this composition.
/// - `right: comptime effects.Stream(A, E, R)`: Right-hand typed description in this composition.
/// - `select_bus: comptime fn (*const R) *pubsub.PubSub(A)`: Compile-time typed selector from the current environment; its result must
///   remain valid through use.
/// - `map_bus_error: comptime fn (pubsub.Error) E`: Compile-time typed function selected at comptime; no runtime callback lookup
///   is required.
/// - `on_drop: comptime fn () E`: Compile-time typed callback; its signature is checked at comptime where static.
/// Returns (`effects.Effect(void, E, R)`): effect running both producers concurrently into the same caller-owned bus.
pub fn mergeInto(
    comptime A: type,
    comptime E: type,
    comptime R: type,
    comptime left: effects.Stream(A, E, R),
    comptime right: effects.Stream(A, E, R),
    comptime select_bus: fn (*const R) *pubsub.PubSub(A),
    comptime map_bus_error: fn (pubsub.Error) E,
    comptime on_drop: fn () E,
) effects.Effect(void, E, R) {
    const Unit = effects.Effect(void, E, R);
    const Source = enum { left, right };
    const sources = [_]Source{ .left, .right };
    return Unit.deferEffect(struct {
        fn start(env: *const R) Unit {
            var running = Unit.forEachConcurrent(Source, sources.len, &sources, struct {
                fn produce(source: Source) Unit {
                    return switch (source) {
                        .left => publishTo(A, E, R, left, select_bus, map_bus_error, on_drop),
                        .right => publishTo(A, E, R, right, select_bus, map_bus_error, on_drop),
                    };
                }
            }.produce);
            return running.flatMapOwned(env, struct {
                fn finished(values: outcomes.OwnedSlice(void), current: *const R) Unit {
                    var owned = values;
                    owned.deinit();
                    return Unit.fromOwnedSuccess(current, {});
                }
            }.finished);
        }
    }.start);
}

/// Complete an exclusively owned bus when both producers settle. A subscriber
/// already attached to the bus drains retained items and then observes `end`.
/// The returned Effect retains any producer failure; callers must join it.
/// A shared bus must use `mergeInto` and let its owner decide when to shut down.
///
/// Parameters:
/// - `A: comptime type`: Value type produced by a successful computation.
/// - `E: comptime type`: Expected application-failure type; defects and interruption remain separate outcomes.
/// - `R: comptime type`: Caller-defined environment type that supplies dependencies and runtime capabilities when work executes.
/// - `left: comptime effects.Stream(A, E, R)`: Left-hand typed description in this composition.
/// - `right: comptime effects.Stream(A, E, R)`: Right-hand typed description in this composition.
/// - `select_bus: comptime fn (*const R) *pubsub.PubSub(A)`: Compile-time typed selector from the current environment; its result must
///   remain valid through use.
/// - `map_bus_error: comptime fn (pubsub.Error) E`: Compile-time typed function selected at comptime; no runtime callback lookup
///   is required.
/// - `on_drop: comptime fn () E`: Compile-time typed callback; its signature is checked at comptime where static.
/// Returns (`effects.Effect(void, E, R)`): effect running both producers and closing their exclusively owned bus after they settle.
pub fn mergeIntoAndClose(
    comptime A: type,
    comptime E: type,
    comptime R: type,
    comptime left: effects.Stream(A, E, R),
    comptime right: effects.Stream(A, E, R),
    comptime select_bus: fn (*const R) *pubsub.PubSub(A),
    comptime map_bus_error: fn (pubsub.Error) E,
    comptime on_drop: fn () E,
) effects.Effect(void, E, R) {
    const Unit = effects.Effect(void, E, R);
    return (comptime Unit.succeed({})).acquireRelease(struct {
        fn produce(_: void) Unit {
            return mergeInto(A, E, R, left, right, select_bus, map_bus_error, on_drop);
        }
    }.produce, struct {
        fn close(_: void, _: outcomes.PrimaryOutcome(E), env: *const R) Unit {
            select_bus(env).shutdown();
            return Unit.fromOwnedSuccess(env, {});
        }
    }.close);
}

/// Subscribe before starting the producer, then consume until the owned
/// bus is closed and drained. The producer fiber is scoped and joined, so its
/// typed failure cannot be mistaken for a successful end-of-stream. The caller
/// retains ownership of the bus and deinitializes it after this Effect exits.
fn runProducer(
    comptime A: type,
    comptime E: type,
    comptime R: type,
    comptime producer: effects.Effect(void, E, R),
    comptime select_bus: fn (*const R) *pubsub.PubSub(A),
    comptime map_bus_error: fn (pubsub.Error) E,
    comptime map_gap: fn (u64) E,
    comptime consume: fn (A, *const R) effects.Effect(void, E, R),
) effects.Effect(void, E, R) {
    const Unit = effects.Effect(void, E, R);
    const Bus = pubsub.PubSub(A);
    const Sub = Bus.Subscription;
    const Scope = scopes.Scope(E);
    const Acquire = effects.Effect(*Sub, E, R);
    const Delivery = pubsub.Delivery(A);

    const Adapter = struct {
        fn drain(subscription: *Sub) Unit {
            return Unit.deferBorrowed(Sub, subscription, struct {
                fn next(current: *const Sub, env: *const R) Unit {
                    var taken = @constCast(current).take(R);
                    var mapped = taken.mapErrorOwned(env, map_bus_error);
                    return mapped.flatMapCapturedOwned(env, void, @constCast(current), struct {
                        fn received(owner: *const *Sub, delivery: Delivery, supplied: *const R) Unit {
                            return switch (delivery) {
                                .item => |item| blk: {
                                    var using = consume(item, supplied);
                                    break :blk using.flatMapCapturedOwned(supplied, void, owner.*, struct {
                                        fn again(sub: *const *Sub, _: void, _: *const R) Unit {
                                            return drain(sub.*);
                                        }
                                    }.again);
                                },
                                .gap => |count| Unit.fromOwnedOutcome(supplied, .{ .fail = map_gap(count) }),
                                .end => Unit.fromOwnedSuccess(supplied, {}),
                            };
                        }
                    }.received);
                }
            }.next);
        }

        fn subscribe(env: *const R) outcomes.Exit(*Sub, E) {
            return switch (select_bus(env).subscribe()) {
                .success => |sub| .{ .success = sub },
                .fail => |problem| .{ .fail = map_bus_error(problem) },
                .die => |defect| .{ .die = defect },
                .interrupted => .interrupted,
                .finalization_failed => unreachable,
            };
        }

        fn release(sub: *Sub) Unit {
            return Unit.fromBorrowed(Sub, sub, struct {
                fn close(owner: *const Sub, _: *const R) outcomes.Exit(void, E) {
                    @constCast(owner).deinit();
                    return .{ .success = {} };
                }
            }.close);
        }

        fn joined(fiber: *const *effects.Fiber(void, E), _: void, _: *const R) Unit {
            return Unit.joinFiber(fiber.*);
        }

        fn started(sub: *const *Sub, fiber: *effects.Fiber(void, E), env: *const R) Unit {
            var consuming = drain(sub.*);
            return consuming.flatMapCapturedOwned(env, void, fiber, joined);
        }

        fn subscribed(scope: *const *Scope, sub: *Sub, env: *const R) Unit {
            var starting = producer.fork(scope.*);
            return starting.flatMapCapturedOwned(env, void, sub, started);
        }

        fn run(scope: *const Scope, env: *const R) Unit {
            var acquiring = Acquire.fromOutcome(subscribe).scopeAcquire(@constCast(scope), release);
            return acquiring.flatMapCapturedOwned(env, void, @constCast(scope), subscribed);
        }

        fn inScope(scope: *Scope) Unit {
            return Unit.deferBorrowed(Scope, scope, run);
        }
    };
    return Unit.scoped(Adapter.inScope);
}

/// Run one source through a bounded caller-owned bus, with subscription,
/// producer lifetime, drain and typed producer completion handled together.
///
/// Parameters:
/// - `A: comptime type`: Value type produced by a successful computation.
/// - `E: comptime type`: Expected application-failure type; defects and interruption remain separate outcomes.
/// - `R: comptime type`: Caller-defined environment type that supplies dependencies and runtime capabilities when work executes.
/// - `source: comptime effects.Stream(A, E, R)`: Compile-time typed input value; move or clone owning payloads according to its type
///   contract.
/// - `select_bus: comptime fn (*const R) *pubsub.PubSub(A)`: Compile-time typed selector from the current environment; its result must
///   remain valid through use.
/// - `map_bus_error: comptime fn (pubsub.Error) E`: Compile-time typed function selected at comptime; no runtime callback lookup
///   is required.
/// - `map_gap: comptime fn (u64) E`: Compile-time typed function selected at comptime; no runtime callback lookup is
///   required.
/// - `on_drop: comptime fn () E`: Compile-time typed callback; its signature is checked at comptime where static.
/// - `consume: comptime fn (A, *const R) effects.Effect(void, E, R)`: Compile-time typed function selected at comptime; no runtime callback lookup is
///   required.
/// Returns (`effects.Effect(void, E, R)`): effect that consumes one source through a bounded bus and joins its producer.
pub fn runBuffered(
    comptime A: type,
    comptime E: type,
    comptime R: type,
    comptime source: effects.Stream(A, E, R),
    comptime select_bus: fn (*const R) *pubsub.PubSub(A),
    comptime map_bus_error: fn (pubsub.Error) E,
    comptime map_gap: fn (u64) E,
    comptime on_drop: fn () E,
    comptime consume: fn (A, *const R) effects.Effect(void, E, R),
) effects.Effect(void, E, R) {
    return runProducer(A, E, R, publishToAndClose(A, E, R, source, select_bus, map_bus_error, on_drop), select_bus, map_bus_error, map_gap, consume);
}

/// Run two source streams concurrently through the same bounded bus. Both
/// branches share the caller's runtime capabilities and are joined on exit.
///
/// Parameters:
/// - `A: comptime type`: Value type produced by a successful computation.
/// - `E: comptime type`: Expected application-failure type; defects and interruption remain separate outcomes.
/// - `R: comptime type`: Caller-defined environment type that supplies dependencies and runtime capabilities when work executes.
/// - `left: comptime effects.Stream(A, E, R)`: Left-hand typed description in this composition.
/// - `right: comptime effects.Stream(A, E, R)`: Right-hand typed description in this composition.
/// - `select_bus: comptime fn (*const R) *pubsub.PubSub(A)`: Compile-time typed selector from the current environment; its result must
///   remain valid through use.
/// - `map_bus_error: comptime fn (pubsub.Error) E`: Compile-time typed function selected at comptime; no runtime callback lookup
///   is required.
/// - `map_gap: comptime fn (u64) E`: Compile-time typed function selected at comptime; no runtime callback lookup is
///   required.
/// - `on_drop: comptime fn () E`: Compile-time typed callback; its signature is checked at comptime where static.
/// - `consume: comptime fn (A, *const R) effects.Effect(void, E, R)`: Compile-time typed function selected at comptime; no runtime callback lookup is
///   required.
/// Returns (`effects.Effect(void, E, R)`): effect that consumes two merged source streams and joins both producers.
pub fn runMerged(
    comptime A: type,
    comptime E: type,
    comptime R: type,
    comptime left: effects.Stream(A, E, R),
    comptime right: effects.Stream(A, E, R),
    comptime select_bus: fn (*const R) *pubsub.PubSub(A),
    comptime map_bus_error: fn (pubsub.Error) E,
    comptime map_gap: fn (u64) E,
    comptime on_drop: fn () E,
    comptime consume: fn (A, *const R) effects.Effect(void, E, R),
) effects.Effect(void, E, R) {
    return runProducer(A, E, R, mergeIntoAndClose(A, E, R, left, right, select_bus, map_bus_error, on_drop), select_bus, map_bus_error, map_gap, consume);
}
