//! Local single-flight cache backed by caller-supplied typed storage Effects.
//! The memory cache has its own TTL; shared persistence alone does not provide
//! a cross-process single-flight guarantee.
const std = @import("std");
const effects = @import("../effect.zig");
const outcomes = @import("../outcome.zig");
const cache = @import("../cache.zig");
const storage = @import("key_value_store.zig");
const Cancellation = @import("../internal/runtime.zig").Cancellation;

pub fn Read(comptime A: type) type {
    return union(enum) {
        pub const effect_owned_payload = true;
        found: A,
        missing,
        expired,

        pub fn deinit(self: *@This()) void {
            if (self.* == .found) outcomes.discardPayload(&self.found);
            self.* = undefined;
        }

        pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
            return switch (self.*) {
                .found => |value| .{ .found = try outcomes.clonePayload(value) },
                else => self.*,
            };
        }
    };
}

pub fn Failure(comptime StoreError: type, comptime LookupError: type) type {
    return union(enum) {
        pub const effect_owned_payload = true;
        storage: StoreError,
        lookup: LookupError,

        pub fn deinit(self: *@This()) void {
            switch (self.*) {
                .storage => outcomes.discardPayload(&self.storage),
                .lookup => outcomes.discardPayload(&self.lookup),
            }
            self.* = undefined;
        }

        pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
            return switch (self.*) {
                .storage => |value| .{ .storage = try outcomes.clonePayload(value) },
                .lookup => |value| .{ .lookup = try outcomes.clonePayload(value) },
            };
        }
    };
}

pub fn Value(comptime A: type) type {
    return struct {
        pub const effect_owned_payload = true;
        data: A,
        source: enum { persisted, computed },
        /// Present for a computed value, including ambiguous persistence.
        write: ?storage.WriteAcknowledgement,

        pub fn deinit(self: *@This()) void {
            outcomes.discardPayload(&self.data);
            self.* = undefined;
        }

        pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
            return .{ .data = try outcomes.clonePayload(self.data), .source = self.source, .write = self.write };
        }
    };
}

pub fn InvalidationReport(comptime E: type) type {
    return struct {
        pub const effect_owned_payload = true;
        acknowledgement: storage.WriteAcknowledgement,
        local: outcomes.Exit(void, E),

        pub fn deinit(self: *@This()) void {
            self.local.deinit();
            self.* = undefined;
        }

        pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
            const copied = self.local.clone();
            if (copied == .die and copied.die.kind == .out_of_memory and
                !(self.local == .die and self.local.die.kind == .out_of_memory))
                return error.OutOfMemory;
            return .{ .acknowledgement = self.acknowledgement, .local = copied };
        }
    };
}

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
        pub const Error = E;
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

        pub fn get(self: *Self, key: Key, token: ?*const Cancellation) outcomes.Exit(Loaded, E) {
            return self.memory.getToken(key, token);
        }

        pub fn invalidateLocal(self: *Self, key: Key) outcomes.Exit(void, E) {
            return self.memory.invalidate(key);
        }

        /// Remove persistent data first, then invalidate local memory. An
        /// ambiguous removal still invalidates locally and remains explicit.
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

        pub fn shutdown(self: *Self) outcomes.Exit(void, E) {
            return self.memory.shutdown();
        }

        pub fn deinit(self: *Self) void {
            self.memory.deinit();
            self.* = undefined;
        }
    };
}
