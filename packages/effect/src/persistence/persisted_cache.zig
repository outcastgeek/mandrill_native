//! Local single-flight cache backed by caller-supplied typed storage Effects.
//! The memory cache has its own TTL; shared persistence alone does not provide
//! a cross-process single-flight guarantee.
const std = @import("std");
const effects = @import("../effect.zig");
const outcomes = @import("../outcome.zig");
const cache = @import("../cache.zig");
const storage = @import("key_value_store.zig");
const Cancellation = @import("../internal/runtime.zig").Cancellation;

/// Specialize the typed read outcome for persistence operations.
/// Parameters:
/// - `A: comptime type`: Value type produced by a successful computation.
/// Returns (`type`): typed cache read outcome, including stored-value status.
pub fn Read(comptime A: type) type {
    return union(enum) {
        /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements `deinit`
        /// and must be released after use.
        pub const effect_owned_payload = true;
        found: A,
        missing,
        expired,

        /// Release resources owned by this value.
        /// Parameters:
        /// - `self: *@This()`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// Returns (`void`): no value.
        pub fn deinit(self: *@This()) void {
            if (self.* == .found) outcomes.discardPayload(&self.found);
            self.* = undefined;
        }

        /// Duplicate this value; marked owning payloads are cloned, while ordinary
        /// borrowed fields retain their original lifetime.
        /// Parameters:
        /// - `self: *const @This()`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
        /// Returns (`error{OutOfMemory}!@This()`): copied value on success; `OutOfMemory` if owned storage could not be cloned. The source remains valid.
        pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
            return switch (self.*) {
                .found => |value| .{ .found = try outcomes.clonePayload(value) },
                else => self.*,
            };
        }
    };
}

/// Specialize the persistence failure variants from their typed channels.
/// Parameters:
/// - `StoreError: comptime type`: Compile-time typed store failure.
/// - `LookupError: comptime type`: Compile-time typed lookup failure.
/// Returns (`type`): typed cache failure union preserving persistence and decode errors.
pub fn Failure(comptime StoreError: type, comptime LookupError: type) type {
    return union(enum) {
        /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements `deinit`
        /// and must be released after use.
        pub const effect_owned_payload = true;
        storage: StoreError,
        lookup: LookupError,

        /// Release resources owned by this value.
        /// Parameters:
        /// - `self: *@This()`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// Returns (`void`): no value.
        pub fn deinit(self: *@This()) void {
            switch (self.*) {
                .storage => outcomes.discardPayload(&self.storage),
                .lookup => outcomes.discardPayload(&self.lookup),
            }
            self.* = undefined;
        }

        /// Duplicate this value; marked owning payloads are cloned, while ordinary
        /// borrowed fields retain their original lifetime.
        /// Parameters:
        /// - `self: *const @This()`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
        /// Returns (`error{OutOfMemory}!@This()`): copied value on success; `OutOfMemory` if owned storage could not be cloned. The source remains valid.
        pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
            return switch (self.*) {
                .storage => |value| .{ .storage = try outcomes.clonePayload(value) },
                .lookup => |value| .{ .lookup = try outcomes.clonePayload(value) },
            };
        }
    };
}

/// Specialize the typed value outcome for persistence operations.
/// Parameters:
/// - `A: comptime type`: Value type produced by a successful computation.
/// Returns (`type`): decoded cached value with its persistence metadata.
pub fn Value(comptime A: type) type {
    return struct {
        /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements `deinit`
        /// and must be released after use.
        pub const effect_owned_payload = true;
        data: A,
        source: enum { persisted, computed },
        /// Present for a computed value, including ambiguous persistence.
        write: ?storage.WriteAcknowledgement,

        /// Release resources owned by this value.
        /// Parameters:
        /// - `self: *@This()`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// Returns (`void`): no value.
        pub fn deinit(self: *@This()) void {
            outcomes.discardPayload(&self.data);
            self.* = undefined;
        }

        /// Duplicate this value; marked owning payloads are cloned, while ordinary
        /// borrowed fields retain their original lifetime.
        /// Parameters:
        /// - `self: *const @This()`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
        /// Returns (`error{OutOfMemory}!@This()`): copied value on success; `OutOfMemory` if owned storage could not be cloned. The source remains valid.
        pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
            return .{ .data = try outcomes.clonePayload(self.data), .source = self.source, .write = self.write };
        }
    };
}

/// Specialize the typed invalidation report outcome for persistence operations.
/// Parameters:
/// - `E: comptime type`: Expected application-failure type; defects and interruption remain separate outcomes.
/// Returns (`type`): typed report of persisted-cache invalidation progress.
pub fn InvalidationReport(comptime E: type) type {
    return struct {
        /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements `deinit`
        /// and must be released after use.
        pub const effect_owned_payload = true;
        acknowledgement: storage.WriteAcknowledgement,
        local: outcomes.Exit(void, E),

        /// Release resources owned by this value.
        /// Parameters:
        /// - `self: *@This()`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// Returns (`void`): no value.
        pub fn deinit(self: *@This()) void {
            self.local.deinit();
            self.* = undefined;
        }

        /// Duplicate this value; marked owning payloads are cloned, while ordinary
        /// borrowed fields retain their original lifetime.
        /// Parameters:
        /// - `self: *const @This()`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
        /// Returns (`error{OutOfMemory}!@This()`): copied value on success; `OutOfMemory` if owned storage could not be cloned. The source remains valid.
        pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
            const copied = self.local.clone();
            if (copied == .die and copied.die.kind == .out_of_memory and
                !(self.local == .die and self.local.die.kind == .out_of_memory))
                return error.OutOfMemory;
            return .{ .acknowledgement = self.acknowledgement, .local = copied };
        }
    };
}

/// Specialize a cache backed by persisted values and their typed codecs.
/// Parameters:
/// - `Key: comptime type`: Key type used to identify or look up entries in this component.
/// - `A: comptime type`: Value type produced by a successful computation.
/// - `StoreError: comptime type`: Compile-time typed store failure.
/// - `LookupError: comptime type`: Compile-time typed lookup failure.
/// - `R: comptime type`: Caller-defined environment type that supplies dependencies and runtime capabilities when work executes.
/// - `read_store: comptime fn (Key, *const R) effects.Effect(Read(A), StoreError, R)`: Compile-time typed function selected at comptime; no runtime callback lookup is
///   required.
/// - `lookup: comptime fn (Key, *const R) effects.Effect(A, LookupError, R)`: Compile-time typed function selected at comptime; no runtime callback lookup is
///   required.
/// - `write_store: comptime fn (Key, *const A, *const R) effects.Effect(storage.WriteAcknowledgement, StoreError, R)`: Compile-time typed function selected at comptime; no runtime callback lookup
///   is required.
/// - `remove_store: comptime fn (Key, *const R) effects.Effect(storage.WriteAcknowledgement, StoreError, R)`: Compile-time typed function selected at comptime; no runtime callback lookup
///   is required.
/// Returns (`type`): schema-backed cache using stable keys, expiry, and a caller-selected store.
pub fn PersistedCache(
    comptime Key: type,
    comptime A: type,
    comptime StoreError: type,
    comptime LookupError: type,
    comptime R: type,
    comptime read_store: fn (Key, *const R) effects.Effect(Read(A), StoreError, R),
    comptime lookup: fn (Key, *const R) effects.Effect(A, LookupError, R),
    comptime write_store: fn (Key, *const A, *const R) effects.Effect(storage.WriteAcknowledgement, StoreError, R),
    comptime remove_store: fn (Key, *const R) effects.Effect(storage.WriteAcknowledgement, StoreError, R),
) type {
    const E = Failure(StoreError, LookupError);
    const Loaded = Value(A);
    const Memory = cache.Cache(Key, Loaded, E);
    comptime {
        if (!@hasField(R, "allocator") or @FieldType(R, "allocator") != std.mem.Allocator or
            !@hasField(R, "io") or @FieldType(R, "io") != std.Io or
            !@hasField(R, "async_engine"))
            @compileError("PersistedCache requires allocator, io and an async engine in R");
    }
    return struct {
        const Self = @This();
        /// Expected application failure type `E` carried by this persisted cache.
        pub const Error = E;
        /// Typed result channel described by `Loaded`.
        pub const Result = Loaded;
        const Context = struct {
            allocator: std.mem.Allocator,
            env: *const R,

            fn storeError(problem: StoreError) E {
                return .{ .storage = problem };
            }
            fn lookupError(problem: LookupError) E {
                return .{ .lookup = problem };
            }

            fn deinit(data: *anyopaque) void {
                const self: *@This() = @ptrCast(@alignCast(data));
                self.allocator.destroy(self);
            }

            fn load(data: *anyopaque, key: Key, token: *const Cancellation) outcomes.Exit(Loaded, E) {
                const self: *@This() = @ptrCast(@alignCast(data));
                if (token.isRequested()) return .interrupted;
                var read_effect = read_store(key, self.env);
                var mapped_read = read_effect.mapErrorOwned(self.env, storeError);
                const read_result = effects.run(self.env, mapped_read);
                mapped_read.deinit();
                const present = switch (read_result) {
                    .success => |value| value,
                    else => return outcomes.castFailure(Loaded, read_result),
                };
                switch (present) {
                    .found => |value| return .{ .success = .{ .data = value, .source = .persisted, .write = null } },
                    .missing, .expired => {},
                }
                if (token.isRequested()) return .interrupted;
                var lookup_effect = lookup(key, self.env);
                var mapped_lookup = lookup_effect.mapErrorOwned(self.env, lookupError);
                const fetched = effects.run(self.env, mapped_lookup);
                mapped_lookup.deinit();
                var value = switch (fetched) {
                    .success => |item| item,
                    else => return outcomes.castFailure(Loaded, fetched),
                };
                if (token.isRequested()) {
                    outcomes.discardPayload(&value);
                    return .interrupted;
                }
                var writing = write_store(key, &value, self.env);
                var mapped_write = writing.mapErrorOwned(self.env, storeError);
                const written = effects.run(self.env, mapped_write);
                mapped_write.deinit();
                return switch (written) {
                    .success => |ack| .{ .success = .{ .data = value, .source = .computed, .write = ack } },
                    else => blk: {
                        outcomes.discardPayload(&value);
                        break :blk outcomes.castFailure(Loaded, written);
                    },
                };
            }
        };

        memory: *Memory,
        context: *Context,

        /// R remains alive until the cache is shut down and joined. The
        /// caller chooses memory TTL, capacity, failure caching and admission.
        ///
        /// Parameters:
        /// - `env: *const R`: Borrowed caller environment supplying allocator, I/O, and selected
        ///   capabilities.
        /// - `config: cache.Config(Key, E)`: Caller-selected, validated behavior or resource bounds; no hidden default
        ///   is installed.
        /// Returns (`outcomes.Exit(Self, E)`): Carries `Self` on `.success` or `E` on `.fail`; `.die`, `.interrupt`, and `.cleanup` preserve other terminal causes.
        pub fn create(env: *const R, config: cache.Config(Key, E)) outcomes.Exit(Self, E) {
            const context = env.allocator.create(Context) catch return .{ .die = outcomes.Defect.out_of_memory };
            context.* = .{ .allocator = env.allocator, .env = env };
            const initialized = Memory.create(env, config, .{ .userdata = context, .run = Context.load, .deinit = Context.deinit });
            return switch (initialized) {
                .success => |owner| .{ .success = .{ .memory = owner, .context = context } },
                .fail => |problem| blk: {
                    env.allocator.destroy(context);
                    break :blk .{ .fail = problem };
                },
                .die => |defect| blk: {
                    env.allocator.destroy(context);
                    break :blk .{ .die = defect };
                },
                .interrupted => blk: {
                    env.allocator.destroy(context);
                    break :blk .interrupted;
                },
                .finalization_failed => |failure| blk: {
                    env.allocator.destroy(context);
                    break :blk .{ .finalization_failed = failure };
                },
            };
        }

        /// Look up the selected value.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// - `key: Key`: Stable identity used for matching, deduplication, or replay.
        /// - `token: ?*const Cancellation`: Cancellation token observed by this operation; its owner must outlive
        ///   pending work.
        /// Returns (`outcomes.Exit(Loaded, E)`): Carries `Loaded` on `.success` or `E` on `.fail`; `.die`, `.interrupt`, and `.cleanup` preserve other terminal causes.
        pub fn get(self: *Self, key: Key, token: ?*const Cancellation) outcomes.Exit(Loaded, E) {
            return self.memory.getToken(key, token);
        }

        /// Invalidate a locally cached persisted value.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// - `key: Key`: Stable identity used for matching, deduplication, or replay.
        /// Returns (`outcomes.Exit(void, E)`): Carries no value on `.success` or `E` on `.fail`; `.die`, `.interrupt`, and `.cleanup` preserve other terminal causes.
        pub fn invalidateLocal(self: *Self, key: Key) outcomes.Exit(void, E) {
            return self.memory.invalidate(key);
        }

        /// Remove persistent data first, then invalidate local memory. An
        /// ambiguous removal still invalidates locally and remains explicit.
        ///
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// - `key: Key`: Stable identity used for matching, deduplication, or replay.
        /// Returns (`outcomes.Exit(InvalidationReport(E), E)`): Carries `InvalidationReport(E)` on `.success` or `E` on `.fail`; `.die`, `.interrupt`, and `.cleanup` preserve other terminal causes.
        pub fn invalidate(self: *Self, key: Key) outcomes.Exit(InvalidationReport(E), E) {
            var removing = remove_store(key, self.context.env);
            var mapped = removing.mapErrorOwned(self.context.env, Context.storeError);
            const outcome = effects.run(self.context.env, mapped);
            mapped.deinit();
            return switch (outcome) {
                .success => |ack| .{ .success = .{ .acknowledgement = ack, .local = self.memory.invalidate(key) } },
                else => outcomes.castFailure(InvalidationReport(E), outcome),
            };
        }

        /// Stop the capability and release its retained resources.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// Returns (`outcomes.Exit(void, E)`): Carries no value on `.success` or `E` on `.fail`; `.die`, `.interrupt`, and `.cleanup` preserve other terminal causes.
        pub fn shutdown(self: *Self) outcomes.Exit(void, E) {
            return self.memory.shutdown();
        }

        /// Release resources owned by this value.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// Returns (`void`): no value.
        pub fn deinit(self: *Self) void {
            self.memory.deinit();
            self.* = undefined;
        }
    };
}
