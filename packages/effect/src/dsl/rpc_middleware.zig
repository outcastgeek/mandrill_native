//! Inferred runtime middleware declarations lower to the existing RPC owner.
const std = @import("std");
const effects = @import("../effect.zig");
const outcomes = @import("../outcome.zig");
const runtime = @import("../internal/runtime.zig");
const middleware = @import("../rpc/middleware.zig");
const declarations = @import("declarations.zig");
const roles = @import("roles.zig");
const inference = @import("inference.zig");
const ownership = @import("ownership.zig");

fn Environment(comptime P: type) type {
    if (@typeInfo(P) != .pointer or @typeInfo(P).pointer.size != .one) @compileError("withRpcMiddleware requires a borrowed environment pointer");
    return @typeInfo(P).pointer.child;
}
fn requireChannel(comptime T: type) void {
    if (ownership.containsOwned(T) and !outcomes.ownsPayload(T)) @compileError("RPC middleware owning channels require declared ownership before Effect composition");
}
fn Declaration(comptime P: type, comptime Carrier: type) type {
    return struct {
        const R = Environment(P);
        const Info = declarations.Info(Carrier, "withRpcMiddleware");
        const supplied = Info.definition;
        comptime {
            roles.validate(roles.recordType(supplied), "withRpcMiddleware", &.{ "acquire", "project", "use" }, &.{"release"});
        }
        const C = Info.Inputs;
        const Stored = ownership.Capture(C);
        const acquire = roles.get("acquire", supplied);
        const project = roles.get("project", supplied);
        const use = roles.get("use", supplied);
        const Acquire = inference.result(acquire, &.{ *const C, *const R }, "withRpcMiddleware.acquire");
        comptime {
            inference.requireEffect(Acquire, "withRpcMiddleware.acquire");
            if (Acquire.Environment != R) @compileError("withRpcMiddleware.acquire environment mismatch; use an explicit projection");
            requireChannel(Acquire.Success);
            requireChannel(Acquire.Error);
        }
        const Service = Acquire.Success;
        const M = Acquire.Error;
        const Projected = inference.result(project, &.{ *const C, *const Service, *const R }, "withRpcMiddleware.project");
        const effect_projection = @typeInfo(Projected) == .@"struct" and @hasDecl(Projected, "Success") and @hasDecl(Projected, "Error") and @hasDecl(Projected, "Environment");
        const HandlerEnv = if (effect_projection) Projected.Success else if (@typeInfo(Projected) == .@"union" and @hasField(Projected, "ok") and @hasField(Projected, "err")) @FieldType(Projected, "ok") else Projected;
        comptime {
            if (Projected != HandlerEnv and Projected != outcomes.Result(HandlerEnv, M) and Projected != effects.Effect(HandlerEnv, M, R)) @compileError("withRpcMiddleware.project requires a handler environment, exact Result(HandlerEnv,MiddlewareError), or exact Effect(HandlerEnv,MiddlewareError,R)");
            requireChannel(HandlerEnv);
        }
        const Body = inference.result(use, &.{ *const C, *const HandlerEnv }, "withRpcMiddleware.use");
        comptime {
            inference.requireEffect(Body, "withRpcMiddleware.use");
            if (Body.Environment != HandlerEnv) @compileError("withRpcMiddleware.use must require the projected handler environment");
            requireChannel(Body.Success);
            requireChannel(Body.Error);
        }
        const Result = effects.Effect(Body.Success, middleware.Failure(Body.Error, M), R);
        const Cleanup = effects.Effect(void, M, R);
        comptime {
            if (roles.has(roles.recordType(supplied), "release") and inference.result(roles.get("release", supplied), &.{ *const C, *const Service, *const outcomes.PrimaryOutcome(Result.Error), *const R }, "withRpcMiddleware.release") != Cleanup) @compileError("withRpcMiddleware.release requires exact Effect(void,MiddlewareError,R)");
        }
        fn cleanup(stored: *const Stored, service: *const Service, primary: *const outcomes.PrimaryOutcome(Result.Error), current: *const R) Cleanup {
            return roles.get("release", supplied)(&stored.value, service, primary, current);
        }
        fn connect(stored: *const Stored, service: *const Service, current: *const R) Projected {
            return project(&stored.value, service, current);
        }
        fn handle(stored: *const Stored, current: *const HandlerEnv) Body {
            return use(&stored.value, current);
        }
        fn run(stored: *const Stored, _: void, current: *const R) Result {
            var managed: middleware.Middleware(Service, M, R) = .{ .acquire = acquire(&stored.value, current) };
            if (comptime roles.has(roles.recordType(supplied), "release")) return managed.applyManagedTo(Stored, Body.Success, Body.Error, HandlerEnv, connect, handle, cleanup, stored, current);
            return managed.applyOwnedTo(Stored, Body.Success, Body.Error, HandlerEnv, connect, handle, stored, current);
        }
    };
}

/// Consume runtime configuration once. Acquisition is fresh per execution;
/// projection and use borrow generated storage through the complete handler.
/// Native owning services/environments close in environment-then-service order.
/// An optional release Effect runs under the existing resource bracket. A
/// stream escaping this use region requires a session owner.
pub fn scoped(env: anytype, input: anytype) Declaration(@TypeOf(env), @TypeOf(input)).Result {
    const Selected = Declaration(@TypeOf(env), @TypeOf(input));
    const allocator: std.mem.Allocator = runtime.services(Selected.R, env).allocatorValue() orelse {
        var abandoned = roles.get("inputs", input);
        ownership.discard(&abandoned);
        return Selected.Result.die(.{ .kind = .invalid_configuration, .message = "withRpcMiddleware requires construction environment allocator" });
    };
    var seed = effects.Effect(void, Selected.Result.Error, Selected.R).succeed({});
    return seed.flatMapCapturedAllocated(allocator, Selected.Result.Success, Selected.Stored{ .value = roles.get("inputs", input) }, Selected.run);
}

fn SessionDeclaration(comptime P: type, comptime Carrier: type) type {
    return struct {
        const Info = declarations.Info(Carrier, "withRpcSession");
        const supplied = Info.definition;
        comptime {
            roles.validate(roles.recordType(supplied), "withRpcSession", &.{ "server", "acquire", "project", "use" }, &.{"release"});
        }
        const Server = roles.get("server", supplied);
        comptime {
            if (@TypeOf(Server) != type) @compileError("withRpcSession.server requires the RPC StreamServer type");
            @import("../rpc/stream_server.zig").requireServer(Server);
        }
        const C = Info.Inputs;
        const H = Server.Environment;
        const D = Server.DomainError;
        const Scope = @import("../scope.zig").Scope(D);
        const use = roles.get("use", supplied);
        const Body = inference.result(use, &.{ *const C, *Scope, *const H }, "withRpcSession.use");
        comptime {
            inference.requireEffect(Body, "withRpcSession.use");
            if (Body.Environment != H or Body.Error != Server.Failure) @compileError("withRpcSession.use requires the selected server's exact error and environment channels");
        }
        fn scopeError(problem: D) Server.Failure {
            return .{ .domain = problem };
        }
        fn inScope(stored: *const *const C, scope: *Scope, current: *const H) Body {
            return use(stored.*, scope, current);
        }
        fn managedUse(configuration: *const C, current: *const H) Body {
            return Body.scopedErrorCaptured(D, current, configuration, scopeError, inScope);
        }
        const lower_definition = if (roles.has(roles.recordType(supplied), "release")) declarations.metadata(.{
            .acquire = roles.get("acquire", supplied),
            .project = roles.get("project", supplied),
            .use = managedUse,
            .release = roles.get("release", supplied),
        }) else declarations.metadata(.{
            .acquire = roles.get("acquire", supplied),
            .project = roles.get("project", supplied),
            .use = managedUse,
        });
        const Lower = struct { comptime definition: type = lower_definition, inputs: C };
        const Result = Declaration(P, Lower).Result;
    };
}

/// A canonical stream server fixes the reader domain and provider errors. The
/// generated request scope drains its readers and accepted child work before
/// projected environment and middleware service release.
pub fn session(env: anytype, input: anytype) SessionDeclaration(@TypeOf(env), @TypeOf(input)).Result {
    const Selected = SessionDeclaration(@TypeOf(env), @TypeOf(input));
    return scoped(env, Selected.Lower{ .inputs = roles.get("inputs", input) });
}
