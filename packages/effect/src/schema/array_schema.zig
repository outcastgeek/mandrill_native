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
        pub const Decoded = outcomes.OwnedSlice(C.Decoded);
        pub const Environment = R;

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

        pub fn decodeDirect(env: *const R, json: *const std.json.Value) outcomes.Exit(Decoded, Issue) {
            var state: structural.DecodeState = .{};
            return decodeWithin(env, json, &state);
        }

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

        pub fn decode(json: *const std.json.Value) effects.Effect(Decoded, Issue, R) {
            return effects.Effect(Decoded, Issue, R).fromBorrowed(std.json.Value, json, struct {
                fn run(input: *const std.json.Value, env: *const R) outcomes.Exit(Decoded, Issue) {
                    return decodeDirect(env, input);
                }
            }.run);
        }

        /// The temporary allocator belongs to the parent's caller-owned
        /// scratch arena, never to a second user-supplied runtime argument.
        pub fn encodeValueDirect(env: *const R, value: *const Decoded, temporary: std.mem.Allocator) outcomes.Exit(std.json.Value, Issue) {
            var state: structural.EncodeState = .{ .scratch = temporary };
            return encodeWithin(env, value, &state);
        }

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

        pub fn validate(value: *const Decoded) effects.Effect(void, Issue, R) {
            return effects.Effect(void, Issue, R).fromBorrowed(Decoded, value, struct {
                fn run(input: *const Decoded, env: *const R) outcomes.Exit(void, Issue) {
                    return validateDirect(env, input);
                }
            }.run);
        }

        pub fn encodeBytes(value: *const Decoded) effects.Effect(outcomes.OwnedSlice(u8), Issue, R) {
            return effects.Effect(outcomes.OwnedSlice(u8), Issue, R).fromBorrowed(Decoded, value, struct {
                fn run(input: *const Decoded, env: *const R) outcomes.Exit(outcomes.OwnedSlice(u8), Issue) {
                    return encodeBytesDirect(env, input);
                }
            }.run);
        }

        pub fn parseBytesDirect(env: *const R, bytes: []const u8, limits: parser.Limits) outcomes.Exit(legacy.Parsed(Decoded), Issue) {
            return parser.parseWith(Decoded, env, bytes, .reject, limits, decodeDirect);
        }
    };
}
