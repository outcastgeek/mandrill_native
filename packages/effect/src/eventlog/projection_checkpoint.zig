//! Projection state and its journal cursor form one atomic checkpoint. A
//! projection without this capability may rebuild from journal history, but
//! it cannot claim an atomic durable state/cursor transition.
const effects = @import("../effect.zig");
const outcomes = @import("../outcome.zig");
const journal = @import("event_journal.zig");
const EventError = @import("event_log_error.zig").Error;

/// Projection state and schema version paired with its replay cursor.
pub const Checkpoint = struct {
    /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements `deinit` and
    /// must be released after use.
    pub const effect_owned_payload = true;
    cursor: journal.Cursor,
    state_schema_version: u32,
    encoded_state: outcomes.OwnedSlice(u8),

    /// Release resources owned by this value.
    /// Parameters:
    /// - `self: *@This()`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
    /// Returns (`void`): no value.
    pub fn deinit(self: *@This()) void {
        self.encoded_state.deinit();
        self.* = undefined;
    }

    /// Duplicate this value; marked owning payloads are cloned, while ordinary
    /// borrowed fields retain their original lifetime.
    /// Parameters:
    /// - `self: *const @This()`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
    /// Returns (`error{OutOfMemory}!@This()`): copied value on success; `OutOfMemory` if owned storage could not be cloned. The source remains valid.
    pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
        return .{
            .cursor = self.cursor,
            .state_schema_version = self.state_schema_version,
            .encoded_state = try self.encoded_state.clone(),
        };
    }
};

/// Version marker for one stored projection checkpoint.
pub const Revision = union(enum) { absent, present: journal.Cursor };
/// Atomic checkpoint publication result, distinguishing applied from conflict.
pub const CommitResult = union(enum) {
    committed: journal.Cursor,
    conflict: Revision,
    possibly_committed,
};

/// A compare-and-publish must validate the prior cursor and publish the next
/// encoded projection state and cursor indivisibly. The caller does not infer
/// this guarantee from a key/value store or from a notification channel.
/// Missing/compacted journal history remains a typed EventLog failure.
///
/// Parameters:
/// - `R: comptime type`: Caller-defined environment type that supplies dependencies and runtime capabilities when work executes.
/// - `ProviderError: comptime type`: Failure type reported by the selected provider and retained in the typed error channel.
/// Returns (`type`): provider for atomic projection-state and cursor publication.
pub fn AtomicCheckpointStore(comptime R: type, comptime ProviderError: type) type {
    return struct {
        const Self = @This();
        /// Typed failure channel combining domain and provider errors.
        pub const Failure = EventError(ProviderError);
        /// Lazy Effect for read effect; success, error, and environment types are fixed by
        /// `effects.Effect(?Checkpoint, Failure, R)`.
        pub const ReadEffect = effects.Effect(?Checkpoint, Failure, R);
        /// Lazy Effect for commit effect; success, error, and environment types are fixed by
        /// `effects.Effect(CommitResult, Failure, R)`.
        pub const CommitEffect = effects.Effect(CommitResult, Failure, R);
        userdata: ?*anyopaque,
        vtable: *const VTable,

        /// Typed function-pointer interface for this capability. Implement every
        /// hook with the exact signature below; keep the table valid while the
        /// capability or pending work can invoke it.
        pub const VTable = struct {
            /// Read the selected value.
            /// Parameters (callback order):
            /// - `1: ?*anyopaque`: opaque provider or handle context; keep it live until the hook and any retained work complete.
            /// - `2: *const R`: borrowed generic environment `R`, including caller-supplied runtime services.
            /// - `3: []const u8`: borrowed input slice; keep it valid through deferred use or copy before retaining it.
            /// Returns (`ReadEffect`): lazy typed Effect; run it with its declared environment to obtain Exit.
            read: *const fn (?*anyopaque, *const R, []const u8) ReadEffect,
            /// Compare and publish through this provider.
            /// Parameters (callback order):
            /// - `1: ?*anyopaque`: opaque provider or handle context; keep it live until the hook and any retained work complete.
            /// - `2: *const R`: borrowed generic environment `R`, including caller-supplied runtime services.
            /// - `3: []const u8`: borrowed input slice; keep it valid through deferred use or copy before retaining it.
            /// - `4: Revision`: typed domain value consumed by this hook.
            /// - `5: journal.Cursor`: typed domain value consumed by this hook.
            /// - `6: u32`: typed input consumed by this hook.
            /// - `7: []const u8`: borrowed input slice; keep it valid through deferred use or copy before retaining it.
            /// Returns (`CommitEffect`): lazy typed Effect; run it with its declared environment to obtain Exit.
            compare_and_publish: *const fn (?*anyopaque, *const R, []const u8, Revision, journal.Cursor, u32, []const u8) CommitEffect,
        };

        /// Wrap a caller-supplied provider implementation.
        /// Parameters:
        /// - `data: ?*anyopaque`: Borrowed provider context passed unchanged to every hook.
        ///   Its owner keeps it alive through any pending effect or operation.
        /// - `vtable: *const VTable`: Borrowed table of exactly typed callbacks; keep it
        ///   alive for every use of this capability, including deferred execution.
        /// Returns (`Self`): A non-owning adapter referencing `data` and `vtable`; no
        ///   provider state is allocated or transferred by this constructor.
        pub fn fromProvider(data: ?*anyopaque, vtable: *const VTable) Self {
            return .{ .userdata = data, .vtable = vtable };
        }

        /// Read through the selected capability.
        /// Parameters:
        /// - `select_store: comptime fn (*const R) *const Self`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_projection_id: comptime fn (*const R) []const u8`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// Returns (`ReadEffect`): stored projection state and cursor, if a checkpoint exists.
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

        /// Publish a checkpoint only when the expected revision matches.
        /// Parameters:
        /// - `select_store: comptime fn (*const R) *const Self`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_projection_id: comptime fn (*const R) []const u8`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// - `select_expected: comptime fn (*const R) Revision`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// - `select_next: comptime fn (*const R) journal.Cursor`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_schema_version: comptime fn (*const R) u32`: Compile-time typed selector from the current environment;
        ///   its result must remain valid through use.
        /// - `select_encoded_state: comptime fn (*const R) []const u8`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// Returns (`CommitEffect`): applied revision or conflict after atomically comparing the previous cursor.
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
