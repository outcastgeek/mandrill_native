//! Named tagged unions with compile-time checked Zig constructors and tags.
const std = @import("std");
const outcomes = @import("../outcome.zig");
const effects = @import("../effect.zig");
const Issue = @import("schema_issue.zig").Issue;
const legacy = @import("../schema.zig");
const structural = @import("internal/structural.zig");
const ast = @import("schema_ast.zig");
const representation = @import("internal/representation.zig");

/// The wire tag is explicit metadata; renaming a Zig field cannot silently
/// change data already stored or exchanged with another process.
pub fn taggedCase(comptime field_name: []const u8, comptime wire_tag: []const u8, comptime codec: anytype) @TypeOf(.{
    .field_name = field_name,
    .wire_tag = wire_tag,
    .codec = codec,
    .nested = false,
}) {
    return .{ .field_name = field_name, .wire_tag = wire_tag, .codec = codec, .nested = false };
}

pub fn nestedCase(comptime field_name: []const u8, comptime wire_tag: []const u8, comptime Child: type) @TypeOf(.{
    .field_name = field_name,
    .wire_tag = wire_tag,
    .codec = Child,
    .nested = true,
}) {
    return .{ .field_name = field_name, .wire_tag = wire_tag, .codec = Child, .nested = true };
}

fn caseCodecType(comptime item: anytype) type {
    return if (item.nested) item.codec else @TypeOf(item.codec);
}

pub fn Variant(
    comptime T: type,
    comptime R: type,
    comptime discriminator: []const u8,
    comptime payload_name: []const u8,
    comptime cases: anytype,
) type {
    comptime {
        const info = @typeInfo(T);
        if (info != .@"union" or info.@"union".tag_type == null)
            @compileError("Variant requires a tagged Zig union");
        if (@typeInfo(@TypeOf(cases)) != .@"struct" or !@typeInfo(@TypeOf(cases)).@"struct".is_tuple)
            @compileError("Variant cases must be a tuple");
        if (cases.len == 0 or cases.len != info.@"union".fields.len)
            @compileError("Variant requires exactly one case for each Zig union field");
        if (discriminator.len == 0 or payload_name.len == 0 or
            !std.unicode.utf8ValidateSlice(discriminator) or !std.unicode.utf8ValidateSlice(payload_name) or
            std.mem.eql(u8, discriminator, payload_name))
            @compileError("Variant discriminator and payload keys must be distinct nonempty UTF-8");
        if (!@hasField(R, "allocator") or @FieldType(R, "allocator") != std.mem.Allocator)
            @compileError("Variant requires R.allocator");
        for (cases, 0..) |item, index| {
            if (!@hasField(T, item.field_name)) @compileError("Variant case is not a Zig union field");
            if (item.wire_tag.len == 0 or !std.unicode.utf8ValidateSlice(item.wire_tag))
                @compileError("Variant wire tags must be nonempty UTF-8");
            for (cases, 0..) |earlier, earlier_index| {
                if (earlier_index < index) {
                    if (std.mem.eql(u8, earlier.field_name, item.field_name)) @compileError("duplicate Variant Zig case");
                    if (std.mem.eql(u8, earlier.wire_tag, item.wire_tag)) @compileError("duplicate Variant wire tag");
                }
            }
            const C = caseCodecType(item);
            if (item.nested) {
                if (!@hasDecl(C, "Decoded") or !@hasDecl(C, "Environment") or C.Environment != R or
                    !@hasDecl(C, "decodeWithin") or !@hasDecl(C, "encodeWithin"))
                    @compileError("nested Variant cases require a structural codec in the same environment");
            } else {
                if (C.Representation != std.json.Value or C.DecodingEnvironment != void or C.EncodingEnvironment != void)
                    @compileError("Variant cases require pure JSON codecs");
            }
            if (C.Decoded != @FieldType(T, item.field_name))
                @compileError("Variant case codec does not match its Zig payload");
            if (outcomes.ownsPayload(C.Decoded) and !outcomes.ownsPayload(T))
                @compileError("a variant with an owned case must declare its own deinit/clone ownership contract");
        }
    }
    return struct {
        pub const Decoded = T;
        pub const Environment = R;

        fn encodedCases() [cases.len]ast.Case {
            var result: [cases.len]ast.Case = undefined;
            for (cases, 0..) |item, index| {
                result[index] = .{ .tag = item.wire_tag, .payload = representation.nodePointer(item.codec) };
            }
            return result;
        }

        pub fn encodedNode() *const ast.Node {
            const Metadata = struct {
                const case_nodes = encodedCases();
                const node: ast.Node = .{ .tagged_union = .{
                    .discriminator = discriminator,
                    .payload_name = payload_name,
                    .cases = &case_nodes,
                } };
            };
            return &Metadata.node;
        }

        pub fn make(comptime name: []const u8, payload: @FieldType(T, name)) T {
            inline for (cases) |item| {
                if (std.mem.eql(u8, item.field_name, name)) return @unionInit(T, name, payload);
            }
            @compileError("Zig case is not part of this variant schema");
        }

        pub fn project(comptime name: []const u8, value: *const T) ?*const @FieldType(T, name) {
            inline for (cases) |item| {
                if (std.mem.eql(u8, item.field_name, name)) {
                    if (!std.mem.eql(u8, @tagName(std.meta.activeTag(value.*)), name)) return null;
                    return &@field(value.*, name);
                }
            }
            @compileError("Zig case is not part of this variant schema");
        }

        fn at(env: *const R, name: []const u8, kind: @import("schema_issue.zig").Kind) outcomes.Exit(T, Issue) {
            var problem = Issue.init(env, .decode, kind);
            problem.prependField(name) catch {
                problem.deinit();
                return .{ .die = outcomes.Defect.out_of_memory };
            };
            return .{ .fail = problem };
        }

        fn decodeCase(comptime item: anytype, env: *const R, payload: *const std.json.Value, state: *structural.DecodeState) outcomes.Exit(caseCodecType(item).Decoded, Issue) {
            if (comptime item.nested) return item.codec.decodeWithin(env, payload, state);
            return switch (item.codec.decodeDirect(payload)) {
                .ok => |value| .{ .success = value },
                .err => |problem| .{ .fail = problem },
            };
        }

        fn encodeCase(comptime item: anytype, env: *const R, value: *const caseCodecType(item).Decoded, state: *structural.EncodeState) outcomes.Exit(std.json.Value, Issue) {
            if (comptime item.nested) return item.codec.encodeWithin(env, value, state);
            return switch (item.codec.encodeDirect(value)) {
                .ok => |encoded| .{ .success = encoded },
                .err => |problem| .{ .fail = problem },
            };
        }

        pub fn decodeDirect(env: *const R, json: *const std.json.Value) outcomes.Exit(T, Issue) {
            var state: structural.DecodeState = .{};
            return decodeWithin(env, json, &state);
        }

        pub fn decodeWithin(env: *const R, json: *const std.json.Value, state: *structural.DecodeState) outcomes.Exit(T, Issue) {
            if (json.* != .object) return .{ .fail = Issue.init(env, .decode, .{ .expected = .object }) };
            for (json.object.keys()) |key| {
                if (!std.mem.eql(u8, key, discriminator) and !std.mem.eql(u8, key, payload_name))
                    return at(env, key, .unknown_field);
            }
            const tag_json = json.object.get(discriminator) orelse return at(env, discriminator, .missing_field);
            if (tag_json != .string) return at(env, discriminator, .{ .expected = .string });
            if (!std.unicode.utf8ValidateSlice(tag_json.string)) return at(env, discriminator, .invalid_utf8);
            inline for (cases) |item| {
                if (std.mem.eql(u8, tag_json.string, item.wire_tag)) {
                    const payload = json.object.get(payload_name) orelse return at(env, payload_name, .missing_field);
                    const decoded = decodeCase(item, env, &payload, state);
                    return switch (decoded) {
                        .success => |value| .{ .success = @unionInit(T, item.field_name, value) },
                        .fail => |original| blk: {
                            var issue = original;
                            issue.bindAllocator(env);
                            issue.prependField(payload_name) catch {
                                issue.deinit();
                                break :blk .{ .die = outcomes.Defect.out_of_memory };
                            };
                            break :blk .{ .fail = issue };
                        },
                        else => outcomes.castFailure(T, decoded),
                    };
                }
            }
            return at(env, discriminator, .{ .expected = .tagged_case });
        }

        pub fn decode(json: *const std.json.Value) effects.Effect(T, Issue, R) {
            return effects.Effect(T, Issue, R).fromBorrowed(std.json.Value, json, struct {
                fn run(input: *const std.json.Value, env: *const R) outcomes.Exit(T, Issue) {
                    return decodeDirect(env, input);
                }
            }.run);
        }

        /// Structural parents pass a scratch allocator derived from R. The
        /// resulting JSON value borrows that scratch storage until stringify.
        pub fn encodeValueDirect(env: *const R, value: *const T, temporary: std.mem.Allocator) outcomes.Exit(std.json.Value, Issue) {
            var state: structural.EncodeState = .{ .scratch = temporary };
            return encodeWithin(env, value, &state);
        }

        pub fn encodeWithin(env: *const R, value: *const T, state: *structural.EncodeState) outcomes.Exit(std.json.Value, Issue) {
            var object: std.json.ObjectMap = .{};
            const active = @tagName(std.meta.activeTag(value.*));
            inline for (cases) |item| {
                if (std.mem.eql(u8, active, item.field_name)) {
                    object.put(state.scratch, discriminator, .{ .string = item.wire_tag }) catch
                        return .{ .die = outcomes.Defect.out_of_memory };
                    const payload = @field(value.*, item.field_name);
                    const encoded = encodeCase(item, env, &payload, state);
                    switch (encoded) {
                        .success => |json_payload| object.put(state.scratch, payload_name, json_payload) catch
                            return .{ .die = outcomes.Defect.out_of_memory },
                        .fail => |original| {
                            var issue = original;
                            issue.bindAllocator(env);
                            issue.prependField(payload_name) catch {
                                issue.deinit();
                                return .{ .die = outcomes.Defect.out_of_memory };
                            };
                            return .{ .fail = issue };
                        },
                        else => return outcomes.castFailure(std.json.Value, encoded),
                    }
                    return .{ .success = .{ .object = object } };
                }
            }
            unreachable; // Exhaustive comptime case validation above.
        }

        pub fn encodeBytesDirect(env: *const R, value: *const T) outcomes.Exit(outcomes.OwnedSlice(u8), Issue) {
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

        pub fn validateDirect(env: *const R, value: *const T) outcomes.Exit(void, Issue) {
            var arena = std.heap.ArenaAllocator.init(env.allocator);
            defer arena.deinit();
            var state: structural.EncodeState = .{ .scratch = arena.allocator() };
            const checked = encodeWithin(env, value, &state);
            return switch (checked) {
                .success => .{ .success = {} },
                else => outcomes.castFailure(void, checked),
            };
        }

        pub fn validate(value: *const T) effects.Effect(void, Issue, R) {
            return effects.Effect(void, Issue, R).fromBorrowed(T, value, struct {
                fn run(input: *const T, env: *const R) outcomes.Exit(void, Issue) {
                    return validateDirect(env, input);
                }
            }.run);
        }

        pub fn encodeBytes(value: *const T) effects.Effect(outcomes.OwnedSlice(u8), Issue, R) {
            return effects.Effect(outcomes.OwnedSlice(u8), Issue, R).fromBorrowed(T, value, struct {
                fn run(input: *const T, env: *const R) outcomes.Exit(outcomes.OwnedSlice(u8), Issue) {
                    return encodeBytesDirect(env, input);
                }
            }.run);
        }

        pub fn parseBytesDirect(
            env: *const R,
            bytes: []const u8,
            limits: @import("schema_builder.zig").ParseLimits,
        ) outcomes.Exit(legacy.Parsed(T), Issue) {
            return @import("schema_builder.zig").parseWith(T, env, bytes, .reject, limits, decodeDirect);
        }
    };
}
