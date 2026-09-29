//! Typed RPC middleware acquires a service in the caller's R, projects a
//! handler environment, and preserves domain and middleware failures apart.
const std = @import("std");
const outcomes = @import("../outcome.zig");
const effects = @import("../effect.zig");
const variants = @import("../schema/variant_schema.zig");
const protocol = @import("protocol.zig");

const error_tag_field = "tag";
const error_value_field = "value";
const domain_tag = "domain";
const middleware_tag = "middleware";

pub fn Failure(comptime DomainError: type, comptime MiddlewareError: type) type {
    return union(enum) {
        pub const effect_owned_payload = true;
        domain: DomainError,
        middleware: MiddlewareError,

        pub fn deinit(self: *@This()) void {
            switch (self.*) {
                .domain => outcomes.discardPayload(&self.domain),
                .middleware => outcomes.discardPayload(&self.middleware),
            }
            self.* = undefined;
        }

        pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
            return switch (self.*) {
                .domain => |value| .{ .domain = try outcomes.clonePayload(value) },
                .middleware => |value| .{ .middleware = try outcomes.clonePayload(value) },
            };
        }
    };
}

/// Derives the reference union representation from the two typed JSON case
/// codecs. The case tags and field names are one stable wire contract.
pub fn errorSchema(comptime R: type, comptime domain_codec: anytype, comptime middleware_codec: anytype) type {
    const D = @TypeOf(domain_codec).Decoded;
    const M = @TypeOf(middleware_codec).Decoded;
    return variants.Variant(Failure(D, M), R, error_tag_field, error_value_field, .{
        variants.taggedCase("domain", domain_tag, domain_codec),
        variants.taggedCase("middleware", middleware_tag, middleware_codec),
    });
}

/// Retains the operation identity, request/success codecs and key extractor;
/// only the domain-error codec is extended with the middleware case.
pub fn operation(
    comptime original: anytype,
    comptime middleware_codec: anytype,
    comptime R: type,
) protocol.Unary(@TypeOf(original.request), @TypeOf(original.success), errorSchema(R, original.failure, middleware_codec)) {
    const ErrorSchema = errorSchema(R, original.failure, middleware_codec);
    const Wrapped = protocol.Unary(@TypeOf(original.request), @TypeOf(original.success), ErrorSchema);
    var result = Wrapped.init(original.name, original.version, original.request, original.success, ErrorSchema{});
    result.idempotency_key = original.idempotency_key;
    return result;
}

pub fn Middleware(comptime Service: type, comptime MiddlewareError: type, comptime R: type) type {
    return struct {
        const Self = @This();
        pub const Provided = Service;
        pub const Required = R;
        pub const Error = MiddlewareError;
        acquire: effects.Effect(Service, MiddlewareError, R),

        pub fn fromEffect(comptime acquisition: effects.Effect(Service, MiddlewareError, R)) Self {
            return .{ .acquire = acquisition };
        }

        /// `connect` constructs the handler's projected environment from the
        /// acquired service and caller's R. Its value stays alive through the
        /// handler Effect, including suspension and cleanup.
        pub fn applyTo(
            comptime self: Self,
            comptime Request: type,
            comptime Output: type,
            comptime DomainError: type,
            comptime HandlerEnv: type,
            comptime connect: fn (*const Service, *const R) HandlerEnv,
            comptime handler: fn (*const Request, *const HandlerEnv) effects.Effect(Output, DomainError, HandlerEnv),
            request: *const Request,
            env: *const R,
        ) effects.Effect(Output, Failure(DomainError, MiddlewareError), R) {
            const Combined = Failure(DomainError, MiddlewareError);
            const Program = effects.Effect(Output, Combined, R);
            const Owner = struct {
                pub const effect_owned_payload = true;
                const Owned = @This();
                const Inner = struct { service: Service, projected: HandlerEnv };
                allocator: std.mem.Allocator,
                parent: *const R,
                request: *const Request,
                inner: *Inner,

                fn init(current_env: *const R, request_value: *const Request, service: Service) error{OutOfMemory}!Owned {
                    const pointer = current_env.allocator.create(Inner) catch {
                        var abandoned = service;
                        outcomes.discardPayload(&abandoned);
                        return error.OutOfMemory;
                    };
                    pointer.service = service;
                    pointer.projected = connect(&pointer.service, current_env);
                    return .{ .allocator = current_env.allocator, .parent = current_env, .request = request_value, .inner = pointer };
                }

                pub fn deinit(owner: *Owned) void {
                    outcomes.discardPayload(&owner.inner.projected);
                    outcomes.discardPayload(&owner.inner.service);
                    owner.allocator.destroy(owner.inner);
                    owner.* = undefined;
                }

                pub fn clone(owner: *const Owned) error{OutOfMemory}!Owned {
                    return init(owner.parent, owner.request, try outcomes.clonePayload(owner.inner.service));
                }
            };
            comptime {
                if (!@hasField(R, "allocator") or @FieldType(R, "allocator") != std.mem.Allocator or
                    !@hasField(R, "io") or @FieldType(R, "io") != std.Io)
                    @compileError("RPC middleware requires allocator and io in R");
                if (!@hasField(HandlerEnv, "allocator") or @FieldType(HandlerEnv, "allocator") != std.mem.Allocator or
                    !@hasField(HandlerEnv, "io") or @FieldType(HandlerEnv, "io") != std.Io)
                    @compileError("RPC handler environment must preserve allocator and io capabilities");
            }
            var acquired = self.acquire.mapError(struct {
                fn middlewareError(problem: MiddlewareError) Combined {
                    return .{ .middleware = problem };
                }
            }.middlewareError);
            return acquired.flatMapCapturedOwned(env, Output, request, struct {
                fn afterAcquire(request_ref: *const *const Request, service: Service, supplied: *const R) Program {
                    const owner = Owner.init(supplied, request_ref.*, service) catch return Program.die(outcomes.Defect.out_of_memory);
                    const Unit = effects.Effect(void, Combined, R);
                    var ready = Unit.fromOwnedSuccess(supplied, {});
                    return ready.flatMapCapturedOwned(supplied, Output, owner, struct {
                        fn run(capture: *const Owner, _: void, current: *const R) Program {
                            var handled = handler(capture.request, &capture.inner.projected);
                            var provided = handled.provideOwned(&capture.inner.projected, R, current);
                            return provided.mapErrorOwned(current, struct {
                                fn domainError(problem: DomainError) Combined {
                                    return .{ .domain = problem };
                                }
                            }.domainError);
                        }
                    }.run);
                }
            }.afterAcquire);
        }

        pub fn apply(
            comptime self: Self,
            comptime Request: type,
            comptime Output: type,
            comptime DomainError: type,
            comptime HandlerEnv: type,
            comptime connect: fn (*const Service, *const R) HandlerEnv,
            comptime handler: fn (*const Request, *const HandlerEnv) effects.Effect(Output, DomainError, HandlerEnv),
            comptime select_request: fn (*const R) *const Request,
        ) effects.Effect(Output, Failure(DomainError, MiddlewareError), R) {
            const Program = effects.Effect(Output, Failure(DomainError, MiddlewareError), R);
            return Program.deferEffect(struct {
                fn start(env: *const R) Program {
                    return self.applyTo(Request, Output, DomainError, HandlerEnv, connect, handler, select_request(env), env);
                }
            }.start);
        }

        /// Derives the extended operation and its typed handler together, so
        /// a middleware failure cannot be registered with the old error codec.
        pub fn wrapUnary(
            comptime self: Self,
            comptime original: anytype,
            comptime middleware_codec: anytype,
            comptime HandlerEnv: type,
            comptime connect: fn (*const Service, *const R) HandlerEnv,
            comptime handler: fn (*const @TypeOf(original).Request, *const HandlerEnv) effects.Effect(@TypeOf(original).Success, @TypeOf(original).DomainError, HandlerEnv),
        ) protocol.UnaryHandler(@TypeOf(operation(original, middleware_codec, R)), R) {
            const Request = @TypeOf(original).Request;
            const Output = @TypeOf(original).Success;
            const DomainError = @TypeOf(original).DomainError;
            const wrapped = operation(original, middleware_codec, R);
            return wrapped.handledBy(R, struct {
                fn invoke(request_value: *const Request, env: *const R) effects.Effect(Output, Failure(DomainError, MiddlewareError), R) {
                    return self.applyTo(Request, Output, DomainError, HandlerEnv, connect, handler, request_value, env);
                }
            }.invoke);
        }
    };
}
