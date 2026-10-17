const std = @import("std");
const fx = @import("effect");
const Issue = fx.SchemaIssue.Issue;
const Counts = struct { live: usize = 0, text: usize = 0, limits: usize = 0, mapped: usize = 0 };
const Env = struct { allocator: std.mem.Allocator, counts: *Counts };
const Owner = struct {
    pub const effect_owned_payload = true;
    counts: *Counts,
    pub fn deinit(self: *@This()) void {
        self.counts.live -= 1;
    }
};
const Inputs = struct { owner: Owner, bytes: fx.OwnedSlice(u8), budget: fx.SchemaBuilder.ParseLimits, tag: u32 = 7 };
const Record = fx.dsl.schemaRecord(Env, .{ .fields = .{
    .name = .{ .codec = fx.JsonCodec.borrowed_string, .presence = .required },
    .extras = .{ .extensions = true },
}, .policy = .{ .unknown_fields = .preserve, .duplicate_fields = .reject, .issues = .first } });
const budget: fx.SchemaBuilder.ParseLimits = .{ .max_document_bytes = 256, .max_depth = 8, .max_value_bytes = 128, .max_integer_digits = 8 };
const Roles = struct {
    fn text(input: anytype, env: *const Env) []u8 {
        std.debug.assert(input.owner.counts.live == 1);
        env.counts.text += 1;
        return input.bytes.items;
    }
    fn limits(input: anytype, env: *const Env) fx.SchemaBuilder.ParseLimits {
        env.counts.limits += 1;
        return input.budget;
    }
    const Failure = struct { issue: Issue, tag: u32 };
    fn map(input: anytype, issue: Issue, env: *const Env) Failure {
        std.debug.assert(input.owner.counts.live == 1);
        env.counts.mapped += 1;
        return .{ .issue = issue, .tag = input.tag };
    }
};
fn inputs(allocator: std.mem.Allocator, counts: *Counts, bytes: []const u8, limits: fx.SchemaBuilder.ParseLimits) !Inputs {
    const copied = try allocator.dupe(u8, bytes);
    counts.live += 1;
    return .{ .owner = .{ .counts = counts }, .bytes = .{ .items = copied, .allocator = allocator }, .budget = limits };
}

test "CODEC-04 CODEC-07 CAP-08 LIFE-11 named captured schema parsing is lazy exact Effect and parsed views outlive input capture teardown" {
    var counts: Counts = .{};
    const env: Env = .{ .allocator = std.testing.allocator, .counts = &counts };
    var described = fx.dsl.parseSchema(&env, try inputs(env.allocator, &counts, "{\"name\":\"retained\",\"extra\":\"also retained\"}", budget), .{ .schema = Record, .text = Roles.text, .limits = Roles.limits });
    comptime if (@TypeOf(described) != fx.Effect(fx.Parsed(Record.Decoded), Issue, Env)) @compileError("parsing must return exact Effect with inferred parsed value and schema environment");
    try std.testing.expectEqual(@as(usize, 0), counts.text);
    try std.testing.expectEqual(@as(usize, 0), counts.limits);
    var result = fx.run(&env, described);
    described.deinit();
    try std.testing.expect(result == .success);
    try std.testing.expectEqual(@as(usize, 0), counts.live);
    try std.testing.expectEqual(@as(usize, 1), counts.text);
    try std.testing.expectEqual(@as(usize, 1), counts.limits);
    var retained = try result.success.clone();
    result.deinit();
    defer retained.deinit();
    try std.testing.expectEqualStrings("retained", Record.get("name", &retained.value).*);
    try std.testing.expectEqualStrings("also retained", Record.get("extras", &retained.value).values.get("extra").?.string);
}

test "CODEC-07 INF-07 captured issue mapping derives composite error ownership and preserves field direction path after capture destruction" {
    var counts: Counts = .{};
    const env: Env = .{ .allocator = std.testing.allocator, .counts = &counts };
    var described = fx.dsl.parseSchema(&env, try inputs(env.allocator, &counts, "{\"name\":false}", budget), .{ .schema = Record, .text = Roles.text, .limits = Roles.limits, .map_error = Roles.map });
    const Failure = fx.dsl.ownership.Capture(Roles.Failure);
    comptime if (@TypeOf(described).Error != Failure) @compileError("native composite mapped errors must receive generated ownership before Effect composition");
    var result = fx.run(&env, described);
    described.deinit();
    defer result.deinit();
    try std.testing.expect(result == .fail);
    try std.testing.expectEqual(@as(usize, 0), counts.live);
    try std.testing.expectEqual(@as(usize, 1), counts.mapped);
    try std.testing.expectEqual(@as(u32, 7), result.fail.value.tag);
    try std.testing.expect(result.fail.value.issue.direction == .decode);
    try std.testing.expectEqualStrings("name", result.fail.value.issue.path.?[0].field.items);
}

test "CODEC-07 named parser applies selected byte depth digit and duplicate policies before returning mapped issues" {
    inline for (.{ .bytes, .depth, .digits, .duplicate }) |mode| {
        var counts: Counts = .{};
        const env: Env = .{ .allocator = std.testing.allocator, .counts = &counts };
        var selected = budget;
        const source = switch (mode) {
            .bytes => blk: {
                selected.max_document_bytes = 4;
                break :blk "{\"name\":\"ok\"}";
            },
            .depth => blk: {
                selected.max_depth = 1;
                break :blk "{\"name\":\"ok\"}";
            },
            .digits => blk: {
                selected.max_integer_digits = 2;
                break :blk "{\"name\":\"ok\",\"extra\":123}";
            },
            .duplicate => "{\"name\":\"first\",\"name\":\"last\"}",
            else => unreachable,
        };
        var described = fx.dsl.parseSchema(&env, try inputs(env.allocator, &counts, source, selected), .{ .schema = Record, .text = Roles.text, .limits = Roles.limits, .map_error = Roles.map });
        var result = fx.run(&env, described);
        described.deinit();
        defer result.deinit();
        try std.testing.expect(result == .fail);
        try std.testing.expectEqual(@as(usize, 1), counts.text);
        try std.testing.expectEqual(@as(usize, 1), counts.limits);
        try std.testing.expectEqual(@as(usize, 1), counts.mapped);
        try std.testing.expectEqual(@as(usize, 0), counts.live);
        const issue = &result.fail.value.issue;
        switch (mode) {
            .bytes => try std.testing.expect(issue.kind == .size_limit and issue.detail.?.resource.resource == .input_bytes),
            .depth => try std.testing.expect(issue.kind == .depth_limit and issue.detail.?.resource.resource == .json_depth),
            .digits => try std.testing.expect(issue.kind == .integer_digits and issue.detail.?.integer_digits.limit == 2),
            .duplicate => try std.testing.expect(issue.kind == .duplicate_field),
            else => unreachable,
        }
    }
}

test "CODEC-04 CAP-03 static named parser retains ordinary Effect composition without capture allocation" {
    const Static = struct {
        fn text() []const u8 {
            return "{\"name\":\"static\"}";
        }
        fn length(value: fx.Parsed(Record.Decoded)) usize {
            var owner = value;
            defer owner.deinit();
            return Record.get("name", &owner.value).*.len;
        }
    };
    var counts: Counts = .{ .live = 0 };
    const env: Env = .{ .allocator = std.testing.allocator, .counts = &counts };
    var described = fx.dsl.parseSchema(&env, @as(void, {}), .{ .schema = Record, .text = Static.text, .limits = .{ .max_document_bytes = 256, .max_depth = 8, .max_value_bytes = 128, .max_integer_digits = 8 } });
    try std.testing.expect(described.ownership == null);
    var composed = described.mapOwned(&env, Static.length);
    defer composed.deinit();
    var result = fx.run(&env, composed);
    defer result.deinit();
    try std.testing.expect(result == .success and result.success == 6);
}

test "CODEC-04 LIFE-07 captured parser construction parsing and mapped failure release all initialized owners under every allocation failure" {
    const Check = struct {
        fn run(allocator: std.mem.Allocator) !void {
            inline for (.{ "{\"name\":\"ok\",\"extra\":\"value\"}", "{\"name\":false}" }) |source| {
                var counts: Counts = .{};
                defer std.debug.assert(counts.live == 0);
                const env: Env = .{ .allocator = allocator, .counts = &counts };
                const captured = try inputs(allocator, &counts, source, budget);
                var described = fx.dsl.parseSchema(&env, captured, .{ .schema = Record, .text = Roles.text, .limits = Roles.limits, .map_error = Roles.map });
                var result = fx.run(&env, described);
                described.deinit();
                defer result.deinit();
                if (result == .die and result.die.kind == .out_of_memory) return error.OutOfMemory;
                if (comptime std.mem.eql(u8, source, "{\"name\":false}")) try std.testing.expect(result == .fail) else try std.testing.expect(result == .success);
            }
        }
    };
    try std.testing.checkAllAllocationFailures(std.testing.allocator, Check.run, .{});
}

test "CODEC-04 CODEC-07 LIFE-01 captured parsing maps owned primary and finalization issues while preserving all other outcomes" {
    const Mode = enum { success, fail, die, interrupted, finalization, late_cancel };
    const Runtime = struct { allocator: std.mem.Allocator, counts: *Counts, mode: Mode, cancellation: *fx.Cancellation };
    const Local = struct {
        fn issue(env: *const Runtime, name: []const u8) ?Issue {
            var made = Issue.init(env, .decode, .invalid_value);
            made.prependField(name) catch {
                made.deinit();
                return null;
            };
            return made;
        }
        fn default(env: *const Runtime) fx.Exit(i64, Issue) {
            return switch (env.mode) {
                .success => .{ .success = 12 },
                .fail => .{ .fail = issue(env, "source") orelse return .{ .die = fx.Defect.out_of_memory } },
                .die => .{ .die = fx.Defect.init("parse-default") },
                .interrupted => .interrupted,
                .finalization => blk: {
                    var primary = issue(env, "factory") orelse return .{ .die = fx.Defect.out_of_memory };
                    var cleanup = issue(env, "release") orelse {
                        primary.deinit();
                        return .{ .die = fx.Defect.out_of_memory };
                    };
                    cleanup.direction = .encode;
                    break :blk .{ .finalization_failed = .{ .primary = .{ .failed = primary }, .first = .{ .release_failure = cleanup } } };
                },
                .late_cancel => blk: {
                    env.cancellation.request();
                    break :blk .{ .success = 12 };
                },
            };
        }
        fn text(input: *const Inputs, env: *const Runtime) []const u8 {
            env.counts.text += 1;
            return input.bytes.items;
        }
        fn map(input: *const Inputs, original: Issue, env: *const Runtime) Roles.Failure {
            std.debug.assert(input.owner.counts.live == 1);
            env.counts.mapped += 1;
            return .{ .issue = original, .tag = input.tag };
        }
    };
    const Defaulted = fx.dsl.schemaRecord(Runtime, .{ .fields = .{
        .number = .{ .codec = fx.JsonCodec.signed_integer, .presence = .defaulted, .default = Local.default },
    }, .policy = .{ .unknown_fields = .reject, .duplicate_fields = .reject, .issues = .first } });
    for ([_]Mode{ .success, .fail, .die, .interrupted, .finalization, .late_cancel }) |mode| {
        var counts: Counts = .{};
        var cancellation: fx.Cancellation = .{};
        var allocator = std.testing.FailingAllocator.init(std.testing.allocator, .{});
        const env: Runtime = .{ .allocator = allocator.allocator(), .counts = &counts, .mode = mode, .cancellation = &cancellation };
        var described = fx.dsl.parseSchema(&env, try inputs(env.allocator, &counts, "{}", budget), .{ .schema = Defaulted, .text = Local.text, .limits = budget, .map_error = Local.map });
        var result = fx.run(&env, described);
        described.deinit();
        try std.testing.expectEqual(@as(usize, 0), counts.live);
        try std.testing.expectEqual(@as(usize, 1), counts.text);
        switch (mode) {
            .success => try std.testing.expect(result == .success and result.success.value.number == 12),
            .fail => {
                try std.testing.expect(result == .fail);
                try std.testing.expectEqualStrings("number", result.fail.value.issue.path.?[0].field.items);
                try std.testing.expectEqualStrings("source", result.fail.value.issue.path.?[1].field.items);
                try std.testing.expectEqual(@as(usize, 1), counts.mapped);
            },
            .die => try std.testing.expect(result == .die and std.mem.eql(u8, result.die.message, "parse-default")),
            .interrupted, .late_cancel => try std.testing.expect(result == .interrupted),
            .finalization => {
                try std.testing.expect(result == .finalization_failed);
                try std.testing.expectEqual(@as(usize, 2), counts.mapped);
                try std.testing.expectEqualStrings("factory", result.finalization_failed.primary.failed.value.issue.path.?[0].field.items);
                try std.testing.expectEqualStrings("release", result.finalization_failed.first.release_failure.value.issue.path.?[0].field.items);
                try std.testing.expect(result.finalization_failed.primary.failed.value.issue.direction == .decode);
                try std.testing.expect(result.finalization_failed.first.release_failure.value.issue.direction == .encode);
            },
        }
        result.deinit();
        try std.testing.expectEqual(allocator.allocated_bytes, allocator.freed_bytes);
    }
}

test "CAP-03 LIFE-01 unexecuted captured parser releases inputs without selecting text limits or mapping errors" {
    var counts: Counts = .{};
    const env: Env = .{ .allocator = std.testing.allocator, .counts = &counts };
    var described = fx.dsl.parseSchema(&env, try inputs(env.allocator, &counts, "{\"name\":\"unused\"}", budget), .{ .schema = Record, .text = Roles.text, .limits = Roles.limits, .map_error = Roles.map });
    described.deinit();
    try std.testing.expectEqual(@as(usize, 0), counts.live);
    try std.testing.expectEqual(@as(usize, 0), counts.text);
    try std.testing.expectEqual(@as(usize, 0), counts.limits);
    try std.testing.expectEqual(@as(usize, 0), counts.mapped);
}

test "CODEC-07 NEG-23 runtime selected zero parse bounds retain mapped invalid policy errors and release captures" {
    inline for (.{ .bytes, .depth, .value, .digits }) |mode| {
        var counts: Counts = .{};
        const env: Env = .{ .allocator = std.testing.allocator, .counts = &counts };
        var invalid = budget;
        switch (mode) {
            .bytes => invalid.max_document_bytes = 0,
            .depth => invalid.max_depth = 0,
            .value => invalid.max_value_bytes = 0,
            .digits => invalid.max_integer_digits = 0,
            else => unreachable,
        }
        var described = fx.dsl.parseSchema(&env, try inputs(env.allocator, &counts, "{\"name\":\"ignored\"}", invalid), .{ .schema = Record, .text = Roles.text, .limits = Roles.limits, .map_error = Roles.map });
        try std.testing.expectEqual(@as(usize, 0), counts.text);
        try std.testing.expectEqual(@as(usize, 0), counts.limits);
        var result = fx.run(&env, described);
        described.deinit();
        defer result.deinit();
        try std.testing.expect(result == .fail and result.fail.value.issue.kind == .invalid_value);
        try std.testing.expect(result.fail.value.issue.direction == .decode);
        try std.testing.expectEqual(@as(usize, 1), counts.text);
        try std.testing.expectEqual(@as(usize, 1), counts.limits);
        try std.testing.expectEqual(@as(usize, 1), counts.mapped);
        try std.testing.expectEqual(@as(usize, 0), counts.live);
    }
}

test "CAP-04 CODEC-04 reusable captured parsing selects fresh execution and leaves both parsed documents valid after description release" {
    var counts: Counts = .{};
    const env: Env = .{ .allocator = std.testing.allocator, .counts = &counts };
    var described = fx.dsl.parseSchema(&env, try inputs(env.allocator, &counts, "{\"name\":\"twice\"}", budget), .{ .schema = Record, .text = Roles.text, .limits = Roles.limits });
    var first = fx.run(&env, described);
    var second = fx.run(&env, described);
    try std.testing.expect(first == .success and second == .success);
    try std.testing.expectEqual(@as(usize, 1), counts.live);
    try std.testing.expectEqual(@as(usize, 2), counts.text);
    try std.testing.expectEqual(@as(usize, 2), counts.limits);
    described.deinit();
    try std.testing.expectEqual(@as(usize, 0), counts.live);
    first.deinit();
    defer second.deinit();
    try std.testing.expectEqualStrings("twice", Record.get("name", &second.success.value).*);
}

test "CODEC-04 LIFE-07 captured parser keeps construction and execution allocator ownership separate" {
    var origin = std.testing.FailingAllocator.init(std.testing.allocator, .{});
    var execution = std.testing.FailingAllocator.init(std.testing.allocator, .{});
    var captured: Counts = .{};
    var running: Counts = .{};
    const construction_env: Env = .{ .allocator = origin.allocator(), .counts = &captured };
    const execution_env: Env = .{ .allocator = execution.allocator(), .counts = &running };
    var described = fx.dsl.parseSchema(&construction_env, try inputs(construction_env.allocator, &captured, "{\"name\":\"separate\",\"extra\":\"retained\"}", budget), .{ .schema = Record, .text = Roles.text, .limits = Roles.limits });
    var result = fx.run(&execution_env, described);
    described.deinit();
    try std.testing.expect(result == .success);
    try std.testing.expectEqual(@as(usize, 0), captured.live);
    try std.testing.expectEqual(@as(usize, 0), captured.text);
    try std.testing.expectEqual(@as(usize, 1), running.text);
    try std.testing.expectEqual(origin.allocated_bytes, origin.freed_bytes);
    try std.testing.expect(execution.allocated_bytes > execution.freed_bytes);
    try std.testing.expectEqualStrings("separate", Record.get("name", &result.success.value).*);
    result.deinit();
    try std.testing.expectEqual(execution.allocated_bytes, execution.freed_bytes);
}

test "CODEC-07 LIFE-01 unmapped parser transfers existing finalization metadata without allocating a replacement chain" {
    const Finalization = @FieldType(fx.Exit(i64, Issue), "finalization_failed");
    const Node = @typeInfo(@typeInfo(@FieldType(Finalization, "rest")).optional.child).pointer.child;
    const Runtime = struct { allocator: std.mem.Allocator, failing: *std.testing.FailingAllocator, original: *?*Node };
    const Local = struct {
        fn text() []const u8 {
            return "{}";
        }
        fn default(env: *const Runtime) fx.Exit(i64, Issue) {
            const node = env.allocator.create(Node) catch return .{ .die = fx.Defect.out_of_memory };
            node.* = .{ .failure = .{ .release_failure = Issue.init(env, .decode, .{ .custom = "second" }) }, .allocator = env.allocator };
            env.original.* = node;
            env.failing.fail_index = env.failing.alloc_index;
            return .{ .finalization_failed = .{ .primary = .succeeded, .first = .{ .release_failure = Issue.init(env, .decode, .{ .custom = "first" }) }, .rest = node, .tail = node } };
        }
    };
    const Defaulted = fx.dsl.schemaRecord(Runtime, .{ .fields = .{
        .number = .{ .codec = fx.JsonCodec.signed_integer, .presence = .defaulted, .default = Local.default },
    }, .policy = .{ .unknown_fields = .reject, .duplicate_fields = .reject, .issues = .first } });
    var allocator = std.testing.FailingAllocator.init(std.testing.allocator, .{});
    var original: ?*Node = null;
    const env: Runtime = .{ .allocator = allocator.allocator(), .failing = &allocator, .original = &original };
    const described = fx.dsl.parseSchema(&env, @as(void, {}), .{ .schema = Defaulted, .text = Local.text, .limits = budget });
    var result = fx.run(&env, described);
    try std.testing.expect(result == .finalization_failed);
    try std.testing.expect(result.finalization_failed.rest == original);
    var iterator = result.finalization_failed.iterator();
    try std.testing.expectEqualStrings("first", iterator.nextFailure().?.release_failure.kind.custom);
    try std.testing.expectEqualStrings("second", iterator.nextFailure().?.release_failure.kind.custom);
    try std.testing.expect(iterator.nextFailure() == null);
    result.deinit();
    try std.testing.expectEqual(allocator.allocated_bytes, allocator.freed_bytes);
}

test "DECL-03 CAP-02 INF-07 one parse declaration separates exact static metadata from generated owned runtime inputs" {
    var counts: Counts = .{};
    const env: Env = .{ .allocator = std.testing.allocator, .counts = &counts };
    var described = fx.dsl.parseDeclaration(&env, .{
        .definition = fx.dsl.declareMetadata(.{ .schema = Record, .text = Roles.text, .limits = Roles.limits, .map_error = Roles.map }),
        .inputs = try inputs(env.allocator, &counts, "{\"name\":\"one declaration\"}", budget),
    });
    comptime if (@TypeOf(described) != fx.Effect(fx.Parsed(Record.Decoded), fx.dsl.ownership.Capture(Roles.Failure), Env)) @compileError("declaration lowering must retain inferred exact Effect channels");
    try std.testing.expectEqual(@as(usize, 0), counts.text);
    var result = fx.run(&env, described);
    described.deinit();
    defer result.deinit();
    try std.testing.expect(result == .success);
    try std.testing.expectEqual(@as(usize, 0), counts.live);
    try std.testing.expectEqual(@as(usize, 1), counts.text);
    try std.testing.expectEqual(@as(usize, 1), counts.limits);
    try std.testing.expectEqualStrings("one declaration", Record.get("name", &result.success.value).*);
}

test "DECL-03 ROLE-01 static declaration namespaces share the existing parser authority and Effect composition" {
    const Static = struct {
        fn text() []const u8 {
            return "{\"name\":\"namespace\"}";
        }
    };
    const Namespace = struct {
        pub const definition = fx.dsl.declareMetadata(.{ .schema = Record, .text = Static.text, .limits = budget });
        pub const inputs = @as(void, {});
    };
    var counts: Counts = .{};
    const env: Env = .{ .allocator = std.testing.allocator, .counts = &counts };
    var described = fx.dsl.parseDeclaration(&env, Namespace);
    try std.testing.expect(described.ownership == null);
    var result = fx.run(&env, described);
    described.deinit();
    defer result.deinit();
    try std.testing.expect(result == .success);
    try std.testing.expectEqualStrings("namespace", Record.get("name", &result.success.value).*);
}
