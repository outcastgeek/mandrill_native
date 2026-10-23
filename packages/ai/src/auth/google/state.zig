const std = @import("std");
/// Metadata availability is shared by this Models owner, without process-wide
/// state or a background task. The probing caller owns and joins its I/O.
pub const State = struct {
    mutex: std.Io.Mutex = .init,
    changed: std.Io.Condition = .init,
    checking: bool = false,
    availability: ?bool = null,
    residency: ?bool = null,
};
