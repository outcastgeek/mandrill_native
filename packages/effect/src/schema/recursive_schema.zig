//! Lazy boxed structural composition for recursive Zig application types.
//! A schema factory is evaluated at comptime only when an operation uses it;
//! the box breaks the otherwise infinite type-level expansion.
const std = @import("std");
const outcomes = @import("../outcome.zig");
const effects = @import("../effect.zig");
const Issue = @import("schema_issue.zig").Issue;
const structural = @import("internal/structural.zig");
const parser = @import("internal/parser.zig");
const legacy = @import("../schema.zig");
const ast = @import("schema_ast.zig");

pub fn OwnedBox(comptime T: type) type {
    return struct {
        const Self = @This();
        pub const effect_owned_payload = true;
        value: *T,
        allocator: std.mem.Allocator,

        /// Consumes `item`, including when the caller allocator cannot retain
        /// it. The allocator is taken from R, never passed separately.
        pub fn init(env: anytype, item: T) outcomes.Exit(Self, Issue) {
            const R = @typeInfo(@TypeOf(env)).pointer.child;
            if (!@hasField(R, "allocator") or @FieldType(R, "allocator") != std.mem.Allocator)
                @compileError("OwnedBox requires R.allocator");
            const storage = env.allocator.create(T) catch {
                var discarded = item;
                outcomes.discardPayload(&discarded);
                return .{ .die = outcomes.Defect.out_of_memory };
            };
            storage.* = item;
            return .{ .success = .{ .value = storage, .allocator = env.allocator } };
        }

        pub fn deinit(self: *Self) void {
            outcomes.discardPayload(self.value);
            self.allocator.destroy(self.value);
            self.* = undefined;
        }

        pub fn clone(self: *const Self) error{OutOfMemory}!Self {
            const copied = try self.allocator.create(T);
            errdefer self.allocator.destroy(copied);
            copied.* = try outcomes.clonePayload(self.value.*);
            return .{ .value = copied, .allocator = self.allocator };
        }
    };
}

/// `max_depth` is an explicit caller-authored schema policy. Each box consumes
/// one depth unit in invocation-local state, so cycles fail as typed issues.
pub fn Box(comptime T: type, comptime R: type, comptime max_depth: usize, comptime reference_name: []const u8, comptime schema: fn () type) type {
    comptime {
        if (max_depth == 0) @compileError("recursive schema depth must permit at least one box");
        if (reference_name.len == 0 or !std.unicode.utf8ValidateSlice(reference_name))
            @compileError("recursive schema requires a stable named reference");
        if (!@hasField(R, "allocator") or @FieldType(R, "allocator") != std.mem.Allocator)
            @compileError("recursive schema requires R.allocator");
    }
    return struct {
        const Self = @This();
        pub const Decoded = OwnedBox(T);
        pub const Environment = R;

        pub fn encodedNode() *const ast.Node {
            const Metadata = struct {
                const node: ast.Node = .{ .reference = reference_name };
            };
            return &Metadata.node;
        }

        fn child() type {
            const C = schema();
            if (!@hasDecl(C, "Decoded") or C.Decoded != T or
                !@hasDecl(C, "Environment") or C.Environment != R or
                !@hasDecl(C, "decodeWithin") or !@hasDecl(C, "encodeWithin"))
                @compileError("recursive schema factory must return a structural schema for T in R");
            return C;
        }

        pub fn decodeWithin(env: *const R, json: *const std.json.Value, state: *structural.DecodeState) outcomes.Exit(Decoded, Issue) {
            if (!state.enter(max_depth)) return .{ .fail = Issue.init(env, .decode, .depth_limit) };
            defer state.leave();
            const C = comptime child();
            const decoded = C.decodeWithin(env, json, state);
            const value = switch (decoded) {
                .success => |item| item,
                else => return outcomes.castFailure(Decoded, decoded),
            };
            return OwnedBox(T).init(env, value);
        }

        pub fn decodeDirect(env: *const R, json: *const std.json.Value) outcomes.Exit(Decoded, Issue) {
            var state: structural.DecodeState = .{};
            return decodeWithin(env, json, &state);
        }

        pub fn decode(json: *const std.json.Value) effects.Effect(Decoded, Issue, R) {
            return effects.Effect(Decoded, Issue, R).fromBorrowed(std.json.Value, json, struct {
                fn run(input: *const std.json.Value, env: *const R) outcomes.Exit(Decoded, Issue) {
                    return decodeDirect(env, input);
                }
            }.run);
        }

        pub fn encodeWithin(env: *const R, value: *const Decoded, state: *structural.EncodeState) outcomes.Exit(std.json.Value, Issue) {
            if (!state.enter(max_depth)) return .{ .fail = Issue.init(env, .encode, .depth_limit) };
            defer state.leave();
            const C = comptime child();
            return C.encodeWithin(env, value.value, state);
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
