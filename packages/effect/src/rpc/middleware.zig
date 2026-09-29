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

/// Specialize the rpc failure variants from their typed channels.
/// Parameters:
/// - `DomainError: comptime type`: Application failure type produced by domain-level operations.
/// - `MiddlewareError: comptime type`: Compile-time typed failure introduced by middleware.
/// Returns (`type`): typed middleware failure union combining service and handler errors.
pub fn Failure(comptime DomainError: type, comptime MiddlewareError: type) type {
    return union(enum) {
        /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements `deinit`
        /// and must be released after use.
        pub const effect_owned_payload = true;
        domain: DomainError,
        middleware: MiddlewareError,

        /// Release resources owned by this value.
        /// Parameters:
        /// - `self: *@This()`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// Returns (`void`): no value.
        pub fn deinit(self: *@This()) void {
            switch (self.*) {
                .domain => outcomes.discardPayload(&self.domain),
                .middleware => outcomes.discardPayload(&self.middleware),
            }
            self.* = undefined;
        }

        /// Duplicate this value; marked owning payloads are cloned, while ordinary
        /// borrowed fields retain their original lifetime.
        /// Parameters:
        /// - `self: *const @This()`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
        /// Returns (`error{OutOfMemory}!@This()`): copied value on success; `OutOfMemory` if owned storage could not be cloned. The source remains valid.
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
///
/// Parameters:
/// - `R: comptime type`: Caller-defined environment type that supplies dependencies and runtime capabilities when work executes.
/// - `domain_codec: comptime anytype`: Compile-time typed codec/schema fixing the corresponding decoded and encoded
///   value types.
/// - `middleware_codec: comptime anytype`: Compile-time typed codec/schema fixing the corresponding decoded and
///   encoded value types.
/// Returns (`type`): union schema derived from the middleware and domain failure cases.
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
///
/// Parameters:
/// - `original: comptime anytype`: Compile-time statically inferred input checked when this operation is
///   instantiated.
/// - `middleware_codec: comptime anytype`: Compile-time typed codec/schema fixing the corresponding decoded and
///   encoded value types.
/// - `R: comptime type`: Caller-defined environment type that supplies dependencies and runtime capabilities when work executes.
/// Returns (`protocol.Unary(@TypeOf(original.request), @TypeOf(original.success), errorSchema(R, original.failure, middleware_codec))`): A unary RPC descriptor with a middleware-adjusted failure codec.
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

/// Specialize middleware that wraps a typed RPC service and can add its own failure channel.
/// Parameters:
/// - `Service: comptime type`: Compile-time service type acquired by the middleware.
/// - `MiddlewareError: comptime type`: Compile-time typed failure introduced by middleware.
/// - `R: comptime type`: Caller-defined environment type that supplies dependencies and runtime capabilities when work executes.
/// Returns (`type`): scoped RPC service wrapper that may add its own typed failure channel.
pub fn Middleware(comptime Service: type, comptime MiddlewareError: type, comptime R: type) type {
    return struct {
        const Self = @This();
        /// Service type supplied by this middleware.
        pub const Provided = Service;
        /// Environment type required by this middleware.
        pub const Required = R;
        /// Expected middleware failure type fixed by the service definition.
        pub const Error = MiddlewareError;
        acquire: effects.Effect(Service, MiddlewareError, R),

        /// Adapt a lazy Effect to this operation.
        /// Parameters:
        /// - `acquisition: comptime effects.Effect(Service, MiddlewareError, R)`: Typed Effect that acquires the resource before the body runs.
        /// Returns (`Self`): middleware service acquisition backed by the supplied Effect and its scope.
        pub fn fromEffect(comptime acquisition: effects.Effect(Service, MiddlewareError, R)) Self {
            return .{ .acquire = acquisition };
        }

        /// `connect` constructs the handler's projected environment from the
        /// acquired service and caller's R. Its value stays alive through the
        /// handler Effect, including suspension and cleanup.
        ///
        /// Parameters:
        /// - `self: comptime Self`: Compile-time current value passed by value; ownership follows this
        ///   operation’s contract.
        /// - `Request: comptime type`: Request payload type accepted by the selected operation.
        /// - `Output: comptime type`: Service or value type produced by the handler.
        /// - `DomainError: comptime type`: Expected application error type produced by the handler.
        /// - `HandlerEnv: comptime type`: Compile-time environment type required by the handler.
        /// - `connect: comptime fn (*const Service, *const R) HandlerEnv`: Compile-time typed function selected at comptime; no runtime callback
        ///   lookup is required.
        /// - `handler: comptime fn (*const Request, *const HandlerEnv) effects.Effect(Output, DomainError, HandlerEnv)`: Compile-time typed callback; its signature is checked at comptime where
        ///   static.
        /// - `request: *const Request`: Borrowed input; keep its owner alive until the operation has copied or
        ///   joined it.
        /// - `env: *const R`: Borrowed caller environment supplying allocator, I/O, and selected
        ///   capabilities.
        /// Returns (`effects.Effect(Output, Failure(DomainError, MiddlewareError), R)`): wrapped handler result using the acquired service and combined typed failures.
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
                /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements
                /// `deinit` and must be released after use.
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

                /// Release resources owned by this value.
                /// Parameters:
                /// - `owner: *Owned`: Owner identity used for claim and fencing checks.
                /// Returns (`void`): no value.
                pub fn deinit(owner: *Owned) void {
                    outcomes.discardPayload(&owner.inner.projected);
                    outcomes.discardPayload(&owner.inner.service);
                    owner.allocator.destroy(owner.inner);
                    owner.* = undefined;
                }

                /// Duplicate this value; marked owning payloads are cloned, while ordinary
                /// borrowed fields retain their original lifetime.
                /// Parameters:
                /// - `owner: *const Owned`: Owner identity used for claim and fencing checks.
                /// Returns (`error{OutOfMemory}!Owned`): A `Owned` on success; `OutOfMemory` if required allocation fails.
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

        /// Wrap the RPC service with typed middleware.
        /// Parameters:
        /// - `self: comptime Self`: Compile-time current value passed by value; ownership follows this
        ///   operation’s contract.
        /// - `Request: comptime type`: Request payload type accepted by the selected operation.
        /// - `Output: comptime type`: Service or value type produced by the handler.
        /// - `DomainError: comptime type`: Expected application error type produced by the handler.
        /// - `HandlerEnv: comptime type`: Compile-time environment type required by the handler.
        /// - `connect: comptime fn (*const Service, *const R) HandlerEnv`: Compile-time typed function selected at comptime; no runtime callback
        ///   lookup is required.
        /// - `handler: comptime fn (*const Request, *const HandlerEnv) effects.Effect(Output, DomainError, HandlerEnv)`: Compile-time typed callback; its signature is checked at comptime where
        ///   static.
        /// - `select_request: comptime fn (*const R) *const Request`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// Returns (`effects.Effect(Output, Failure(DomainError, MiddlewareError), R)`): wrapped handler result after middleware acquisition and scoped release.
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
        ///
        /// Parameters:
        /// - `self: comptime Self`: Compile-time current value passed by value; ownership follows this
        ///   operation’s contract.
        /// - `original: comptime anytype`: Compile-time statically inferred input checked when this operation is
        ///   instantiated.
        /// - `middleware_codec: comptime anytype`: Compile-time typed codec/schema fixing the corresponding decoded
        ///   and encoded value types.
        /// - `HandlerEnv: comptime type`: Compile-time environment type required by the handler.
        /// - `connect: comptime fn (*const Service, *const R) HandlerEnv`: Compile-time typed function selected at comptime; no runtime callback
        ///   lookup is required.
        /// - `handler: comptime fn (*const @TypeOf(original).Request, *const HandlerEnv) effects.Effect(@TypeOf(original).Success, @TypeOf(original).DomainError, HandlerEnv)`: Compile-time typed callback; its signature is checked at comptime where
        ///   static.
        /// Returns (`protocol.UnaryHandler(@TypeOf(operation(original, middleware_codec, R)), R)`): A completed typed handler binding its operation to the selected environment.
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
