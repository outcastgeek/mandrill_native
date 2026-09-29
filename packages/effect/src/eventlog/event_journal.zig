//! Transport-neutral event history contracts. Notifications can wake readers;
//! this journal and its cursors remain the authority for committed progress.
const outcomes = @import("../outcome.zig");
const effects = @import("../effect.zig");
const EventError = @import("event_log_error.zig").Error;

/// Owned bytes buffer paired with its allocator; release it with `deinit`.
pub const Bytes = outcomes.OwnedSlice(u8);
/// Sequence zero is the position before the first event; committed events
/// have consecutive positive sequences. `readAfter` excludes this cursor.
pub const Cursor = struct {
    sequence: u64,
    /// Cursor preceding the first journal event.
    pub const before_first: Cursor = .{ .sequence = 0 };
};

/// Encoded event and stable identity paired with its journal cursor.
pub const Entry = struct {
    /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements `deinit` and
    /// must be released after use.
    pub const effect_owned_payload = true;
    tag: Bytes,
    event_id: Bytes,
    encoded: Bytes,
    cursor: Cursor,
    schema_version: u32,

    /// Release resources owned by this value.
    /// Parameters:
    /// - `self: *@This()`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
    /// Returns (`void`): no value.
    pub fn deinit(self: *@This()) void {
        self.tag.deinit();
        self.event_id.deinit();
        self.encoded.deinit();
        self.* = undefined;
    }

    /// Duplicate this value; marked owning payloads are cloned, while ordinary
    /// borrowed fields retain their original lifetime.
    /// Parameters:
    /// - `self: *const @This()`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
    /// Returns (`error{OutOfMemory}!@This()`): copied value on success; `OutOfMemory` if owned storage could not be cloned. The source remains valid.
    pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
        var tag = try self.tag.clone();
        errdefer tag.deinit();
        var event_id = try self.event_id.clone();
        errdefer event_id.deinit();
        return .{ .tag = tag, .event_id = event_id, .encoded = try self.encoded.clone(), .cursor = self.cursor, .schema_version = self.schema_version };
    }
};

/// This is data accepted by a journal provider. Both slices remain borrowed
/// until the returned Effect settles; a provider copies what it retains.
pub const Proposal = struct {
    tag: []const u8,
    event_id: []const u8,
    encoded: []const u8,
    schema_version: u32,
};

/// Result of appending an event, including deduplication status.
pub const AppendAcknowledgement = union(enum) {
    committed: Cursor,
    accepted,
    rejected,
    uncertain,
};

/// Owned bounded page of entries with next cursor and continuation flag.
pub const ReadPage = struct {
    /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements `deinit` and
    /// must be released after use.
    pub const effect_owned_payload = true;
    entries: outcomes.OwnedSlice(Entry),
    next: Cursor,
    more: bool,

    /// Release resources owned by this value.
    /// Parameters:
    /// - `self: *@This()`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
    /// Returns (`void`): no value.
    pub fn deinit(self: *@This()) void {
        self.entries.deinit();
        self.* = undefined;
    }

    /// Duplicate this value; marked owning payloads are cloned, while ordinary
    /// borrowed fields retain their original lifetime.
    /// Parameters:
    /// - `self: *const @This()`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
    /// Returns (`error{OutOfMemory}!@This()`): copied value on success; `OutOfMemory` if owned storage could not be cloned. The source remains valid.
    pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
        return .{ .entries = try self.entries.clone(), .next = self.next, .more = self.more };
    }
};

/// Caller-selected limits on entries and encoded bytes returned per page.
pub const ReadPolicy = struct {
    /// The caller selects a strictly positive bound before admission.
    max_entries: usize,
    max_encoded_bytes: usize,

    /// Check whether this value satisfies its structural invariants.
    /// Parameters:
    /// - `self: ReadPolicy`: Receiver passed by value: the struct itself is copied, while any referenced storage keeps its original ownership and lifetime.
    /// Returns (`bool`): `true` when both page-entry and encoded-byte limits are positive.
    pub fn valid(self: ReadPolicy) bool {
        return self.max_entries > 0 and self.max_encoded_bytes > 0;
    }
};

/// A journal implementation may be volatile or retained. This interface
/// promises ordered append/read and explicit uncertainty, not physical
/// durability. A stronger projection/checkpoint atomic capability is separate.
///
/// Parameters:
/// - `R: comptime type`: Caller-defined environment type that supplies dependencies and runtime capabilities when work executes.
/// - `ProviderError: comptime type`: Failure type reported by the selected provider and retained in the typed error channel.
/// Returns (`type`): event journal capability for append and bounded ordered reads.
pub fn Journal(comptime R: type, comptime ProviderError: type) type {
    return struct {
        const Self = @This();
        /// Typed failure channel combining domain and provider errors.
        pub const Failure = EventError(ProviderError);
        /// Lazy Effect for append effect; success, error, and environment types are fixed by
        /// `effects.Effect(AppendAcknowledgement, Failure, R)`.
        pub const AppendEffect = effects.Effect(AppendAcknowledgement, Failure, R);
        /// Lazy Effect for read effect; success, error, and environment types are fixed by
        /// `effects.Effect(ReadPage, Failure, R)`.
        pub const ReadEffect = effects.Effect(ReadPage, Failure, R);
        /// Lazy Effect for lookup effect; success, error, and environment types are fixed by
        /// `effects.Effect(?Entry, Failure, R)`.
        pub const LookupEffect = effects.Effect(?Entry, Failure, R);
        userdata: ?*anyopaque,
        vtable: *const VTable,

        /// Typed function-pointer interface for this capability. Implement every
        /// hook with the exact signature below; keep the table valid while the
        /// capability or pending work can invoke it.
        pub const VTable = struct {
            /// Append through this provider.
            /// Parameters (callback order):
            /// - `1: ?*anyopaque`: opaque provider or handle context; keep it live until the hook and any retained work complete.
            /// - `2: *const R`: borrowed generic environment `R`, including caller-supplied runtime services.
            /// - `3: Proposal`: typed domain value consumed by this hook.
            /// Returns (`AppendEffect`): lazy typed Effect; run it with its declared environment to obtain Exit.
            append: *const fn (?*anyopaque, *const R, Proposal) AppendEffect,
            /// Read after through this provider.
            /// Parameters (callback order):
            /// - `1: ?*anyopaque`: opaque provider or handle context; keep it live until the hook and any retained work complete.
            /// - `2: *const R`: borrowed generic environment `R`, including caller-supplied runtime services.
            /// - `3: Cursor`: typed domain value consumed by this hook.
            /// - `4: ReadPolicy`: caller-selected policy controlling this operation.
            /// Returns (`ReadEffect`): lazy typed Effect; run it with its declared environment to obtain Exit.
            read_after: *const fn (?*anyopaque, *const R, Cursor, ReadPolicy) ReadEffect,
            /// Find event through this provider.
            /// Parameters (callback order):
            /// - `1: ?*anyopaque`: opaque provider or handle context; keep it live until the hook and any retained work complete.
            /// - `2: *const R`: borrowed generic environment `R`, including caller-supplied runtime services.
            /// - `3: []const u8`: borrowed input slice; keep it valid through deferred use or copy before retaining it.
            /// Returns (`LookupEffect`): lazy typed Effect; run it with its declared environment to obtain Exit.
            find_event: *const fn (?*anyopaque, *const R, []const u8) LookupEffect,
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

        /// Append an event to the journal.
        /// Parameters:
        /// - `select_journal: comptime fn (*const R) *const Self`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// - `select_proposal: comptime fn (*const R) Proposal`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// Returns (`AppendEffect`): journal cursor or typed append failure after atomic identity checking.
        pub fn append(comptime select_journal: fn (*const R) *const Self, comptime select_proposal: fn (*const R) Proposal) AppendEffect {
            return AppendEffect.deferEffect(struct {
                fn start(env: *const R) AppendEffect {
                    const proposal = select_proposal(env);
                    if (proposal.event_id.len == 0 or proposal.tag.len == 0)
                        return AppendEffect.fromOwnedOutcome(env, .{ .fail = .invalid_identity });
                    const journal = select_journal(env);
                    return journal.vtable.append(journal.userdata, env, proposal);
                }
            }.start);
        }

        /// Read journal entries after the supplied cursor.
        /// Parameters:
        /// - `select_journal: comptime fn (*const R) *const Self`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// - `select_cursor: comptime fn (*const R) Cursor`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_policy: comptime fn (*const R) ReadPolicy`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// Returns (`ReadEffect`): bounded page of entries after the supplied cursor.
        pub fn readAfter(
            comptime select_journal: fn (*const R) *const Self,
            comptime select_cursor: fn (*const R) Cursor,
            comptime select_policy: fn (*const R) ReadPolicy,
        ) ReadEffect {
            return ReadEffect.deferEffect(struct {
                fn start(env: *const R) ReadEffect {
                    const policy = select_policy(env);
                    if (!policy.valid()) return ReadEffect.fromOwnedOutcome(env, .{ .fail = .invalid_policy });
                    const journal = select_journal(env);
                    return journal.vtable.read_after(journal.userdata, env, select_cursor(env), policy);
                }
            }.start);
        }

        /// Find an event by its stable identity.
        /// Parameters:
        /// - `select_journal: comptime fn (*const R) *const Self`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// - `select_id: comptime fn (*const R) []const u8`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// Returns (`LookupEffect`): stored entry for the stable event identity, if present.
        pub fn findEvent(comptime select_journal: fn (*const R) *const Self, comptime select_id: fn (*const R) []const u8) LookupEffect {
            return LookupEffect.deferEffect(struct {
                fn start(env: *const R) LookupEffect {
                    const id = select_id(env);
                    if (id.len == 0) return LookupEffect.fromOwnedOutcome(env, .{ .fail = .invalid_identity });
                    const journal = select_journal(env);
                    return journal.vtable.find_event(journal.userdata, env, id);
                }
            }.start);
        }
    };
}
