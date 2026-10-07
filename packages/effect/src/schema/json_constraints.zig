//! Static constraint plans, invocation-local candidate state and bounded
//! provider-supplied pattern execution. No regex runtime is selected by core.
const std = @import("std");
const outcomes = @import("../outcome.zig");
const effects = @import("../effect.zig");
const Issue = @import("schema_issue.zig").Issue;
const ast = @import("schema_ast.zig");
const values = @import("json_value.zig");
const state = @import("internal/structural.zig");
/// Provider-reported pattern failure; timeout is distinguished from unsupported dialect or invalid syntax.
pub const PatternError = enum {
    /// Selected provider does not implement the declared dialect.
    unsupported_dialect,
    /// Provider rejected the pattern definition before matching.
    invalid_syntax,
    /// Matching exceeded a supplied time/work budget.
    timeout,
    /// Provider could not complete matching for another typed reason.
    execution_failed,
};
/// Borrowed pattern definition plus explicit provider execution budgets. Keep dialect/source alive through matching.
pub const Pattern = struct {
    /// Explicit provider dialect; not guessed from source syntax.
    dialect: []const u8,
    /// Borrowed pattern source, preserved in diagnostics/export metadata.
    source: []const u8,
    /// Positive maximum provider execution duration, in std.Io.Duration nanoseconds.
    timeout: std.Io.Duration,
    /// Positive inclusive work/step budget enforced by the selected provider.
    maximum_steps: usize,
};
/// Caller-selected matcher. R carries allocator and provider services; no regex engine is selected globally.
/// The provider must honor timeout/maximum_steps and return typed failures or other Exit channels.
pub fn PatternCapability(comptime R: type) type {
    return struct {
        /// Borrow text/definition for one call; enforce the supplied budgets using R capabilities.
        match: *const fn (*const R, Pattern, []const u8) outcomes.Exit(bool, PatternError),
        /// Invoke the matcher once and copy pattern diagnostics into an owned Issue on typed failure.
        /// Preserve defects, interruption and finalization failures; successful false is an ordinary non-match.
        pub fn matches(self: @This(), env: *const R, pattern: Pattern, text: []const u8, direction: @import("schema_issue.zig").Direction) outcomes.Exit(bool, Issue) {
            if (pattern.dialect.len == 0 or pattern.maximum_steps == 0 or pattern.timeout.nanoseconds <= 0) return .{ .fail = Issue.init(env, direction, .invalid_value) };
            const result = self.match(env, pattern, text);
            return switch (result) {
                .success => |matched| .{ .success = matched },
                .fail => |problem| blk: {
                    const dialect = env.allocator.dupe(u8, pattern.dialect) catch break :blk .{ .die = outcomes.Defect.out_of_memory };
                    const source = env.allocator.dupe(u8, pattern.source) catch {
                        env.allocator.free(dialect);
                        break :blk .{ .die = outcomes.Defect.out_of_memory };
                    };
                    var issue = Issue.init(env, direction, if (problem == .timeout) .pattern_timeout else .pattern_failure);
                    issue.detail = .{ .pattern = .{ .dialect = .{ .items = dialect, .allocator = env.allocator }, .source = .{ .items = source, .allocator = env.allocator }, .timeout_nanoseconds = pattern.timeout.nanoseconds } };
                    break :blk .{ .fail = issue };
                },
                else => switch (direction) {
                    inline else => |selected| outcomes.mapError(Issue, result, env.allocator, struct {
                        fn map(problem: PatternError) Issue {
                            return Issue.pure(selected, if (problem == .timeout) .pattern_timeout else .pattern_failure);
                        }
                    }.map),
                },
            };
        }
    };
}
fn constraintIssue(env: anytype, direction: @import("schema_issue.zig").Direction, name: []const u8, expected: []const u8, actual: []const u8) outcomes.Exit(void, Issue) {
    const owned_name = env.allocator.dupe(u8, name) catch return .{ .die = outcomes.Defect.out_of_memory };
    const owned_expected = env.allocator.dupe(u8, expected) catch {
        env.allocator.free(owned_name);
        return .{ .die = outcomes.Defect.out_of_memory };
    };
    const owned_actual = env.allocator.dupe(u8, actual) catch {
        env.allocator.free(owned_expected);
        env.allocator.free(owned_name);
        return .{ .die = outcomes.Defect.out_of_memory };
    };
    var issue = Issue.init(env, direction, .constraint);
    issue.detail = .{ .constraint = .{ .name = .{ .items = owned_name, .allocator = env.allocator }, .expected = .{ .items = owned_expected, .allocator = env.allocator }, .actual = .{ .items = owned_actual, .allocator = env.allocator } } };
    return .{ .fail = issue };
}
fn atIndex(comptime A: type, env: anytype, result: outcomes.Exit(A, Issue), index: usize) outcomes.Exit(A, Issue) {
    if (result != .fail) return result;
    var issue = result.fail;
    issue.bindAllocator(env);
    issue.prependIndex(index) catch {
        issue.deinit();
        return .{ .die = outcomes.Defect.out_of_memory };
    };
    return .{ .fail = issue };
}
fn atField(comptime A: type, env: anytype, result: outcomes.Exit(A, Issue), name: []const u8) outcomes.Exit(A, Issue) {
    if (result != .fail) return result;
    var issue = result.fail;
    issue.bindAllocator(env);
    issue.prependField(name) catch {
        issue.deinit();
        return .{ .die = outcomes.Defect.out_of_memory };
    };
    return .{ .fail = issue };
}
/// Dispatch a statically typed child using its direct or structural decode contract and this invocation’s state.
pub fn decodeChild(comptime child: anytype, env: anytype, input: *const std.json.Value, traversal: *state.DecodeState) outcomes.Exit((if (@TypeOf(child) == type) child else @TypeOf(child)).Decoded, Issue) {
    const C = if (@TypeOf(child) == type) child else @TypeOf(child);
    if (comptime @TypeOf(child) == type) return C.decodeWithin(env, input, traversal);
    return switch (child.decodeDirect(input)) {
        .ok => |value| .{ .success = value },
        .err => |issue| .{ .fail = issue },
    };
}
fn checkChild(comptime child: anytype, env: anytype, input: *const std.json.Value, traversal: *state.DecodeState) outcomes.Exit(void, Issue) {
    var result = decodeChild(child, env, input, traversal);
    return switch (result) {
        .success => blk: {
            outcomes.discardPayload(&result.success);
            break :blk .{ .success = {} };
        },
        else => outcomes.castFailure(void, result),
    };
}
/// Inclusive Unicode-scalar length bounds and optional format/pattern checks in both directions.
pub const StringPolicy = struct {
    /// Inclusive minimum Unicode code point count, not UTF-8 bytes or graphemes.
    minimum_scalars: usize,
    /// Inclusive maximum Unicode code point count.
    maximum_scalars: usize,
    /// none skips format checks; uuid validates shape, uuid_v4 also validates version/variant.
    format: enum { none, uuid, uuid_v4 },
    /// Optional borrowed provider-specific pattern and its explicit execution limits.
    pattern: ?Pattern = null,
};
/// Build a borrowing UTF-8 string codec requiring R.allocator for issue paths/details.
/// UUID v4 checks version and variant bits. Pattern validation uses the explicitly selected R capability.
pub fn String(comptime R: type, comptime policy: StringPolicy, comptime select: ?fn (*const R) PatternCapability(R)) type {
    comptime {
        if (policy.minimum_scalars > policy.maximum_scalars) @compileError("invalid static Unicode string bounds");
        if (policy.pattern != null and select == null) @compileError("pattern constraints require a caller pattern capability");
        if (policy.pattern) |pattern| if (pattern.dialect.len == 0 or pattern.timeout.nanoseconds <= 0 or pattern.maximum_steps == 0) @compileError("pattern execution requires an explicit dialect, timeout and step bound");
    }
    return struct {
        /// Application value produced on success; ownership follows the codec’s declared value type.
        pub const Decoded = []const u8;
        /// Caller environment supplying allocation and any selected schema capabilities.
        pub const Environment = R;
        /// Public evidence for checked JSON writing without constructing an intermediate DOM.
        pub const supports_direct_writer = true;
        fn check(env: *const R, value: *const []const u8, direction: @import("schema_issue.zig").Direction) outcomes.Exit(void, Issue) {
            const count = std.unicode.utf8CountCodepoints(value.*) catch return .{ .fail = Issue.init(env, direction, .invalid_utf8) };
            if (count < policy.minimum_scalars or count > policy.maximum_scalars) {
                var bounds: [2 * std.fmt.count("{d}", .{std.math.maxInt(usize)}) + 2]u8 = undefined;
                const expected = std.fmt.bufPrint(&bounds, "{d}..{d}", .{ policy.minimum_scalars, policy.maximum_scalars }) catch unreachable;
                return constraintIssue(env, direction, "unicodeScalars", expected, value.*);
            }
            if (policy.format != .none) {
                if (value.len != 36) return constraintIssue(env, direction, "format", @tagName(policy.format), value.*);
                for (value.*, 0..) |character, index| {
                    const separator = index == 8 or index == 13 or index == 18 or index == 23;
                    if (if (separator) character != '-' else !std.ascii.isHex(character)) return constraintIssue(env, direction, "format", @tagName(policy.format), value.*);
                }
            }
            if (policy.format == .uuid_v4 and (value.*[14] != '4' or std.mem.indexOfScalar(u8, "89aAbB", value.*[19]) == null)) return constraintIssue(env, direction, "format", "uuid_v4", value.*);
            if (policy.pattern) |pattern| {
                const matched = select.?(env).matches(env, pattern, value.*, direction);
                if (matched != .success) return outcomes.castFailure(void, matched);
                if (!matched.success) return constraintIssue(env, direction, "pattern", pattern.source, value.*);
            }
            return .{ .success = {} };
        }
        /// Check the application value in encode direction without changing its representation.
        pub fn validateDirect(env: *const R, value: *const []const u8) outcomes.Exit(void, Issue) {
            return check(env, value, .encode);
        }
        /// Decode using existing invocation-local traversal. String results borrow input; composite results retain owned documents/children.
        pub fn decodeWithin(env: *const R, input: *const std.json.Value, _: *state.DecodeState) outcomes.Exit(Decoded, Issue) {
            if (input.* != .string) return .{ .fail = Issue.init(env, .decode, .{ .expected = .string }) };
            const checked = check(env, &input.string, .decode);
            if (checked != .success) return outcomes.castFailure(Decoded, checked);
            return .{ .success = input.string };
        }
        /// Decode immediately with fresh local traversal state; release any owning success or Issue.
        pub fn decodeDirect(env: *const R, input: *const std.json.Value) outcomes.Exit(Decoded, Issue) {
            var traversal: state.DecodeState = .{};
            return decodeWithin(env, input, &traversal);
        }
        /// Encode using the caller’s scratch scope; preserve input owners while borrowed string/number views are in use.
        pub fn encodeWithin(env: *const R, input: *const Decoded, _: *state.EncodeState) outcomes.Exit(std.json.Value, Issue) {
            const checked = check(env, input, .encode);
            if (checked != .success) return outcomes.castFailure(std.json.Value, checked);
            return .{ .success = .{ .string = input.* } };
        }
        /// Validate the string once in encode direction and emit it without constructing a DOM; retain bound details on failure.
        pub fn writeJson(env: *const R, input: *const Decoded, writer: *@import("json_writer.zig").Writer) outcomes.Exit(void, Issue) {
            const checked = check(env, input, .encode);
            if (checked != .success) return checked;
            writer.string(input.*) catch |problem| return outcomes.castFailure(void, @import("json_writer.zig").failureWith(env, .encode, problem, writer));
            return .{ .success = {} };
        }
        /// Return immutable encoded-shape metadata; it does not execute callbacks or construct application values.
        pub fn encodedNode() *const ast.Node {
            const Metadata = struct {
                const primitive: ast.Node = .{ .primitive = .string };
                const length = ast.Constraint{ .length = .{ .minimum = policy.minimum_scalars, .maximum = policy.maximum_scalars, .unit = .unicode_scalars } };
                const rules = blk: {
                    const count = 1 + @as(usize, if (policy.pattern != null) 1 else 0) + @as(usize, if (policy.format != .none) 1 else 0);
                    var result: [count]ast.Constraint = undefined;
                    result[0] = length;
                    var index: usize = 1;
                    if (policy.pattern) |pattern| {
                        result[index] = .{ .pattern = .{ .dialect = pattern.dialect, .text = pattern.source } };
                        index += 1;
                    }
                    if (policy.format != .none) result[index] = .{ .format = if (policy.format == .uuid_v4) .uuid_v4 else .uuid };
                    break :blk result;
                };
                const node: ast.Node = .{ .constrained = .{ .base = &primitive, .rules = &rules } };
            };
            return &Metadata.node;
        }
    };
}
/// Copy a UTF-8 enum string into caller-owned storage. Unknown values either fail or retain their original spelling.
pub fn Enumeration(comptime R: type, comptime known: []const []const u8, comptime unknown: enum { reject, preserve }) type {
    comptime {
        if (known.len == 0) @compileError("enum schema requires known values");
        for (known, 0..) |name, index| {
            if (!std.unicode.utf8ValidateSlice(name)) @compileError("enum values must be UTF-8");
            for (known[0..index]) |prior| if (std.mem.eql(u8, name, prior)) @compileError("duplicate enum value");
        }
    }
    return struct {
        /// Application value produced on success; ownership follows the codec’s declared value type.
        pub const Decoded = struct {
            /// Exit/Result cleanup owns this value; release it once or clone it for an independent observation.
            pub const effect_owned_payload = true;
            /// Owned enum spelling, including preserved unknown values.
            text: outcomes.OwnedSlice(u8),
            /// Known declaration index, or null for a preserved unknown spelling.
            known_index: ?usize,
            /// Release this decoded value’s owned storage and child payloads once; borrowed views become invalid.
            pub fn deinit(self: *@This()) void {
                self.text.deinit();
            }
            /// Create an independent owner, cloning child payloads and rebasing views into the copied document.
            pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
                return .{ .text = try self.text.clone(), .known_index = self.known_index };
            }
        };
        /// Caller environment supplying allocation and any selected schema capabilities.
        pub const Environment = R;
        /// Return immutable encoded-shape metadata; it does not execute callbacks or construct application values.
        pub fn encodedNode() *const ast.Node {
            const Metadata = struct {
                const node: ast.Node = .{ .enumeration = .{ .values = known, .preserve_unknown = unknown == .preserve } };
            };
            return &Metadata.node;
        }
        /// Decode using existing invocation-local traversal. String results borrow input; composite results retain owned documents/children.
        pub fn decodeWithin(env: *const R, input: *const std.json.Value, _: *state.DecodeState) outcomes.Exit(Decoded, Issue) {
            if (input.* != .string) return .{ .fail = Issue.init(env, .decode, .{ .expected = .string }) };
            if (!std.unicode.utf8ValidateSlice(input.string)) return .{ .fail = Issue.init(env, .decode, .invalid_utf8) };
            var matched: ?usize = null;
            inline for (known, 0..) |name, index| if (std.mem.eql(u8, name, input.string)) {
                matched = index;
            };
            if (matched == null and unknown == .reject) return .{ .fail = Issue.init(env, .decode, .constraint) };
            return .{ .success = .{ .text = .{ .items = env.allocator.dupe(u8, input.string) catch return .{ .die = outcomes.Defect.out_of_memory }, .allocator = env.allocator }, .known_index = matched } };
        }
        /// Encode using the caller’s scratch scope; preserve input owners while borrowed string/number views are in use.
        pub fn encodeWithin(env: *const R, input: *const Decoded, _: *state.EncodeState) outcomes.Exit(std.json.Value, Issue) {
            if (!std.unicode.utf8ValidateSlice(input.text.items)) return .{ .fail = Issue.init(env, .encode, .invalid_utf8) };
            var found: ?usize = null;
            inline for (known, 0..) |name, index| if (std.mem.eql(u8, name, input.text.items)) {
                found = index;
            };
            if (found != input.known_index or (unknown == .reject and found == null)) return .{ .fail = Issue.init(env, .encode, .constraint) };
            return .{ .success = .{ .string = input.text.items } };
        }
    };
}
/// Inclusive element-count bounds and optional mathematical JSON uniqueness.
pub const ArrayPolicy = struct {
    /// Inclusive minimum element count.
    minimum: usize,
    /// Inclusive maximum element count, including prefix and rest.
    maximum: usize,
    /// Reject mathematically equal JSON values, including differently spelled equal numbers.
    unique: bool,
};
/// Build a homogeneous array using the Prefix ownership/validation path and an explicit maximum count.
pub fn Array(comptime R: type, comptime child: anytype, comptime policy: ArrayPolicy) type {
    if (policy.minimum > policy.maximum) @compileError("invalid static array bounds");
    return Prefix(R, .{}, child, policy);
}
/// Build a typed positional prefix plus optional homogeneous rest codec.
/// Required positions must precede optional positions; no rest means excess elements are rejected.
/// Success owns a copied document and decoded children; clone rebases document-backed string views.
pub fn Prefix(comptime R: type, comptime prefix: anytype, comptime rest: anytype, comptime policy: ArrayPolicy) type {
    comptime {
        if (policy.minimum > policy.maximum) @compileError("invalid static prefix bounds");
        var optional_seen = false;
        var required: usize = 0;
        for (prefix) |element| {
            if (!element.required) optional_seen = true else {
                if (optional_seen) @compileError("required prefix cannot follow an optional prefix");
                required += 1;
            }
        }
        if (required > policy.maximum or (@TypeOf(rest) == @TypeOf(null) and policy.minimum > prefix.len)) @compileError("unsatisfiable static prefix bounds");
    }
    const has_rest = @TypeOf(rest) != @TypeOf(null);
    const Rest = if (has_rest) (if (@TypeOf(rest) == type) rest else @TypeOf(rest)).Decoded else void;
    const PrefixValues = blk: {
        var types: [prefix.len]type = undefined;
        inline for (prefix, 0..) |element, index| types[index] = ?(if (@TypeOf(element.codec) == type) element.codec else @TypeOf(element.codec)).Decoded;
        break :blk @Tuple(&types);
    };
    return struct {
        /// Application value produced on success; ownership follows the codec’s declared value type.
        pub const Decoded = struct {
            /// Exit/Result cleanup owns this value; release it once or clone it for an independent observation.
            pub const effect_owned_payload = true;
            /// Owned source document anchoring child string/token views.
            original: values.Owned,
            /// Optional typed values for present prefix positions.
            prefix_values: PrefixValues,
            /// Owned decoded tail, or null when no rest codec is configured.
            rest_values: ?outcomes.OwnedSlice(Rest),
            /// Release this decoded value’s owned storage and child payloads once; borrowed views become invalid.
            pub fn deinit(self: *@This()) void {
                inline for (0..prefix.len) |index| if (self.prefix_values[index]) |*value| outcomes.discardPayload(value);
                if (self.rest_values) |*value| value.deinit();
                self.original.deinit();
                self.* = undefined;
            }
            /// Create an independent owner, cloning child payloads and rebasing views into the copied document.
            pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
                var result: @This() = .{ .original = try self.original.clone(), .prefix_values = undefined, .rest_values = null };
                inline for (0..prefix.len) |index| result.prefix_values[index] = null;
                errdefer result.deinit();
                inline for (0..prefix.len) |index| if (self.prefix_values[index]) |value| {
                    result.prefix_values[index] = try values.cloneDecoded(value, &self.original.value, &result.original.value);
                };
                if (self.rest_values) |old| {
                    const items = try old.allocator.alloc(Rest, old.items.len);
                    var count: usize = 0;
                    errdefer {
                        for (items[0..count]) |*item| outcomes.discardPayload(item);
                        old.allocator.free(items);
                    }
                    for (old.items, items) |value, *slot| {
                        slot.* = try values.cloneDecoded(value, &self.original.value, &result.original.value);
                        count += 1;
                    }
                    result.rest_values = .{ .items = items, .allocator = old.allocator };
                }
                return result;
            }
        };
        /// Caller environment supplying allocation and any selected schema capabilities.
        pub const Environment = R;
        /// Decode using existing invocation-local traversal. String results borrow input; composite results retain owned documents/children.
        pub fn decodeWithin(env: *const R, input: *const std.json.Value, traversal: *state.DecodeState) outcomes.Exit(Decoded, Issue) {
            if (input.* != .array) return .{ .fail = Issue.init(env, .decode, .{ .expected = .array }) };
            const items = input.array.items;
            if (items.len < policy.minimum or items.len > policy.maximum or (!has_rest and items.len > prefix.len)) return .{ .fail = Issue.init(env, .decode, .constraint) };
            if (policy.unique) for (items, 0..) |*item, index| for (items[0..index]) |*prior| if (values.equal(item, prior)) return .{ .fail = Issue.init(env, .decode, .constraint) };
            var result: Decoded = .{ .original = values.Owned.copy(env, input) catch return .{ .die = outcomes.Defect.out_of_memory }, .prefix_values = undefined, .rest_values = null };
            inline for (0..prefix.len) |index| result.prefix_values[index] = null;
            var transferred = false;
            defer if (!transferred) result.deinit();
            inline for (prefix, 0..) |element, index| {
                if (index >= items.len) {
                    if (element.required) return atIndex(Decoded, env, .{ .fail = Issue.init(env, .decode, .missing_field) }, index);
                } else {
                    const checked = atIndex((if (@TypeOf(element.codec) == type) element.codec else @TypeOf(element.codec)).Decoded, env, decodeChild(element.codec, env, &result.original.value.array.items[index], traversal), index);
                    if (checked != .success) return outcomes.castFailure(Decoded, checked);
                    result.prefix_values[index] = checked.success;
                }
            }
            if (comptime has_rest) {
                const tail = result.original.value.array.items[@min(items.len, prefix.len)..];
                const decoded = env.allocator.alloc(Rest, tail.len) catch return .{ .die = outcomes.Defect.out_of_memory };
                var count: usize = 0;
                for (tail, decoded) |*item, *slot| {
                    const checked = atIndex(Rest, env, decodeChild(rest, env, item, traversal), prefix.len + count);
                    if (checked != .success) {
                        for (decoded[0..count]) |*value| outcomes.discardPayload(value);
                        env.allocator.free(decoded);
                        return outcomes.castFailure(Decoded, checked);
                    }
                    slot.* = checked.success;
                    count += 1;
                }
                result.rest_values = .{ .items = decoded, .allocator = env.allocator };
            }
            transferred = true;
            return .{ .success = result };
        }
        /// Encode using the caller’s scratch scope; preserve input owners while borrowed string/number views are in use.
        pub fn encodeWithin(env: *const R, input: *const Decoded, traversal: *state.EncodeState) outcomes.Exit(std.json.Value, Issue) {
            var array = std.json.Array.init(traversal.scratch);
            var absent_seen = false;
            inline for (prefix, 0..) |element, index| {
                if (input.prefix_values[index]) |*value| {
                    if (absent_seen) return .{ .fail = Issue.init(env, .encode, .missing_field) };
                    const encoded = atIndex(std.json.Value, env, encodeChild(element.codec, env, value, traversal), index);
                    if (encoded != .success) return encoded;
                    array.append(encoded.success) catch return .{ .die = outcomes.Defect.out_of_memory };
                } else {
                    if (element.required) return atIndex(std.json.Value, env, .{ .fail = Issue.init(env, .encode, .missing_field) }, index);
                    absent_seen = true;
                }
            }
            if (comptime has_rest) if (input.rest_values) |tail| {
                if (absent_seen and tail.items.len != 0) return .{ .fail = Issue.init(env, .encode, .missing_field) };
                for (tail.items, 0..) |*item, tail_index| {
                    const encoded = atIndex(std.json.Value, env, encodeChild(rest, env, item, traversal), prefix.len + tail_index);
                    if (encoded != .success) return encoded;
                    array.append(encoded.success) catch return .{ .die = outcomes.Defect.out_of_memory };
                }
            };
            if (array.items.len < policy.minimum or array.items.len > policy.maximum) return .{ .fail = Issue.init(env, .encode, .constraint) };
            if (policy.unique) for (array.items, 0..) |*item, index| for (array.items[0..index]) |*prior| if (values.equal(item, prior)) return .{ .fail = Issue.init(env, .encode, .constraint) };
            return .{ .success = .{ .array = array } };
        }
        /// Return immutable encoded-shape metadata; it does not execute callbacks or construct application values.
        pub fn encodedNode() *const ast.Node {
            const Metadata = struct {
                const Entry = @typeInfo(@FieldType(ast.PrefixArray, "elements")).pointer.child;
                fn elements() [prefix.len]Entry {
                    var result: [prefix.len]Entry = undefined;
                    inline for (prefix, 0..) |element, index| result[index] = .{ .schema = @import("internal/representation.zig").nodePointer(element.codec), .required = element.required };
                    return result;
                }
                const entries = elements();
                const node: ast.Node = .{ .prefix_array = .{ .elements = &entries, .rest = if (!has_rest) null else @import("internal/representation.zig").nodePointer(rest), .minimum = policy.minimum, .maximum = policy.maximum, .unique = policy.unique } };
            };
            return &Metadata.node;
        }
    };
}
/// Dispatch a statically typed child using its encode contract and the caller’s existing scratch scope.
pub fn encodeChild(comptime child: anytype, env: anytype, input: *const (if (@TypeOf(child) == type) child else @TypeOf(child)).Decoded, traversal: *state.EncodeState) outcomes.Exit(std.json.Value, Issue) {
    if (comptime @TypeOf(child) == type) return child.encodeWithin(env, input, traversal);
    return switch (child.encodeDirect(input)) {
        .ok => |value| .{ .success = value },
        .err => |issue| .{ .fail = issue },
    };
}

/// Inclusive count bounds for distinct parsed object properties.
pub const ObjectPolicy = struct {
    /// Inclusive minimum distinct parsed property count.
    minimum_properties: usize,
    /// Inclusive maximum distinct parsed property count.
    maximum_properties: usize,
};
/// Declared properties, every matching pattern, and additional values are
/// decoded independently. Their typed results are retained for encoding;
/// encoding never invokes a decoder to reconstruct them.
pub fn Object(comptime R: type, comptime properties: anytype, comptime name_codec: anytype, comptime patterns: anytype, comptime additional: anytype, comptime policy: ObjectPolicy, comptime select: ?fn (*const R) PatternCapability(R)) type {
    const has_additional = @TypeOf(additional) != @TypeOf(null);
    const Additional = if (has_additional) (if (@TypeOf(additional) == type) additional else @TypeOf(additional)).Decoded else void;
    const Known = blk: {
        var types: [properties.len]type = undefined;
        inline for (properties, 0..) |property, index| types[index] = ?(if (@TypeOf(property.codec) == type) property.codec else @TypeOf(property.codec)).Decoded;
        break :blk @Tuple(&types);
    };
    const PatternValues = blk: {
        var types: [patterns.len]type = undefined;
        inline for (patterns, 0..) |pattern, index| types[index] = ?(if (@TypeOf(pattern.codec) == type) pattern.codec else @TypeOf(pattern.codec)).Decoded;
        break :blk @Tuple(&types);
    };
    comptime {
        if (policy.minimum_properties > policy.maximum_properties) @compileError("invalid static object property bounds");
        if (patterns.len != 0 and select == null) @compileError("object patterns require a caller pattern capability");
        for (properties, 0..) |property, index| {
            if (!std.unicode.utf8ValidateSlice(property.name)) @compileError("property names must be UTF-8");
            for (properties, 0..) |prior, prior_index| if (prior_index < index and std.mem.eql(u8, property.name, prior.name)) @compileError("duplicate object property");
        }
        for (patterns) |pattern| if (pattern.pattern.dialect.len == 0 or pattern.pattern.maximum_steps == 0 or pattern.pattern.timeout.nanoseconds <= 0) @compileError("patterns require an explicit dialect and bounded execution");
    }
    return struct {
        /// Caller environment supplying allocation and any selected schema capabilities.
        pub const Environment = R;
        const Entry = struct {
            name: outcomes.OwnedSlice(u8),
            known: Known,
            pattern: PatternValues,
            extra: ?Additional = null,
            fn deinit(self: *@This()) void {
                inline for (0..properties.len) |index| if (self.known[index]) |*value| outcomes.discardPayload(value);
                inline for (0..patterns.len) |index| if (self.pattern[index]) |*value| outcomes.discardPayload(value);
                if (self.extra) |*value| outcomes.discardPayload(value);
                self.name.deinit();
            }
        };
        /// Application value produced on success; ownership follows the codec’s declared value type.
        pub const Decoded = struct {
            /// Exit/Result cleanup owns this value; release it once or clone it for an independent observation.
            pub const effect_owned_payload = true;
            /// Owned source document anchoring child string/token views.
            original: values.Owned,
            /// Owned property names and all applicable decoded child results.
            entries: []Entry,
            /// Originating allocator for the decoded property-entry array.
            allocator: std.mem.Allocator,
            /// Release this decoded value’s owned storage and child payloads once; borrowed views become invalid.
            pub fn deinit(self: *@This()) void {
                for (self.entries) |*entry| entry.deinit();
                self.allocator.free(self.entries);
                self.original.deinit();
                self.* = undefined;
            }
            /// Create an independent owner, cloning child payloads and rebasing views into the copied document.
            pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
                var result: @This() = .{ .original = try self.original.clone(), .entries = undefined, .allocator = self.allocator };
                errdefer result.original.deinit();
                result.entries = try self.allocator.alloc(Entry, self.entries.len);
                var count: usize = 0;
                errdefer {
                    for (result.entries[0..count]) |*entry| entry.deinit();
                    self.allocator.free(result.entries);
                }
                for (self.entries, result.entries) |entry, *slot| {
                    slot.* = .{ .name = try entry.name.clone(), .known = undefined, .pattern = undefined };
                    inline for (0..properties.len) |index| slot.known[index] = null;
                    inline for (0..patterns.len) |index| slot.pattern[index] = null;
                    count += 1;
                    inline for (0..properties.len) |index| if (entry.known[index]) |value| {
                        slot.known[index] = try values.cloneDecoded(value, &self.original.value, &result.original.value);
                    };
                    inline for (0..patterns.len) |index| if (entry.pattern[index]) |value| {
                        slot.pattern[index] = try values.cloneDecoded(value, &self.original.value, &result.original.value);
                    };
                    if (entry.extra) |value| slot.extra = try values.cloneDecoded(value, &self.original.value, &result.original.value);
                }
                return result;
            }
        };
        fn nameCheck(env: *const R, name: []const u8, direction: @import("schema_issue.zig").Direction) outcomes.Exit(void, Issue) {
            if (!std.unicode.utf8ValidateSlice(name)) return .{ .fail = Issue.init(env, direction, .invalid_utf8) };
            if (comptime @TypeOf(name_codec) != @TypeOf(null)) {
                const checked = if (@TypeOf(name_codec) == type) name_codec.validateDirect(env, &name) else blk: {
                    const result = name_codec.validateDirect(&name);
                    break :blk switch (result) {
                        .ok => outcomes.Exit(void, Issue){ .success = {} },
                        .err => |issue| outcomes.Exit(void, Issue){ .fail = issue },
                    };
                };
                if (checked == .fail) {
                    var issue = checked.fail;
                    issue.direction = direction;
                    issue.bindAllocator(env);
                    issue.prependPropertyName(name) catch {
                        issue.deinit();
                        return .{ .die = outcomes.Defect.out_of_memory };
                    };
                    return .{ .fail = issue };
                }
                return checked;
            }
            return .{ .success = {} };
        }
        /// Decode using existing invocation-local traversal. String results borrow input; composite results retain owned documents/children.
        pub fn decodeWithin(env: *const R, input: *const std.json.Value, traversal: *state.DecodeState) outcomes.Exit(Decoded, Issue) {
            if (input.* != .object) return .{ .fail = Issue.init(env, .decode, .{ .expected = .object }) };
            const count = input.object.count();
            if (count < policy.minimum_properties or count > policy.maximum_properties) return .{ .fail = Issue.init(env, .decode, .constraint) };
            inline for (properties) |property| if (property.required and !input.object.contains(property.name)) return atField(Decoded, env, .{ .fail = Issue.init(env, .decode, .missing_field) }, property.name);
            var original = values.Owned.copy(env, input) catch return .{ .die = outcomes.Defect.out_of_memory };
            const entries = env.allocator.alloc(Entry, count) catch {
                original.deinit();
                return .{ .die = outcomes.Defect.out_of_memory };
            };
            var initialized: usize = 0;
            var transferred = false;
            defer if (!transferred) {
                for (entries[0..initialized]) |*entry| entry.deinit();
                env.allocator.free(entries);
                original.deinit();
            };
            var iterator = original.value.object.iterator();
            while (iterator.next()) |item| {
                const checked_name = nameCheck(env, item.key_ptr.*, .decode);
                if (checked_name != .success) return outcomes.castFailure(Decoded, checked_name);
                const entry = &entries[initialized];
                entry.* = .{ .name = .{ .items = env.allocator.dupe(u8, item.key_ptr.*) catch return .{ .die = outcomes.Defect.out_of_memory }, .allocator = env.allocator }, .known = undefined, .pattern = undefined };
                inline for (0..properties.len) |index| entry.known[index] = null;
                inline for (0..patterns.len) |index| entry.pattern[index] = null;
                initialized += 1;
                var matched = false;
                inline for (properties, 0..) |property, index| if (std.mem.eql(u8, property.name, item.key_ptr.*)) {
                    const checked = atField((if (@TypeOf(property.codec) == type) property.codec else @TypeOf(property.codec)).Decoded, env, decodeChild(property.codec, env, item.value_ptr, traversal), item.key_ptr.*);
                    if (checked != .success) return outcomes.castFailure(Decoded, checked);
                    entry.known[index] = checked.success;
                    matched = true;
                };
                inline for (patterns, 0..) |pattern, index| {
                    const matching = select.?(env).matches(env, pattern.pattern, item.key_ptr.*, .decode);
                    if (matching != .success) return atField(Decoded, env, outcomes.castFailure(Decoded, matching), item.key_ptr.*);
                    if (matching.success) {
                        const checked = atField((if (@TypeOf(pattern.codec) == type) pattern.codec else @TypeOf(pattern.codec)).Decoded, env, decodeChild(pattern.codec, env, item.value_ptr, traversal), item.key_ptr.*);
                        if (checked != .success) return outcomes.castFailure(Decoded, checked);
                        entry.pattern[index] = checked.success;
                        matched = true;
                    }
                }
                if (!matched) {
                    if (comptime has_additional) {
                        const checked = atField((if (@TypeOf(additional) == type) additional else @TypeOf(additional)).Decoded, env, decodeChild(additional, env, item.value_ptr, traversal), item.key_ptr.*);
                        if (checked != .success) return outcomes.castFailure(Decoded, checked);
                        entry.extra = checked.success;
                    } else return atField(Decoded, env, .{ .fail = Issue.init(env, .decode, .unknown_field) }, item.key_ptr.*);
                }
            }
            transferred = true;
            return .{ .success = .{ .original = original, .entries = entries, .allocator = env.allocator } };
        }
        fn combine(env: *const R, selected: *?std.json.Value, encoded: outcomes.Exit(std.json.Value, Issue)) outcomes.Exit(void, Issue) {
            if (encoded != .success) return outcomes.castFailure(void, encoded);
            if (selected.*) |*prior| {
                if (!values.equal(prior, &encoded.success)) return .{ .fail = Issue.init(env, .encode, .conflict) };
            } else selected.* = encoded.success;
            return .{ .success = {} };
        }
        /// Encode using the caller’s scratch scope; preserve input owners while borrowed string/number views are in use.
        pub fn encodeWithin(env: *const R, input: *const Decoded, traversal: *state.EncodeState) outcomes.Exit(std.json.Value, Issue) {
            if (input.entries.len < policy.minimum_properties or input.entries.len > policy.maximum_properties) return .{ .fail = Issue.init(env, .encode, .constraint) };
            var result: std.json.Value = .{ .object = .{} };
            for (input.entries) |*entry| {
                const checked_name = nameCheck(env, entry.name.items, .encode);
                if (checked_name != .success) return outcomes.castFailure(std.json.Value, checked_name);
                if (result.object.contains(entry.name.items)) return .{ .fail = Issue.init(env, .encode, .duplicate_field) };
                var selected: ?std.json.Value = null;
                inline for (properties, 0..) |property, index| if (entry.known[index]) |*value| {
                    if (!std.mem.eql(u8, property.name, entry.name.items)) return .{ .fail = Issue.init(env, .encode, .invalid_value) };
                    const checked = combine(env, &selected, atField(std.json.Value, env, encodeChild(property.codec, env, value, traversal), entry.name.items));
                    if (checked != .success) return outcomes.castFailure(std.json.Value, checked);
                };
                inline for (patterns, 0..) |pattern, index| {
                    const matching = select.?(env).matches(env, pattern.pattern, entry.name.items, .encode);
                    if (matching != .success) return atField(std.json.Value, env, outcomes.castFailure(std.json.Value, matching), entry.name.items);
                    if (matching.success != (entry.pattern[index] != null)) return .{ .fail = Issue.init(env, .encode, .invalid_value) };
                    if (entry.pattern[index]) |*value| {
                        const checked = combine(env, &selected, atField(std.json.Value, env, encodeChild(pattern.codec, env, value, traversal), entry.name.items));
                        if (checked != .success) return outcomes.castFailure(std.json.Value, checked);
                    }
                }
                if (comptime has_additional) if (entry.extra) |*value| {
                    if (selected != null) return .{ .fail = Issue.init(env, .encode, .invalid_value) };
                    const encoded = atField(std.json.Value, env, encodeChild(additional, env, value, traversal), entry.name.items);
                    if (encoded != .success) return atField(std.json.Value, env, encoded, entry.name.items);
                    selected = encoded.success;
                };
                result.object.put(traversal.scratch, entry.name.items, selected orelse return .{ .fail = Issue.init(env, .encode, .unknown_field) }) catch return .{ .die = outcomes.Defect.out_of_memory };
            }
            inline for (properties) |property| if (property.required and !result.object.contains(property.name)) return atField(std.json.Value, env, .{ .fail = Issue.init(env, .encode, .missing_field) }, property.name);
            return .{ .success = result };
        }
        /// Return immutable encoded-shape metadata; it does not execute callbacks or construct application values.
        pub fn encodedNode() *const ast.Node {
            const Metadata = struct {
                fn declared() [properties.len]ast.Field {
                    var result: [properties.len]ast.Field = undefined;
                    inline for (properties, 0..) |property, index| result[index] = .{ .name = property.name, .presence = if (property.required) .required else .optional, .schema = @import("internal/representation.zig").nodePointer(property.codec) };
                    return result;
                }
                fn patterned() [patterns.len]ast.PatternProperty {
                    var result: [patterns.len]ast.PatternProperty = undefined;
                    inline for (patterns, 0..) |pattern, index| result[index] = .{ .dialect = pattern.pattern.dialect, .source = pattern.pattern.source, .schema = @import("internal/representation.zig").nodePointer(pattern.codec) };
                    return result;
                }
                const fields = declared();
                const pattern_fields = patterned();
                const base: ast.Node = .{ .record = .{ .fields = &fields, .unknown_fields = .ignore, .duplicate_fields = .last_wins, .issues = .first } };
                const node: ast.Node = .{ .object = .{ .base = &base, .property_names = if (@TypeOf(name_codec) == @TypeOf(null)) null else @import("internal/representation.zig").nodePointer(name_codec), .minimum = policy.minimum_properties, .maximum = policy.maximum_properties, .patterns = &pattern_fields, .additional = if (has_additional) @import("internal/representation.zig").nodePointer(additional) else null } };
            };
            return &Metadata.node;
        }
    };
}

/// Give a structural schema bounded public Effect and byte operations.
pub const withLimits = @import("json_surface.zig").WithLimits;
