//! Statically checked JSON record composition. Field names, codecs, presence,
//! and default constructors are definition data, resolved at comptime.
const std = @import("std");
const outcomes = @import("../outcome.zig");
const effects = @import("../effect.zig");
const ast = @import("schema_ast.zig");
const Issue = @import("schema_issue.zig").Issue;
const legacy = @import("../schema.zig");
const parser = @import("internal/parser.zig");
const structural = @import("internal/structural.zig");
const representation = @import("internal/representation.zig");

/// Resource bounds accepted by the schema parser.
pub const ParseLimits = parser.Limits;
/// Parse a schema representation with caller-selected limits.
pub const parseWith = parser.parseWith;

/// Policy for unknown fields, duplicate fields, issue collection, and issue count.
pub const RecordPolicy = struct {
    unknown_fields: ast.UnknownFields,
    duplicate_fields: ast.DuplicateFields,
    issues: ast.Issues,
    max_issues: ?usize = null,

    /// Check whether this schema record policy is internally consistent.
    ///
    /// Parameters:
    /// - `self: RecordPolicy`: Receiver passed by value: the struct itself is copied, while any referenced storage keeps its original ownership and lifetime.
    /// Returns (`bool`): `true` when first-issue mode has no count or accumulation has a positive count.
    pub fn valid(self: RecordPolicy) bool {
        return switch (self.issues) {
            .first => self.max_issues == null,
            .accumulate => self.max_issues != null and self.max_issues.? > 0,
        };
    }
};

/// Owns the map allocation. Keys and JSON values borrow the parsed document;
/// `Parsed(T)` keeps that document alive for decoded records.
pub const UnknownFields = struct {
    /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements `deinit` and
    /// must be released after use.
    pub const effect_owned_payload = true;
    values: std.json.ObjectMap,
    allocator: std.mem.Allocator,

    /// Release resources owned by this value.
    /// Parameters:
    /// - `self: *UnknownFields`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
    /// Returns (`void`): no value.
    pub fn deinit(self: *UnknownFields) void {
        self.values.deinit(self.allocator);
        self.* = undefined;
    }

    /// Duplicate this value; marked owning payloads are cloned, while ordinary
    /// borrowed fields retain their original lifetime.
    /// Parameters:
    /// - `self: *const UnknownFields`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
    /// Returns (`error{OutOfMemory}!UnknownFields`): A `UnknownFields` on success; `OutOfMemory` if required allocation fails.
    pub fn clone(self: *const UnknownFields) error{OutOfMemory}!UnknownFields {
        var values: std.json.ObjectMap = .{};
        errdefer values.deinit(self.allocator);
        for (self.values.keys(), self.values.values()) |key, value| try values.put(self.allocator, key, value);
        return .{ .values = values, .allocator = self.allocator };
    }
};

/// Declares a required JSON field and its pure codec; absence fails decoding.
///
/// Parameters:
/// - `name: comptime []const u8`: Compile-time stable, nonempty identity component validated at definition time.
/// - `codec: comptime anytype`: Compile-time typed codec/schema fixing the corresponding decoded and encoded value
///   types.
/// Returns (`@TypeOf(.{ .name = name, .codec = codec, .presence = ast.Presence.required, .default_value = {}, .extension = false, .nested = false })`): A compile-time descriptor containing the declared field or case metadata.
pub fn required(comptime name: []const u8, comptime codec: anytype) @TypeOf(.{ .name = name, .codec = codec, .presence = ast.Presence.required, .default_value = {}, .extension = false, .nested = false }) {
    return .{ .name = name, .codec = codec, .presence = ast.Presence.required, .default_value = {}, .extension = false, .nested = false };
}

/// Declares an optional JSON field whose decoded Zig type is optional.
///
/// Parameters:
/// - `name: comptime []const u8`: Compile-time stable, nonempty identity component validated at definition time.
/// - `codec: comptime anytype`: Compile-time typed codec/schema fixing the corresponding decoded and encoded value
///   types.
/// Returns (`@TypeOf(.{ .name = name, .codec = codec, .presence = ast.Presence.optional, .default_value = {}, .extension = false, .nested = false })`): A compile-time descriptor containing the declared field or case metadata.
pub fn optional(comptime name: []const u8, comptime codec: anytype) @TypeOf(.{ .name = name, .codec = codec, .presence = ast.Presence.optional, .default_value = {}, .extension = false, .nested = false }) {
    return .{ .name = name, .codec = codec, .presence = ast.Presence.optional, .default_value = {}, .extension = false, .nested = false };
}

/// Declares a field whose typed constructor runs only when input omits it.
///
/// Parameters:
/// - `name: comptime []const u8`: Compile-time stable, nonempty identity component validated at definition time.
/// - `codec: comptime anytype`: Compile-time typed codec/schema fixing the corresponding decoded and encoded value
///   types.
/// - `constructor: comptime anytype`: Compile-time statically inferred input checked when this operation is
///   instantiated.
/// Returns (`@TypeOf(.{ .name = name, .codec = codec, .presence = ast.Presence.defaulted, .default_value = constructor, .extension = false, .nested = false })`): A compile-time descriptor containing the declared field or case metadata.
pub fn defaulted(comptime name: []const u8, comptime codec: anytype, comptime constructor: anytype) @TypeOf(.{ .name = name, .codec = codec, .presence = ast.Presence.defaulted, .default_value = constructor, .extension = false, .nested = false }) {
    return .{ .name = name, .codec = codec, .presence = ast.Presence.defaulted, .default_value = constructor, .extension = false, .nested = false };
}

/// Declares a required field backed by a structural child schema.
///
/// Parameters:
/// - `name: comptime []const u8`: Compile-time stable, nonempty identity component validated at definition time.
/// - `Child: comptime type`: Compile-time child schema or value type.
/// Returns (`@TypeOf(.{ .name = name, .codec = Child, .presence = ast.Presence.required, .default_value = {}, .extension = false, .nested = true })`): A compile-time descriptor containing the declared field or case metadata.
pub fn nestedRequired(comptime name: []const u8, comptime Child: type) @TypeOf(.{ .name = name, .codec = Child, .presence = ast.Presence.required, .default_value = {}, .extension = false, .nested = true }) {
    return .{ .name = name, .codec = Child, .presence = ast.Presence.required, .default_value = {}, .extension = false, .nested = true };
}

/// Declares an optional field backed by a structural child schema.
///
/// Parameters:
/// - `name: comptime []const u8`: Compile-time stable, nonempty identity component validated at definition time.
/// - `Child: comptime type`: Compile-time child schema or value type.
/// Returns (`@TypeOf(.{ .name = name, .codec = Child, .presence = ast.Presence.optional, .default_value = {}, .extension = false, .nested = true })`): A compile-time descriptor containing the declared field or case metadata.
pub fn nestedOptional(comptime name: []const u8, comptime Child: type) @TypeOf(.{ .name = name, .codec = Child, .presence = ast.Presence.optional, .default_value = {}, .extension = false, .nested = true }) {
    return .{ .name = name, .codec = Child, .presence = ast.Presence.optional, .default_value = {}, .extension = false, .nested = true };
}

/// Declares a nested field with a typed default constructor.
///
/// Parameters:
/// - `name: comptime []const u8`: Compile-time stable, nonempty identity component validated at definition time.
/// - `Child: comptime type`: Compile-time child schema or value type.
/// - `constructor: comptime anytype`: Compile-time statically inferred input checked when this operation is
///   instantiated.
/// Returns (`@TypeOf(.{ .name = name, .codec = Child, .presence = ast.Presence.defaulted, .default_value = constructor, .extension = false, .nested = true })`): A compile-time descriptor containing the declared field or case metadata.
pub fn nestedDefaulted(comptime name: []const u8, comptime Child: type, comptime constructor: anytype) @TypeOf(.{ .name = name, .codec = Child, .presence = ast.Presence.defaulted, .default_value = constructor, .extension = false, .nested = true }) {
    return .{ .name = name, .codec = Child, .presence = ast.Presence.defaulted, .default_value = constructor, .extension = false, .nested = true };
}

/// A record is unavailable until its fields and policy are supplied. Every
/// field constructor itself requires the matching codec and, for defaults,
/// the typed default callback before this stage can finish.
/// Staged record builder. `fields` must pair every Zig field with a typed
/// codec, and `policy` must be supplied before `build` exposes the schema.
/// Allocating decode/encode operations take their allocator from `R`.
///
/// Parameters:
/// - `T: comptime type`: Value type stored or transported by the returned component.
/// - `R: comptime type`: Caller-defined environment type that supplies dependencies and runtime capabilities when work executes.
/// Returns (`type`): staged record builder requiring field definitions and a parsing policy before `build` is available.
pub fn Builder(comptime T: type, comptime R: type) type {
    return struct {
        /// Starts record construction; the result has no build operation yet.
        ///
        /// Returns (`type`): first stage; only `fields` is available until every Zig field has a codec.
        pub fn begin() type {
            return struct {
                /// Pairs the Zig record fields with typed codecs, presence, and defaults.
                ///
                /// Parameters:
                /// - `definitions: comptime anytype`: Compile-time statically inferred input checked when this
                ///   operation is instantiated.
                /// Returns (`type`): stage retaining the field tuple; `policy` must be selected next.
                pub fn fields(comptime definitions: anytype) type {
                    return struct {
                        /// Selects unknown-field, duplicate-field, and issue collection behavior.
                        ///
                        /// Parameters:
                        /// - `selected: comptime RecordPolicy`: Compile-time the selected input required by this typed
                        ///   operation.
                        /// Returns (`type`): completed stage exposing `Definition` and `build` for the checked record schema.
                        pub fn policy(comptime selected: RecordPolicy) type {
                            const Schema = Record(T, R, definitions, selected);
                            return struct {
                                /// Completed schema or protocol definition type selected by this builder.
                                pub const Definition = Schema;

                                /// Completes the checked record schema after fields and policy are supplied.
                                ///
                                /// Returns (`type`): the checked record schema type, ready for typed encode and decode.
                                pub fn build() type {
                                    return Schema;
                                }
                            };
                        }
                    };
                }
            };
        }
    };
}

/// Reserves one field for owned unknown JSON properties when preservation is selected.
///
/// Parameters:
/// - `field_name: comptime []const u8`: Compile-time field name used as the stable schema key.
/// Returns (`@TypeOf(.{ .name = field_name, .codec = {}, .presence = ast.Presence.required, .default_value = {}, .extension = true, .nested = false })`): A compile-time descriptor containing the declared field or case metadata.
pub fn extensions(comptime field_name: []const u8) @TypeOf(.{ .name = field_name, .codec = {}, .presence = ast.Presence.required, .default_value = {}, .extension = true, .nested = false }) {
    return .{ .name = field_name, .codec = {}, .presence = ast.Presence.required, .default_value = {}, .extension = true, .nested = false };
}

fn codecType(comptime field: anytype) type {
    return if (field.nested) field.codec else @TypeOf(field.codec);
}

/// A record codec must be specialized on the environment that owns errors and
/// encoded bytes. Decoded borrowed strings remain tied to their JSON input.
/// Comptime-checked bidirectional JSON record schema. It rejects missing,
/// duplicate, or mismatched fields at compilation; runtime input errors are
/// returned as typed `Issue` values. Document-backed decoded views remain
/// owned by their `Parsed(T)` result.
///
/// Parameters:
/// - `T: comptime type`: Value type stored or transported by the returned component.
/// - `R: comptime type`: Caller-defined environment type that supplies dependencies and runtime capabilities when work executes.
/// - `fields: comptime anytype`: Compile-time statically inferred input checked when this operation is instantiated.
/// - `policy: comptime RecordPolicy`: Compile-time caller-selected, validated behavior or resource bounds; no hidden
///   default is installed.
/// Returns (`type`): bidirectional schema for `T` with the supplied field codecs, environment, and parse policy.
pub fn Record(comptime T: type, comptime R: type, comptime fields: anytype, comptime policy: RecordPolicy) type {
    comptime {
        if (!policy.valid()) @compileError("record issue policy requires an explicit positive bound only when accumulating");
        const info = @typeInfo(T);
        if (info != .@"struct" or info.@"struct".is_tuple) @compileError("SchemaBuilder.Record requires a named Zig struct");
        if (@typeInfo(@TypeOf(fields)) != .@"struct" or !@typeInfo(@TypeOf(fields)).@"struct".is_tuple)
            @compileError("SchemaBuilder.Record fields must be a tuple");
        if (!@hasField(R, "allocator") or @FieldType(R, "allocator") != std.mem.Allocator)
            @compileError("SchemaBuilder.Record requires R.allocator");
        if (fields.len != info.@"struct".field_names.len)
            @compileError("each Zig record field needs exactly one schema field");
        var extensions_count: usize = 0;
        for (fields, 0..) |field, index| {
            if (field.name.len == 0 or !std.unicode.utf8ValidateSlice(field.name))
                @compileError("schema field names must be nonempty UTF-8");
            if (!@hasField(T, field.name)) @compileError("schema field does not exist in the Zig struct");
            for (fields, 0..) |previous, previous_index| {
                if (previous_index < index and std.mem.eql(u8, previous.name, field.name))
                    @compileError("duplicate schema field");
            }
            if (field.extension) {
                extensions_count += 1;
                if (@FieldType(T, field.name) != UnknownFields)
                    @compileError("extension field must have SchemaBuilder.UnknownFields type");
                if (!outcomes.ownsPayload(T))
                    @compileError("a record preserving unknown fields must own and release its extension field");
                continue;
            }
            const C = codecType(field);
            if (field.nested) {
                if (!@hasDecl(C, "Decoded") or !@hasDecl(C, "Environment") or C.Environment != R or
                    !@hasDecl(C, "decodeWithin") or !@hasDecl(C, "encodeWithin"))
                    @compileError("nested fields require a structural codec in the same environment");
            } else {
                if (!@hasDecl(C, "Representation") or C.Representation != std.json.Value or
                    !@hasDecl(C, "DecodingEnvironment") or C.DecodingEnvironment != void or
                    !@hasDecl(C, "EncodingEnvironment") or C.EncodingEnvironment != void)
                    @compileError("record fields require pure JSON codecs");
            }
            const expected = if (field.presence == .optional) ?C.Decoded else C.Decoded;
            if (@FieldType(T, field.name) != expected) @compileError("schema field decoded type does not match Zig field type");
            if (outcomes.ownsPayload(expected) and !outcomes.ownsPayload(T))
                @compileError("a record with owned fields must declare its own deinit/clone ownership contract");
            if (field.presence == .defaulted) {
                if (@TypeOf(field.default_value) != fn (*const R) outcomes.Exit(C.Decoded, Issue))
                    @compileError("default constructor must return a typed Exit and accept R");
            }
        }
        if ((policy.unknown_fields == .preserve and extensions_count != 1) or
            (policy.unknown_fields != .preserve and extensions_count != 0))
            @compileError("unknown-field preservation requires exactly one extension field");
    }
    return struct {
        const Self = @This();
        /// Application value type produced by this codec.
        pub const Decoded = T;
        /// Exact caller environment type required when the description runs.
        pub const Environment = R;

        pub const supports_direct_writer = blk: {
            for (fields) |field| {
                if (field.extension) continue;
                if (!@import("json_writer.zig").capable(field.codec)) break :blk false;
            }
            break :blk true;
        };
        /// Eligible records serialize fields once without constructing a JSON tree.
        pub fn writeJson(env: *const R, value: *const T, writer: *@import("json_writer.zig").Writer) outcomes.Exit(void, Issue) {
            if (!supports_direct_writer) @compileError("record child has no direct JSON writer evidence");
            writer.enter() catch |problem| return outcomes.castFailure(void, @import("json_writer.zig").failureWith(env, .encode, problem, writer));
            defer writer.leave();
            writer.write("{") catch |problem| return outcomes.castFailure(void, @import("json_writer.zig").failureWith(env, .encode, problem, writer));
            var first = true;
            inline for (fields) |field| {
                if (comptime field.extension) {
                    const extras = &@field(value.*, field.name);
                    for (extras.values.keys(), extras.values.values()) |key, json_value| {
                        inline for (fields) |known| if (!known.extension and std.mem.eql(u8, key, known.name)) return .{ .fail = Issue.init(env, .encode, .duplicate_field) };
                        if (!first) writer.write(",") catch |problem| return outcomes.castFailure(void, @import("json_writer.zig").failureWith(env, .encode, problem, writer));
                        first = false;
                        writer.quoted(key) catch |problem| return outcomes.castFailure(void, @import("json_writer.zig").failureWith(env, .encode, problem, writer));
                        writer.write(":") catch |problem| return outcomes.castFailure(void, @import("json_writer.zig").failureWith(env, .encode, problem, writer));
                        writer.json(&json_value) catch |problem| return outcomes.castFailure(void, @import("json_writer.zig").failureWith(env, .encode, problem, writer));
                    }
                    continue;
                }
                const present = if (comptime field.presence == .optional) @field(value.*, field.name) != null else true;
                if (present) {
                    if (!first) writer.write(",") catch |problem| return outcomes.castFailure(void, @import("json_writer.zig").failureWith(env, .encode, problem, writer));
                    first = false;
                    writer.quoted(field.name) catch |problem| return outcomes.castFailure(void, @import("json_writer.zig").failureWith(env, .encode, problem, writer));
                    writer.write(":") catch |problem| return outcomes.castFailure(void, @import("json_writer.zig").failureWith(env, .encode, problem, writer));
                    const input = if (comptime field.presence == .optional) &@field(value.*, field.name).? else &@field(value.*, field.name);
                    const result = if (comptime field.nested) field.codec.writeJson(env, input, writer) else field.codec.writeJson(input, writer);
                    switch (result) {
                        .success => {},
                        .fail => |issue| return outcomes.castFailure(void, fieldFailure(env, field.name, issue)),
                        else => return result,
                    }
                }
            }
            writer.write("}") catch |problem| return outcomes.castFailure(void, @import("json_writer.zig").failureWith(env, .encode, problem, writer));
            return .{ .success = {} };
        }

        fn encodedFields() [fields.len - (if (policy.unknown_fields == .preserve) @as(usize, 1) else 0)]ast.Field {
            var result: [fields.len - (if (policy.unknown_fields == .preserve) @as(usize, 1) else 0)]ast.Field = undefined;
            comptime var index: usize = 0;
            inline for (fields) |field| {
                if (field.extension) continue;
                result[index] = .{
                    .name = field.name,
                    .presence = field.presence,
                    .schema = representation.nodePointer(field.codec),
                };
                index += 1;
            }
            return result;
        }

        /// Return the structural representation node for this schema.
        ///
        /// Returns (`*const ast.Node`): A borrowed read-only pointer; keep its owner alive while using it.
        pub fn encodedNode() *const ast.Node {
            const Metadata = struct {
                const field_nodes = encodedFields();
                const node: ast.Node = .{ .record = .{
                    .fields = &field_nodes,
                    .unknown_fields = policy.unknown_fields,
                    .duplicate_fields = policy.duplicate_fields,
                    .issues = policy.issues,
                } };
            };
            return &Metadata.node;
        }

        /// Retrieve  from the current typed state.
        ///
        /// Parameters:
        /// - `name: comptime []const u8`: Compile-time stable, nonempty identity component validated at definition
        ///   time.
        /// - `value: *const T`: Borrowed input; keep its owner alive until the operation has copied or
        ///   joined it.
        /// Returns (`*const @FieldType(T, name)`): A borrowed read-only pointer; keep its owner alive while using it.
        pub fn get(comptime name: []const u8, value: *const T) *const @FieldType(T, name) {
            if (!@hasField(T, name)) @compileError("unknown schema record field");
            inline for (fields) |field| {
                if (std.mem.eql(u8, field.name, name)) return &@field(value.*, name);
            }
            @compileError("Zig field is not part of this record schema");
        }

        fn fieldFailure(env: *const R, name: []const u8, original: Issue) outcomes.Exit(T, Issue) {
            var issue = original;
            issue.bindAllocator(env);
            issue.prependField(name) catch {
                issue.deinit();
                return .{ .die = outcomes.Defect.out_of_memory };
            };
            return .{ .fail = issue };
        }

        const Collector = struct {
            list: std.array_list.Managed(Issue),
            limit_reached: bool = false,
            maximum: usize,

            fn init(allocator: std.mem.Allocator, maximum: ?usize) Collector {
                return .{ .list = std.array_list.Managed(Issue).init(allocator), .maximum = @min(policy.max_issues orelse 1, maximum orelse (policy.max_issues orelse 1)) };
            }

            fn deinit(self: *Collector) void {
                for (self.list.items) |*issue| issue.deinit();
                self.list.deinit();
            }

            fn add(self: *Collector, issue: Issue) error{OutOfMemory}!void {
                self.list.append(issue) catch {
                    var discarded = issue;
                    discarded.deinit();
                    return error.OutOfMemory;
                };
                self.limit_reached = self.list.items.len >= self.maximum;
            }

            fn finish(self: *Collector, env: *const R) outcomes.Exit(T, Issue) {
                const owned = self.list.toOwnedSlice() catch return .{ .die = outcomes.Defect.out_of_memory };
                return .{ .fail = Issue.many(env, .decode, owned, self.limit_reached) };
            }
        };

        fn noteFailure(env: *const R, collector: *Collector, failure: outcomes.Exit(T, Issue)) ?outcomes.Exit(T, Issue) {
            if (comptime policy.issues == .first) return failure;
            if (failure != .fail) return failure;
            collector.add(failure.fail) catch return .{ .die = outcomes.Defect.out_of_memory };
            if (collector.limit_reached) return collector.finish(env);
            return null;
        }

        fn collectExtensions(env: *const R, json: *const std.json.Value) outcomes.Exit(UnknownFields, Issue) {
            var values: std.json.ObjectMap = .{};
            for (json.object.keys()) |key| {
                var recognized = false;
                inline for (fields) |field| {
                    if (!field.extension and std.mem.eql(u8, key, field.name)) recognized = true;
                }
                if (recognized) continue;
                if (!std.unicode.utf8ValidateSlice(key)) {
                    values.deinit(env.allocator);
                    return .{ .fail = Issue.init(env, .decode, .invalid_utf8) };
                }
                values.put(env.allocator, key, json.object.get(key).?) catch {
                    values.deinit(env.allocator);
                    return .{ .die = outcomes.Defect.out_of_memory };
                };
            }
            return .{ .success = .{ .values = values, .allocator = env.allocator } };
        }

        fn decodeField(comptime field: anytype, env: *const R, encoded: *const std.json.Value, state: *structural.DecodeState) outcomes.Exit(codecType(field).Decoded, Issue) {
            if (comptime field.nested) return field.codec.decodeWithin(env, encoded, state);
            return switch (field.codec.decodeDirect(encoded)) {
                .ok => |value| .{ .success = value },
                .err => |issue| .{ .fail = issue },
            };
        }

        /// The caller retains `json` for the lifetime of borrowed decoded
        /// fields. A failed partial construction releases each owned field.
        ///
        /// Parameters:
        /// - `env: *const R`: Borrowed caller environment supplying allocator, I/O, and selected
        ///   capabilities.
        /// - `json: *const std.json.Value`: JSON value or document to decode, encode, or validate.
        /// Returns (`outcomes.Exit(T, Issue)`): Carries `T` on `.success` or `Issue` on `.fail`; `.die`, `.interrupted`, and `.finalization_failed` preserve other terminal causes.
        pub fn decodeDirect(env: *const R, json: *const std.json.Value) outcomes.Exit(T, Issue) {
            var state: structural.DecodeState = .{};
            return decodeWithin(env, json, &state);
        }

        /// Decodes a child value in an existing parsed-document ownership scope.
        ///
        /// Parameters:
        /// - `env: *const R`: Borrowed caller environment supplying allocator, I/O, and selected
        ///   capabilities.
        /// - `json: *const std.json.Value`: JSON value or document to decode, encode, or validate.
        /// - `state: *structural.DecodeState`: Mutable operation state, retained for the duration of this call.
        /// Returns (`outcomes.Exit(T, Issue)`): Carries `T` on `.success` or `Issue` on `.fail`; `.die`, `.interrupted`, and `.finalization_failed` preserve other terminal causes.
        pub fn decodeWithin(env: *const R, json: *const std.json.Value, state: *structural.DecodeState) outcomes.Exit(T, Issue) {
            if (json.* != .object) return .{ .fail = Issue.init(env, .decode, .{ .expected = .object }) };
            var collector = Collector.init(env.allocator, state.maximum_issues);
            defer collector.deinit();
            if (policy.unknown_fields == .reject) {
                for (json.object.keys()) |key| {
                    var recognized = false;
                    inline for (fields) |field| {
                        if (!field.extension and std.mem.eql(u8, key, field.name)) recognized = true;
                    }
                    if (!recognized) {
                        if (noteFailure(env, &collector, fieldFailure(env, key, Issue.init(env, .decode, .unknown_field)))) |terminal|
                            return terminal;
                    }
                }
            }
            var result: T = undefined;
            var initialized: [fields.len]bool = @splat(false);
            inline for (fields, 0..) |field, index| {
                if (comptime field.extension) {
                    const collected = collectExtensions(env, json);
                    switch (collected) {
                        .success => |value| {
                            @field(result, field.name) = value;
                            initialized[index] = true;
                        },
                        else => {
                            releasePartial(&result, &initialized);
                            return outcomes.castFailure(T, collected);
                        },
                    }
                    continue;
                }
                const present = json.object.get(field.name);
                if (present) |encoded| {
                    const decoded = decodeField(field, env, &encoded, state);
                    switch (decoded) {
                        .success => |value| {
                            @field(result, field.name) = value;
                            initialized[index] = true;
                        },
                        .fail => |issue| {
                            if (noteFailure(env, &collector, fieldFailure(env, field.name, issue))) |terminal| {
                                releasePartial(&result, &initialized);
                                return terminal;
                            }
                        },
                        else => {
                            releasePartial(&result, &initialized);
                            return outcomes.castFailure(T, decoded);
                        },
                    }
                } else {
                    switch (field.presence) {
                        .required => {
                            if (noteFailure(env, &collector, fieldFailure(env, field.name, Issue.init(env, .decode, .missing_field)))) |terminal| {
                                releasePartial(&result, &initialized);
                                return terminal;
                            }
                        },
                        .optional => {
                            @field(result, field.name) = null;
                            initialized[index] = true;
                        },
                        .defaulted => {
                            const made = field.default_value(env);
                            switch (made) {
                                .success => |value| {
                                    const checked = if (comptime field.nested) field.codec.validateDirect(env, &value) else blk: {
                                        const valid = field.codec.validateDirect(&value);
                                        break :blk switch (valid) {
                                            .ok => outcomes.Exit(void, Issue){ .success = {} },
                                            .err => |issue| outcomes.Exit(void, Issue){ .fail = issue },
                                        };
                                    };
                                    if (checked != .success) {
                                        var discarded = value;
                                        outcomes.discardPayload(&discarded);
                                        if (checked == .fail) {
                                            var issue = checked.fail;
                                            issue.direction = .decode;
                                            if (noteFailure(env, &collector, fieldFailure(env, field.name, issue))) |terminal| {
                                                releasePartial(&result, &initialized);
                                                return terminal;
                                            }
                                        } else {
                                            releasePartial(&result, &initialized);
                                            return outcomes.castFailure(T, checked);
                                        }
                                    } else {
                                        @field(result, field.name) = value;
                                        initialized[index] = true;
                                    }
                                },
                                .fail => |issue| {
                                    if (noteFailure(env, &collector, fieldFailure(env, field.name, issue))) |terminal| {
                                        releasePartial(&result, &initialized);
                                        return terminal;
                                    }
                                },
                                else => {
                                    releasePartial(&result, &initialized);
                                    return outcomes.castFailure(T, made);
                                },
                            }
                        },
                    }
                }
            }
            if (collector.list.items.len != 0) {
                releasePartial(&result, &initialized);
                return collector.finish(env);
            }
            return .{ .success = result };
        }

        fn releasePartial(value: *T, initialized: *[fields.len]bool) void {
            inline for (fields, 0..) |field, index| {
                if (initialized[index]) outcomes.discardPayload(&@field(value.*, field.name));
            }
        }

        /// Describes decoding as an Effect using the allocator and services in R.
        ///
        /// Parameters:
        /// - `json: *const std.json.Value`: JSON value or document to decode, encode, or validate.
        /// Returns (`effects.Effect(T, Issue, R)`): decoded record `T` or a bounded list of typed field issues.
        pub fn decode(json: *const std.json.Value) effects.Effect(T, Issue, R) {
            return effects.Effect(T, Issue, R).fromBorrowed(std.json.Value, json, struct {
                fn run(input: *const std.json.Value, env: *const R) outcomes.Exit(T, Issue) {
                    return decodeDirect(env, input);
                }
            }.run);
        }

        /// Scans before allocating a JSON tree so the caller's explicit
        /// document and nesting limits apply even to malformed input.
        ///
        /// Parameters:
        /// - `env: *const R`: Borrowed caller environment supplying allocator, I/O, and selected
        ///   capabilities.
        /// - `bytes: []const u8`: Borrowed input; keep its owner alive until the operation has copied or
        ///   joined it.
        /// - `limits: ParseLimits`: Caller-selected, validated behavior or resource bounds; no hidden default
        ///   is installed.
        /// Returns (`outcomes.Exit(legacy.Parsed(T), Issue)`): Carries `legacy.Parsed(T)` on `.success` or `Issue` on `.fail`; `.die`, `.interrupted`, and `.finalization_failed` preserve other terminal causes.
        pub fn parseBytesDirect(
            env: *const R,
            bytes: []const u8,
            limits: ParseLimits,
        ) outcomes.Exit(legacy.Parsed(T), Issue) {
            return parseWith(T, env, bytes, policy.duplicate_fields, limits, decodeDirect);
        }

        fn encodeField(comptime field: anytype, env: *const R, value: *const codecType(field).Decoded, state: *structural.EncodeState) outcomes.Exit(std.json.Value, Issue) {
            if (comptime field.nested) return field.codec.encodeWithin(env, value, state);
            return switch (field.codec.encodeDirect(value)) {
                .ok => |encoded| .{ .success = encoded },
                .err => |issue| .{ .fail = issue },
            };
        }

        /// Structural composition shares the temporary representation arena;
        /// no nested serialization/parsing or extra per-field allocation.
        ///
        /// Parameters:
        /// - `env: *const R`: Borrowed caller environment supplying allocator, I/O, and selected
        ///   capabilities.
        /// - `value: *const T`: Borrowed input; keep its owner alive until the operation has copied or
        ///   joined it.
        /// - `temporary: std.mem.Allocator`: Temporary state retained only for the duration of this operation.
        /// Returns (`outcomes.Exit(std.json.Value, Issue)`): Carries `std.json.Value` on `.success` or `Issue` on `.fail`; `.die`, `.interrupted`, and `.finalization_failed` preserve other terminal causes.
        pub fn encodeValueDirect(env: *const R, value: *const T, temporary: std.mem.Allocator) outcomes.Exit(std.json.Value, Issue) {
            var state: structural.EncodeState = .{ .scratch = temporary };
            return encodeWithin(env, value, &state);
        }

        /// Encodes a child record within an existing JSON ownership scope.
        ///
        /// Parameters:
        /// - `env: *const R`: Borrowed caller environment supplying allocator, I/O, and selected
        ///   capabilities.
        /// - `value: *const T`: Borrowed input; keep its owner alive until the operation has copied or
        ///   joined it.
        /// - `state: *structural.EncodeState`: Mutable operation state, retained for the duration of this call.
        /// Returns (`outcomes.Exit(std.json.Value, Issue)`): Carries `std.json.Value` on `.success` or `Issue` on `.fail`; `.die`, `.interrupted`, and `.finalization_failed` preserve other terminal causes.
        pub fn encodeWithin(env: *const R, value: *const T, state: *structural.EncodeState) outcomes.Exit(std.json.Value, Issue) {
            var object: std.json.ObjectMap = .{};
            inline for (fields) |field| {
                if (comptime field.extension) {
                    const extra = &@field(value.*, field.name);
                    for (extra.values.keys(), extra.values.values()) |key, json_value| {
                        if (key.len == 0 or !std.unicode.utf8ValidateSlice(key))
                            return .{ .fail = Issue.init(env, .encode, .invalid_utf8) };
                        var collision = false;
                        inline for (fields) |known| {
                            if (!known.extension and std.mem.eql(u8, key, known.name)) collision = true;
                        }
                        if (collision) return outcomes.castFailure(std.json.Value, fieldFailure(env, key, Issue.init(env, .encode, .duplicate_field)));
                        object.put(state.scratch, key, json_value) catch return .{ .die = outcomes.Defect.out_of_memory };
                    }
                    continue;
                }
                const present = if (comptime field.presence == .optional) @field(value.*, field.name) != null else true;
                if (present) {
                    const field_value = if (comptime field.presence == .optional) @field(value.*, field.name).? else @field(value.*, field.name);
                    const encoded = encodeField(field, env, &field_value, state);
                    switch (encoded) {
                        .success => |json_value| object.put(state.scratch, field.name, json_value) catch return .{ .die = outcomes.Defect.out_of_memory },
                        .fail => |issue| return outcomes.castFailure(std.json.Value, fieldFailure(env, field.name, issue)),
                        else => return outcomes.castFailure(std.json.Value, encoded),
                    }
                }
            }
            return .{ .success = .{ .object = object } };
        }

        /// Encodes into caller-owned bytes. The temporary JSON object and any
        /// nested representation storage are freed before this returns.
        ///
        /// Parameters:
        /// - `env: *const R`: Borrowed caller environment supplying allocator, I/O, and selected
        ///   capabilities.
        /// - `value: *const T`: Borrowed input; keep its owner alive until the operation has copied or
        ///   joined it.
        /// Returns (`outcomes.Exit(outcomes.OwnedSlice(u8), Issue)`): Carries `outcomes.OwnedSlice(u8)` on `.success` or `Issue` on `.fail`; `.die`, `.interrupted`, and `.finalization_failed` preserve other terminal causes.
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

        /// Checks the application-side value through every field encoder,
        /// without decoding input or executing default constructors.
        ///
        /// Parameters:
        /// - `env: *const R`: Borrowed caller environment supplying allocator, I/O, and selected
        ///   capabilities.
        /// - `value: *const T`: Borrowed input; keep its owner alive until the operation has copied or
        ///   joined it.
        /// Returns (`outcomes.Exit(void, Issue)`): Carries no value on `.success` or `Issue` on `.fail`; `.die`, `.interrupted`, and `.finalization_failed` preserve other terminal causes.
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

        /// Describes typed record validation as an Effect in R.
        ///
        /// Parameters:
        /// - `value: *const T`: Borrowed input; keep its owner alive until the operation has copied or
        ///   joined it.
        /// Returns (`effects.Effect(void, Issue, R)`): unit success when the JSON record satisfies every declared field policy, otherwise typed issues.
        pub fn validate(value: *const T) effects.Effect(void, Issue, R) {
            return effects.Effect(void, Issue, R).fromBorrowed(T, value, struct {
                fn run(input: *const T, env: *const R) outcomes.Exit(void, Issue) {
                    return validateDirect(env, input);
                }
            }.run);
        }

        /// Describes JSON byte encoding as an Effect; the result owns its buffer.
        ///
        /// Parameters:
        /// - `value: *const T`: Borrowed input; keep its owner alive until the operation has copied or
        ///   joined it.
        /// Returns (`effects.Effect(outcomes.OwnedSlice(u8), Issue, R)`): owned JSON bytes encoded from `T` under the declared record policy.
        pub fn encodeBytes(value: *const T) effects.Effect(outcomes.OwnedSlice(u8), Issue, R) {
            return effects.Effect(outcomes.OwnedSlice(u8), Issue, R).fromBorrowed(T, value, struct {
                fn run(input: *const T, env: *const R) outcomes.Exit(outcomes.OwnedSlice(u8), Issue) {
                    return encodeBytesDirect(env, input);
                }
            }.run);
        }
    };
}
