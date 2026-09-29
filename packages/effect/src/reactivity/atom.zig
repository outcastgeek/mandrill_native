//! Inert, typed atom descriptions. They reuse Effect for evaluation and the
//! scoped registry for dependency/invalidation state; descriptions themselves
//! allocate nothing and carry no process-global runtime.
const std = @import("std");
const effects = @import("../effect.zig");
const outcomes = @import("../outcome.zig");
const registry = @import("atom_registry.zig");
const invalidation = @import("invalidation_registry.zig");

/// Typed atom mutation result retaining commit and invalidation status.
pub const Mutation = union(enum) {
    committed: invalidation.InvalidateResult,
    committed_invalidation_failed: registry.Error,
    committed_invalidation_defect: outcomes.Defect,
    committed_invalidation_interrupted,
};

/// Specialize a reactive atom with a typed value and environment.
/// Parameters:
/// - `A: comptime type`: Value type produced by a successful computation.
/// - `E: comptime type`: Expected application-failure type; defects and interruption remain separate outcomes.
/// - `R: comptime type`: Caller-defined environment type that supplies dependencies and runtime capabilities when work executes.
/// Returns (`type`): typed reactive atom definition with stable key and read/write capabilities.
pub fn Atom(comptime A: type, comptime E: type, comptime R: type) type {
    return struct {
        const Self = @This();
        /// Value type carried by the successful channel.
        pub const Success = A;
        /// Domain-specific error type carried by the operation.
        pub const DomainError = E;
        /// Exact caller environment type required when the description runs.
        pub const Environment = R;
        /// Typed selector from `R` to a stable atom key.
        pub const KeySelector = *const fn (*const R) []const u8;

        name: []const u8,
        version: u32,
        select_key: KeySelector,
        dependencies: []const KeySelector,
        read_fn: *const fn (*const R) effects.Effect(A, E, R),
        write_fn: ?*const fn (*const A, *const R) effects.Effect(void, E, R),

        /// Build a read-only atom interface.
        /// Parameters:
        /// - `name: comptime []const u8`: Compile-time stable, nonempty identity component validated at definition
        ///   time.
        /// - `version: comptime u32`: Compile-time stable version used to reject incompatible encoded or
        ///   replayed data.
        /// - `select_key: comptime fn (*const R) []const u8`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `dependencies: comptime []const KeySelector`: Compile-time dependency selectors required to construct this component.
        /// - `read_operation: comptime fn (*const R) effects.Effect(A, E, R)`: Compile-time typed function selected at comptime; no runtime
        ///   callback lookup is required.
        /// Returns (`Self`): atom definition with a typed read operation and no mutation operation.
        pub fn readOnly(comptime name: []const u8, comptime version: u32, comptime select_key: fn (*const R) []const u8, comptime dependencies: []const KeySelector, comptime read_operation: fn (*const R) effects.Effect(A, E, R)) Self {
            const definition: Self = .{ .name = name, .version = version, .select_key = select_key, .dependencies = dependencies, .read_fn = read_operation, .write_fn = null };
            definition.validate();
            return definition;
        }

        /// Validate the value against its declared contract.
        /// Parameters:
        /// - `self: comptime Self`: Compile-time current value passed by value; ownership follows this
        ///   operation’s contract.
        /// Returns (`void`): no value.
        pub fn validate(comptime self: Self) void {
            if (comptime self.name.len == 0 or !std.unicode.utf8ValidateSlice(self.name))
                @compileError("atom names must be stable nonempty UTF-8");
            if (comptime self.version == 0) @compileError("atom version must be nonzero");
        }

        /// Build an atom interface with typed updates.
        /// Parameters:
        /// - `name: comptime []const u8`: Compile-time stable, nonempty identity component validated at definition
        ///   time.
        /// - `version: comptime u32`: Compile-time stable version used to reject incompatible encoded or
        ///   replayed data.
        /// - `select_key: comptime fn (*const R) []const u8`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `dependencies: comptime []const KeySelector`: Compile-time dependency selectors required to construct this component.
        /// - `read_operation: comptime fn (*const R) effects.Effect(A, E, R)`: Compile-time typed function selected at comptime; no runtime
        ///   callback lookup is required.
        /// - `write_operation: comptime fn (*const A, *const R) effects.Effect(void, E, R)`: Compile-time typed function selected at comptime; no runtime
        ///   callback lookup is required.
        /// Returns (`Self`): atom definition with matching typed read and write operations.
        pub fn writable(comptime name: []const u8, comptime version: u32, comptime select_key: fn (*const R) []const u8, comptime dependencies: []const KeySelector, comptime read_operation: fn (*const R) effects.Effect(A, E, R), comptime write_operation: fn (*const A, *const R) effects.Effect(void, E, R)) Self {
            var result = readOnly(name, version, select_key, dependencies, read_operation);
            result.write_fn = write_operation;
            return result;
        }

        /// Register a definition or subscription with its owner.
        /// Parameters:
        /// - `self: comptime Self`: Compile-time current value passed by value; ownership follows this
        ///   operation’s contract.
        /// - `select_registry: comptime fn (*const R) *registry.Registry`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// Returns (`effects.Effect(registry.DependencyUpdate, registry.Error, R)`): typed atom definition registered under its stable identity.
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

        /// Read through the selected capability.
        /// Parameters:
        /// - `self: comptime Self`: Compile-time current value passed by value; ownership follows this
        ///   operation’s contract.
        /// Returns (`effects.Effect(A, E, R)`): current atom value read through its typed Effect body.
        pub fn read(comptime self: Self) effects.Effect(A, E, R) {
            self.validate();
            const Program = effects.Effect(A, E, R);
            return Program.deferEffect(struct {
                fn start(env: *const R) Program {
                    return self.read_fn(env);
                }
            }.start);
        }

        /// Write through the selected capability.
        /// Parameters:
        /// - `self: comptime Self`: Compile-time current value passed by value; ownership follows this
        ///   operation’s contract.
        /// - `select_value: comptime fn (*const R) *const A`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_registry: comptime fn (*const R) *registry.Registry`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// Returns (`effects.Effect(Mutation, E, R)`): typed mutation result from the writable atom body.
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
        ///
        /// Parameters:
        /// - `self: comptime Self`: Compile-time current value passed by value; ownership follows this
        ///   operation’s contract.
        /// - `B: comptime type`: Success value type after this operation transforms the original value.
        /// - `name: comptime []const u8`: Compile-time stable, nonempty identity component validated at definition
        ///   time.
        /// - `version: comptime u32`: Compile-time stable version used to reject incompatible encoded or
        ///   replayed data.
        /// - `select_derived_key: comptime fn (*const R) []const u8`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// - `transform: comptime fn (A) B`: Compile-time typed callback; its signature is checked at comptime where
        ///   static.
        /// Returns (`Atom(B, E, R)`): A composed description whose evaluation remains deferred.
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
