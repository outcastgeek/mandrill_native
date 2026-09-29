//! Typed result storage over a caller-supplied encoded key/value capability.
//! A persisted record owns a versioned full Exit wire value. Missing, expiry,
//! malformed bytes and incompatible schema versions remain distinct outcomes.
const std = @import("std");
const outcomes = @import("../outcome.zig");
const effects = @import("../effect.zig");
const keys = @import("../primary_key.zig");
const deadlines = @import("../deadline.zig");
const wire = @import("../schema/outcome_codec.zig");
const typed = @import("../schema/typed_outcome_codec.zig");
const Issue = @import("../schema/schema_issue.zig").Issue;
const storage = @import("key_value_store.zig");
const PersistenceError = @import("persistence_error.zig").Error;

pub fn ReadResult(comptime A: type, comptime E: type) type {
    return union(enum) {
        const Self = @This();
        pub const effect_owned_payload = true;
        found: typed.DecodedExit(A, E),
        missing,
        expired,

        pub fn deinit(self: *Self) void {
            if (self.* == .found) self.found.deinit();
            self.* = undefined;
        }

        pub fn clone(self: *const Self) error{OutOfMemory}!Self {
            return switch (self.*) {
                .found => |value| .{ .found = try value.clone() },
                else => self.*,
            };
        }
    };
}

pub fn Persistable(
    comptime A: type,
    comptime E: type,
    comptime R: type,
    comptime ProviderError: type,
    comptime DecodeR: type,
    comptime EncodeR: type,
) type {
    return struct {
        const Self = @This();
        const Store = storage.KeyValueStore(R, ProviderError);
        const Failure = PersistenceError(ProviderError);
        const Outcome = outcomes.Exit(A, E);
        const Codec = typed.PayloadCodec(A, E, DecodeR, EncodeR);
        const Read = ReadResult(A, E);
        const Header = struct {
            const schema_version_bytes: usize = @sizeOf(u32);
        };
        const WriteRequest = struct {
            pub const effect_owned_payload = true;
            key: outcomes.OwnedSlice(u8),
            bytes: outcomes.OwnedSlice(u8),
            expiry: deadlines.Deadline,

            pub fn deinit(self: *@This()) void {
                self.key.deinit();
                self.bytes.deinit();
                self.* = undefined;
            }

            pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
                var key = try self.key.clone();
                errdefer key.deinit();
                return .{ .key = key, .bytes = try self.bytes.clone(), .expiry = self.expiry };
            }
        };

        codec: Codec,
        schema_version: u32,
        limits: wire.Limits,
        select_store: *const fn (*const R) *const Store,
        select_key: *const fn (*const R) []const u8,
        select_expiry: *const fn (*const R) deadlines.Deadline,
        select_decode_environment: *const fn (*const R) *const DecodeR,
        select_encode_environment: *const fn (*const R) *const EncodeR,

        pub fn define(
            comptime codec: Codec,
            comptime schema_version: u32,
            comptime limits: wire.Limits,
            comptime select_store: fn (*const R) *const Store,
            comptime select_key: fn (*const R) []const u8,
            comptime select_expiry: fn (*const R) deadlines.Deadline,
            comptime select_decode_environment: fn (*const R) *const DecodeR,
            comptime select_encode_environment: fn (*const R) *const EncodeR,
        ) Self {
            const definition: Self = .{
                .codec = codec,
                .schema_version = schema_version,
                .limits = limits,
                .select_store = select_store,
                .select_key = select_key,
                .select_expiry = select_expiry,
                .select_decode_environment = select_decode_environment,
                .select_encode_environment = select_encode_environment,
            };
            definition.validate();
            return definition;
        }

        pub fn validate(comptime self: Self) void {
            if (comptime self.schema_version == 0) @compileError("persisted schema version zero is reserved for missing version metadata");
            if (comptime !self.limits.valid()) @compileError("persisted outcome wire limits must be valid");
        }

        fn mapIssue(issue: Issue) Failure {
            return .{ .schema = issue };
        }

        fn ownKey(comptime self: Self, env: *const R) outcomes.Exit(outcomes.OwnedSlice(u8), Failure) {
            const borrowed = self.select_key(env);
            if (keys.inspectEncoded(borrowed) == .err) return .{ .fail = .malformed };
            const copied = env.allocator.dupe(u8, borrowed) catch return .{ .die = outcomes.Defect.out_of_memory };
            return .{ .success = .{ .items = copied, .allocator = env.allocator } };
        }

        fn readBytes(comptime self: Self, env: *const R, encoded: []const u8) outcomes.Exit(Read, Failure) {
            if (encoded.len < Header.schema_version_bytes) return .{ .fail = .{ .schema = Issue.init(env, .decode, .invalid_value) } };
            var actual_version: u32 = 0;
            for (encoded[0..Header.schema_version_bytes]) |digit| actual_version = (actual_version << @bitSizeOf(u8)) | digit;
            if (actual_version != self.schema_version) return .{ .fail = .{ .incompatible_version = .{
                .expected = self.schema_version,
                .stored = actual_version,
            } } };
            const raw = wire.decode(env, encoded[Header.schema_version_bytes..], self.limits);
            const parsed = switch (raw) {
                .success => |value| value,
                else => return outcomes.mapError(Failure, outcomes.castFailure(Read, raw), env.allocator, mapIssue),
            };
            const decoded = self.codec.decodeDirect(self.select_decode_environment(env), parsed);
            const result: outcomes.Exit(Read, Issue) = switch (decoded) {
                .success => |value| .{ .success = .{ .found = value } },
                else => outcomes.castFailure(Read, decoded),
            };
            return outcomes.mapError(Failure, result, env.allocator, mapIssue);
        }

        fn encodeBytes(comptime self: Self, env: *const R, source: *const Outcome) outcomes.Exit(outcomes.OwnedSlice(u8), Failure) {
            const prepared = self.codec.encodeDirect(self.select_encode_environment(env), source);
            var representation = switch (prepared) {
                .success => |value| value,
                else => return outcomes.mapError(Failure, outcomes.castFailure(outcomes.OwnedSlice(u8), prepared), env.allocator, mapIssue),
            };
            defer representation.deinit();
            const encoded = wire.encode(env, &representation, self.limits);
            var bytes = switch (encoded) {
                .success => |value| value,
                else => return outcomes.mapError(Failure, encoded, env.allocator, mapIssue),
            };
            defer bytes.deinit();
            const total = std.math.add(usize, Header.schema_version_bytes, bytes.items.len) catch
                return .{ .fail = .{ .schema = Issue.init(env, .encode, .size_limit) } };
            const buffer = env.allocator.alloc(u8, total) catch return .{ .die = outcomes.Defect.out_of_memory };
            var version = self.schema_version;
            var index = Header.schema_version_bytes;
            while (index > 0) {
                index -= 1;
                buffer[index] = @truncate(version);
                version >>= @bitSizeOf(u8);
            }
            std.mem.copyForwards(u8, buffer[Header.schema_version_bytes..], bytes.items);
            return .{ .success = .{ .items = buffer, .allocator = env.allocator } };
        }

        fn prepareWrite(comptime self: Self, env: *const R, source: *const Outcome) outcomes.Exit(WriteRequest, Failure) {
            const prepared_key = self.ownKey(env);
            var key = switch (prepared_key) {
                .success => |value| value,
                else => return outcomes.castFailure(WriteRequest, prepared_key),
            };
            const encoded = self.encodeBytes(env, source);
            const bytes = switch (encoded) {
                .success => |value| value,
                else => {
                    key.deinit();
                    return outcomes.castFailure(WriteRequest, encoded);
                },
            };
            return .{ .success = .{ .key = key, .bytes = bytes, .expiry = self.select_expiry(env) } };
        }

        pub fn read(comptime self: Self, construction_env: *const R) effects.Effect(Read, Failure, R) {
            self.validate();
            const Acquire = effects.Effect(outcomes.OwnedSlice(u8), Failure, R);
            var acquire = Acquire.fromOutcome(struct {
                fn run(env: *const R) outcomes.Exit(outcomes.OwnedSlice(u8), Failure) {
                    return self.ownKey(env);
                }
            }.run);
            var loaded = acquire.acquireReleaseOwned(construction_env, struct {
                fn use(key: outcomes.OwnedSlice(u8), env: *const R) Store.ReadEffect {
                    const store = self.select_store(env);
                    return store.vtable.read(store.userdata, env, key.items);
                }
            }.use, struct {
                fn release(key: outcomes.OwnedSlice(u8), _: outcomes.PrimaryOutcome(Failure)) effects.Effect(void, Failure, R) {
                    var owned = key;
                    owned.deinit();
                    return effects.Effect(void, Failure, R).succeed({});
                }
            }.release);
            return loaded.flatMapOwned(construction_env, struct {
                fn decode(found: storage.Read, env: *const R) effects.Effect(Read, Failure, R) {
                    const Out = effects.Effect(Read, Failure, R);
                    return switch (found) {
                        .missing => Out.fromOwnedOutcome(env, .{ .success = .missing }),
                        .expired => Out.fromOwnedOutcome(env, .{ .success = .expired }),
                        .found => |buffer| blk: {
                            var owned = buffer;
                            defer owned.deinit();
                            break :blk Out.fromOwnedOutcome(env, self.readBytes(env, owned.items));
                        },
                    };
                }
            }.decode);
        }

        pub fn write(comptime self: Self, comptime select_outcome: fn (*const R) *const Outcome) Store.WriteEffect {
            self.validate();
            return Store.WriteEffect.deferEffect(struct {
                fn start(env: *const R) Store.WriteEffect {
                    return self.writeValue(env, select_outcome(env));
                }
            }.start);
        }

        /// A dynamic lookup result can be persisted without installing that
        /// result as another field of R. The borrowed outcome is encoded into
        /// owned bytes before the storage provider can suspend.
        pub fn writeValue(comptime self: Self, construction_env: *const R, source: *const Outcome) Store.WriteEffect {
            self.validate();
            const Acquire = effects.Effect(WriteRequest, Failure, R);
            var acquire = Acquire.fromBorrowed(Outcome, source, struct {
                fn run(value: *const Outcome, env: *const R) outcomes.Exit(WriteRequest, Failure) {
                    return self.prepareWrite(env, value);
                }
            }.run);
            return acquire.acquireReleaseOwned(construction_env, struct {
                fn use(request: WriteRequest, env: *const R) Store.WriteEffect {
                    const store = self.select_store(env);
                    return store.vtable.write(store.userdata, env, request.key.items, request.bytes.items, request.expiry);
                }
            }.use, struct {
                fn release(request: WriteRequest, _: outcomes.PrimaryOutcome(Failure)) effects.Effect(void, Failure, R) {
                    var owned = request;
                    owned.deinit();
                    return effects.Effect(void, Failure, R).succeed({});
                }
            }.release);
        }

        pub fn remove(comptime self: Self) Store.WriteEffect {
            self.validate();
            const Acquire = effects.Effect(outcomes.OwnedSlice(u8), Failure, R);
            const acquire = Acquire.fromOutcome(struct {
                fn run(env: *const R) outcomes.Exit(outcomes.OwnedSlice(u8), Failure) {
                    return self.ownKey(env);
                }
            }.run);
            return acquire.acquireRelease(struct {
                fn use(key: outcomes.OwnedSlice(u8), env: *const R) Store.WriteEffect {
                    const store = self.select_store(env);
                    return store.vtable.remove(store.userdata, env, key.items);
                }
            }.use, struct {
                fn release(key: outcomes.OwnedSlice(u8), _: outcomes.PrimaryOutcome(Failure)) effects.Effect(void, Failure, R) {
                    var owned = key;
                    owned.deinit();
                    return effects.Effect(void, Failure, R).succeed({});
                }
            }.release);
        }
    };
}
