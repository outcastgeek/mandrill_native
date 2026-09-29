//! Encoded reactive state transfer. Snapshots contain stable identity and
//! schema-versioned bytes, never tasks, subscriptions, services or closures.
const std = @import("std");
const outcomes = @import("../outcome.zig");
const atom_ref = @import("atom_ref.zig");
const registry = @import("atom_registry.zig");
const Issue = @import("../schema/schema_issue.zig").Issue;

/// Stored atom key, schema version, revision, and encoded state.
pub const Snapshot = struct {
    /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements `deinit` and
    /// must be released after use.
    pub const effect_owned_payload = true;
    key: outcomes.OwnedSlice(u8),
    schema_version: u32,
    revision: u64,
    bytes: outcomes.OwnedSlice(u8),

    /// Release resources owned by this value.
    /// Parameters:
    /// - `self: *@This()`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
    /// Returns (`void`): no value.
    pub fn deinit(self: *@This()) void {
        self.key.deinit();
        self.bytes.deinit();
        self.* = undefined;
    }

    /// Duplicate this value; marked owning payloads are cloned, while ordinary
    /// borrowed fields retain their original lifetime.
    /// Parameters:
    /// - `self: *const @This()`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
    /// Returns (`error{OutOfMemory}!@This()`): copied value on success; `OutOfMemory` if owned storage could not be cloned. The source remains valid.
    pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
        var key = try self.key.clone();
        errdefer key.deinit();
        return .{ .key = key, .schema_version = self.schema_version, .revision = self.revision, .bytes = try self.bytes.clone() };
    }
};

/// Specialize the typed reactivity error channel from its compile-time error type.
/// Parameters:
/// - `T: comptime type`: Value type stored or transported by the returned component.
/// Returns (`type`): typed hydration failure union retaining source and validation errors.
pub fn Error(comptime T: type) type {
    _ = T;
    return union(enum) {
        const Self = @This();
        /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements `deinit`
        /// and must be released after use.
        pub const effect_owned_payload = true;
        atom: atom_ref.Error,
        codec: Issue,
        wrong_identity,
        incompatible_version,
        stale,
        conflicting_revision,
        revision_exhausted,

        /// Release resources owned by this value.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// Returns (`void`): no value.
        pub fn deinit(self: *Self) void {
            if (self.* == .codec) self.codec.deinit();
            self.* = undefined;
        }

        /// Duplicate this value; marked owning payloads are cloned, while ordinary
        /// borrowed fields retain their original lifetime.
        /// Parameters:
        /// - `self: *const Self`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
        /// Returns (`error{OutOfMemory}!Self`): copied value on success; `OutOfMemory` if owned storage could not be cloned. The source remains valid.
        pub fn clone(self: *const Self) error{OutOfMemory}!Self {
            return switch (self.*) {
                .codec => |issue| .{ .codec = try issue.clone() },
                else => self.*,
            };
        }
    };
}

/// Result of importing a snapshot into a hydrated atom.
pub const ImportResult = union(enum) {
    applied: atom_ref.Write,
    unchanged,
};

/// All mutations to this value pass through this owner so revision comparison
/// and publication share one serialized boundary. `Codec.encode` returns
/// owned bytes; `Codec.decode` returns an owned value. The codec must represent
/// data only and reject nontransferable/pending runtime objects.
///
/// Parameters:
/// - `T: comptime type`: Value type stored or transported by the returned component.
/// - `R: comptime type`: Caller-defined environment type that supplies dependencies and runtime capabilities when work executes.
/// - `version: comptime u32`: Compile-time stable version used to reject incompatible encoded or replayed data.
/// - `Codec: comptime type`: Compile-time codec type with a decoded value and encoded representation.
/// Returns (`type`): hydrated observable value with revision and source identity.
pub fn State(comptime T: type, comptime R: type, comptime version: u32, comptime Codec: type) type {
    comptime {
        if (version == 0) @compileError("hydration schema version must be nonzero");
        if (!@hasField(R, "allocator") or @FieldType(R, "allocator") != std.mem.Allocator or
            !@hasField(R, "io") or @FieldType(R, "io") != std.Io)
            @compileError("hydration requires allocator and io in R");
        if (!@hasDecl(Codec, "encode") or !@hasDecl(Codec, "decode"))
            @compileError("hydration requires typed encoded-value codec methods");
    }
    return struct {
        const Self = @This();
        /// Typed failure channel combining domain and provider errors.
        pub const Failure = Error(T);
        io: std.Io,
        mutex: std.Io.Mutex = .init,
        atom: atom_ref.AtomRef(T),
        revision: u64,

        /// Create a hydrated observable value from its source, key, and initial revision.
        /// Parameters:
        /// - `env: *const R`: Borrowed caller environment supplying allocator, I/O, and selected
        ///   capabilities.
        /// - `source: *registry.Registry`: Mutable input or buffer; ownership transfer is specified by the returned
        ///   type.
        /// - `key: []const u8`: Stable identity used for matching, deduplication, or replay.
        /// - `initial: T`: Value restored from the serialized state into the hydrated reference.
        /// - `capacity: usize`: buffer capacity for this hydrated value’s change notices.
        /// - `initial_revision: u64`: Starting revision for optimistic concurrency checks.
        /// Returns (`outcomes.Exit(Self, Failure)`): Carries `Self` on `.success` or `Failure` on `.fail`; `.die`, `.interrupt`, and `.cleanup` preserve other terminal causes.
        pub fn init(env: *const R, source: *registry.Registry, key: []const u8, initial: T, capacity: usize, initial_revision: u64) outcomes.Exit(Self, Failure) {
            return switch (atom_ref.AtomRef(T).init(env, source, key, initial, capacity)) {
                .success => |atom| .{ .success = .{ .io = env.io, .atom = atom, .revision = initial_revision } },
                .fail => |problem| .{ .fail = .{ .atom = problem } },
                .die => |defect| .{ .die = defect },
                .interrupted => .interrupted,
                .finalization_failed => unreachable,
            };
        }

        /// Read the current observable state.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// Returns (`outcomes.Exit(*atom_ref.AtomRef(T).Observation, Failure)`): Carries `*atom_ref.AtomRef(T).Observation` on `.success` or `Failure` on `.fail`; `.die`, `.interrupt`, and `.cleanup` preserve other terminal causes.
        pub fn observe(self: *Self) outcomes.Exit(*atom_ref.AtomRef(T).Observation, Failure) {
            return switch (self.atom.observe()) {
                .success => |value| .{ .success = value },
                .fail => |problem| .{ .fail = .{ .atom = problem } },
                .die => |defect| .{ .die = defect },
                .interrupted => .interrupted,
                .finalization_failed => unreachable,
            };
        }

        fn publish(self: *Self, value: T) outcomes.Exit(atom_ref.Write, Failure) {
            var owned = value;
            defer outcomes.discardPayload(&owned);
            return switch (self.atom.setNow(owned)) {
                .success => |result| .{ .success = result },
                .fail => |problem| .{ .fail = .{ .atom = problem } },
                .die => |defect| .{ .die = defect },
                .interrupted => .interrupted,
                .finalization_failed => unreachable,
            };
        }

        /// Replace the current value.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// - `value: T`: replacement value associated with the next committed revision.
        /// Returns (`outcomes.Exit(atom_ref.Write, Failure)`): Carries `atom_ref.Write` on `.success` or `Failure` on `.fail`; `.die`, `.interrupt`, and `.cleanup` preserve other terminal causes.
        pub fn set(self: *Self, value: T) outcomes.Exit(atom_ref.Write, Failure) {
            self.mutex.lockUncancelable(self.io);
            defer self.mutex.unlock(self.io);
            if (self.revision == std.math.maxInt(u64)) return .{ .fail = .revision_exhausted };
            const result = self.publish(value);
            if (result == .success) self.revision += 1;
            return result;
        }

        /// Capture the current observable state.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// - `env: *const R`: Borrowed caller environment supplying allocator, I/O, and selected
        ///   capabilities.
        /// Returns (`outcomes.Exit(Snapshot, Failure)`): Carries `Snapshot` on `.success` or `Failure` on `.fail`; `.die`, `.interrupt`, and `.cleanup` preserve other terminal causes.
        pub fn snapshot(self: *Self, env: *const R) outcomes.Exit(Snapshot, Failure) {
            self.mutex.lockUncancelable(self.io);
            defer self.mutex.unlock(self.io);
            var current = switch (self.atom.get()) {
                .success => |value| value,
                .fail => |problem| return .{ .fail = .{ .atom = problem } },
                .die => |defect| return .{ .die = defect },
                .interrupted => return .interrupted,
                .finalization_failed => unreachable,
            };
            defer outcomes.discardPayload(&current);
            const encoded = Codec.encode(&current, env);
            const bytes = switch (encoded) {
                .success => |value| value,
                .fail => |problem| return .{ .fail = .{ .codec = problem } },
                .die => |defect| return .{ .die = defect },
                .interrupted => return .interrupted,
                .finalization_failed => unreachable,
            };
            const key = env.allocator.dupe(u8, self.atom.key.items) catch {
                var abandoned = bytes;
                abandoned.deinit();
                return .{ .die = outcomes.Defect.out_of_memory };
            };
            return .{ .success = .{ .key = .{ .items = key, .allocator = env.allocator }, .schema_version = version, .revision = self.revision, .bytes = bytes } };
        }

        /// Restore state from the supplied snapshot or checkpoint.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// - `env: *const R`: Borrowed caller environment supplying allocator, I/O, and selected
        ///   capabilities.
        /// - `source: *const Snapshot`: Borrowed input; keep its owner alive until the operation has copied or
        ///   joined it.
        /// Returns (`outcomes.Exit(ImportResult, Failure)`): Carries `ImportResult` on `.success` or `Failure` on `.fail`; `.die`, `.interrupt`, and `.cleanup` preserve other terminal causes.
        pub fn restore(self: *Self, env: *const R, source: *const Snapshot) outcomes.Exit(ImportResult, Failure) {
            self.mutex.lockUncancelable(self.io);
            defer self.mutex.unlock(self.io);
            if (!std.mem.eql(u8, self.atom.key.items, source.key.items))
                return .{ .fail = .wrong_identity };
            if (source.schema_version != version) return .{ .fail = .incompatible_version };
            if (source.revision < self.revision) return .{ .fail = .stale };
            if (source.revision == self.revision) {
                var current = switch (self.atom.get()) {
                    .success => |value| value,
                    .fail => |problem| return .{ .fail = .{ .atom = problem } },
                    .die => |defect| return .{ .die = defect },
                    .interrupted => return .interrupted,
                    .finalization_failed => unreachable,
                };
                defer outcomes.discardPayload(&current);
                const encoded = Codec.encode(&current, env);
                var bytes = switch (encoded) {
                    .success => |value| value,
                    .fail => |problem| return .{ .fail = .{ .codec = problem } },
                    .die => |defect| return .{ .die = defect },
                    .interrupted => return .interrupted,
                    .finalization_failed => unreachable,
                };
                defer bytes.deinit();
                return if (std.mem.eql(u8, bytes.items, source.bytes.items))
                    .{ .success = .unchanged }
                else
                    .{ .fail = .conflicting_revision };
            }
            const decoded = Codec.decode(source.bytes.items, env);
            const value = switch (decoded) {
                .success => |item| item,
                .fail => |problem| return .{ .fail = .{ .codec = problem } },
                .die => |defect| return .{ .die = defect },
                .interrupted => return .interrupted,
                .finalization_failed => unreachable,
            };
            const written = self.publish(value);
            return switch (written) {
                .success => |result| blk: {
                    self.revision = source.revision;
                    break :blk .{ .success = .{ .applied = result } };
                },
                .fail => |problem| .{ .fail = problem },
                .die => |defect| .{ .die = defect },
                .interrupted => .interrupted,
                .finalization_failed => unreachable,
            };
        }

        /// Release resources owned by this value.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// Returns (`void`): no value.
        pub fn deinit(self: *Self) void {
            self.atom.deinit();
            self.* = undefined;
        }
    };
}
