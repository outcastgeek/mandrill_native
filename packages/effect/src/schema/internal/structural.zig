//! Invocation-local traversal state for structural codecs. Recursive Box
//! nodes account for depth; no thread-local or shared mutable schema state.
const std = @import("std");

pub const DecodeState = struct {
    depth: usize = 0,

    pub fn enter(self: *DecodeState, max_depth: usize) bool {
        if (self.depth >= max_depth) return false;
        self.depth += 1;
        return true;
    }

    pub fn leave(self: *DecodeState) void {
        self.depth -= 1;
    }
};

pub const EncodeState = struct {
    scratch: std.mem.Allocator,
    depth: usize = 0,

    pub fn enter(self: *EncodeState, max_depth: usize) bool {
        if (self.depth >= max_depth) return false;
        self.depth += 1;
        return true;
    }

    pub fn leave(self: *EncodeState) void {
        self.depth -= 1;
    }
};
