//! Explicit, independent input, preparation, issue and output budgets.
const std = @import("std");
const outcomes = @import("../outcome.zig");
const effects = @import("../effect.zig");
const Issue = @import("schema_issue.zig").Issue;
const structural = @import("internal/structural.zig");
const writer = @import("json_writer.zig");
const parser = @import("internal/parser.zig");
const preparation = @import("internal/preparation.zig");
const ast = @import("schema_ast.zig");
/// Independent UTF-8 input, schema preparation, issue and output budgets selected from R.
pub const Limits = struct {
    /// Inclusive input document/value byte and JSON nesting budgets.
    input: parser.Limits,
    /// Inclusive immutable shape node/edge/depth budgets, checked without runtime preparation allocation.
    schema: preparation.Limits,
    /// Maximum retained branch/record issues in each accumulating collector.
    maximum_issues: usize,
    /// Maximum integer-part decimal digits after expanding an exponent.
    maximum_integer_digits: usize,
    /// Independent budgets for the encoded document; never an input-document byte limit.
    output: writer.Limits,
    /// Require positive input/schema/issue/digit/output bounds before parsing or encoding.
    pub fn valid(self: @This()) bool {
        return self.input.max_document_bytes > 0 and self.input.max_depth > 0 and self.input.max_value_bytes > 0 and self.schema.valid() and self.maximum_issues > 0 and self.maximum_integer_digits > 0 and self.output.valid();
    }
};
fn resource(env: anytype, direction: @import("schema_issue.zig").Direction, which: @import("schema_issue.zig").Detail.Resource, limit: usize, actual: usize) outcomes.Exit(void, Issue) {
    var issue = Issue.init(env, direction, .resource_limit);
    issue.detail = .{ .resource = .{ .resource = which, .limit = limit, .actual = actual } };
    return .{ .fail = issue };
}
/// Validate a borrowed DOM without serializing it or applying an output-byte cap.
pub fn inspect(env: anytype, value: *const std.json.Value, limits: writer.Limits, depth: usize, direction: @import("schema_issue.zig").Direction) outcomes.Exit(void, Issue) {
    if (depth >= limits.max_json_depth) return resource(env, direction, .json_depth, limits.max_json_depth, depth + 1);
    switch (value.*) {
        .string, .number_string => |text| {
            if (text.len > limits.max_value_bytes) return resource(env, direction, .value_bytes, limits.max_value_bytes, text.len);
            if (value.* == .string) {
                if (!std.unicode.utf8ValidateSlice(text)) return .{ .fail = Issue.init(env, direction, .invalid_utf8) };
            } else {
                const number = @import("json_number_codec.zig").Number.parse(text) catch return .{ .fail = Issue.init(env, direction, .invalid_number) };
                const digits = number.integerDigits() catch return resource(env, direction, .integer_digits, limits.max_integer_digits, std.math.maxInt(usize));
                if (digits > limits.max_integer_digits) return resource(env, direction, .integer_digits, limits.max_integer_digits, digits);
            }
        },
        .integer => |value_| {
            var buffer: [std.fmt.count("{d}", .{std.math.minInt(i64)})]u8 = undefined;
            const text = std.fmt.bufPrint(&buffer, "{d}", .{value_}) catch unreachable;
            if (text.len > limits.max_value_bytes) return resource(env, direction, .value_bytes, limits.max_value_bytes, text.len);
            const digits = (@import("json_number_codec.zig").Number.parse(text) catch unreachable).integerDigits() catch unreachable;
            if (digits > limits.max_integer_digits) return resource(env, direction, .integer_digits, limits.max_integer_digits, digits);
        },
        .float => |value_| {
            if (!std.math.isFinite(value_)) return .{ .fail = Issue.init(env, direction, .invalid_number) };
            var buffer: [std.fmt.count("{e}", .{-std.math.floatMax(f64)}) + std.fmt.count("{e}", .{std.math.floatMin(f64)})]u8 = undefined;
            const text = std.fmt.bufPrint(&buffer, "{e}", .{value_}) catch unreachable;
            const encoded: std.json.Value = .{ .number_string = text };
            return inspect(env, &encoded, limits, depth, direction);
        },
        .array => |array| for (array.items, 0..) |*item, index| {
            const checked = inspect(env, item, limits, depth + 1, direction);
            if (checked == .fail) {
                var issue = checked.fail;
                issue.prependIndex(index) catch {
                    issue.deinit();
                    return .{ .die = outcomes.Defect.out_of_memory };
                };
                return .{ .fail = issue };
            }
            if (checked != .success) return checked;
        },
        .object => |object| {
            var iterator = object.iterator();
            while (iterator.next()) |item| {
                if (item.key_ptr.*.len > limits.max_value_bytes) return resource(env, direction, .value_bytes, limits.max_value_bytes, item.key_ptr.*.len);
                if (!std.unicode.utf8ValidateSlice(item.key_ptr.*)) return .{ .fail = Issue.init(env, direction, .invalid_utf8) };
                const checked = inspect(env, item.value_ptr, limits, depth + 1, direction);
                if (checked == .fail) {
                    var issue = checked.fail;
                    issue.prependField(item.key_ptr.*) catch {
                        issue.deinit();
                        return .{ .die = outcomes.Defect.out_of_memory };
                    };
                    return .{ .fail = issue };
                }
                if (checked != .success) return checked;
            }
        },
        else => {},
    }
    return .{ .success = {} };
}
const Frame = struct { node: *const ast.Node, depth: usize };
fn children(comptime node: *const ast.Node) []const *const ast.Node {
    return switch (node.*) {
        .primitive, .exact_number, .enumeration, .reference => &.{},
        .nullable, .array => |child| &.{child},
        .constrained => |value| &.{value.base},
        .alternatives => |value| value.branches,
        .record => |value| blk: {
            const Metadata = struct {
                const nodes = nodes: {
                    var result: [value.fields.len]*const ast.Node = undefined;
                    for (value.fields, 0..) |field, index| result[index] = field.schema;
                    break :nodes result;
                };
            };
            break :blk &Metadata.nodes;
        },
        .tagged_union => |value| blk: {
            const Metadata = struct {
                const nodes = nodes: {
                    const count = count: {
                        var total: usize = 0;
                        for (value.cases) |case| if (case.payload != null) {
                            total += 1;
                        };
                        break :count total;
                    };
                    var result: [count]*const ast.Node = undefined;
                    var index: usize = 0;
                    for (value.cases) |case| if (case.payload) |payload| {
                        result[index] = payload;
                        index += 1;
                    };
                    break :nodes result;
                };
            };
            break :blk &Metadata.nodes;
        },
        .prefix_array => |value| blk: {
            const Metadata = struct {
                const nodes = nodes: {
                    var result: [value.elements.len + @as(usize, if (value.rest != null) 1 else 0)]*const ast.Node = undefined;
                    for (value.elements, 0..) |element, index| result[index] = element.schema;
                    if (value.rest) |child| result[value.elements.len] = child;
                    break :nodes result;
                };
            };
            break :blk &Metadata.nodes;
        },
        .object => |value| blk: {
            const Metadata = struct {
                const nodes = nodes: {
                    var result: [1 + value.patterns.len + @as(usize, if (value.property_names != null) 1 else 0) + @as(usize, if (value.additional != null) 1 else 0)]*const ast.Node = undefined;
                    result[0] = value.base;
                    var index: usize = 1;
                    for (value.patterns) |pattern| {
                        result[index] = pattern.schema;
                        index += 1;
                    }
                    if (value.property_names) |child| {
                        result[index] = child;
                        index += 1;
                    }
                    if (value.additional) |child| result[index] = child;
                    break :nodes result;
                };
            };
            break :blk &Metadata.nodes;
        },
    };
}
fn staticPlan(comptime pending: anytype, comptime seen: anytype, comptime edges: usize, comptime maximum_depth: usize) type {
    if (pending.len == 0) return struct {
        const nodes = seen.len;
        const edge_count = edges;
        const depth = maximum_depth;
    };
    const frame = pending[0];
    for (seen) |prior| if (prior == frame.node) return staticPlan(pending[1..].*, seen, edges, maximum_depth);
    if (ast.validateLocal(frame.node) == .err) @compileError("invalid static JSON schema metadata");
    const child_nodes = children(frame.node);
    var next: [pending.len - 1 + child_nodes.len]Frame = undefined;
    for (child_nodes, 0..) |child, index| next[index] = .{ .node = child, .depth = frame.depth + 1 };
    for (pending[1..], child_nodes.len..) |item, index| next[index] = item;
    return staticPlan(next, seen ++ [_]*const ast.Node{frame.node}, edges + child_nodes.len, @max(maximum_depth, frame.depth + 1));
}
/// Attach caller-selected bounded adapters to a structural codec in its existing R.
/// A Writer.Limits selector checks DOM values/encoded output. A Limits selector
/// additionally exposes bounded UTF-8 parsing and immutable preparation budgets.
/// Parsed success retains document-backed strings until deinit; every run uses
/// local traversal/candidate state. Static direct-writer evidence chooses tree-free
/// encoding, otherwise the representation is checked in a temporary arena.
pub fn WithLimits(comptime C: type, comptime select: anytype) type {
    const R = C.Environment;
    const Policy = @typeInfo(@TypeOf(select)).@"fn".return_type.?;
    const bounded_input = Policy == Limits;
    if (Policy != Limits and Policy != writer.Limits) @compileError("JSON surface selector must return explicit JSON surface or writer limits");
    return struct {
        /// Application value produced on success; ownership follows the codec’s declared value type.
        pub const Decoded = C.Decoded;
        /// Caller environment supplying allocation and any selected schema capabilities.
        pub const Environment = R;
        /// Return immutable encoded-shape metadata; it does not execute callbacks or construct application values.
        pub const encodedNode = C.encodedNode;
        /// Child decoder using the caller’s existing per-invocation traversal state and ownership scope.
        pub const decodeWithin = C.decodeWithin;
        /// Child representation encoder using the caller’s scratch/traversal scope; returned views remain scoped to their owners.
        pub const encodeWithin = C.encodeWithin;
        /// Public evidence for checked JSON writing without constructing an intermediate DOM.
        pub const supports_direct_writer = writer.capable(C);
        /// Delegate to the statically eligible child writer. Borrow value and output for this call; preserve every Exit channel.
        pub fn writeJson(env: *const R, value: *const Decoded, output: *writer.Writer) outcomes.Exit(void, Issue) {
            if (!supports_direct_writer) @compileError("codec has no direct JSON writer evidence");
            return C.writeJson(env, value, output);
        }
        const Plan = staticPlan([_]Frame{.{ .node = C.encodedNode(), .depth = 0 }}, [_]*const ast.Node{}, 0, 0);
        fn prepare(env: *const R, policy: Limits) outcomes.Exit(void, Issue) {
            // Immutable counts are compiled once. Runtime only compares budgets.
            if (Plan.depth > policy.schema.max_depth) return resource(env, .definition, .schema_depth, policy.schema.max_depth, Plan.depth);
            if (Plan.nodes > policy.schema.max_nodes) return resource(env, .definition, .schema_nodes, policy.schema.max_nodes, Plan.nodes);
            if (Plan.edge_count > policy.schema.max_edges) return resource(env, .definition, .schema_edges, policy.schema.max_edges, Plan.edge_count);
            return .{ .success = {} };
        }
        /// Validate a borrowed DOM against input value/depth/digit and static-schema budgets before decoding.
        /// Does not apply an output-byte cap or serialize input just to validate it.
        pub fn decodeDirect(env: *const R, value: *const std.json.Value) outcomes.Exit(Decoded, Issue) {
            const policy = select(env);
            if (!policy.valid()) return .{ .fail = Issue.init(env, .decode, .invalid_value) };
            const input_limits = if (comptime bounded_input) writer.Limits{ .max_output_bytes = policy.input.max_document_bytes, .max_json_depth = policy.input.max_depth, .max_value_bytes = policy.input.max_value_bytes, .max_integer_digits = policy.maximum_integer_digits } else policy;
            const checked = inspect(env, value, input_limits, 0, .decode);
            if (checked != .success) return outcomes.castFailure(Decoded, checked);
            if (comptime bounded_input) {
                const ready = prepare(env, policy);
                if (ready != .success) return outcomes.castFailure(Decoded, ready);
            }
            var traversal: structural.DecodeState = .{ .maximum_depth = if (bounded_input) policy.schema.max_depth else null, .maximum_issues = if (bounded_input) policy.maximum_issues else null };
            return C.decodeWithin(env, value, &traversal);
        }
        /// Parse bounded UTF-8 through std.json with the chosen duplicate-field policy.
        /// Success owns Parsed(A); retain that owner while decoded views borrow its document.
        /// Input bytes are borrowed only during this call; syntax/limit errors become owned Issues and OOM becomes .die.
        pub fn parseBytesDirect(env: *const R, bytes: []const u8, comptime duplicates: ast.DuplicateFields) outcomes.Exit(@import("../schema.zig").Parsed(Decoded), Issue) {
            if (!bounded_input) @compileError("UTF-8 parsing requires independent input and schema limits");
            const policy = select(env);
            if (!policy.valid()) return .{ .fail = Issue.init(env, .decode, .invalid_value) };
            var input_limits = policy.input;
            input_limits.max_integer_digits = policy.maximum_integer_digits;
            return parser.parseWith(Decoded, env, bytes, duplicates, input_limits, decodeDirect);
        }
        /// Lazy borrowed-byte adapter for parseBytesDirect. Keep the slice pointer and its bytes alive until join.
        pub fn parseBytes(bytes: *const []const u8, comptime duplicates: ast.DuplicateFields) effects.Effect(@import("../schema.zig").Parsed(Decoded), Issue, R) {
            return effects.Effect(@import("../schema.zig").Parsed(Decoded), Issue, R).fromBorrowed([]const u8, bytes, struct {
                fn run(input: *const []const u8, env: *const R) outcomes.Exit(@import("../schema.zig").Parsed(Decoded), Issue) {
                    return parseBytesDirect(env, input.*, duplicates);
                }
            }.run);
        }
        /// Lazy borrowed-DOM adapter; each run selects its own R limits and traversal state.
        pub fn decode(value: *const std.json.Value) effects.Effect(Decoded, Issue, R) {
            return effects.Effect(Decoded, Issue, R).fromBorrowed(std.json.Value, value, struct {
                fn run(input: *const std.json.Value, env: *const R) outcomes.Exit(Decoded, Issue) {
                    return decodeDirect(env, input);
                }
            }.run);
        }
        /// Encode once into caller-owned bytes: direct writer when eligible, otherwise a checked temporary representation.
        /// Scratch is released before returning; failures preserve typed issues and other Exit channels.
        pub fn encodeBytesDirect(env: *const R, value: *const Decoded) outcomes.Exit(outcomes.OwnedSlice(u8), Issue) {
            const policy = select(env);
            if (!policy.valid()) return .{ .fail = Issue.init(env, .encode, .invalid_value) };
            const limits = if (comptime bounded_input) policy.output else policy;
            if (comptime bounded_input) {
                const ready = prepare(env, policy);
                if (ready != .success) return outcomes.castFailure(outcomes.OwnedSlice(u8), ready);
            }
            var output = writer.Writer.init(env, limits);
            defer output.deinit();
            if (comptime supports_direct_writer) {
                const encoded = C.writeJson(env, value, &output);
                if (encoded != .success) return outcomes.castFailure(outcomes.OwnedSlice(u8), encoded);
            } else {
                var arena = std.heap.ArenaAllocator.init(env.allocator);
                defer arena.deinit();
                var traversal: structural.EncodeState = .{ .scratch = arena.allocator(), .maximum_depth = if (bounded_input) policy.schema.max_depth else null };
                const encoded = C.encodeWithin(env, value, &traversal);
                if (encoded != .success) return outcomes.castFailure(outcomes.OwnedSlice(u8), encoded);
                output.json(&encoded.success) catch |problem| return writer.failureWith(env, .encode, problem, &output);
            }
            return .{ .success = output.finish() catch return .{ .die = outcomes.Defect.out_of_memory } };
        }
        /// Lazy encoding adapter; borrow the value until join and release successful bytes with deinit.
        pub fn encodeBytes(value: *const Decoded) effects.Effect(outcomes.OwnedSlice(u8), Issue, R) {
            return effects.Effect(outcomes.OwnedSlice(u8), Issue, R).fromBorrowed(Decoded, value, struct {
                fn run(input: *const Decoded, env: *const R) outcomes.Exit(outcomes.OwnedSlice(u8), Issue) {
                    return encodeBytesDirect(env, input);
                }
            }.run);
        }
        /// Execute this surface’s encoding once and discard its successful bytes; validation uses encoding rules.
        pub fn validateDirect(env: *const R, value: *const Decoded) outcomes.Exit(void, Issue) {
            var encoded = encodeBytesDirect(env, value);
            if (encoded != .success) return outcomes.castFailure(void, encoded);
            encoded.success.deinit();
            return .{ .success = {} };
        }
    };
}
