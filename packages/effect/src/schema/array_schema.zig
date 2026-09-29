//! Bounded typed JSON arrays composed from pure or structural child codecs.
const std = @import("std");
const outcomes = @import("../outcome.zig");
const effects = @import("../effect.zig");
const Issue = @import("schema_issue.zig").Issue;
const parser = @import("internal/parser.zig");
const legacy = @import("../schema.zig");
const structural = @import("internal/structural.zig");
const ast = @import("schema_ast.zig");
const representation = @import("internal/representation.zig");

/// Build a structural array schema from a typed child schema.
/// Parameters:
/// - `child: comptime anytype`: Compile-time statically inferred input checked when this operation is instantiated.
/// - `R: comptime type`: Caller-defined environment type that supplies dependencies and runtime capabilities when work executes.
/// - `max_elements: comptime usize`: Compile-time capacity bound for this collection.
/// Returns (`type`): structural array schema that validates each element with the child schema.
pub fn Array(comptime child: anytype, comptime R: type, comptime max_elements: usize) type {
    const nested = @TypeOf(child) == type;
    const C = if (nested) child else @TypeOf(child);
    comptime {
        if (!@hasField(R, "allocator") or @FieldType(R, "allocator") != std.mem.Allocator)
            @compileError("ArraySchema requires R.allocator");
        if (nested) {
            if (!@hasDecl(C, "Decoded") or !@hasDecl(C, "Environment") or C.Environment != R or
                !@hasDecl(C, "decodeWithin") or !@hasDecl(C, "encodeWithin"))
                @compileError("nested array elements need a structural codec in the same environment");
        } else {
            if (C.Representation != std.json.Value or C.DecodingEnvironment != void or C.EncodingEnvironment != void)
                @compileError("array elements need a pure JSON codec");
        }
    }
    return struct {
        const Self = @This();
        /// Application value type produced by this codec.
        pub const Decoded = outcomes.OwnedSlice(C.Decoded);
        /// Exact caller environment type required when the description runs.
        pub const Environment = R;

        /// Return the structural representation node for this schema.
        /// Returns (`*const ast.Node`): A borrowed read-only pointer; keep its owner alive while using it.
        pub fn encodedNode() *const ast.Node {
            const Metadata = struct {
                const array_node: ast.Node = .{ .array = representation.nodePointer(child) };
                const rules = [_]ast.Constraint{.{ .length = .{ .maximum = max_elements, .unit = .elements } }};
                const node: ast.Node = .{ .constrained = .{ .base = &array_node, .rules = &rules } };
            };
            return &Metadata.node;
        }

        fn decodeChild(env: *const R, input: *const std.json.Value, state: *structural.DecodeState) outcomes.Exit(C.Decoded, Issue) {
            if (comptime nested) return child.decodeWithin(env, input, state);
            return switch (child.decodeDirect(input)) {
                .ok => |value| .{ .success = value },
                .err => |issue| .{ .fail = issue },
            };
        }

        fn encodeChild(env: *const R, input: *const C.Decoded, state: *structural.EncodeState) outcomes.Exit(std.json.Value, Issue) {
            if (comptime nested) return child.encodeWithin(env, input, state);
            return switch (child.encodeDirect(input)) {
                .ok => |value| .{ .success = value },
                .err => |issue| .{ .fail = issue },
            };
        }

        fn atIndex(env: *const R, index: usize, original: Issue) outcomes.Exit(Decoded, Issue) {
            var issue = original;
            issue.bindAllocator(env);
            issue.prependIndex(index) catch {
                issue.deinit();
                return .{ .die = outcomes.Defect.out_of_memory };
            };
            return .{ .fail = issue };
        }

        /// Decode immediately and return the typed validation outcome.
        /// Parameters:
        /// - `env: *const R`: Borrowed caller environment supplying allocator, I/O, and selected
        ///   capabilities.
        /// - `json: *const std.json.Value`: JSON value or document to decode, encode, or validate.
        /// Returns (`outcomes.Exit(Decoded, Issue)`): Carries `Decoded` on `.success` or `Issue` on `.fail`; `.die`, `.interrupt`, and `.cleanup` preserve other terminal causes.
        pub fn decodeDirect(env: *const R, json: *const std.json.Value) outcomes.Exit(Decoded, Issue) {
            var state: structural.DecodeState = .{};
            return decodeWithin(env, json, &state);
        }

        /// Decode within the caller-supplied depth or size limits.
        /// Parameters:
        /// - `env: *const R`: Borrowed caller environment supplying allocator, I/O, and selected
        ///   capabilities.
        /// - `json: *const std.json.Value`: JSON value or document to decode, encode, or validate.
        /// - `state: *structural.DecodeState`: Mutable operation state, retained for the duration of this call.
        /// Returns (`outcomes.Exit(Decoded, Issue)`): Carries `Decoded` on `.success` or `Issue` on `.fail`; `.die`, `.interrupt`, and `.cleanup` preserve other terminal causes.
        pub fn decodeWithin(env: *const R, json: *const std.json.Value, state: *structural.DecodeState) outcomes.Exit(Decoded, Issue) {
            if (json.* != .array) return .{ .fail = Issue.init(env, .decode, .{ .expected = .array }) };
            if (json.array.items.len > max_elements) return .{ .fail = Issue.init(env, .decode, .size_limit) };
            const values = env.allocator.alloc(C.Decoded, json.array.items.len) catch return .{ .die = outcomes.Defect.out_of_memory };
            var count: usize = 0;
            for (json.array.items, 0..) |*item, index| {
                const decoded = decodeChild(env, item, state);
                switch (decoded) {
                    .success => |value| {
                        values[index] = value;
                        count += 1;
                    },
                    .fail => |issue| {
                        for (values[0..count]) |*value| outcomes.discardPayload(value);
                        env.allocator.free(values);
                        return atIndex(env, index, issue);
                    },
                    else => {
                        for (values[0..count]) |*value| outcomes.discardPayload(value);
                        env.allocator.free(values);
                        return outcomes.castFailure(Decoded, decoded);
                    },
                }
            }
            return .{ .success = .{ .items = values, .allocator = env.allocator } };
        }

        /// Decode a value from its wire representation.
        /// Parameters:
        /// - `json: *const std.json.Value`: JSON value or document to decode, encode, or validate.
        /// Returns (`effects.Effect(Decoded, Issue, R)`): decoded array whose elements passed the child schema.
        pub fn decode(json: *const std.json.Value) effects.Effect(Decoded, Issue, R) {
            return effects.Effect(Decoded, Issue, R).fromBorrowed(std.json.Value, json, struct {
                fn run(input: *const std.json.Value, env: *const R) outcomes.Exit(Decoded, Issue) {
                    return decodeDirect(env, input);
                }
            }.run);
        }

        /// The temporary allocator belongs to the parent's caller-owned
        /// scratch arena, never to a second user-supplied runtime argument.
        ///
        /// Parameters:
        /// - `env: *const R`: Borrowed caller environment supplying allocator, I/O, and selected
        ///   capabilities.
        /// - `value: *const Decoded`: Borrowed input; keep its owner alive until the operation has copied or
        ///   joined it.
        /// - `temporary: std.mem.Allocator`: Temporary state retained only for the duration of this operation.
        /// Returns (`outcomes.Exit(std.json.Value, Issue)`): Carries `std.json.Value` on `.success` or `Issue` on `.fail`; `.die`, `.interrupt`, and `.cleanup` preserve other terminal causes.
        pub fn encodeValueDirect(env: *const R, value: *const Decoded, temporary: std.mem.Allocator) outcomes.Exit(std.json.Value, Issue) {
            var state: structural.EncodeState = .{ .scratch = temporary };
            return encodeWithin(env, value, &state);
        }

        /// Encode within the caller-supplied depth or size limits.
        /// Parameters:
        /// - `env: *const R`: Borrowed caller environment supplying allocator, I/O, and selected
        ///   capabilities.
        /// - `value: *const Decoded`: Borrowed input; keep its owner alive until the operation has copied or
        ///   joined it.
        /// - `state: *structural.EncodeState`: Mutable operation state, retained for the duration of this call.
        /// Returns (`outcomes.Exit(std.json.Value, Issue)`): Carries `std.json.Value` on `.success` or `Issue` on `.fail`; `.die`, `.interrupt`, and `.cleanup` preserve other terminal causes.
        pub fn encodeWithin(env: *const R, value: *const Decoded, state: *structural.EncodeState) outcomes.Exit(std.json.Value, Issue) {
            if (value.items.len > max_elements) return .{ .fail = Issue.init(env, .encode, .size_limit) };
            var encoded = std.json.Array.init(state.scratch);
            for (value.items, 0..) |*item, index| {
                const next = encodeChild(env, item, state);
                switch (next) {
                    .success => |json_value| encoded.append(json_value) catch return .{ .die = outcomes.Defect.out_of_memory },
                    .fail => |original| {
                        var issue = original;
                        issue.bindAllocator(env);
                        issue.prependIndex(index) catch {
                            issue.deinit();
                            return .{ .die = outcomes.Defect.out_of_memory };
                        };
                        return .{ .fail = issue };
                    },
                    else => return outcomes.castFailure(std.json.Value, next),
                }
            }
            return .{ .success = .{ .array = encoded } };
        }

        /// Encode directly into owned bytes.
        /// Parameters:
        /// - `env: *const R`: Borrowed caller environment supplying allocator, I/O, and selected
        ///   capabilities.
        /// - `value: *const Decoded`: Borrowed input; keep its owner alive until the operation has copied or
        ///   joined it.
        /// Returns (`outcomes.Exit(outcomes.OwnedSlice(u8), Issue)`): Carries `outcomes.OwnedSlice(u8)` on `.success` or `Issue` on `.fail`; `.die`, `.interrupt`, and `.cleanup` preserve other terminal causes.
        pub fn encodeBytesDirect(env: *const R, value: *const Decoded) outcomes.Exit(outcomes.OwnedSlice(u8), Issue) {
            var arena = std.heap.ArenaAllocator.init(env.allocator);
            defer arena.deinit();
            var state: structural.EncodeState = .{ .scratch = arena.allocator() };
            const encoded = encodeWithin(env, value, &state);
            const json = switch (encoded) {
                .success => |item| item,
                else => return outcomes.castFailure(outcomes.OwnedSlice(u8), encoded),
            };
            const bytes = std.json.Stringify.valueAlloc(env.allocator, json, .{}) catch
                return .{ .die = outcomes.Defect.out_of_memory };
            return .{ .success = .{ .items = bytes, .allocator = env.allocator } };
        }

        /// Validate input immediately against this schema.
        /// Parameters:
        /// - `env: *const R`: Borrowed caller environment supplying allocator, I/O, and selected
        ///   capabilities.
        /// - `value: *const Decoded`: Borrowed input; keep its owner alive until the operation has copied or
        ///   joined it.
        /// Returns (`outcomes.Exit(void, Issue)`): Carries no value on `.success` or `Issue` on `.fail`; `.die`, `.interrupt`, and `.cleanup` preserve other terminal causes.
        pub fn validateDirect(env: *const R, value: *const Decoded) outcomes.Exit(void, Issue) {
            var arena = std.heap.ArenaAllocator.init(env.allocator);
            defer arena.deinit();
            var state: structural.EncodeState = .{ .scratch = arena.allocator() };
            const checked = encodeWithin(env, value, &state);
            return switch (checked) {
                .success => .{ .success = {} },
                else => outcomes.castFailure(void, checked),
            };
        }

        /// Validate the value against its declared contract.
        /// Parameters:
        /// - `value: *const Decoded`: Borrowed input; keep its owner alive until the operation has copied or
        ///   joined it.
        /// Returns (`effects.Effect(void, Issue, R)`): unit success when every array element satisfies the child schema.
        pub fn validate(value: *const Decoded) effects.Effect(void, Issue, R) {
            return effects.Effect(void, Issue, R).fromBorrowed(Decoded, value, struct {
                fn run(input: *const Decoded, env: *const R) outcomes.Exit(void, Issue) {
                    return validateDirect(env, input);
                }
            }.run);
        }

        /// Describe encoding to owned bytes as a lazy Effect.
        /// Parameters:
        /// - `value: *const Decoded`: Borrowed input; keep its owner alive until the operation has copied or
        ///   joined it.
        /// Returns (`effects.Effect(outcomes.OwnedSlice(u8), Issue, R)`): owned JSON bytes for the validated array representation.
        pub fn encodeBytes(value: *const Decoded) effects.Effect(outcomes.OwnedSlice(u8), Issue, R) {
            return effects.Effect(outcomes.OwnedSlice(u8), Issue, R).fromBorrowed(Decoded, value, struct {
                fn run(input: *const Decoded, env: *const R) outcomes.Exit(outcomes.OwnedSlice(u8), Issue) {
                    return encodeBytesDirect(env, input);
                }
            }.run);
        }

        /// Parse encoded bytes immediately under declared limits.
        /// Parameters:
        /// - `env: *const R`: Borrowed caller environment supplying allocator, I/O, and selected
        ///   capabilities.
        /// - `bytes: []const u8`: Borrowed input; keep its owner alive until the operation has copied or
        ///   joined it.
        /// - `limits: parser.Limits`: Caller-selected, validated behavior or resource bounds; no hidden default
        ///   is installed.
        /// Returns (`outcomes.Exit(legacy.Parsed(Decoded), Issue)`): Carries `legacy.Parsed(Decoded)` on `.success` or `Issue` on `.fail`; `.die`, `.interrupt`, and `.cleanup` preserve other terminal causes.
        pub fn parseBytesDirect(env: *const R, bytes: []const u8, limits: parser.Limits) outcomes.Exit(legacy.Parsed(Decoded), Issue) {
            return parser.parseWith(Decoded, env, bytes, .reject, limits, decodeDirect);
        }
    };
}
