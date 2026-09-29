//! Explicit rejection of weak-key cache semantics unavailable without GC ephemerons.
const diagnostics = @import("internal/diagnostics.zig");
/// Zig has no tracing garbage collector or ephemerons. Strong retention, raw
/// address keys and reference counts cannot reproduce the source's weak-key
/// semantics. Requesting that contract is an explicit compile-time rejection.
pub const Unsupported = enum { garbage_collector_ephemerons_required };
/// Explicit marker that weak-key semantics require GC ephemerons.
pub const support: Unsupported = .garbage_collector_ephemerons_required;
/// Reject weak-key cache construction at comptime because Zig has no GC ephemeron contract.
/// Parameters:
/// - `Key: comptime type`: Key type used to identify or look up entries in this component.
/// - `A: comptime type`: Value type produced by a successful computation.
/// - `E: comptime type`: Expected application-failure type; defects and interruption remain separate outcomes.
/// Returns (`type`): No type is produced; use always fails compilation with the missing ephemeron requirement.
pub fn WeakCache(comptime Key: type, comptime A: type, comptime E: type) type {
    _ = Key;
    _ = A;
    _ = E;
    @compileError(diagnostics.weakcache_requires_gc_ephemeron_semantics_which_this_zig_port_does_not_provide);
}
