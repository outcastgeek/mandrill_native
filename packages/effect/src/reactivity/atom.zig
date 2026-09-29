//! Inert, typed atom descriptions. They reuse Effect for evaluation and the
//! scoped registry for dependency/invalidation state; descriptions themselves
//! allocate nothing and carry no process-global runtime.
const std = @import("std");
const effects = @import("../effect.zig");
const outcomes = @import("../outcome.zig");
const registry = @import("atom_registry.zig");
const invalidation = @import("invalidation_registry.zig");

pub const Mutation = union(enum) {
    committed: invalidation.InvalidateResult,
    committed_invalidation_failed: registry.Error,
    committed_invalidation_defect: outcomes.Defect,
    committed_invalidation_interrupted,
};

pub fn Atom(comptime A: type, comptime E: type, comptime R: type) type {
    return struct {
        const Self = @This();
        pub const Success = A;
        pub const DomainError = E;
        pub const Environment = R;
        pub const KeySelector = *const fn (*const R) []const u8;

        name: []const u8,
        version: u32,
        select_key: KeySelector,
        dependencies: []const KeySelector,
        read_fn: *const fn (*const R) effects.Effect(A, E, R),
        write_fn: ?*const fn (*const A, *const R) effects.Effect(void, E, R),

        pub fn readOnly(comptime name: []const u8, comptime version: u32, comptime select_key: fn (*const R) []const u8, comptime dependencies: []const KeySelector, comptime read_operation: fn (*const R) effects.Effect(A, E, R)) Self {
            const definition: Self = .{ .name = name, .version = version, .select_key = select_key, .dependencies = dependencies, .read_fn = read_operation, .write_fn = null };
            definition.validate();
            return definition;
        }

        pub fn validate(comptime self: Self) void {
            if (comptime self.name.len == 0 or !std.unicode.utf8ValidateSlice(self.name))
                @compileError("atom names must be stable nonempty UTF-8");
            if (comptime self.version == 0) @compileError("atom version must be nonzero");
        }

        pub fn writable(comptime name: []const u8, comptime version: u32, comptime select_key: fn (*const R) []const u8, comptime dependencies: []const KeySelector, comptime read_operation: fn (*const R) effects.Effect(A, E, R), comptime write_operation: fn (*const A, *const R) effects.Effect(void, E, R)) Self {
            var result = readOnly(name, version, select_key, dependencies, read_operation);
            result.write_fn = write_operation;
            return result;
        }

        pub fn register(comptime self: Self, comptime select_registry: fn (*const R) *registry.Registry) effects.Effect(registry.DependencyUpdate, registry.Error, R) {
            self.validate();
            const Program = effects.Effect(registry.DependencyUpdate, registry.Error, R);
            return Program.fromOutcome(struct {
                fn run(env: *const R) outcomes.Exit(registry.DependencyUpdate, registry.Error) {
                    var selected: [self.dependencies.len][]const u8 = undefined;
                    inline for (self.dependencies, 0..) |choose, index|
                        selected[index] = choose(env);
                    return select_registry(env).replaceDependencies(self.select_key(env), &selected);
                }
            }.run);
        }

        pub fn read(comptime self: Self) effects.Effect(A, E, R) {
            self.validate();
            const Program = effects.Effect(A, E, R);
            return Program.deferEffect(struct {
                fn start(env: *const R) Program {
                    return self.read_fn(env);
                }
            }.start);
        }

        pub fn write(comptime self: Self, comptime select_value: fn (*const R) *const A, comptime select_registry: fn (*const R) *registry.Registry) effects.Effect(Mutation, E, R) {
            self.validate();
            if (self.write_fn == null) @compileError("a read-only atom cannot be written");
            const Program = effects.Effect(Mutation, E, R);
            return Program.deferEffect(struct {
                fn start(env: *const R) Program {
                    var written = self.write_fn.?(select_value(env), env);
                    return written.flatMapOwned(env, struct {
                        fn afterCommit(_: void, supplied: *const R) Program {
                            const notice = select_registry(supplied).invalidate(self.select_key(supplied));
                            const result: Mutation = switch (notice) {
                                .success => |value| .{ .committed = value },
                                .fail => |problem| .{ .committed_invalidation_failed = problem },
                                .die => |defect| .{ .committed_invalidation_defect = defect },
                                .interrupted => .committed_invalidation_interrupted,
                                .finalization_failed => unreachable,
                            };
                            return Program.fromOwnedSuccess(supplied, result);
                        }
                    }.afterCommit);
                }
            }.start);
        }

        /// The derived atom's direct edge is the source atom. Transitive
        /// dependents are computed by the scoped registry, so no copied graph
        /// of runtime closures is placed in this description.
        pub fn map(comptime self: Self, comptime B: type, comptime name: []const u8, comptime version: u32, comptime select_derived_key: fn (*const R) []const u8, comptime transform: fn (A) B) Atom(B, E, R) {
            self.validate();
            return Atom(B, E, R).readOnly(name, version, select_derived_key, &.{self.select_key}, struct {
                fn evaluate(env: *const R) effects.Effect(B, E, R) {
                    var source = self.read_fn(env);
                    return source.mapOwned(env, transform);
                }
            }.evaluate);
        }
    };
}
