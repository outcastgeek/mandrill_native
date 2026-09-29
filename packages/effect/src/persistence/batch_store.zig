//! Ordered batch storage capability. A provider may stop at an item boundary;
//! every request retains its original index, and unattempted items remain
//! distinguishable from missing records and failed operations.
const std = @import("std");
const outcomes = @import("../outcome.zig");
const effects = @import("../effect.zig");
const storage = @import("key_value_store.zig");
const PersistenceError = @import("persistence_error.zig").Error;

/// Specialize one typed item within a batch-store request.
/// Parameters:
/// - `A: comptime type`: Value type produced by a successful computation.
/// - `E: comptime type`: Expected application-failure type; defects and interruption remain separate outcomes.
/// Returns (`type`): one typed item in a batch-store request.
pub fn Item(comptime A: type, comptime E: type) type {
    return union(enum) {
        const Self = @This();
        /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements `deinit`
        /// and must be released after use.
        pub const effect_owned_payload = true;
        not_attempted,
        attempted: outcomes.Exit(A, E),

        /// Release resources owned by this value.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// Returns (`void`): no value.
        pub fn deinit(self: *Self) void {
            if (self.* == .attempted) self.attempted.deinit();
            self.* = undefined;
        }

        /// Duplicate this value; marked owning payloads are cloned, while ordinary
        /// borrowed fields retain their original lifetime.
        /// Parameters:
        /// - `self: *const Self`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
        /// Returns (`error{OutOfMemory}!Self`): copied value on success; `OutOfMemory` if owned storage could not be cloned. The source remains valid.
        pub fn clone(self: *const Self) error{OutOfMemory}!Self {
            return switch (self.*) {
                .not_attempted => .not_attempted,
                .attempted => |value| .{ .attempted = value.clone() },
            };
        }
    };
}

/// Specialize the typed report outcome for persistence operations.
/// Parameters:
/// - `A: comptime type`: Value type produced by a successful computation.
/// - `E: comptime type`: Expected application-failure type; defects and interruption remain separate outcomes.
/// Returns (`type`): ordered partial-progress report from a batch-store operation.
pub fn Report(comptime A: type, comptime E: type) type {
    return struct {
        const Self = @This();
        /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements `deinit`
        /// and must be released after use.
        pub const effect_owned_payload = true;
        items: []Item(A, E),
        allocator: std.mem.Allocator,

        /// A conforming provider returns exactly one result per input, in
        /// request order. Once it stops, all following entries are unattempted.
        ///
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// Returns (`void`): no value.
        pub fn deinit(self: *Self) void {
            for (self.items) |*item| item.deinit();
            self.allocator.free(self.items);
            self.* = undefined;
        }

        /// Duplicate this value; marked owning payloads are cloned, while ordinary
        /// borrowed fields retain their original lifetime.
        /// Parameters:
        /// - `self: *const Self`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
        /// Returns (`error{OutOfMemory}!Self`): copied value on success; `OutOfMemory` if owned storage could not be cloned. The source remains valid.
        pub fn clone(self: *const Self) error{OutOfMemory}!Self {
            const copied = try self.allocator.alloc(Item(A, E), self.items.len);
            var count: usize = 0;
            errdefer {
                for (copied[0..count]) |*item| item.deinit();
                self.allocator.free(copied);
            }
            for (self.items) |*item| {
                copied[count] = try item.clone();
                count += 1;
            }
            return .{ .items = copied, .allocator = self.allocator };
        }
    };
}

/// One keyed, encoded write in an ordered batch request.
pub const Write = struct {
    key: []const u8,
    bytes: []const u8,
    expiry: storage.Expiry,
};

/// This is an ordered partial-progress capability, not an atomic multi-key
/// transaction. Failure of one item cannot be represented as whole-batch
/// success; the report records the failure at its exact index. Borrowed input
/// slices remain valid until the returned Effect completes.
///
/// Parameters:
/// - `R: comptime type`: Caller-defined environment type that supplies dependencies and runtime capabilities when work executes.
/// - `ProviderError: comptime type`: Failure type reported by the selected provider and retained in the typed error channel.
/// Returns (`type`): provider contract for bounded ordered batch persistence.
pub fn BatchStore(comptime R: type, comptime ProviderError: type) type {
    return struct {
        const Self = @This();
        /// Typed failure channel combining domain and provider errors.
        pub const Failure = PersistenceError(ProviderError);
        /// Per-item read report for a batch-store operation.
        pub const ReadReport = Report(storage.Read, Failure);
        /// Per-item write report for a batch-store operation.
        pub const WriteReport = Report(storage.WriteAcknowledgement, Failure);
        /// Lazy Effect for read effect; success, error, and environment types are fixed by
        /// `effects.Effect(ReadReport, Failure, R)`.
        pub const ReadEffect = effects.Effect(ReadReport, Failure, R);
        /// Lazy Effect for write effect; success, error, and environment types are fixed by
        /// `effects.Effect(WriteReport, Failure, R)`.
        pub const WriteEffect = effects.Effect(WriteReport, Failure, R);

        userdata: ?*anyopaque,
        vtable: *const VTable,

        /// Typed function-pointer interface for this capability. Implement every
        /// hook with the exact signature below; keep the table valid while the
        /// capability or pending work can invoke it.
        pub const VTable = struct {
            /// Read many through this provider.
            /// Parameters (callback order):
            /// - `1: ?*anyopaque`: opaque provider or handle context; keep it live until the hook and any retained work complete.
            /// - `2: *const R`: borrowed generic environment `R`, including caller-supplied runtime services.
            /// - `3: []const []const u8`: borrowed input slice; keep it valid through deferred use or copy before retaining it.
            /// Returns (`ReadEffect`): lazy typed Effect; run it with its declared environment to obtain Exit.
            read_many: *const fn (?*anyopaque, *const R, []const []const u8) ReadEffect,
            /// Write many through this provider.
            /// Parameters (callback order):
            /// - `1: ?*anyopaque`: opaque provider or handle context; keep it live until the hook and any retained work complete.
            /// - `2: *const R`: borrowed generic environment `R`, including caller-supplied runtime services.
            /// - `3: []const Write`: borrowed input slice; keep it valid through deferred use or copy before retaining it.
            /// Returns (`WriteEffect`): lazy typed Effect; run it with its declared environment to obtain Exit.
            write_many: *const fn (?*anyopaque, *const R, []const Write) WriteEffect,
            /// Remove many through this provider.
            /// Parameters (callback order):
            /// - `1: ?*anyopaque`: opaque provider or handle context; keep it live until the hook and any retained work complete.
            /// - `2: *const R`: borrowed generic environment `R`, including caller-supplied runtime services.
            /// - `3: []const []const u8`: borrowed input slice; keep it valid through deferred use or copy before retaining it.
            /// Returns (`WriteEffect`): lazy typed Effect; run it with its declared environment to obtain Exit.
            remove_many: *const fn (?*anyopaque, *const R, []const []const u8) WriteEffect,
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
        /// - `select_keys: comptime fn (*const R) []const []const u8`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// Returns (`ReadEffect`): ordered batch read report with per-item status and typed provider failure.
        pub fn read(comptime select_store: fn (*const R) *const Self, comptime select_keys: fn (*const R) []const []const u8) ReadEffect {
            return ReadEffect.deferEffect(struct {
                fn run(env: *const R) ReadEffect {
                    const store = select_store(env);
                    return store.vtable.read_many(store.userdata, env, select_keys(env));
                }
            }.run);
        }

        /// Write through the selected capability.
        /// Parameters:
        /// - `select_store: comptime fn (*const R) *const Self`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_writes: comptime fn (*const R) []const Write`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// Returns (`WriteEffect`): ordered partial-progress report for the requested batch writes.
        pub fn write(comptime select_store: fn (*const R) *const Self, comptime select_writes: fn (*const R) []const Write) WriteEffect {
            return WriteEffect.deferEffect(struct {
                fn run(env: *const R) WriteEffect {
                    const store = select_store(env);
                    return store.vtable.write_many(store.userdata, env, select_writes(env));
                }
            }.run);
        }

        /// Remove the selected entry.
        /// Parameters:
        /// - `select_store: comptime fn (*const R) *const Self`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_keys: comptime fn (*const R) []const []const u8`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// Returns (`WriteEffect`): ordered partial-progress report for the requested batch removals.
        pub fn remove(comptime select_store: fn (*const R) *const Self, comptime select_keys: fn (*const R) []const []const u8) WriteEffect {
            return WriteEffect.deferEffect(struct {
                fn run(env: *const R) WriteEffect {
                    const store = select_store(env);
                    return store.vtable.remove_many(store.userdata, env, select_keys(env));
                }
            }.run);
        }
    };
}
