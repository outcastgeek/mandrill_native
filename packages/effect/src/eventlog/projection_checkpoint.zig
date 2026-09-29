//! Projection state and its journal cursor form one atomic checkpoint. A
//! projection without this capability may rebuild from journal history, but
//! it cannot claim an atomic durable state/cursor transition.
const effects = @import("../effect.zig");
const outcomes = @import("../outcome.zig");
const journal = @import("event_journal.zig");
const EventError = @import("event_log_error.zig").Error;

pub const Checkpoint = struct {
    pub const effect_owned_payload = true;
    cursor: journal.Cursor,
    state_schema_version: u32,
    encoded_state: outcomes.OwnedSlice(u8),

    pub fn deinit(self: *@This()) void {
        self.encoded_state.deinit();
        self.* = undefined;
    }

    pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
        return .{
            .cursor = self.cursor,
            .state_schema_version = self.state_schema_version,
            .encoded_state = try self.encoded_state.clone(),
        };
    }
};

pub const Revision = union(enum) { absent, present: journal.Cursor };
pub const CommitResult = union(enum) {
    committed: journal.Cursor,
    conflict: Revision,
    possibly_committed,
};

/// A compare-and-publish must validate the prior cursor and publish the next
/// encoded projection state and cursor indivisibly. The caller does not infer
/// this guarantee from a key/value store or from a notification channel.
/// Missing/compacted journal history remains a typed EventLog failure.
pub fn AtomicCheckpointStore(comptime R: type, comptime ProviderError: type) type {
    return struct {
        const Self = @This();
        pub const Failure = EventError(ProviderError);
        pub const ReadEffect = effects.Effect(?Checkpoint, Failure, R);
        pub const CommitEffect = effects.Effect(CommitResult, Failure, R);
        userdata: ?*anyopaque,
        vtable: *const VTable,

        pub const VTable = struct {
            read: *const fn (?*anyopaque, *const R, []const u8) ReadEffect,
            compare_and_publish: *const fn (?*anyopaque, *const R, []const u8, Revision, journal.Cursor, u32, []const u8) CommitEffect,
        };

        pub fn fromProvider(data: ?*anyopaque, vtable: *const VTable) Self {
            return .{ .userdata = data, .vtable = vtable };
        }

        pub fn read(comptime select_store: fn (*const R) *const Self, comptime select_projection_id: fn (*const R) []const u8) ReadEffect {
            return ReadEffect.deferEffect(struct {
                fn start(env: *const R) ReadEffect {
                    const id = select_projection_id(env);
                    if (id.len == 0) return ReadEffect.fail(.invalid_identity);
                    const store = select_store(env);
                    return store.vtable.read(store.userdata, env, id);
                }
            }.start);
        }

        pub fn compareAndPublish(
            comptime select_store: fn (*const R) *const Self,
            comptime select_projection_id: fn (*const R) []const u8,
            comptime select_expected: fn (*const R) Revision,
            comptime select_next: fn (*const R) journal.Cursor,
            comptime select_schema_version: fn (*const R) u32,
            comptime select_encoded_state: fn (*const R) []const u8,
        ) CommitEffect {
            return CommitEffect.deferEffect(struct {
                fn start(env: *const R) CommitEffect {
                    const id = select_projection_id(env);
                    if (id.len == 0) return CommitEffect.fail(.invalid_identity);
                    const store = select_store(env);
                    return store.vtable.compare_and_publish(
                        store.userdata,
                        env,
                        id,
                        select_expected(env),
                        select_next(env),
                        select_schema_version(env),
                        select_encoded_state(env),
                    );
                }
            }.start);
        }
    };
}
