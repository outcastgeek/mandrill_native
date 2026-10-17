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
            var acquired = self.acquire.mapError(struct {
                fn middlewareError(problem: MiddlewareError) Combined {
                    return .{ .middleware = problem };
                }
            }.middlewareError);
            return applyPreparedTo(Request, Output, DomainError, HandlerEnv, &acquired, struct {
                fn project(_: *const Request, service: *const Service, current: *const R) HandlerEnv {
                    return connect(service, current);
                }
            }.project, handler, null, request, env);
        }

        /// Consume a runtime acquisition through the same service/environment
        /// owner used by applyTo. The input may carry invocation configuration;
        /// its enclosing owner must cover the returned Effect's execution.
        pub fn applyOwnedTo(
            self: *Self,
            comptime Request: type,
            comptime Output: type,
            comptime DomainError: type,
            comptime HandlerEnv: type,
            comptime connect: anytype,
            comptime handler: fn (*const Request, *const HandlerEnv) effects.Effect(Output, DomainError, HandlerEnv),
            request: *const Request,
            env: *const R,
        ) effects.Effect(Output, Failure(DomainError, MiddlewareError), R) {
            const Combined = Failure(DomainError, MiddlewareError);
            var acquired = self.acquire.mapErrorOwned(env, struct {
                fn middlewareError(problem: MiddlewareError) Combined {
                    return .{ .middleware = problem };
                }
            }.middlewareError);
            self.* = undefined;
            return applyPreparedTo(Request, Output, DomainError, HandlerEnv, &acquired, connect, handler, null, request, env);
        }

        /// Run an effectful release under the existing resource bracket. Native
        /// environment cleanup precedes release; native service cleanup follows
        /// its complete drain, including failure or cancellation.
        pub fn applyManagedTo(
            self: *Self,
            comptime Request: type,
            comptime Output: type,
            comptime DomainError: type,
            comptime HandlerEnv: type,
            comptime connect: anytype,
            comptime handler: fn (*const Request, *const HandlerEnv) effects.Effect(Output, DomainError, HandlerEnv),
            comptime release: fn (*const Request, *const Service, *const outcomes.PrimaryOutcome(Failure(DomainError, MiddlewareError)), *const R) effects.Effect(void, MiddlewareError, R),
            request: *const Request,
            env: *const R,
        ) effects.Effect(Output, Failure(DomainError, MiddlewareError), R) {
            const Combined = Failure(DomainError, MiddlewareError);
            var acquired = self.acquire.mapErrorOwned(env, struct {
                fn middlewareError(problem: MiddlewareError) Combined {
                    return .{ .middleware = problem };
                }
            }.middlewareError);
            self.* = undefined;
            return applyPreparedTo(Request, Output, DomainError, HandlerEnv, &acquired, connect, handler, release, request, env);
        }

        fn applyPreparedTo(
            comptime Request: type,
            comptime Output: type,
            comptime DomainError: type,
            comptime HandlerEnv: type,
            acquired: *effects.Effect(Service, Failure(DomainError, MiddlewareError), R),
            comptime connect: anytype,
            comptime handler: fn (*const Request, *const HandlerEnv) effects.Effect(Output, DomainError, HandlerEnv),
            comptime release: anytype,
            request: *const Request,
            env: *const R,
        ) effects.Effect(Output, Failure(DomainError, MiddlewareError), R) {
            const Combined = Failure(DomainError, MiddlewareError);
            const Program = effects.Effect(Output, Combined, R);
            const Acquisition = effects.Effect(Service, Combined, R);
            const Unit = effects.Effect(void, Combined, R);
            const Projection = @import("../dsl/inference.zig").result(connect, &.{ *const Request, *const Service, *const R }, "RPC middleware.project");
            if (Projection != HandlerEnv and Projection != outcomes.Result(HandlerEnv, MiddlewareError) and Projection != effects.Effect(HandlerEnv, MiddlewareError, R)) @compileError("RPC middleware.project requires HandlerEnv, exact Result(HandlerEnv,MiddlewareError), or exact Effect(HandlerEnv,MiddlewareError,R)");
            comptime {
                if (!@hasField(R, "allocator") or @FieldType(R, "allocator") != std.mem.Allocator or
                    !@hasField(R, "io") or @FieldType(R, "io") != std.Io)
                    @compileError("RPC middleware requires allocator and io in R");
                if (!@hasField(HandlerEnv, "allocator") or @FieldType(HandlerEnv, "allocator") != std.mem.Allocator or
                    !@hasField(HandlerEnv, "io") or @FieldType(HandlerEnv, "io") != std.Io)
                    @compileError("RPC handler environment must preserve allocator and io capabilities");
            }
            const Source = struct {
                pub const effect_owned_payload = true;
                acquisition: Acquisition,
                request: *const Request,
                pub fn deinit(self: *@This()) void {
                    self.acquisition.deinit();
                }
            };
            const Region = struct {
                const RetainedRegion = @This();
                pub const effect_owned_payload = true;
                const Inner = struct { service: ?Service = null, projected: ?HandlerEnv = null };
                allocator: std.mem.Allocator,
                request: *const Request,
                acquisition: Acquisition,
                inner: *Inner,

                fn dropProjection(self: *const @This()) void {
                    if (self.inner.projected) |*projected| outcomes.discardPayload(projected);
                    self.inner.projected = null;
                }
                pub fn deinit(self: *@This()) void {
                    self.dropProjection();
                    if (self.inner.service) |*service| outcomes.discardPayload(service);
                    self.allocator.destroy(self.inner);
                }
                fn acquire(self: *const @This(), _: *const R) Acquisition {
                    return self.acquisition.borrow();
                }
                fn use(self: *const @This(), service: Service, current: *const R) Program {
                    self.inner.service = service;
                    if (comptime Projection == effects.Effect(HandlerEnv, MiddlewareError, R)) {
                        var projected = connect(self.request, &self.inner.service.?, current);
                        var checked = projected.mapErrorOwned(current, struct {
                            fn middlewareError(problem: MiddlewareError) Combined {
                                return .{ .middleware = problem };
                            }
                        }.middlewareError);
                        return checked.flatMapCapturedOwned(current, Output, self, struct {
                            fn installed(stored: *const *const RetainedRegion, environment: HandlerEnv, supplied: *const R) Program {
                                stored.*.inner.projected = environment;
                                return stored.*.handle(supplied);
                            }
                        }.installed);
                    }
                    if (comptime Projection == HandlerEnv) {
                        self.inner.projected = connect(self.request, &self.inner.service.?, current);
                    } else switch (connect(self.request, &self.inner.service.?, current)) {
                        .ok => |projected| self.inner.projected = projected,
                        .err => |problem| return Program.fromOwnedOutcome(current, .{ .fail = .{ .middleware = problem } }),
                    }
                    return self.handle(current);
                }
                fn handle(self: *const @This(), current: *const R) Program {
                    var handled = handler(self.request, &self.inner.projected.?);
                    var provided = handled.provideOwned(&self.inner.projected.?, R, current);
                    return provided.mapErrorOwned(current, struct {
                        fn domainError(problem: DomainError) Combined {
                            return .{ .domain = problem };
                        }
                    }.domainError);
                }
                fn close(self: *const @This(), service: Service, primary: *const outcomes.PrimaryOutcome(Combined), current: *const R) Unit {
                    // The bracket also calls release after a successful late
                    // acquisition when cancellation skipped the body.
                    if (self.inner.service == null) self.inner.service = service;
                    self.dropProjection();
                    if (comptime @TypeOf(release) == @TypeOf(null)) return Unit.succeed({});
                    var cleanup = release(self.request, &self.inner.service.?, primary, current);
                    return cleanup.mapErrorOwned(current, struct {
                        fn middlewareError(problem: MiddlewareError) Combined {
                            return .{ .middleware = problem };
                        }
                    }.middlewareError);
                }
            };
            const source: Source = .{ .acquisition = acquired.*, .request = request };
            acquired.* = undefined;
            var seed = Unit.succeed({});
            return seed.flatMapCapturedOwned(env, Output, source, struct {
                fn run(stored: *const Source, _: void, current: *const R) Program {
                    const pointer = current.allocator.create(Region.Inner) catch return Program.die(outcomes.Defect.out_of_memory);
                    pointer.* = .{};
                    const region: Region = .{ .allocator = current.allocator, .request = stored.request, .acquisition = stored.acquisition.borrow(), .inner = pointer };
                    return Acquisition.acquireReleaseFactoryCapturedOwned(current, region, Region.acquire, Region.use, Region.close);
                }
            }.run);
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
