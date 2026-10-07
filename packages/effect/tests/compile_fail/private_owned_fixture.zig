pub const Payload = struct {
    const effect_owned_payload = true;
    pub fn clone(_: *const @This()) error{OutOfMemory}!@This() {
        return .{};
    }
    pub fn deinit(_: *@This()) void {}
};
