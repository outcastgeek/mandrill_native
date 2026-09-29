//! Compile-time atom declarations keep names, dependencies, write capability
//! and Effect result types checked at definition time.
const atoms = @import("atom.zig");
const effects = @import("../effect.zig");

pub fn AtomBuilder(comptime A: type, comptime E: type, comptime R: type) type {
    return struct {
        const Defined = atoms.Atom(A, E, R);

        pub fn begin(comptime name: []const u8, comptime version: u32) type {
            return struct {
                pub fn key(comptime select_key: fn (*const R) []const u8) type {
                    return struct {
                        pub fn dependsOn(comptime dependencies: []const Defined.KeySelector) type {
                            return struct {
                                pub fn fromEffect(comptime read: fn (*const R) effects.Effect(A, E, R)) type {
                                    return struct {
                                        pub fn readOnly() Defined {
                                            return Defined.readOnly(name, version, select_key, dependencies, read);
                                        }

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

        pub fn readOnly(comptime name: []const u8, comptime version: u32, comptime key: fn (*const R) []const u8, comptime dependencies: []const Defined.KeySelector, comptime read: fn (*const R) @import("../effect.zig").Effect(A, E, R)) Defined {
            return Defined.readOnly(name, version, key, dependencies, read);
        }

        pub fn writable(comptime name: []const u8, comptime version: u32, comptime key: fn (*const R) []const u8, comptime dependencies: []const Defined.KeySelector, comptime read: fn (*const R) @import("../effect.zig").Effect(A, E, R), comptime write: fn (*const A, *const R) @import("../effect.zig").Effect(void, E, R)) Defined {
            return Defined.writable(name, version, key, dependencies, read, write);
        }
    };
}
