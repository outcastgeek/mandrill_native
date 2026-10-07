//! Invocation-local traversal state for structural codecs. Recursive Box
//! nodes account for depth; no thread-local or shared mutable schema state.
const std = @import("std");

/// Decoder traversal state and resource counters for structural schemas.
pub const DecodeState = struct {
    depth: usize = 0,
    maximum_depth: ?usize = null,
    maximum_issues: ?usize = null,

    /// Enter a structural node while tracking recursion and depth.
    /// Parameters:
    /// - `self: *DecodeState`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
    /// - `max_depth: usize`: Maximum nesting depth accepted by the parser.
    /// Returns (`bool`): `true` after incrementing depth below `max_depth`; `false` without entering at the bound.
    pub fn enter(self: *DecodeState, max_depth: usize) bool {
        if (self.depth >= @min(max_depth, self.maximum_depth orelse max_depth)) return false;
        self.depth += 1;
        return true;
    }

    /// Leave the current structural node and release traversal state.
    /// Parameters:
    /// - `self: *DecodeState`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
    /// Returns (`void`): no value.
    pub fn leave(self: *DecodeState) void {
        self.depth -= 1;
    }
};

/// Encoder traversal state and resource counters for structural schemas.
pub const EncodeState = struct {
    scratch: std.mem.Allocator,
    depth: usize = 0,
    maximum_depth: ?usize = null,

    /// Enter a structural node while tracking recursion and depth.
    /// Parameters:
    /// - `self: *EncodeState`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
    /// - `max_depth: usize`: Maximum nesting depth accepted by the parser.
    /// Returns (`bool`): `true` after incrementing depth below `max_depth`; `false` without entering at the bound.
    pub fn enter(self: *EncodeState, max_depth: usize) bool {
        if (self.depth >= @min(max_depth, self.maximum_depth orelse max_depth)) return false;
        self.depth += 1;
        return true;
    }

    /// Leave the current structural node and release traversal state.
    /// Parameters:
    /// - `self: *EncodeState`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
    /// Returns (`void`): no value.
    pub fn leave(self: *EncodeState) void {
        self.depth -= 1;
    }
};
