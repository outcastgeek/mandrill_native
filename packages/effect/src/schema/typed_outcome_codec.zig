//! Typed bridge between Exit(A,E) and versioned StoredExit data. The caller
//! supplies success/error codecs for the two directions; the stored format
//! owns every defect diagnostic, and decoded Exit views stay inside an owner.
const std = @import("std");
const outcomes = @import("../outcome.zig");
const effects = @import("../effect.zig");
const generic_codec = @import("codec.zig");
const stored = @import("outcome_codec.zig");
const Issue = @import("schema_issue.zig").Issue;

fn borrowedDefect(value: stored.DefectData) outcomes.Defect {
    return .{ .kind = value.kind, .message = value.diagnostic.items };
}

fn copyDefect(env: anytype, value: outcomes.Defect) outcomes.Exit(stored.DefectData, Issue) {
    const diagnostic = env.allocator.dupe(u8, value.message) catch return .{ .die = outcomes.Defect.out_of_memory };
    return .{ .success = .{
        .kind = value.kind,
        .diagnostic = .{ .items = diagnostic, .allocator = env.allocator },
    } };
}

fn discardPrimary(comptime E: type, value: *outcomes.PrimaryOutcome(E)) void {
    if (value.* == .failed) outcomes.discardPayload(&value.failed);
    value.* = undefined;
}

fn discardCleanup(comptime E: type, value: *outcomes.CleanupFailure(E)) void {
    if (value.* == .release_failure) outcomes.discardPayload(&value.release_failure);
    value.* = undefined;
}

fn unmarkedBorrow(comptime T: type) bool {
    if (outcomes.ownsPayload(T)) return false;
    return switch (@typeInfo(T)) {
        .pointer => true,
        .optional => |value| unmarkedBorrow(value.child),
        .array => |value| unmarkedBorrow(value.child),
        .vector => |value| unmarkedBorrow(value.child),
        .@"struct" => |value| blk: {
            inline for (value.fields) |field| {
                if (unmarkedBorrow(field.type)) break :blk true;
            }
            break :blk false;
        },
        .@"union" => |value| blk: {
            inline for (value.fields) |field| {
                if (unmarkedBorrow(field.type)) break :blk true;
            }
            break :blk false;
        },
        else => false,
    };
}

fn into(comptime Target: type, comptime tag: []const u8, result: anytype) outcomes.Exit(Target, @TypeOf(result).Error) {
    return switch (result) {
        .success => |value| .{ .success = @unionInit(Target, tag, value) },
        else => outcomes.castFailure(Target, result),
    };
}

/// Specialize decoding of stored Effect terminal outcomes.
/// Parameters:
/// - `A: comptime type`: Value type produced by a successful computation.
/// - `E: comptime type`: Expected application-failure type; defects and interruption remain separate outcomes.
/// Returns (`type`): decoded terminal outcome retaining success, typed failure, defect, interruption, and cleanup cases.
pub fn DecodedExit(comptime A: type, comptime E: type) type {
    return struct {
        const Self = @This();
        /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements `deinit`
        /// and must be released after use.
        pub const effect_owned_payload = true;
        value: outcomes.Exit(A, E),
        encoded: stored.StoredExit,

        fn rebind(self: *Self) void {
            switch (self.value) {
                .die => {
                    if (self.encoded.primary == .defect)
                        self.value.die.message = self.encoded.primary.defect.diagnostic.items;
                },
                .finalization_failed => {
                    if (self.value.finalization_failed.primary == .defected and self.encoded.primary == .defect)
                        self.value.finalization_failed.primary.defected.message = self.encoded.primary.defect.diagnostic.items;
                    if (self.value.finalization_failed.first == .release_defect and self.encoded.cleanup.items[0] == .defect)
                        self.value.finalization_failed.first.release_defect.message = self.encoded.cleanup.items[0].defect.diagnostic.items;
                    var node = self.value.finalization_failed.rest;
                    for (self.encoded.cleanup.items[1..]) |entry| {
                        const current = node orelse break;
                        if (current.failure == .release_defect and entry == .defect)
                            current.failure.release_defect.message = entry.defect.diagnostic.items;
                        node = current.next;
                    }
                },
                else => {},
            }
        }

        /// Release resources owned by this value.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// Returns (`void`): no value.
        pub fn deinit(self: *Self) void {
            self.value.deinit();
            self.encoded.deinit();
            self.* = undefined;
        }

        /// Duplicate this value; marked owning payloads are cloned, while ordinary
        /// borrowed fields retain their original lifetime.
        /// Parameters:
        /// - `self: *const Self`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
        /// Returns (`error{OutOfMemory}!Self`): copied value on success; `OutOfMemory` if owned storage could not be cloned. The source remains valid.
        pub fn clone(self: *const Self) error{OutOfMemory}!Self {
            var copied_encoded = try self.encoded.clone();
            errdefer copied_encoded.deinit();
            const copied_value = self.value.clone();
            if (copied_value == .die and copied_value.die.kind == .out_of_memory and
                !(self.value == .die and self.value.die.kind == .out_of_memory))
                return error.OutOfMemory;
            var copy: Self = .{ .value = copied_value, .encoded = copied_encoded };
            copy.rebind();
            return copy;
        }
    };
}

/// Define a typed payload codec with its compile-time value and representation types.
/// Parameters:
/// - `A: comptime type`: Value type produced by a successful computation.
/// - `E: comptime type`: Expected application-failure type; defects and interruption remain separate outcomes.
/// - `DecodeR: comptime type`: Environment type passed to the decoding side of the codec.
/// - `EncodeR: comptime type`: Environment type passed to the encoding side of the codec.
/// Returns (`type`): typed success/error payload codec for stored `Exit` values.
pub fn PayloadCodec(
    comptime A: type,
    comptime E: type,
    comptime DecodeR: type,
    comptime EncodeR: type,
) type {
    if (unmarkedBorrow(A) or unmarkedBorrow(E))
        @compileError("stored outcome payloads containing pointers must declare owned clone/deinit semantics");
    return struct {
        const Self = @This();
        /// Application value type produced by this codec.
        pub const Decoded = DecodedExit(A, E);
        /// Encoded stored-outcome representation.
        pub const Encoded = stored.StoredExit;
        /// Environment required to run decoding effects.
        pub const DecodingEnvironment = DecodeR;
        /// Environment required to run encoding effects.
        pub const EncodingEnvironment = EncodeR;

        encode_success: *const fn (*const A, *const EncodeR) outcomes.Exit(stored.Bytes, Issue),
        encode_failure: *const fn (*const E, *const EncodeR) outcomes.Exit(stored.Bytes, Issue),
        decode_success: *const fn ([]const u8, *const DecodeR) outcomes.Exit(A, Issue),
        decode_failure: *const fn ([]const u8, *const DecodeR) outcomes.Exit(E, Issue),

        /// Pair success and failure codecs for a persisted typed Exit.
        /// Parameters:
        /// - `encode_a: comptime fn (*const A, *const EncodeR) outcomes.Exit(stored.Bytes, Issue)`: Compile-time typed function selected at comptime; no runtime callback
        ///   lookup is required.
        /// - `encode_e: comptime fn (*const E, *const EncodeR) outcomes.Exit(stored.Bytes, Issue)`: Compile-time typed function selected at comptime; no runtime callback
        ///   lookup is required.
        /// - `decode_a: comptime fn ([]const u8, *const DecodeR) outcomes.Exit(A, Issue)`: Compile-time typed function selected at comptime; no runtime callback
        ///   lookup is required.
        /// - `decode_e: comptime fn ([]const u8, *const DecodeR) outcomes.Exit(E, Issue)`: Compile-time typed function selected at comptime; no runtime callback
        ///   lookup is required.
        /// Returns (`Self`): an outcome codec whose four callbacks handle each payload direction.
        pub fn init(
            comptime encode_a: fn (*const A, *const EncodeR) outcomes.Exit(stored.Bytes, Issue),
            comptime encode_e: fn (*const E, *const EncodeR) outcomes.Exit(stored.Bytes, Issue),
            comptime decode_a: fn ([]const u8, *const DecodeR) outcomes.Exit(A, Issue),
            comptime decode_e: fn ([]const u8, *const DecodeR) outcomes.Exit(E, Issue),
        ) Self {
            return .{
                .encode_success = encode_a,
                .encode_failure = encode_e,
                .decode_success = decode_a,
                .decode_failure = decode_e,
            };
        }

        fn encodePrimary(self: Self, env: *const EncodeR, value: *const outcomes.Exit(A, E)) outcomes.Exit(stored.Primary, Issue) {
            return switch (value.*) {
                .success => into(stored.Primary, "success", self.encode_success(&value.success, env)),
                .fail => into(stored.Primary, "failure", self.encode_failure(&value.fail, env)),
                .die => |defect| into(stored.Primary, "defect", copyDefect(env, defect)),
                .interrupted => .{ .success = .interrupted },
                .finalization_failed => |details| switch (details.primary) {
                    .succeeded => .{ .success = .succeeded_before_cleanup_failure },
                    .failed => into(stored.Primary, "failure", self.encode_failure(&value.finalization_failed.primary.failed, env)),
                    .defected => |defect| into(stored.Primary, "defect", copyDefect(env, defect)),
                    .was_interrupted => .{ .success = .interrupted },
                },
            };
        }

        fn encodeCleanup(self: Self, env: *const EncodeR, value: outcomes.CleanupFailure(E)) outcomes.Exit(stored.Cleanup, Issue) {
            return switch (value) {
                .release_failure => |failure| into(stored.Cleanup, "failure", self.encode_failure(&failure, env)),
                .release_defect => |defect| into(stored.Cleanup, "defect", copyDefect(env, defect)),
                .release_interrupted => .{ .success = .interrupted },
            };
        }

        /// Encode immediately and return a typed outcome.
        /// Parameters:
        /// - `self: Self`: Receiver passed by value: the struct itself is copied, while any referenced storage keeps its original ownership and lifetime.
        /// - `env: *const EncodeR`: Borrowed caller environment supplying allocator, I/O, and selected
        ///   capabilities.
        /// - `value: *const outcomes.Exit(A, E)`: Borrowed input; keep its owner alive until the operation has copied or
        ///   joined it.
        /// Returns (`outcomes.Exit(stored.StoredExit, Issue)`): Carries `stored.StoredExit` on `.success` or `Issue` on `.fail`; `.die`, `.interrupt`, and `.cleanup` preserve other terminal causes.
        pub fn encodeDirect(self: Self, env: *const EncodeR, value: *const outcomes.Exit(A, E)) outcomes.Exit(stored.StoredExit, Issue) {
            const primary_result = self.encodePrimary(env, value);
            var primary = switch (primary_result) {
                .success => |encoded| encoded,
                else => return outcomes.castFailure(stored.StoredExit, primary_result),
            };
            var keep_primary = false;
            defer if (!keep_primary) primary.deinit();
            var cleanup_count: usize = 0;
            if (value.* == .finalization_failed) {
                cleanup_count = 1;
                var node = value.finalization_failed.rest;
                while (node) |current| {
                    cleanup_count = std.math.add(usize, cleanup_count, 1) catch return .{ .fail = Issue.init(env, .encode, .size_limit) };
                    node = current.next;
                }
            }
            const cleanup = env.allocator.alloc(stored.Cleanup, cleanup_count) catch return .{ .die = outcomes.Defect.out_of_memory };
            var initialized: usize = 0;
            var keep_cleanup = false;
            defer if (!keep_cleanup) {
                for (cleanup[0..initialized]) |*entry| entry.deinit();
                env.allocator.free(cleanup);
            };
            if (value.* == .finalization_failed) {
                var iterator = value.finalization_failed.iterator();
                while (iterator.nextFailure()) |failure| {
                    const encoded_cleanup = self.encodeCleanup(env, failure);
                    cleanup[initialized] = switch (encoded_cleanup) {
                        .success => |encoded| encoded,
                        else => return outcomes.castFailure(stored.StoredExit, encoded_cleanup),
                    };
                    initialized += 1;
                }
            }
            keep_primary = true;
            keep_cleanup = true;
            return .{ .success = .{
                .primary = primary,
                .cleanup = .{ .items = cleanup, .allocator = env.allocator },
            } };
        }

        fn decodePrimary(self: Self, env: *const DecodeR, primary: stored.Primary) outcomes.Exit(outcomes.Exit(A, E), Issue) {
            return switch (primary) {
                .success => |bytes| into(outcomes.Exit(A, E), "success", self.decode_success(bytes.items, env)),
                .failure => |bytes| into(outcomes.Exit(A, E), "fail", self.decode_failure(bytes.items, env)),
                .defect => |value| .{ .success = .{ .die = borrowedDefect(value) } },
                .interrupted => .{ .success = .interrupted },
                .succeeded_before_cleanup_failure => .{ .fail = Issue.init(env, .decode, .invalid_value) },
            };
        }

        fn decodeFinalPrimary(self: Self, env: *const DecodeR, primary: stored.Primary) outcomes.Exit(outcomes.PrimaryOutcome(E), Issue) {
            return switch (primary) {
                .success => .{ .fail = Issue.init(env, .decode, .invalid_value) },
                .succeeded_before_cleanup_failure => .{ .success = .succeeded },
                .failure => |bytes| into(outcomes.PrimaryOutcome(E), "failed", self.decode_failure(bytes.items, env)),
                .defect => |value| .{ .success = .{ .defected = borrowedDefect(value) } },
                .interrupted => .{ .success = .was_interrupted },
            };
        }

        fn decodeCleanup(self: Self, env: *const DecodeR, cleanup: stored.Cleanup) outcomes.Exit(outcomes.CleanupFailure(E), Issue) {
            return switch (cleanup) {
                .failure => |bytes| into(outcomes.CleanupFailure(E), "release_failure", self.decode_failure(bytes.items, env)),
                .defect => |value| .{ .success = .{ .release_defect = borrowedDefect(value) } },
                .interrupted => .{ .success = .release_interrupted },
            };
        }

        /// Decode immediately and return the typed validation outcome.
        /// Parameters:
        /// - `self: Self`: Receiver passed by value: the struct itself is copied, while any referenced storage keeps its original ownership and lifetime.
        /// - `env: *const DecodeR`: Borrowed caller environment supplying allocator, I/O, and selected
        ///   capabilities.
        /// - `encoded: stored.StoredExit`: stored terminal outcome with its encoded primary and cleanup cases.
        /// Returns (`outcomes.Exit(Decoded, Issue)`): Carries `Decoded` on `.success` or `Issue` on `.fail`; `.die`, `.interrupt`, and `.cleanup` preserve other terminal causes.
        pub fn decodeDirect(self: Self, env: *const DecodeR, encoded: stored.StoredExit) outcomes.Exit(Decoded, Issue) {
            var owned = encoded;
            var keep_source = false;
            defer if (!keep_source) owned.deinit();
            if (!owned.valid()) return .{ .fail = Issue.init(env, .decode, .invalid_value) };
            if (owned.cleanup.items.len == 0) {
                const decoded = self.decodePrimary(env, owned.primary);
                return switch (decoded) {
                    .success => |value| blk: {
                        keep_source = true;
                        break :blk .{ .success = .{ .value = value, .encoded = owned } };
                    },
                    else => outcomes.castFailure(Decoded, decoded),
                };
            }
            const decoded_primary = self.decodeFinalPrimary(env, owned.primary);
            var primary = switch (decoded_primary) {
                .success => |value| value,
                else => return outcomes.castFailure(Decoded, decoded_primary),
            };
            const decoded_first = self.decodeCleanup(env, owned.cleanup.items[0]);
            const first = switch (decoded_first) {
                .success => |value| value,
                else => {
                    discardPrimary(E, &primary);
                    return outcomes.castFailure(Decoded, decoded_first);
                },
            };
            var decoded: outcomes.Exit(A, E) = .{ .finalization_failed = .{ .primary = primary, .first = first } };
            var keep_decoded = false;
            defer if (!keep_decoded) decoded.deinit();
            for (owned.cleanup.items[1..]) |entry| {
                const decoded_cleanup = self.decodeCleanup(env, entry);
                var failure = switch (decoded_cleanup) {
                    .success => |value| value,
                    else => return outcomes.castFailure(Decoded, decoded_cleanup),
                };
                const node = env.allocator.create(outcomes.CleanupNode(E)) catch {
                    discardCleanup(E, &failure);
                    return .{ .die = outcomes.Defect.out_of_memory };
                };
                node.* = .{ .failure = failure, .allocator = env.allocator };
                if (decoded.finalization_failed.tail) |tail| tail.next = node else decoded.finalization_failed.rest = node;
                decoded.finalization_failed.tail = node;
            }
            keep_decoded = true;
            keep_source = true;
            return .{ .success = .{ .value = decoded, .encoded = owned } };
        }

        /// Expose this schema through the Codec interface.
        /// Parameters:
        /// - `self: comptime Self`: Compile-time current value passed by value; ownership follows this
        ///   operation’s contract.
        /// Returns (`generic_codec.Codec(Decoded, Encoded, DecodeR, EncodeR)`): A typed codec or schema preserving the declared decoded value, representation, and environments.
        pub fn asCodec(comptime self: Self) generic_codec.Codec(Decoded, Encoded, DecodeR, EncodeR) {
            return generic_codec.Codec(Decoded, Encoded, DecodeR, EncodeR).init(struct {
                fn decode(encoded: *const Encoded) effects.Effect(Decoded, Issue, DecodeR) {
                    return effects.Effect(Decoded, Issue, DecodeR).fromBorrowed(Encoded, encoded, struct {
                        fn run(source: *const Encoded, env: *const DecodeR) outcomes.Exit(Decoded, Issue) {
                            const copy = source.clone() catch return .{ .die = outcomes.Defect.out_of_memory };
                            return self.decodeDirect(env, copy);
                        }
                    }.run);
                }
            }.decode, struct {
                fn encode(value: *const Decoded) effects.Effect(Encoded, Issue, EncodeR) {
                    return effects.Effect(Encoded, Issue, EncodeR).fromBorrowed(Decoded, value, struct {
                        fn run(source: *const Decoded, env: *const EncodeR) outcomes.Exit(Encoded, Issue) {
                            return self.encodeDirect(env, &source.value);
                        }
                    }.run);
                }
            }.encode);
        }

        /// Complete versioned wire codec for storage and transport. The
        /// separate decode/encode environments select their own allocators
        /// and other capabilities through R; no byte buffer is borrowed past
        /// the returned Effect's lifetime.
        ///
        /// Parameters:
        /// - `self: comptime Self`: Compile-time current value passed by value; ownership follows this
        ///   operation’s contract.
        /// - `limits: comptime stored.Limits`: Compile-time caller-selected, validated behavior or resource bounds; no
        ///   hidden default is installed.
        /// Returns (`generic_codec.Codec(Decoded, stored.Bytes, DecodeR, EncodeR)`): A typed codec or schema preserving the declared decoded value, representation, and environments.
        pub fn asWireCodec(comptime self: Self, comptime limits: stored.Limits) generic_codec.Codec(Decoded, stored.Bytes, DecodeR, EncodeR) {
            if (!limits.valid()) @compileError("stored outcome wire limits must be valid");
            return generic_codec.Codec(Decoded, stored.Bytes, DecodeR, EncodeR).init(struct {
                fn decode(encoded: *const stored.Bytes) effects.Effect(Decoded, Issue, DecodeR) {
                    return effects.Effect(Decoded, Issue, DecodeR).fromBorrowed(stored.Bytes, encoded, struct {
                        fn run(source: *const stored.Bytes, env: *const DecodeR) outcomes.Exit(Decoded, Issue) {
                            const parsed = stored.decode(env, source.items, limits);
                            return switch (parsed) {
                                .success => |value| self.decodeDirect(env, value),
                                else => outcomes.castFailure(Decoded, parsed),
                            };
                        }
                    }.run);
                }
            }.decode, struct {
                fn encode(value: *const Decoded) effects.Effect(stored.Bytes, Issue, EncodeR) {
                    return effects.Effect(stored.Bytes, Issue, EncodeR).fromBorrowed(Decoded, value, struct {
                        fn run(source: *const Decoded, env: *const EncodeR) outcomes.Exit(stored.Bytes, Issue) {
                            const prepared = self.encodeDirect(env, &source.value);
                            var representation = switch (prepared) {
                                .success => |encoded_value| encoded_value,
                                else => return outcomes.castFailure(stored.Bytes, prepared),
                            };
                            defer representation.deinit();
                            return stored.encode(env, &representation, limits);
                        }
                    }.run);
                }
            }.encode);
        }
    };
}
