//! Bounded, lazy preparation of dynamic schema graphs. Static typed codecs do
//! not enter this path; runtime definitions are checked once per definition set.
const std = @import("std");
const outcomes = @import("../../outcome.zig");
const ast = @import("../schema_ast.zig");

/// Bounds on schema definition preparation and recursive expansion.
pub const Limits = struct {
    max_nodes: usize,
    max_edges: usize,
    max_depth: usize,

    /// Check whether this value satisfies its structural invariants.
    /// Parameters:
    /// - `self: Limits`: Receiver passed by value: the struct itself is copied, while any referenced storage keeps its original ownership and lifetime.
    /// Returns (`bool`): `true` when node, edge, and depth limits are positive.
    pub fn valid(self: Limits) bool {
        return self.max_nodes > 0 and self.max_edges > 0 and self.max_depth > 0;
    }
};

/// Schema-preparation failure for invalid definitions or exceeded bounds.
pub const Error = union(enum) {
    invalid_limits,
    duplicate_definition: usize,
    invalid_definition: struct { index: usize, issue: ast.DefinitionIssue },
    invalid_node: ast.DefinitionIssue,
    missing_reference: []const u8,
    node_limit,
    edge_limit,
    depth_limit,
};

/// Counts gathered while preparing a schema definition.
pub const Stats = struct { definitions: usize, nodes: usize, edges: usize };
const Verdict = outcomes.Exit(Stats, Error);
const Frame = struct { node: *const ast.Node, depth: usize };

/// `definitions` and all its Nodes remain immutable and alive through this
/// object's lifetime. The result is memoized, including validation failures.
/// The tiny atomic gate covers only cold preparation; warmed reads are atomic.
pub const Definitions = struct {
    definitions: []const ast.Definition,
    limits: Limits,
    gate: std.atomic.Value(bool) = .init(false),
    ready: std.atomic.Value(bool) = .init(false),
    cached: Verdict = undefined,

    /// Validate and index schema definitions under explicit resource limits.
    /// Parameters:
    /// - `definitions: []const ast.Definition`: Static definitions to register or validate.
    /// - `limits: Limits`: Caller-selected, validated behavior or resource bounds; no hidden default is
    ///   installed.
    /// Returns (`Definitions`): Prepared structural schema definitions under the selected limits.
    pub fn init(definitions: []const ast.Definition, limits: Limits) Definitions {
        return .{ .definitions = definitions, .limits = limits };
    }

    /// Prepare and validate structural schema definitions.
    /// Parameters:
    /// - `self: *Definitions`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
    /// - `env: anytype`: Borrowed caller environment supplying allocator, I/O, and selected capabilities.
    /// Returns (`Verdict`): Schema preparation success or typed validation issue.
    pub fn prepare(self: *Definitions, env: anytype) Verdict {
        if (self.ready.load(.acquire)) return self.cached;
        while (self.gate.swap(true, .acquire)) std.atomic.spinLoopHint();
        defer self.gate.store(false, .release);
        if (!self.ready.load(.monotonic)) {
            self.cached = self.prepareCold(env);
            // An allocator failure is environmental, not a schema verdict.
            // A later caller may prepare successfully with available memory.
            if (self.cached != .die) self.ready.store(true, .release);
        }
        return self.cached;
    }

    fn prepareCold(self: *const Definitions, env: anytype) Verdict {
        const R = @typeInfo(@TypeOf(env)).pointer.child;
        if (!@hasField(R, "allocator") or @FieldType(R, "allocator") != std.mem.Allocator)
            @compileError("dynamic Schema preparation requires R.allocator");
        if (!self.limits.valid() or self.definitions.len == 0) return .{ .fail = .invalid_limits };
        if (self.definitions.len > self.limits.max_nodes) return .{ .fail = .node_limit };

        var names = std.StringHashMap(*const ast.Node).init(env.allocator);
        defer names.deinit();
        for (self.definitions, 0..) |definition, index| {
            switch (ast.validateDefinition(definition)) {
                .err => |issue| return .{ .fail = .{ .invalid_definition = .{ .index = index, .issue = issue } } },
                .ok => {},
            }
            if (names.contains(definition.name)) return .{ .fail = .{ .duplicate_definition = index } };
            names.put(definition.name, definition.root) catch return .{ .die = outcomes.Defect.out_of_memory };
        }

        var seen = std.AutoHashMap(*const ast.Node, void).init(env.allocator);
        defer seen.deinit();
        var pending = std.array_list.Managed(Frame).init(env.allocator);
        defer pending.deinit();
        for (self.definitions) |definition| {
            pending.append(.{ .node = definition.root, .depth = 0 }) catch return .{ .die = outcomes.Defect.out_of_memory };
        }
        var edges: usize = 0;
        while (pending.pop()) |frame| {
            if (seen.contains(frame.node)) continue;
            if (frame.depth >= self.limits.max_depth) return .{ .fail = .depth_limit };
            if (seen.count() >= self.limits.max_nodes) return .{ .fail = .node_limit };
            seen.put(frame.node, {}) catch return .{ .die = outcomes.Defect.out_of_memory };
            switch (ast.validateLocal(frame.node)) {
                .err => |issue| return .{ .fail = .{ .invalid_node = issue } },
                .ok => {},
            }
            const child_depth = std.math.add(usize, frame.depth, 1) catch return .{ .fail = .depth_limit };
            switch (frame.node.*) {
                .primitive, .exact_number, .enumeration => {},
                .alternatives => |value| for (value.branches) |child| {
                    if (edges >= self.limits.max_edges) return .{ .fail = .edge_limit };
                    edges += 1;
                    pending.append(.{ .node = child, .depth = child_depth }) catch return .{ .die = outcomes.Defect.out_of_memory };
                },
                .prefix_array => |value| {
                    for (value.elements) |element| {
                        if (edges >= self.limits.max_edges) return .{ .fail = .edge_limit };
                        edges += 1;
                        pending.append(.{ .node = element.schema, .depth = child_depth }) catch return .{ .die = outcomes.Defect.out_of_memory };
                    }
                    if (value.rest) |child| {
                        if (edges >= self.limits.max_edges) return .{ .fail = .edge_limit };
                        edges += 1;
                        pending.append(.{ .node = child, .depth = child_depth }) catch return .{ .die = outcomes.Defect.out_of_memory };
                    }
                },
                .object => |value| {
                    if (edges >= self.limits.max_edges) return .{ .fail = .edge_limit };
                    edges += 1;
                    pending.append(.{ .node = value.base, .depth = child_depth }) catch return .{ .die = outcomes.Defect.out_of_memory };
                    for (.{ value.property_names, value.additional }) |optional_child| if (optional_child) |child| {
                        if (edges >= self.limits.max_edges) return .{ .fail = .edge_limit };
                        edges += 1;
                        pending.append(.{ .node = child, .depth = child_depth }) catch return .{ .die = outcomes.Defect.out_of_memory };
                    };
                    for (value.patterns) |pattern| {
                        if (edges >= self.limits.max_edges) return .{ .fail = .edge_limit };
                        edges += 1;
                        pending.append(.{ .node = pattern.schema, .depth = child_depth }) catch return .{ .die = outcomes.Defect.out_of_memory };
                    }
                },
                .nullable, .array => |child| {
                    if (edges >= self.limits.max_edges) return .{ .fail = .edge_limit };
                    edges += 1;
                    pending.append(.{ .node = child, .depth = child_depth }) catch return .{ .die = outcomes.Defect.out_of_memory };
                },
                .record => |record| {
                    for (record.fields) |field| {
                        if (edges >= self.limits.max_edges) return .{ .fail = .edge_limit };
                        edges += 1;
                        pending.append(.{ .node = field.schema, .depth = child_depth }) catch return .{ .die = outcomes.Defect.out_of_memory };
                    }
                },
                .tagged_union => |tagged| {
                    for (tagged.cases) |item| {
                        if (item.payload) |child| {
                            if (edges >= self.limits.max_edges) return .{ .fail = .edge_limit };
                            edges += 1;
                            pending.append(.{ .node = child, .depth = child_depth }) catch return .{ .die = outcomes.Defect.out_of_memory };
                        }
                    }
                },
                .reference => |name| {
                    const target = names.get(name) orelse return .{ .fail = .{ .missing_reference = name } };
                    if (edges >= self.limits.max_edges) return .{ .fail = .edge_limit };
                    edges += 1;
                    pending.append(.{ .node = target, .depth = child_depth }) catch return .{ .die = outcomes.Defect.out_of_memory };
                },
                .constrained => |constrained| {
                    if (edges >= self.limits.max_edges) return .{ .fail = .edge_limit };
                    edges += 1;
                    pending.append(.{ .node = constrained.base, .depth = child_depth }) catch return .{ .die = outcomes.Defect.out_of_memory };
                },
            }
        }
        return .{ .success = .{ .definitions = self.definitions.len, .nodes = seen.count(), .edges = edges } };
    }
};
