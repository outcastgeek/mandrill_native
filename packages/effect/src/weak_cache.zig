const diagnostics = @import("internal/diagnostics.zig");
/// Zig has no tracing garbage collector or ephemerons. Strong retention, raw
/// address keys and reference counts cannot reproduce the source's weak-key
/// semantics. Requesting that contract is an explicit compile-time rejection.
pub const Unsupported = enum { garbage_collector_ephemerons_required };
pub const support: Unsupported = .garbage_collector_ephemerons_required;
pub fn WeakCache(comptime Key: type, comptime A: type, comptime E: type) type {
    _ = Key;
    _ = A;
    _ = E;
    @compileError(diagnostics.weakcache_requires_gc_ephemeron_semantics_which_this_zig_port_does_not_provide);
}
