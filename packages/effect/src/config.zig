//! Schema-backed configuration starts from a provider-neutral byte source.
//! Source failures, missing entries and schema-invalid entries are distinct;
//! secret values are never embedded in diagnostics by this contract.
const std = @import("std");
const effects = @import("effect.zig");
const outcomes = @import("outcome.zig");
const schema_issue = @import("schema/schema_issue.zig");
const schemas = @import("schema.zig");

pub const Read = union(enum) {
    pub const effect_owned_payload = true;
    present: outcomes.OwnedSlice(u8),
    missing,

    pub fn deinit(self: *@This()) void {
        if (self.* == .present) self.present.deinit();
        self.* = undefined;
    }

    pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
        return switch (self.*) {
            .present => |bytes| .{ .present = try bytes.clone() },
            .missing => .missing,
        };
    }
};

pub fn Error(comptime ProviderError: type) type {
    return union(enum) {
        const Self = @This();
        pub const effect_owned_payload = true;
        provider: ProviderError,
        missing,
        invalid: schema_issue.Issue,
        invalid_key,
        unavailable,

        pub fn deinit(self: *Self) void {
            switch (self.*) {
                .provider => outcomes.discardPayload(&self.provider),
                .invalid => self.invalid.deinit(),
                else => {},
            }
            self.* = undefined;
        }

        pub fn clone(self: *const Self) error{OutOfMemory}!Self {
            return switch (self.*) {
                .provider => |value| .{ .provider = try outcomes.clonePayload(value) },
                .invalid => |issue| .{ .invalid = try issue.clone() },
                else => self.*,
            };
        }
    };
}

pub fn Source(comptime R: type, comptime ProviderError: type) type {
    return struct {
        const Self = @This();
        pub const Failure = Error(ProviderError);
        pub const ReadEffect = effects.Effect(Read, Failure, R);
        userdata: ?*anyopaque,
        vtable: *const VTable,

        pub const VTable = struct {
            read: *const fn (?*anyopaque, *const R, []const u8) ReadEffect,
        };

        pub fn fromProvider(data: ?*anyopaque, vtable: *const VTable) Self {
            return .{ .userdata = data, .vtable = vtable };
        }

        pub fn read(comptime name: []const u8, comptime select: fn (*const R) *const Self) ReadEffect {
            if (name.len == 0 or !std.unicode.utf8ValidateSlice(name)) @compileError("configuration keys must be nonempty UTF-8");
            return ReadEffect.deferEffect(struct {
                fn start(env: *const R) ReadEffect {
                    const source = select(env);
                    return source.vtable.read(source.userdata, env, name);
                }
            }.start);
        }
    };
}

/// Load a required setting through the caller's source and decode it with an
/// owning schema. The source bytes are released after parsing; Parsed(A) keeps
/// its independent JSON document alive for any borrowed decoded fields.
/// The key, source projection and schema are definition-time values, so this
/// adds no runtime registry, lookup table or erased codec dispatch.
pub fn required(
    comptime A: type,
    comptime R: type,
    comptime ProviderError: type,
    comptime name: []const u8,
    comptime select: fn (*const R) *const Source(R, ProviderError),
    comptime schema: schemas.Schema(A, schema_issue.Issue),
) effects.Effect(schemas.Parsed(A), Error(ProviderError), R) {
    const Failure = Error(ProviderError);
    const Output = effects.Effect(schemas.Parsed(A), Failure, R);
    return Source(R, ProviderError).read(name, select).flatMap(struct {
        fn decode(read: Read, env: *const R) Output {
            return switch (read) {
                .missing => Output.fromOwnedOutcome(env, .{ .fail = .missing }),
                .present => |bytes| blk: {
                    var owned = bytes;
                    defer owned.deinit();
                    break :blk Output.fromOwnedOutcome(env, decodeBytes(A, R, ProviderError, schema, env, owned.items));
                },
            };
        }
    }.decode);
}

fn decodeBytes(
    comptime A: type,
    comptime R: type,
    comptime ProviderError: type,
    comptime schema: schemas.Schema(A, schema_issue.Issue),
    env: *const R,
    bytes: []const u8,
) outcomes.Exit(schemas.Parsed(A), Error(ProviderError)) {
    const Failure = Error(ProviderError);
    const parsed = schema.parseBytes(env, bytes, struct {
        fn syntax(_: std.json.ParseError(std.json.Scanner)) schema_issue.Issue {
            return schema_issue.Issue.pure(.decode, .invalid_value);
        }
    }.syntax);
    return outcomes.mapError(Failure, parsed, env.allocator, struct {
        fn invalid(issue: schema_issue.Issue) Failure {
            return .{ .invalid = issue };
        }
    }.invalid);
}

/// Missing is a successful optional result. A present but invalid setting
/// remains a typed failure and never silently selects the missing branch.
pub fn optional(
    comptime A: type,
    comptime R: type,
    comptime ProviderError: type,
    comptime name: []const u8,
    comptime select: fn (*const R) *const Source(R, ProviderError),
    comptime schema: schemas.Schema(A, schema_issue.Issue),
) effects.Effect(?schemas.Parsed(A), Error(ProviderError), R) {
    const Failure = Error(ProviderError);
    const Output = effects.Effect(?schemas.Parsed(A), Failure, R);
    return Source(R, ProviderError).read(name, select).flatMap(struct {
        fn decode(read: Read, env: *const R) Output {
            return switch (read) {
                .missing => Output.fromOwnedOutcome(env, .{ .success = null }),
                .present => |bytes| blk: {
                    var owned = bytes;
                    defer owned.deinit();
                    const parsed = decodeBytes(A, R, ProviderError, schema, env, owned.items);
                    const mapped: outcomes.Exit(?schemas.Parsed(A), Failure) = switch (parsed) {
                        .success => |value| .{ .success = value },
                        else => outcomes.castFailure(?schemas.Parsed(A), parsed),
                    };
                    break :blk Output.fromOwnedOutcome(env, mapped);
                },
            };
        }
    }.decode);
}

/// The fallback is an ordinary Effect, so its dependencies and failures stay
/// visible in R/E. It runs only when the key is absent, not on invalid input.
pub fn withDefault(
    comptime A: type,
    comptime R: type,
    comptime ProviderError: type,
    comptime name: []const u8,
    comptime select: fn (*const R) *const Source(R, ProviderError),
    comptime schema: schemas.Schema(A, schema_issue.Issue),
    comptime fallback: fn (*const R) effects.Effect(schemas.Parsed(A), Error(ProviderError), R),
) effects.Effect(schemas.Parsed(A), Error(ProviderError), R) {
    const Output = effects.Effect(schemas.Parsed(A), Error(ProviderError), R);
    return Source(R, ProviderError).read(name, select).flatMap(struct {
        fn decode(read: Read, env: *const R) Output {
            return switch (read) {
                .missing => fallback(env),
                .present => |bytes| blk: {
                    var owned = bytes;
                    defer owned.deinit();
                    break :blk Output.fromOwnedOutcome(env, decodeBytes(A, R, ProviderError, schema, env, owned.items));
                },
            };
        }
    }.decode);
}

/// A volatile source for explicit application settings and contract tests.
/// Values are copied on write and read; no environment variable, file system
/// or service provider is built into the core package.
pub fn MemorySource(comptime R: type) type {
    return struct {
        const Self = @This();
        const Provider = Source(R, void);
        const Failure = Error(void);
        const Entry = struct { key: []u8, value: []u8 };
        const Capture = struct { owner: *Self, name: []const u8 };

        allocator: std.mem.Allocator,
        io: std.Io,
        mutex: std.Io.Mutex = .init,
        values: std.StringHashMap(Entry),

        pub fn init(env: *const R) Self {
            if (R == void or !@hasField(R, "allocator") or @FieldType(R, "allocator") != std.mem.Allocator)
                @compileError("MemorySource requires caller allocator in R");
            if (R == void or !@hasField(R, "io") or @FieldType(R, "io") != std.Io)
                @compileError("MemorySource requires caller io in R");
            return .{ .allocator = env.allocator, .io = env.io, .values = std.StringHashMap(Entry).init(env.allocator) };
        }

        pub fn capability(self: *Self) Provider {
            return Provider.fromProvider(self, &.{ .read = readEffect });
        }

        fn readEffect(data: ?*anyopaque, env: *const R, name: []const u8) Provider.ReadEffect {
            const owner: *Self = @ptrCast(@alignCast(data.?));
            return Provider.ReadEffect.fromOwned(env, Capture{ .owner = owner, .name = name }, struct {
                fn run(capture: *const Capture, read_env: *const R) outcomes.Exit(Read, Failure) {
                    return capture.owner.readNow(read_env.allocator, capture.name);
                }
            }.run);
        }

        fn readNow(self: *Self, allocator: std.mem.Allocator, name: []const u8) outcomes.Exit(Read, Failure) {
            if (name.len == 0 or !std.unicode.utf8ValidateSlice(name)) return .{ .fail = .invalid_key };
            while (true) {
                self.mutex.lockUncancelable(self.io);
                const size = if (self.values.get(name)) |entry| entry.value.len else {
                    self.mutex.unlock(self.io);
                    return .{ .success = .missing };
                };
                self.mutex.unlock(self.io);
                const copy = allocator.alloc(u8, size) catch return .{ .die = outcomes.Defect.out_of_memory };
                self.mutex.lockUncancelable(self.io);
                const current = self.values.get(name) orelse {
                    self.mutex.unlock(self.io);
                    allocator.free(copy);
                    return .{ .success = .missing };
                };
                if (current.value.len != size) {
                    self.mutex.unlock(self.io);
                    allocator.free(copy);
                    continue;
                }
                std.mem.copyForwards(u8, copy, current.value);
                self.mutex.unlock(self.io);
                return .{ .success = .{ .present = .{ .items = copy, .allocator = allocator } } };
            }
        }

        pub fn put(self: *Self, name: []const u8, value: []const u8) outcomes.Exit(void, Failure) {
            if (name.len == 0 or !std.unicode.utf8ValidateSlice(name)) return .{ .fail = .invalid_key };
            const owned_name = self.allocator.dupe(u8, name) catch return .{ .die = outcomes.Defect.out_of_memory };
            var keep_name = false;
            defer if (!keep_name) self.allocator.free(owned_name);
            const owned_value = self.allocator.dupe(u8, value) catch return .{ .die = outcomes.Defect.out_of_memory };
            var keep_value = false;
            defer if (!keep_value) self.allocator.free(owned_value);
            self.mutex.lockUncancelable(self.io);
            defer self.mutex.unlock(self.io);
            if (self.values.getPtr(name)) |entry| {
                self.allocator.free(entry.value);
                entry.value = owned_value;
            } else {
                self.values.ensureUnusedCapacity(1) catch return .{ .die = outcomes.Defect.out_of_memory };
                self.values.putAssumeCapacity(owned_name, .{ .key = owned_name, .value = owned_value });
                keep_name = true;
            }
            keep_value = true;
            return .{ .success = {} };
        }

        pub fn remove(self: *Self, name: []const u8) outcomes.Exit(void, Failure) {
            if (name.len == 0 or !std.unicode.utf8ValidateSlice(name)) return .{ .fail = .invalid_key };
            self.mutex.lockUncancelable(self.io);
            const removed = self.values.fetchRemove(name);
            self.mutex.unlock(self.io);
            if (removed) |entry| {
                self.allocator.free(entry.value.key);
                self.allocator.free(entry.value.value);
            }
            return .{ .success = {} };
        }

        pub fn deinit(self: *Self) void {
            self.mutex.lockUncancelable(self.io);
            var iterator = self.values.iterator();
            while (iterator.next()) |entry| {
                self.allocator.free(entry.value_ptr.key);
                self.allocator.free(entry.value_ptr.value);
            }
            self.values.deinit();
            self.mutex.unlock(self.io);
            self.* = undefined;
        }
    };
}
