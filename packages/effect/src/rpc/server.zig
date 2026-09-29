//! Transport-independent unary server execution. Session providers retain
//! control of receiving, sending, cancellation and their own I/O engine.
const std = @import("std");
const outcomes = @import("../outcome.zig");
const effects = @import("../effect.zig");
const transport = @import("session.zig");
const wire = @import("wire_codec.zig");
const dispatch = @import("internal/dispatch.zig");
const Semaphore = @import("../semaphore.zig").Semaphore;

pub fn UnaryServer(
    comptime handler: anytype,
    comptime request_wire: wire.WireCodec(@TypeOf(handler.operation).Request, @TypeOf(handler).Environment),
    comptime success_wire: wire.WireCodec(@TypeOf(handler.operation).Success, @TypeOf(handler).Environment),
    comptime failure_wire: wire.WireCodec(@TypeOf(handler.operation).DomainError, @TypeOf(handler).Environment),
    comptime ProviderError: type,
    comptime select_session: fn (*const @TypeOf(handler).Environment) *const transport.Session(@TypeOf(handler).Environment, ProviderError),
    comptime select_policy: fn (*const @TypeOf(handler).Environment) transport.Policy,
    comptime select_admission: fn (*const @TypeOf(handler).Environment) *Semaphore,
) type {
    const Route = dispatch.UnaryRoute(handler, request_wire, success_wire, failure_wire, ProviderError, select_policy);
    return BoundedServer(Route, ProviderError, select_session, select_policy, select_admission);
}

/// Builds a closed encoded server from typed handler/codec bindings. The
/// generated routes and runtime dispatch remain private to this package.
pub fn ProtocolServer(
    comptime bindings: anytype,
    comptime ProviderError: type,
    comptime select_session: anytype,
    comptime select_policy: anytype,
    comptime select_admission: anytype,
) type {
    const Info = @typeInfo(@TypeOf(bindings));
    if (Info != .@"struct" or !Info.@"struct".is_tuple or bindings.len == 0)
        @compileError("RPC server needs a nonempty tuple of typed handler/codec bindings");
    const RouteTypes = std.meta.Tuple(&([_]type{type} ** bindings.len));
    comptime var routes: RouteTypes = undefined;
    inline for (bindings, 0..) |binding, index| {
        const B = @TypeOf(binding);
        if (!@hasField(B, "handler") or !@hasField(B, "request") or
            !@hasField(B, "success") or !@hasField(B, "failure"))
            @compileError("RPC server bindings need handler, request, success and failure codecs");
        routes[index] = dispatch.UnaryRoute(binding.handler, binding.request, binding.success, binding.failure, ProviderError, select_policy);
    }
    const Dispatcher = dispatch.EncodedProtocol(routes, ProviderError, select_policy);
    return BoundedServer(Dispatcher, ProviderError, select_session, select_policy, select_admission);
}

/// The server's caller-owned semaphore is the admission authority for every
/// received request; its capacity must match the selected protocol policy.
pub fn BoundedServer(
    comptime Dispatcher: type,
    comptime ProviderError: type,
    comptime select_session: fn (*const Dispatcher.Environment) *const transport.Session(Dispatcher.Environment, ProviderError),
    comptime select_policy: fn (*const Dispatcher.Environment) transport.Policy,
    comptime select_admission: fn (*const Dispatcher.Environment) *Semaphore,
) type {
    const Base = EncodedServer(Dispatcher, ProviderError, select_session, select_policy);
    const R = Dispatcher.Environment;
    return struct {
        pub const Failure = Base.Failure;
        pub const Result = Base.Result;

        fn admissionFailure(problem: @import("../semaphore.zig").Error) Failure {
            return switch (problem) {
                .shutdown => .{ .session = .disconnected },
                .invalid_capacity, .invalid_release => .{ .session = .invalid_configuration },
            };
        }

        pub fn serveOne() Result {
            return Result.deferEffect(struct {
                fn start(env: *const R) Result {
                    const policy = select_policy(env);
                    if (!policy.valid() or select_admission(env).limit != policy.max_inflight)
                        return Result.fromOwnedOutcome(env, .{ .fail = .{ .session = .invalid_configuration } });
                    return Semaphore.withPermit(R, void, Failure, select_admission, admissionFailure, struct {
                        fn use(_: void) Result {
                            return Base.serveOne();
                        }
                    }.use);
                }
            }.start);
        }

        pub fn invokeFrame(frame: []const u8, env: *const R) Dispatcher.Result {
            return Base.invokeFrame(frame, env);
        }
    };
}

/// Runs a checked closed encoded protocol (or one unary route) against a
/// caller-provided session. The dispatcher has no transport-specific state.
pub fn EncodedServer(
    comptime Dispatcher: type,
    comptime ProviderError: type,
    comptime select_session: fn (*const Dispatcher.Environment) *const transport.Session(Dispatcher.Environment, ProviderError),
    comptime select_policy: fn (*const Dispatcher.Environment) transport.Policy,
) type {
    const R = Dispatcher.Environment;
    comptime {
        if (!@hasField(R, "allocator") or @FieldType(R, "allocator") != std.mem.Allocator or
            !@hasField(R, "io") or @FieldType(R, "io") != std.Io)
            @compileError("RPC server requires allocator and io in R");
    }
    const Session = transport.Session(R, ProviderError);
    return struct {
        const Self = @This();
        pub const Failure = Dispatcher.Failure;
        pub const Result = effects.Effect(void, Failure, R);
        const Bytes = outcomes.OwnedSlice(u8);

        fn sessionFailure(problem: Session.Failure) Failure {
            return .{ .session = problem };
        }

        fn sent(_: *const Bytes, _: transport.SendAcknowledgement, env: *const R) Result {
            return Result.fromOwnedSuccess(env, {});
        }

        fn afterEncode(frame: Bytes, env: *const R) Result {
            const session = select_session(env);
            var sending = session.vtable.send(session.userdata, env, frame.items);
            var mapped = sending.mapErrorOwned(env, sessionFailure);
            return mapped.flatMapCapturedOwned(env, void, frame, sent);
        }

        fn received(value: transport.Received, env: *const R) Result {
            var owned = value;
            switch (owned) {
                .closed => return Result.fromOwnedOutcome(env, .{ .fail = .{ .session = .disconnected } }),
                .frame => |frame| {
                    var response = Dispatcher.serveFrame(frame.items, env);
                    owned.deinit();
                    return response.flatMapOwned(env, afterEncode);
                },
            }
        }

        /// Process one complete request frame with typed handler and codecs.
        /// A provider may run several of these in a caller-owned scope, with
        /// its selected concurrency bound and disconnect policy.
        pub fn serveOne() Result {
            return Result.deferEffect(struct {
                fn start(env: *const R) Result {
                    const policy = select_policy(env);
                    if (!policy.valid()) return Result.fromOwnedOutcome(env, .{ .fail = .{ .session = .invalid_configuration } });
                    const session = select_session(env);
                    var receiving = session.vtable.receive(session.userdata, env);
                    var mapped = receiving.mapErrorOwned(env, sessionFailure);
                    return mapped.flatMapOwned(env, received);
                }
            }.start);
        }

        /// Direct encoded invocation is useful to in-process adapters. The
        /// same route handles a frame no matter which session delivered it.
        pub fn invokeFrame(frame: []const u8, env: *const R) Dispatcher.Result {
            return Dispatcher.serveFrame(frame, env);
        }
    };
}
