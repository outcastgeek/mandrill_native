//! Deterministic traversal identities are encoded from execution, declaration
//! and business row keys. The existing collection owner drives all children.
const std = @import("std");
const effects = @import("../effect.zig");
const outcomes = @import("../outcome.zig");
const keys = @import("../primary_key.zig");
const types = @import("workflow_types.zig");
const roles = @import("../dsl/roles.zig");
const inference = @import("../dsl/inference.zig");
const collections = @import("../dsl/collections.zig");
pub const Error = enum { invalid_identity, duplicate_identity };
pub const Iteration = struct { index: usize, key: []const u8 };
fn Values(comptime C: type, comptime R: type, comptime supplied: anytype) type {
    if (roles.has(roles.recordType(supplied), "static_items")) return @TypeOf(roles.get("static_items", supplied));
    return inference.result(roles.get("items", supplied), &.{ *const C, *const R }, "workflow.forEach.items");
}
fn Item(comptime T: type) type {
    return switch (@typeInfo(T)) {
        .array => |a| a.child,
        .pointer => |p| if (p.size == .slice) p.child else if (p.size == .one and @typeInfo(p.child) == .array) @typeInfo(p.child).array.child else @compileError("workflow.forEach.items requires a typed slice or array"),
        else => @compileError("workflow.forEach.items requires a typed slice or array"),
    };
}
pub fn Description(comptime C: type, comptime R: type, comptime E: type, comptime supplied: anytype, comptime admit: anytype, comptime lower: anytype) type {
    roles.validate(roles.recordType(supplied), "workflow.forEach", &.{ "name", "version", "key_field", "execution", "body", "map_error" }, &.{ "items", "static_items" });
    const static_items = roles.has(roles.recordType(supplied), "static_items");
    if (static_items == roles.has(roles.recordType(supplied), "items")) @compileError("workflow.forEach requires exactly one of items or static_items");
    const name: []const u8 = roles.get("name", supplied);
    const version: u32 = roles.get("version", supplied);
    if (name.len == 0 or !std.unicode.utf8ValidateSlice(name) or version == 0) @compileError("workflow.forEach requires a stable nonempty name and nonzero version");
    const I = Item(Values(C, R, supplied));
    const field: []const u8 = roles.get("key_field", supplied);
    if (@typeInfo(I) != .@"struct" or !@hasField(I, field) or @FieldType(I, field) != []const u8) @compileError("workflow.forEach.key_field must select an existing []const u8 business identity");
    const Body = inference.result(roles.get("body", supplied), &.{ *const C, *const I, *const Iteration, *const R }, "workflow.forEach.body");
    admit(Body);
    if (Body.Error != E or Body.Environment != R) @compileError("workflow.forEach body error/environment mismatch");
    if (inference.result(roles.get("execution", supplied), &.{ *const C, *const R }, "workflow.forEach.execution") != types.ExecutionId or inference.unary(roles.get("map_error", supplied), Error, "workflow.forEach.map_error") != E) @compileError("workflow.forEach identity roles must match native execution and current error channels");
    if (static_items) {
        const values = roles.get("static_items", supplied);
        for (values, 0..) |item, index| {
            const id = @field(item, field);
            if (id.len == 0 or !std.unicode.utf8ValidateSlice(id)) @compileError("workflow.forEach static row has an invalid identity");
            for (values[0..index]) |previous| if (std.mem.eql(u8, @field(previous, field), id)) @compileError("workflow.forEach has duplicate static row identities");
        }
    }
    return struct {
        pub const Success = outcomes.OwnedSlice(Body.Success);
        pub const Out = effects.Effect(Success, E, R);
        const Row = struct {
            pub const effect_owned_payload = true;
            item: *const I,
            encoded: outcomes.OwnedSlice(u8),
            iteration: Iteration,
            pub fn deinit(self: *@This()) void {
                self.encoded.deinit();
            }
            pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
                const encoded = try self.encoded.clone();
                return .{ .item = self.item, .encoded = encoded, .iteration = .{ .index = self.iteration.index, .key = encoded.items } };
            }
        };
        const Rows = outcomes.OwnedSlice(Row);
        const declared_items = if (static_items) roles.get("static_items", supplied) else {};
        fn prepare(input: *const C, env: *const R) outcomes.Exit(Rows, E) {
            const execution = roles.get("execution", supplied)(input, env);
            if (execution.validate() != .ok) return .{ .fail = roles.get("map_error", supplied)(.invalid_identity) };
            const values: []const I = if (comptime static_items) &declared_items else roles.get("items", supplied)(input, env);
            // Validate the complete identity set before admitting any child.
            for (values, 0..) |item, index| {
                const id = @field(item, field);
                if (id.len == 0 or !std.unicode.utf8ValidateSlice(id)) return .{ .fail = roles.get("map_error", supplied)(.invalid_identity) };
                for (values[0..index]) |previous| if (std.mem.eql(u8, @field(previous, field), id)) return .{ .fail = roles.get("map_error", supplied)(.duplicate_identity) };
            }
            const rows = env.allocator.alloc(Row, values.len) catch return .{ .die = outcomes.Defect.out_of_memory };
            var initialized: usize = 0;
            while (initialized < rows.len) : (initialized += 1) {
                const encoded = keys.encodeDirect(env, "workflow-traversal", &.{ .{ .bytes = execution.encoded }, .{ .text = name }, .{ .unsigned = version }, .{ .text = @field(values[initialized], field) } });
                if (encoded != .success) {
                    for (rows[0..initialized]) |*row| row.deinit();
                    env.allocator.free(rows);
                    return switch (encoded) {
                        .die => |defect| .{ .die = defect },
                        else => .{ .fail = roles.get("map_error", supplied)(.invalid_identity) },
                    };
                }
                rows[initialized] = .{ .item = &values[initialized], .encoded = encoded.success, .iteration = .{ .index = initialized, .key = encoded.success.items } };
            }
            return .{ .success = .{ .items = rows, .allocator = env.allocator } };
        }
        pub fn run(input: *const C, env: *const R) Out {
            var preparing = effects.Effect(Rows, E, R).fromBorrowed(C, input, prepare);
            return preparing.flatMapCapturedOwned(env, Success, input, struct {
                fn prepared(pointer: *const *const C, rows: Rows, current: *const R) Out {
                    return collections.forEachOwned(current, rows, pointer.*, .{ .body = struct {
                        fn body(stored: *const *const C, row: *const Row, latest: *const R) effects.Effect(Body.Success, E, R) {
                            return lower(roles.get("body", supplied)(stored.*, row.item, &row.iteration, latest));
                        }
                    }.body });
                }
            }.prepared);
        }
    };
}
