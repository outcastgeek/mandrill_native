//! Distinct oneOf/anyOf/allOf semantics. Each run owns its candidates, decoded
//! branch values, raw input, and structured branch issues.
const std = @import("std");
const outcomes = @import("../outcome.zig");
const Issue = @import("schema_issue.zig").Issue;
const ast = @import("schema_ast.zig");
const values = @import("json_value.zig");
const structural = @import("internal/structural.zig");
const constraints = @import("json_constraints.zig");
/// one requires one successful branch, any retains every success, all requires every branch and compatible encodings.
pub const Mode = enum {
    /// Exactly one successful branch; multiple matches fail as ambiguous.
    one,
    /// At least one success; retain all successful decoded values and original JSON.
    any,
    /// Every branch succeeds and their encoded values merge without conflict.
    all,
};
/// Explicit retained-issue and borrowed representation budgets; no hidden branch/size limit is installed.
pub const Policy = struct {
    /// Positive maximum retained failing-branch Issues; extra failures are discarded safely.
    maximum_issues: usize,
    /// Positive JSON value/depth/digit budgets for candidate input representations.
    limits: @import("json_writer.zig").Limits,
};
fn C(comptime child: anytype) type {
    return if (@TypeOf(child) == type) child else @TypeOf(child);
}
fn Types(comptime branches: anytype) type {
    var types: [branches.len]type = undefined;
    inline for (branches, 0..) |branch, index| types[index] = ?C(branch).Decoded;
    return @Tuple(&types);
}
/// Build immutable branch dispatch with invocation-local candidates.
/// Success owns the copied input, successful decoded branches and match indices.
/// Typed branch failures are collected within policy; other Exit channels propagate with partial owners released.
pub fn Alternatives(comptime R: type, comptime branches: anytype, comptime mode: Mode, comptime policy: Policy) type {
    comptime {
        if (branches.len == 0) @compileError("alternative definition cannot be empty");
        if (policy.maximum_issues == 0 or !policy.limits.valid()) @compileError("alternatives require explicit positive issue and representation limits");
    }
    return struct {
        /// Caller environment supplying allocation and any selected schema capabilities.
        pub const Environment = R;
        /// Application value produced on success; ownership follows the codec’s declared value type.
        pub const Decoded = struct {
            /// Exit/Result cleanup owns this value; release it once or clone it for an independent observation.
            pub const effect_owned_payload = true;
            /// Owned deep copy anchoring borrowed decoded branch views.
            original: values.Owned,
            /// Optional decoded payload per branch; each marked owner is released by this result.
            successful: Types(branches),
            /// Owned indices of branches that accepted the input, in declaration order.
            matches: outcomes.OwnedSlice(usize),
            /// Owned compatible allOf merge, or null for modes without a combined document.
            combined: ?values.Owned,
            /// Release successful branch payloads, original/combined documents and match indices exactly once.
            pub fn deinit(self: *@This()) void {
                inline for (0..branches.len) |index| if (self.successful[index]) |*value| outcomes.discardPayload(value);
                self.original.deinit();
                self.matches.deinit();
                if (self.combined) |*value| value.deinit();
                self.* = undefined;
            }
            /// Clone all owners and rebase borrowed child strings into the copied original document.
            pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
                var result: @This() = .{ .original = try self.original.clone(), .successful = undefined, .matches = undefined, .combined = null };
                inline for (0..branches.len) |index| result.successful[index] = null;
                errdefer {
                    inline for (0..branches.len) |index| if (result.successful[index]) |*value| outcomes.discardPayload(value);
                    result.original.deinit();
                    if (result.combined) |*value| value.deinit();
                }
                inline for (0..branches.len) |index| if (self.successful[index]) |value| {
                    result.successful[index] = try values.cloneDecoded(value, &self.original.value, &result.original.value);
                };
                if (self.combined) |*value| result.combined = try value.clone();
                result.matches = try self.matches.clone();
                return result;
            }
        };
        /// Check input budgets, run each branch once and retain the mode’s accepted results with their copied document.
        pub fn decodeWithin(env: *const R, input: *const std.json.Value, traversal: *structural.DecodeState) outcomes.Exit(Decoded, Issue) {
            const checked_input = @import("json_surface.zig").inspect(env, input, policy.limits, 0, .decode);
            if (checked_input != .success) return outcomes.castFailure(Decoded, checked_input);
            var original = values.Owned.copy(env, input) catch return .{ .die = outcomes.Defect.out_of_memory };
            var owns_original = true;
            defer if (owns_original) original.deinit();
            var decoded: Types(branches) = undefined;
            inline for (0..branches.len) |index| decoded[index] = null;
            var matches = std.array_list.Managed(usize).init(env.allocator);
            defer matches.deinit();
            var issues = std.array_list.Managed(Issue).init(env.allocator);
            defer {
                for (issues.items) |*issue| issue.deinit();
                issues.deinit();
            }
            var transferred = false;
            defer if (!transferred) {
                inline for (0..branches.len) |index| if (decoded[index]) |*value| outcomes.discardPayload(value);
            };
            inline for (branches, 0..) |branch, index| {
                const result = constraints.decodeChild(branch, env, &original.value, traversal);
                switch (result) {
                    .success => |value| {
                        decoded[index] = value;
                        matches.append(index) catch return .{ .die = outcomes.Defect.out_of_memory };
                    },
                    .fail => |branch_issue| {
                        var issue = branch_issue;
                        issue.bindAllocator(env);
                        issue.prependAlternative(index) catch {
                            issue.deinit();
                            return .{ .die = outcomes.Defect.out_of_memory };
                        };
                        if (issues.items.len < @min(policy.maximum_issues, traversal.maximum_issues orelse policy.maximum_issues)) issues.append(issue) catch {
                            issue.deinit();
                            return .{ .die = outcomes.Defect.out_of_memory };
                        } else issue.deinit();
                    },
                    else => return outcomes.castFailure(Decoded, result),
                }
            }
            const accepted = switch (mode) {
                .one => matches.items.len == 1,
                .any => matches.items.len > 0,
                .all => matches.items.len == branches.len,
            };
            if (!accepted) {
                const owned_issues = issues.toOwnedSlice() catch return .{ .die = outcomes.Defect.out_of_memory };
                var issue = Issue.many(env, .decode, owned_issues, owned_issues.len == @min(policy.maximum_issues, traversal.maximum_issues orelse policy.maximum_issues) and matches.items.len + owned_issues.len < branches.len);
                issue.kind = .alternatives;
                issue.detail = .{ .alternatives = .{ .items = matches.toOwnedSlice() catch {
                    issue.deinit();
                    return .{ .die = outcomes.Defect.out_of_memory };
                }, .allocator = env.allocator } };
                return .{ .fail = issue };
            }
            var combined: ?values.Owned = null;
            if (mode == .all) {
                var arena = std.heap.ArenaAllocator.init(env.allocator);
                defer arena.deinit();
                var encoding: structural.EncodeState = .{ .scratch = arena.allocator() };
                inline for (branches, 0..) |branch, index| {
                    const encoded = if (@TypeOf(branch) == type) branch.encodeWithin(env, &decoded[index].?, &encoding) else blk: {
                        const result = branch.encodeDirect(&decoded[index].?);
                        break :blk switch (result) {
                            .ok => |value| outcomes.Exit(std.json.Value, Issue){ .success = value },
                            .err => |issue| outcomes.Exit(std.json.Value, Issue){ .fail = issue },
                        };
                    };
                    if (encoded != .success) {
                        if (combined) |*value| value.deinit();
                        return outcomes.castFailure(Decoded, encoded);
                    }
                    if (combined) |*value| {
                        merge(value.arena.allocator(), &value.value, &encoded.success) catch |problem| {
                            value.deinit();
                            return if (problem == error.OutOfMemory) .{ .die = outcomes.Defect.out_of_memory } else .{ .fail = Issue.init(env, .decode, .conflict) };
                        };
                    } else combined = values.Owned.copy(env, &encoded.success) catch {
                        return .{ .die = outcomes.Defect.out_of_memory };
                    };
                }
            }
            const owned_matches = matches.toOwnedSlice() catch {
                if (combined) |*value| value.deinit();
                return .{ .die = outcomes.Defect.out_of_memory };
            };
            transferred = true;
            owns_original = false;
            return .{ .success = .{ .original = original, .successful = decoded, .matches = .{ .items = owned_matches, .allocator = env.allocator }, .combined = combined } };
        }
        /// Decode with fresh traversal/candidate state; independent and concurrent runs share no mutable parse state.
        pub fn decodeDirect(env: *const R, input: *const std.json.Value) outcomes.Exit(Decoded, Issue) {
            var traversal: structural.DecodeState = .{};
            return decodeWithin(env, input, &traversal);
        }
        /// any preserves original representation; one encodes its selected child; all merges compatible encodings or fails conflict.
        /// Returned containers use caller scratch and any borrowed views require input’s owners to remain alive.
        pub fn encodeWithin(env: *const R, input: *const Decoded, traversal: *structural.EncodeState) outcomes.Exit(std.json.Value, Issue) {
            if (mode == .any) return .{ .success = input.original.value };
            var combined: ?std.json.Value = null;
            inline for (branches, 0..) |branch, index| if (input.successful[index]) |*value| {
                const encoded = if (@TypeOf(branch) == type) branch.encodeWithin(env, value, traversal) else blk: {
                    const result = branch.encodeDirect(value);
                    break :blk switch (result) {
                        .ok => |representation| outcomes.Exit(std.json.Value, Issue){ .success = representation },
                        .err => |issue| outcomes.Exit(std.json.Value, Issue){ .fail = issue },
                    };
                };
                if (encoded != .success) return encoded;
                if (mode == .one) return encoded;
                if (combined) |*representation| merge(traversal.scratch, representation, &encoded.success) catch |problem| {
                    return if (problem == error.OutOfMemory) .{ .die = outcomes.Defect.out_of_memory } else .{ .fail = Issue.init(env, .encode, .conflict) };
                } else combined = values.copyValue(traversal.scratch, encoded.success) catch return .{ .die = outcomes.Defect.out_of_memory };
            };
            return if (combined) |representation| .{ .success = representation } else .{ .fail = Issue.init(env, .encode, .alternatives) };
        }
        /// Return immutable encoded-shape metadata; it does not execute callbacks or construct application values.
        pub fn encodedNode() *const ast.Node {
            const Metadata = struct {
                fn nodes() [branches.len]*const ast.Node {
                    var result: [branches.len]*const ast.Node = undefined;
                    inline for (branches, 0..) |branch, index| result[index] = @import("internal/representation.zig").nodePointer(branch);
                    return result;
                }
                const children = nodes();
                const node: ast.Node = .{ .alternatives = .{ .mode = switch (mode) {
                    .one => .one,
                    .any => .any,
                    .all => .all,
                }, .branches = &children } };
            };
            return &Metadata.node;
        }
    };
}
fn merge(allocator: std.mem.Allocator, target: *std.json.Value, source: *const std.json.Value) error{ OutOfMemory, Conflict }!void {
    if (target.* == .object and source.* == .object) {
        var iterator = source.object.iterator();
        while (iterator.next()) |item| {
            if (target.object.getPtr(item.key_ptr.*)) |current| try merge(allocator, current, item.value_ptr) else try target.object.put(allocator, try allocator.dupe(u8, item.key_ptr.*), try values.copyValue(allocator, item.value_ptr.*));
        }
    } else if (!values.equal(target, source)) return error.Conflict;
}
/// Require exactly one matching branch; ambiguity is a typed alternatives issue with match indices.
pub fn oneOf(comptime R: type, comptime branches: anytype, comptime policy: Policy) type {
    return Alternatives(R, branches, .one, policy);
}
/// Accept one or more branches and preserve the original JSON representation on encoding.
pub fn anyOf(comptime R: type, comptime branches: anytype, comptime policy: Policy) type {
    return Alternatives(R, branches, .any, policy);
}
/// Require every branch and merge compatible encoded values; conflicting overlapping fields fail.
pub fn allOf(comptime R: type, comptime branches: anytype, comptime policy: Policy) type {
    return Alternatives(R, branches, .all, policy);
}
