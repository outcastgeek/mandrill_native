//! Compile-time atom declarations keep names, dependencies, write capability
//! and Effect result types checked at definition time.
const atoms = @import("atom.zig");
const effects = @import("../effect.zig");

/// Comptime atom builder for a stable identity, key selector, dependencies,
/// and typed read body. `readOnly` omits mutation; `writable` requires a
/// matching write body. Registry state remains scoped to the caller.
///
/// Parameters:
/// - `A: comptime type`: Value type produced by a successful computation.
/// - `E: comptime type`: Expected application-failure type; defects and interruption remain separate outcomes.
/// - `R: comptime type`: Caller-defined environment type that supplies dependencies and runtime capabilities when work executes.
/// Returns (`type`): staged atom definition for values `A`, failures `E`, and caller environment `R`.
pub fn AtomBuilder(comptime A: type, comptime E: type, comptime R: type) type {
    return struct {
        const Defined = atoms.Atom(A, E, R);

        /// Begins a versioned atom description without evaluating it or allocating registry state.
        ///
        /// Parameters:
        /// - `name: comptime []const u8`: Compile-time stable, nonempty identity component validated at definition
        ///   time.
        /// - `version: comptime u32`: Compile-time stable version used to reject incompatible encoded or
        ///   replayed data.
        /// Returns (`type`): atom stage carrying its stable name and version; supply the key selector next.
        pub fn begin(comptime name: []const u8, comptime version: u32) type {
            return struct {
                /// Projects the stable atom identity from R.
                ///
                /// Parameters:
                /// - `select_key: comptime fn (*const R) []const u8`: Compile-time typed selector from the current environment; its
                ///   result must remain valid through use.
                /// Returns (`type`): atom stage retaining the key selector; dependencies and read body can now be declared.
                pub fn key(comptime select_key: fn (*const R) []const u8) type {
                    return struct {
                        /// Declares dependency selectors used by the scoped registry for invalidation.
                        ///
                        /// Parameters:
                        /// - `dependencies: comptime []const Defined.KeySelector`: Compile-time the dependencies input required by this
                        ///   typed operation.
                        /// Returns (`type`): atom stage with its declared dependency tuple retained at comptime.
                        pub fn dependsOn(comptime dependencies: []const Defined.KeySelector) type {
                            return struct {
                                /// Supplies a typed read Effect; evaluation occurs only when the atom is read.
                                ///
                                /// Parameters:
                                /// - `read: comptime fn (*const R) effects.Effect(A, E, R)`: Compile-time typed function selected at comptime; no
                                ///   runtime callback lookup is required.
                                /// Returns (`type`): completed read-only atom definition with its typed Effect read body.
                                pub fn fromEffect(comptime read: fn (*const R) effects.Effect(A, E, R)) type {
                                    return struct {
                                        /// Completes an atom without a write operation.
                                        ///
                                        /// Returns (`Defined`): The completed statically checked definition from this builder.
                                        pub fn readOnly() Defined {
                                            return Defined.readOnly(name, version, select_key, dependencies, read);
                                        }

                                        /// Completes an atom with a typed write Effect for its declared value type.
                                        ///
                                        /// Parameters:
                                        /// - `write: comptime fn (*const A, *const R) effects.Effect(void, E, R)`: Compile-time typed function selected at
                                        ///   comptime; no runtime callback lookup is required.
                                        /// Returns (`Defined`): The completed statically checked definition from this builder.
                                        pub fn writable(comptime write: fn (*const A, *const R) effects.Effect(void, E, R)) Defined {
                                            return Defined.writable(name, version, select_key, dependencies, read, write);
                                        }
                                    };
                                }
                            };
                        }
                    };
                }
            };
        }

        /// Completes an atom without a write operation.
        ///
        /// Parameters:
        /// - `name: comptime []const u8`: Compile-time stable, nonempty identity component validated at definition
        ///   time.
        /// - `version: comptime u32`: Compile-time stable version used to reject incompatible encoded or
        ///   replayed data.
        /// - `key: comptime fn (*const R) []const u8`: Compile-time stable identity used for matching, deduplication, or replay.
        /// - `dependencies: comptime []const Defined.KeySelector`: Compile-time dependency selectors required to construct this component.
        /// - `read: comptime fn (*const R) @import("../effect.zig").Effect(A, E, R)`: Compile-time typed function selected at comptime; no runtime callback lookup
        ///   is required.
        /// Returns (`Defined`): The completed statically checked definition from this builder.
        pub fn readOnly(comptime name: []const u8, comptime version: u32, comptime key: fn (*const R) []const u8, comptime dependencies: []const Defined.KeySelector, comptime read: fn (*const R) @import("../effect.zig").Effect(A, E, R)) Defined {
            return Defined.readOnly(name, version, key, dependencies, read);
        }

        /// Completes an atom with a typed write Effect for its declared value type.
        ///
        /// Parameters:
        /// - `name: comptime []const u8`: Compile-time stable, nonempty identity component validated at definition
        ///   time.
        /// - `version: comptime u32`: Compile-time stable version used to reject incompatible encoded or
        ///   replayed data.
        /// - `key: comptime fn (*const R) []const u8`: Compile-time stable identity used for matching, deduplication, or replay.
        /// - `dependencies: comptime []const Defined.KeySelector`: Compile-time dependency selectors required to construct this component.
        /// - `read: comptime fn (*const R) @import("../effect.zig").Effect(A, E, R)`: Compile-time typed function selected at comptime; no runtime callback lookup
        ///   is required.
        /// - `write: comptime fn (*const A, *const R) @import("../effect.zig").Effect(void, E, R)`: Compile-time typed function selected at comptime; no runtime callback lookup
        ///   is required.
        /// Returns (`Defined`): The completed statically checked definition from this builder.
        pub fn writable(comptime name: []const u8, comptime version: u32, comptime key: fn (*const R) []const u8, comptime dependencies: []const Defined.KeySelector, comptime read: fn (*const R) @import("../effect.zig").Effect(A, E, R), comptime write: fn (*const A, *const R) @import("../effect.zig").Effect(void, E, R)) Defined {
            return Defined.writable(name, version, key, dependencies, read, write);
        }
    };
}
