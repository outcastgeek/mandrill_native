//! Opaque, reference-counted cancellation shared with Rust async operations.
pub const Token = opaque {};
pub extern fn native_cancel_create() callconv(.c) *Token;
pub extern fn native_cancel_retain(token: *const Token) callconv(.c) void;
pub extern fn native_cancel_release(token: *const Token) callconv(.c) void;
pub extern fn native_cancel_request(token: *const Token) callconv(.c) bool;
pub extern fn native_cancel_requested(token: *const Token) callconv(.c) bool;
pub extern fn native_cancel_finish(token: *const Token) callconv(.c) bool;
