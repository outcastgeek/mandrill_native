//! Representation-generic typed records. Child Effects execute on the shared
//! trampoline; decoding and encoding environments remain separate.
const std = @import("std");
const outcomes = @import("../outcome.zig");
const effects = @import("../effect.zig");
const rt = @import("../internal/runtime.zig");
const Issue = @import("schema_issue.zig").Issue;
const codecs = @import("codec.zig");
const record_value = @import("record_value.zig");

/// Declare a required native record field; a missing representation member becomes a typed Issue.
pub fn required(comptime name: []const u8, comptime child: anytype) Field(name, child, .required, {}) {
    return .{};
}
/// Record-field policy independent of JSON nullability and the chosen native representation.
pub const Presence = enum {
    /// Missing representation member fails decoding.
    required,
    /// Missing member becomes null; null is omitted when encoding.
    optional,
    /// Decode the selected encoded default only when the member is absent.
    defaulted,
};
fn Field(comptime field_name: []const u8, comptime child: anytype, comptime field_presence: Presence, comptime default_value: anytype) type {
    return struct {
        /// Public comptime evidence that this descriptor belongs to the native field builder.
        pub const field_definition = true;
        /// Static named application/representation field; validated at record specialization.
        name: []const u8 = field_name,
        /// Typed child codec used once for each present field and direction.
        codec: @TypeOf(child) = child,
        /// Required/optional/default policy independent of representation nullability.
        presence: Presence = field_presence,
        /// Static representation or callback used only for absent decoding fields.
        default: @TypeOf(default_value) = default_value,
    };
}
/// Declare an optional application field; absence decodes as null and null is omitted on encoding.
pub fn optional(comptime name: []const u8, comptime child: anytype) Field(name, child, .optional, {}) {
    return .{};
}
/// Declare an encoded default callback evaluated only when the field is absent.
/// The decoder consumes the returned representation once and releases its owner after validation.
pub fn defaulted(comptime name: []const u8, comptime child: anytype, comptime callback: anytype) Field(name, child, .defaulted, callback) {
    return .{};
}
/// Constant defaults are encoded representations, decoded/validated only when
/// absent. This avoids validating a default through the other direction's R.
pub fn constantDefault(comptime name: []const u8, comptime child: anytype, comptime value: @TypeOf(child).Representation) Field(name, child, .defaulted, value) {
    return .{};
}
fn childCodec(comptime child: anytype) @TypeOf(if (@hasDecl(@TypeOf(child), "asCodec")) child.asCodec() else child) {
    if (comptime @hasDecl(@TypeOf(child), "asCodec")) return child.asCodec();
    return child;
}

fn borrows(comptime T: type) bool {
    if (outcomes.ownsPayload(T)) return false;
    return switch (@typeInfo(T)) {
        .pointer => true,
        .optional => |info| borrows(info.child),
        .array => |info| borrows(info.child),
        .@"struct" => |info| blk: {
            inline for (info.field_types) |Child| if (borrows(Child)) break :blk true;
            break :blk false;
        },
        .@"union" => |info| blk: {
            inline for (info.field_types) |Child| if (borrows(Child)) break :blk true;
            break :blk false;
        },
        else => false,
    };
}

/// O is a static object contract: Representation, Builder, find, count,
/// nameAt, init(*EncodeEnv,count), put, finish and discard. All potentially
/// allocating operations return a typed Exit, never a hidden allocator.
pub fn Record(comptime T: type, comptime O: type, comptime DecodeEnv: type, comptime EncodeEnv: type, comptime fields: anytype, comptime reject_unknown: bool) type {
    const Rep = O.Representation;
    const Fields = record_value.Fields(T);
    comptime {
        const shape = @typeInfo(Fields);
        if (shape != .@"struct" or shape.@"struct".is_tuple or shape.@"struct".field_names.len != fields.len) @compileError("native record requires every named field exactly once");
        for (fields, 0..) |field, index| {
            if (!@hasField(Fields, field.name)) @compileError("native record field does not exist");
            for (fields, 0..) |prior, prior_index| if (prior_index < index and std.mem.eql(u8, prior.name, field.name)) @compileError("duplicate native record field");
            const C = @TypeOf(childCodec(field.codec));
            if (C.Representation != Rep or (C.DecodingEnvironment != void and C.DecodingEnvironment != DecodeEnv) or (C.EncodingEnvironment != void and C.EncodingEnvironment != EncodeEnv)) @compileError("native field codec representation/environment mismatch");
            if (field.presence == .defaulted and borrows(C.Decoded)) @compileError("native default output must own borrowed data before default storage is released");
            const Expected = if (field.presence == .optional) ?C.Decoded else C.Decoded;
            if (@FieldType(Fields, field.name) != Expected) @compileError("native record field decoded type mismatch");
            if (outcomes.ownsPayload(Expected) and !outcomes.ownsPayload(T)) @compileError("native record with owned fields requires public clone/deinit ownership evidence");
            for (shape.@"struct".field_names, shape.@"struct".field_types, shape.@"struct".field_attrs) |name, Type, attrs| {
                if (std.mem.eql(u8, name, field.name)) {
                    if (attrs.@"comptime") @compileError("native schema fields must store runtime values");
                    _ = attrs.defaultValue(Type);
                }
            }
        }
    }
    return struct {
        /// Application value produced on success; ownership follows the codec’s declared value type.
        pub const Decoded = T;
        /// Wire or provider representation consumed and produced by this codec.
        pub const Representation = Rep;
        /// Services needed by decoding only; encoding services are not acquired by this direction.
        pub const DecodingEnvironment = DecodeEnv;
        /// Services needed by encoding only; decoding services are not acquired by this direction.
        pub const EncodingEnvironment = EncodeEnv;
        const Decode = effects.Effect(T, Issue, DecodeEnv);
        const Encode = effects.Effect(Rep, Issue, EncodeEnv);
        const DecodeFrame = struct {
            parent: *Decode.RuntimeAdapter.Job,
            allocator: std.mem.Allocator,
            input: *const Rep,
            value: T = undefined,
            initialized: [fields.len]bool = @splat(false),
            index: usize = 0,
            fn fail(self: *@This(), result: outcomes.Exit(T, Issue)) rt.Step {
                inline for (fields, 0..) |field, index| if (self.initialized[index]) {
                    outcomes.discardPayload(&@field(record_value.get(&self.value).*, field.name));
                };
                const parent = self.parent;
                self.allocator.destroy(self);
                return parent.adapterFinish(result);
            }
            fn advance(state: *rt.State, data: ?*anyopaque) rt.Step {
                const frame: *@This() = @ptrCast(@alignCast(data.?));
                if (state.cancelled()) return frame.fail(.interrupted);
                inline for (fields, 0..) |field, index| {
                    if (frame.index == index) {
                        const child = comptime childCodec(field.codec);
                        const C = @TypeOf(child);
                        const Child = effects.Effect(C.Decoded, Issue, C.DecodingEnvironment);
                        const Slot = struct {
                            frame: *DecodeFrame,
                            child: Child.RuntimeAdapter.Job = undefined,
                            result: outcomes.Exit(C.Decoded, Issue) = undefined,
                            default_storage: Rep = undefined,
                            owns_default: bool = false,
                            fn complete(next_state: *rt.State, slot_data: ?*anyopaque) rt.Step {
                                const slot: *@This() = @ptrCast(@alignCast(slot_data.?));
                                const current = slot.frame;
                                const result = next_state.publish(slot.result);
                                if (slot.owns_default) outcomes.discardPayload(&slot.default_storage);
                                current.allocator.destroy(slot);
                                switch (result) {
                                    .success => |value| {
                                        @field(record_value.get(&current.value).*, field.name) = value;
                                        current.initialized[index] = true;
                                        current.index += 1;
                                        return .{ .data = current, .advance = DecodeFrame.advance };
                                    },
                                    .fail => |original| {
                                        var issue = original;
                                        issue.bindAllocator(current.parent.env);
                                        issue.prependField(field.name) catch {
                                            issue.deinit();
                                            return current.fail(.{ .die = outcomes.Defect.out_of_memory });
                                        };
                                        return current.fail(.{ .fail = issue });
                                    },
                                    else => return current.fail(outcomes.castFailure(T, result)),
                                }
                            }
                        };
                        const found = O.find(frame.input, field.name);
                        if (found == null and field.presence == .optional) {
                            @field(record_value.get(&frame.value).*, field.name) = null;
                            frame.initialized[index] = true;
                            frame.index += 1;
                            return .{ .data = frame, .advance = DecodeFrame.advance };
                        }
                        if (found == null and field.presence == .required) return frame.fail(outcomes.castFailure(T, atField(frame.parent.env, field.name, Issue.init(frame.parent.env, .decode, .missing_field))));
                        const slot = frame.allocator.create(Slot) catch return frame.fail(.{ .die = outcomes.Defect.out_of_memory });
                        slot.* = .{ .frame = frame };
                        var selected = found;
                        if (comptime field.presence == .defaulted) if (selected == null) {
                            if (@TypeOf(field.default) == Rep) {
                                slot.default_storage = outcomes.clonePayload(field.default) catch {
                                    frame.allocator.destroy(slot);
                                    return frame.fail(.{ .die = outcomes.Defect.out_of_memory });
                                };
                                slot.owns_default = outcomes.ownsPayload(Rep);
                            } else {
                                const result = field.default(frame.parent.env);
                                if (result != .success) {
                                    frame.allocator.destroy(slot);
                                    return frame.fail(outcomes.castFailure(T, result));
                                }
                                slot.default_storage = result.success;
                                slot.owns_default = true;
                            }
                            selected = &slot.default_storage;
                        };
                        const program = child.decode(selected.?);
                        slot.child = .{ .program = program, .owned = program.ownership, .env = if (C.DecodingEnvironment == void) &{} else frame.parent.env, .inherited = state.services, .sink = .{ .output = &slot.result, .next_step = .{ .data = slot, .advance = Slot.complete } } };
                        return slot.child.adapterStep();
                    }
                }
                const result = frame.value;
                const parent = frame.parent;
                frame.allocator.destroy(frame);
                return parent.adapterFinish(.{ .success = result });
            }
        };
        fn startDecode(state: *rt.State, parent: *Decode.RuntimeAdapter.Job) rt.Step {
            const input: *const Rep = @ptrCast(@alignCast(parent.program.value.borrowed));
            if (reject_unknown) {
                for (0..O.count(input)) |index| {
                    const name = O.nameAt(input, index);
                    var known = false;
                    inline for (fields) |field| if (std.mem.eql(u8, name, field.name)) {
                        known = true;
                    };
                    if (!known) return parent.adapterFinish(.{ .fail = Issue.init(parent.env, .decode, .unknown_field) });
                }
            }
            const allocator = state.services.allocatorValue() orelse return parent.adapterFinish(.{ .die = outcomes.Defect.out_of_memory });
            const frame = allocator.create(DecodeFrame) catch return parent.adapterFinish(.{ .die = outcomes.Defect.out_of_memory });
            frame.* = .{ .parent = parent, .allocator = allocator, .input = input };
            return .{ .data = frame, .advance = DecodeFrame.advance };
        }
        /// Construct lazy borrowed-representation decoding on the shared trampoline.
        /// Keep input alive through join; child failure/cancellation releases partially decoded owning fields.
        pub fn decode(input: *const Rep) Decode {
            return .{ .value = .{ .borrowed = @ptrCast(input) }, .start = startDecode };
        }
        const EncodeFrame = struct {
            parent: *Encode.RuntimeAdapter.Job,
            allocator: std.mem.Allocator,
            input: *const T,
            builder: O.Builder,
            index: usize = 0,
            fn fail(self: *@This(), result: outcomes.Exit(Rep, Issue)) rt.Step {
                O.discard(&self.builder);
                const parent = self.parent;
                self.allocator.destroy(self);
                return parent.adapterFinish(result);
            }
            fn advance(state: *rt.State, data: ?*anyopaque) rt.Step {
                const frame: *@This() = @ptrCast(@alignCast(data.?));
                if (state.cancelled()) return frame.fail(.interrupted);
                inline for (fields, 0..) |field, index| {
                    if (frame.index == index) {
                        const child = comptime childCodec(field.codec);
                        const C = @TypeOf(child);
                        const Child = effects.Effect(Rep, Issue, C.EncodingEnvironment);
                        const Slot = struct {
                            frame: *EncodeFrame,
                            child: Child.RuntimeAdapter.Job = undefined,
                            result: outcomes.Exit(Rep, Issue) = undefined,
                            fn complete(next_state: *rt.State, slot_data: ?*anyopaque) rt.Step {
                                const slot: *@This() = @ptrCast(@alignCast(slot_data.?));
                                const current = slot.frame;
                                var result = next_state.publish(slot.result);
                                current.allocator.destroy(slot);
                                if (result != .success) {
                                    if (result == .fail) return current.fail(outcomes.castFailure(Rep, atField(current.parent.env, field.name, result.fail)));
                                    return current.fail(outcomes.castFailure(Rep, result));
                                }
                                // put borrows the encoded value; the contract owns its copy.
                                const inserted = O.put(current.parent.env, &current.builder, field.name, &result.success);
                                outcomes.discardPayload(&result.success);
                                if (inserted != .success) {
                                    if (inserted == .fail) return current.fail(outcomes.castFailure(Rep, atField(current.parent.env, field.name, inserted.fail)));
                                    return current.fail(outcomes.castFailure(Rep, inserted));
                                }
                                current.index += 1;
                                return .{ .data = current, .advance = EncodeFrame.advance };
                            }
                        };
                        if (comptime field.presence == .optional) if (@field(record_value.get(frame.input).*, field.name) == null) {
                            frame.index += 1;
                            return .{ .data = frame, .advance = EncodeFrame.advance };
                        };
                        const value = if (field.presence == .optional) &@field(record_value.get(frame.input).*, field.name).? else &@field(record_value.get(frame.input).*, field.name);
                        const slot = frame.allocator.create(Slot) catch return frame.fail(.{ .die = outcomes.Defect.out_of_memory });
                        slot.* = .{ .frame = frame };
                        const program = child.encode(value);
                        slot.child = .{ .program = program, .owned = program.ownership, .env = if (C.EncodingEnvironment == void) &{} else frame.parent.env, .inherited = state.services, .sink = .{ .output = &slot.result, .next_step = .{ .data = slot, .advance = Slot.complete } } };
                        return slot.child.adapterStep();
                    }
                }
                const result = O.finish(frame.parent.env, &frame.builder);
                if (result != .success) return frame.fail(result);
                const parent = frame.parent;
                frame.allocator.destroy(frame);
                return parent.adapterFinish(result);
            }
        };
        fn startEncode(state: *rt.State, parent: *Encode.RuntimeAdapter.Job) rt.Step {
            const input: *const T = @ptrCast(@alignCast(parent.program.value.borrowed));
            const initialized = O.init(parent.env, fields.len);
            if (initialized != .success) return parent.adapterFinish(outcomes.castFailure(Rep, initialized));
            const allocator = state.services.allocatorValue() orelse {
                var builder = initialized.success;
                O.discard(&builder);
                return parent.adapterFinish(.{ .die = outcomes.Defect.out_of_memory });
            };
            const frame = allocator.create(EncodeFrame) catch {
                var builder = initialized.success;
                O.discard(&builder);
                return parent.adapterFinish(.{ .die = outcomes.Defect.out_of_memory });
            };
            frame.* = .{ .parent = parent, .allocator = allocator, .input = input, .builder = initialized.success };
            return .{ .data = frame, .advance = EncodeFrame.advance };
        }
        /// Construct lazy application-value encoding on the shared trampoline.
        /// Borrow input through join; the object contract owns builder cleanup and successful representation transfer.
        pub fn encode(value: *const T) Encode {
            return .{ .value = .{ .borrowed = @ptrCast(value) }, .start = startEncode };
        }
        /// Expose the same record operations through Codec without changing their direction environments.
        pub fn asCodec() codecs.Codec(T, Rep, DecodeEnv, EncodeEnv) {
            return codecs.Codec(T, Rep, DecodeEnv, EncodeEnv).init(decode, encode);
        }
        /// Encode once, discard the successful representation, and preserve encoding failures/cleanup channels.
        pub fn validate(value: *const T) effects.Effect(void, Issue, EncodeEnv) {
            return asCodec().validate(value);
        }
    };
}
/// Staged record construction: fields supplies typed descriptors, build chooses unknown-field rejection.
/// Native object callbacks allocate only through their supplied direction environment.
pub fn Builder(comptime T: type, comptime O: type, comptime D: type, comptime E: type) type {
    return struct {
        /// Select the complete static field tuple; field names/types/presence are checked when build specializes the record.
        pub fn fields(comptime definitions: anytype) type {
            return struct {
                /// Build the record codec with an explicit unknown-member policy; no decoding or allocation occurs here.
                pub fn build(comptime reject_unknown: bool) type {
                    return Record(T, O, D, E, definitions, reject_unknown);
                }
            };
        }
    };
}

/// Sequence representations may expose arrays, slices, maps or provider-native
/// collections; their entry codec supplies typed key/value construction.
pub fn sequence(comptime child: anytype, comptime S: type, comptime D: type, comptime W: type, comptime maximum: usize) type {
    return @import("sequence_codec.zig").Sequence(childCodec(child), S, D, W, Issue, maximum, struct {
        fn capacity() Issue {
            return Issue.pure(.decode, .size_limit);
        }
    }.capacity, struct {
        fn capacity() Issue {
            return Issue.pure(.encode, .size_limit);
        }
    }.capacity);
}
/// Sequence alias; its representation contract must implement its own array entry operations.
pub const array = sequence;
/// Sequence alias; key/value shape, uniqueness and representation are supplied by the map entry/object contract.
pub const map = sequence;

/// Optional values preserve independent pure and effectful direction contracts.
pub fn option(comptime child: anytype) @TypeOf(@import("domain_codec.zig").optional(@import("domain_codec.zig").ofSchema(Issue, child, identityIssue, identityIssue))) {
    return @import("domain_codec.zig").optional(@import("domain_codec.zig").ofSchema(Issue, child, identityIssue, identityIssue));
}
fn identityIssue(issue: Issue) Issue {
    return issue;
}

fn atField(env: anytype, name: []const u8, original: Issue) outcomes.Exit(void, Issue) {
    var issue = original;
    issue.bindAllocator(env);
    issue.prependField(name) catch {
        issue.deinit();
        return .{ .die = outcomes.Defect.out_of_memory };
    };
    return .{ .fail = issue };
}
