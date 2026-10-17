//! Actual-read tracking with per-evaluation contexts and guarded publication.
//! Tracked descriptions admit only generated nodes. Their legal evaluation
//! boundary returns the ordinary exact Effect(Published(A),E,R).
const std = @import("std");
const effects = @import("../effect.zig");
const outcomes = @import("../outcome.zig");
const registry_module = @import("atom_registry.zig");
const inference = @import("../dsl/inference.zig");
const rt = @import("../internal/runtime.zig");
const ownership = @import("../dsl/ownership.zig");
const Registry = registry_module.Registry;

pub fn Published(comptime A: type) type {
    return struct {
        pub const effect_owned_payload = true;
        value: union(enum) { current: A, stale },
        update: ?registry_module.DependencyUpdate = null,
        pub fn deinit(self: *@This()) void {
            if (self.value == .current) ownership.discard(&self.value.current);
            if (self.update) |*update| update.deinit();
        }
        pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
            var copied: @This() = .{ .value = if (self.value == .current) .{ .current = try ownership.cloneValue(self.value.current) } else .stale };
            errdefer copied.deinit();
            if (self.update) |update| copied.update = try update.clone();
            return copied;
        }
    };
}
fn Context(comptime E: type, comptime R: type, comptime map_error: anytype) type {
    return struct {
        const Self = @This();
        allocator: std.mem.Allocator,
        io: std.Io,
        mutex: std.Io.Mutex = .init,
        registry: *Registry,
        key: []u8,
        ticket: Registry.EvaluationTicket,
        observations: std.ArrayList(Registry.Observation) = .empty,
        active: bool = true,
        conflicted: bool = false,
        fn record(self: *Self, key: []const u8) outcomes.Exit(void, E) {
            self.mutex.lockUncancelable(self.io);
            defer self.mutex.unlock(self.io);
            if (!self.active) return .{ .die = .{ .kind = .invalid_configuration, .message = "tracked read context is no longer active" } };
            const revision = switch (self.registry.observedRevision(key)) {
                .success => |v| v,
                .fail => |e| return .{ .fail = map_error(e) },
                .die => |e| return .{ .die = e },
                .interrupted => return .interrupted,
                .finalization_failed => unreachable,
            };
            for (self.observations.items) |observed| if (std.mem.eql(u8, observed.key, key)) {
                if (observed.revision != revision) self.conflicted = true;
                return .{ .success = {} };
            };
            if (self.observations.items.len == self.registry.graph.limits.max_dependencies_per_node) return .{ .fail = map_error(.{ .graph = .dependency_limit }) };
            const copy = self.allocator.dupe(u8, key) catch return .{ .die = outcomes.Defect.out_of_memory };
            self.observations.append(self.allocator, .{ .key = copy, .revision = revision }) catch {
                self.allocator.free(copy);
                return .{ .die = outcomes.Defect.out_of_memory };
            };
            return .{ .success = {} };
        }
        fn deinit(self: *Self) void {
            for (self.observations.items) |observed| self.allocator.free(observed.key);
            self.observations.deinit(self.allocator);
            self.allocator.free(self.key);
            self.allocator.destroy(self);
        }
        fn close(self: *Self) effects.Effect(void, E, R) {
            return effects.Effect(void, E, R).fromBorrowed(Self, self, struct {
                fn run(ctx: *const Self, _: *const R) outcomes.Exit(void, E) {
                    @constCast(ctx).deinit();
                    return .{ .success = {} };
                }
            }.run);
        }
    };
}

fn require(comptime T: type) void {
    if (@typeInfo(T) != .@"struct" or !@hasDecl(T, "tracked_shape")) @compileError("tracked atom composition requires a generated tracked description, not an ordinary Effect");
    const shape = T.tracked_shape;
    const Canonical = switch (shape.kind) {
        .read => Read(shape.definition),
        .map => Mapped(shape.source, shape.operation),
        .bind => Bound(shape.source, shape.operation),
        .branch => Branch(shape.predicate, shape.left, shape.right),
        else => @compileError("unknown tracked atom descriptor"),
    };
    if (T != Canonical) @compileError("tracked atom composition rejects structural lookalikes and copied markers");
}
pub fn read(comptime atom: anytype) Read(atom) {
    return .{};
}
fn Read(comptime atom: anytype) type {
    atom.validate();
    const A = @TypeOf(atom).Success;
    const E = @TypeOf(atom).DomainError;
    const R = @TypeOf(atom).Environment;
    const AtomValue = atom;
    return struct {
        pub const Success = A;
        pub const Error = E;
        pub const Environment = R;
        pub const tracked_shape = struct {
            pub const kind = .read;
            pub const definition = AtomValue;
        };
        fn lower(_: @This(), ctx: anytype, _: *const R) effects.Effect(A, E, R) {
            const C = @typeInfo(@TypeOf(ctx)).pointer.child;
            return effects.Effect(A, E, R).deferBorrowed(C, ctx, struct {
                fn start(context: *const C, env: *const R) effects.Effect(A, E, R) {
                    const recorded = @constCast(context).record(atom.select_key(env));
                    if (recorded != .success) return effects.Effect(A, E, R).fromOwnedOutcome(env, outcomes.castFailure(A, recorded));
                    return atom.read();
                }
            }.start);
        }
    };
}
pub fn map(source: anytype, comptime callback: anytype) Mapped(@TypeOf(source), callback) {
    return .{ .source = source };
}
fn Mapped(comptime S: type, comptime callback: anytype) type {
    require(S);
    const B = inference.unary(callback, S.Success, "tracked.map");
    const E = S.Error;
    const R = S.Environment;
    return struct {
        pub const Success = B;
        pub const Error = E;
        pub const Environment = R;
        pub const tracked_shape = struct {
            pub const kind = .map;
            pub const source = S;
            pub const operation = callback;
        };
        source: S,
        fn lower(self: @This(), ctx: anytype, env: *const R) effects.Effect(B, E, R) {
            var source = self.source.lower(ctx, env);
            return source.mapOwned(env, callback);
        }
    };
}
pub fn bind(source: anytype, comptime callback: anytype) Bound(@TypeOf(source), callback) {
    return .{ .source = source };
}
fn Bound(comptime S: type, comptime callback: anytype) type {
    require(S);
    const N = inference.unary(callback, S.Success, "tracked.bind");
    require(N);
    if (S.Error != N.Error or S.Environment != N.Environment) @compileError("tracked.bind requires matching E/R; use explicit domain adapters");
    return struct {
        pub const Success = N.Success;
        pub const Error = S.Error;
        pub const Environment = S.Environment;
        pub const tracked_shape = struct {
            pub const kind = .bind;
            pub const source = S;
            pub const operation = callback;
        };
        source: S,
        fn lower(self: @This(), ctx: anytype, env: *const S.Environment) effects.Effect(N.Success, S.Error, S.Environment) {
            const C = @typeInfo(@TypeOf(ctx)).pointer.child;
            const Out = effects.Effect(N.Success, S.Error, S.Environment);
            var source = self.source.lower(ctx, env);
            return source.flatMapCapturedOwned(env, N.Success, ctx, struct {
                fn next(context: *const *C, value: S.Success, current: *const S.Environment) Out {
                    const produced = callback(value);
                    return produced.lower(context.*, current);
                }
            }.next);
        }
    };
}
pub fn branch(predicate: anytype, left: anytype, right: anytype) Branch(@TypeOf(predicate), @TypeOf(left), @TypeOf(right)) {
    return .{ .predicate = predicate, .left = left, .right = right };
}
fn Branch(comptime P: type, comptime L: type, comptime R: type) type {
    require(P);
    require(L);
    require(R);
    if (P.Success != bool or L.Success != R.Success or P.Error != L.Error or L.Error != R.Error or P.Environment != L.Environment or L.Environment != R.Environment) @compileError("tracked branch requires bool predicate and compatible result/error/environment joins");
    return struct {
        pub const Success = L.Success;
        pub const Error = L.Error;
        pub const Environment = L.Environment;
        pub const tracked_shape = struct {
            pub const kind = .branch;
            pub const predicate = P;
            pub const left = L;
            pub const right = R;
        };
        predicate: P,
        left: L,
        right: R,
        fn lower(self: @This(), ctx: anytype, env: *const L.Environment) effects.Effect(L.Success, L.Error, L.Environment) {
            const C = @typeInfo(@TypeOf(ctx)).pointer.child;
            const Out = effects.Effect(L.Success, L.Error, L.Environment);
            const Capture = struct { context: *C, left: L, right: R };
            var source = self.predicate.lower(ctx, env);
            return source.flatMapCapturedOwned(env, L.Success, Capture{ .context = ctx, .left = self.left, .right = self.right }, struct {
                fn next(selected: *const Capture, value: bool, current: *const L.Environment) Out {
                    return if (value) selected.left.lower(selected.context, current) else selected.right.lower(selected.context, current);
                }
            }.next);
        }
    };
}

/// Infer the complete evaluation contract from the tracked program and its
/// authoritative atom definition. The registry error mapper is an explicit
/// application choice; failures and stale evaluations preserve committed edges.
pub fn evaluate(comptime definition: anytype, comptime program: anytype, comptime select_registry: anytype, comptime map_registry_error: anytype) effects.Effect(Published(@TypeOf(program).Success), @TypeOf(program).Error, @TypeOf(program).Environment) {
    const Program = @TypeOf(program);
    comptime require(Program);
    const E = Program.Error;
    const R = Program.Environment;
    const A = Program.Success;
    if (comptime @TypeOf(definition).Success != A or @TypeOf(definition).DomainError != E or @TypeOf(definition).Environment != R) @compileError("tracked evaluation definition/result/context mismatch");
    if (comptime inference.unary(map_registry_error, registry_module.Error, "tracked.registry_error") != E) @compileError("tracked registry error mapper must produce the program E");
    const C = Context(E, R, map_registry_error);
    const Acquire = effects.Effect(*C, E, R);
    const Out = effects.Effect(Published(A), E, R);
    const Adapter = struct {
        fn acquire(env: *const R) outcomes.Exit(*C, E) {
            const owner = select_registry(env);
            const name = definition.select_key(env);
            const ticket = switch (owner.beginEvaluation(name)) {
                .success => |v| v,
                .fail => |e| return .{ .fail = map_registry_error(e) },
                .die => |e| return .{ .die = e },
                .interrupted => return .interrupted,
                .finalization_failed => unreachable,
            };
            const context = env.allocator.create(C) catch return .{ .die = outcomes.Defect.out_of_memory };
            const copied = env.allocator.dupe(u8, name) catch {
                env.allocator.destroy(context);
                return .{ .die = outcomes.Defect.out_of_memory };
            };
            context.* = .{ .allocator = env.allocator, .io = env.io, .registry = owner, .key = copied, .ticket = ticket };
            return .{ .success = context };
        }
        fn use(ctx: *C, _: *const R) Out {
            const Body = effects.Effect(A, E, R);
            const Frame = struct {
                const Self = @This();
                allocator: std.mem.Allocator,
                parent: *Out.RuntimeAdapter.Job,
                context: *C,
                child: Body.RuntimeAdapter.Job = undefined,
                value: outcomes.Exit(A, E) = undefined,
                fn complete(state: *rt.State, data: ?*anyopaque) rt.Step {
                    const frame: *Self = @ptrCast(@alignCast(data.?));
                    const parent = frame.parent;
                    const active = frame.context;
                    const evaluated = frame.value;
                    if (evaluated == .success and state.cancelled()) {
                        var abandoned = evaluated.success;
                        ownership.discard(&abandoned);
                        frame.allocator.destroy(frame);
                        return parent.adapterFinish(.interrupted);
                    }
                    frame.allocator.destroy(frame);
                    if (evaluated != .success) return parent.adapterFinish(outcomes.castFailure(Published(A), evaluated));
                    active.mutex.lockUncancelable(active.io);
                    defer active.mutex.unlock(active.io);
                    active.active = false;
                    if (active.conflicted) {
                        var abandoned = evaluated.success;
                        ownership.discard(&abandoned);
                        return parent.adapterFinish(.{ .success = .{ .value = .stale } });
                    }
                    const publication = active.registry.publishEvaluation(active.key, active.ticket, active.observations.items);
                    if (publication != .success) {
                        var abandoned = evaluated.success;
                        ownership.discard(&abandoned);
                        return parent.adapterFinish(outcomes.mapError(E, outcomes.castFailure(Published(A), publication), parent.env.allocator, map_registry_error));
                    }
                    if (publication.success == .stale) {
                        var abandoned = evaluated.success;
                        ownership.discard(&abandoned);
                        return parent.adapterFinish(.{ .success = .{ .value = .stale } });
                    }
                    // No allocation or generated factory follows the commit.
                    // Protected cleanup carries this acknowledged domain commit
                    // to the operation's completion boundary.
                    state.committed_delivery = true;
                    return parent.adapterFinish(.{ .success = .{ .value = .{ .current = evaluated.success }, .update = publication.success.committed } });
                }
            };
            return .{ .value = .{ .borrowed = ctx }, .start = struct {
                fn start(state: *rt.State, parent: *Out.RuntimeAdapter.Job) rt.Step {
                    const context: *C = @ptrCast(@alignCast(@constCast(parent.program.value.borrowed)));
                    const frame = parent.env.allocator.create(Frame) catch return parent.adapterFinish(.{ .die = outcomes.Defect.out_of_memory });
                    frame.* = .{ .allocator = parent.env.allocator, .parent = parent, .context = context };
                    const body = program.lower(context, parent.env);
                    frame.child = .{ .program = body, .owned = body.ownership, .env = parent.env, .inherited = state.services, .sink = .{ .output = &frame.value, .next_step = .{ .data = frame, .advance = Frame.complete } } };
                    return frame.child.adapterStep();
                }
            }.start };
        }

        fn release(ctx: *C, _: outcomes.PrimaryOutcome(E)) effects.Effect(void, E, R) {
            return ctx.close();
        }
    };
    return comptime Acquire.fromOutcome(Adapter.acquire).acquireRelease(Adapter.use, Adapter.release);
}
