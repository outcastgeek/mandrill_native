//! Typed cases derive native handlers, decoder associations and pure folds
//! from the same definitions. Native replay remains the cursor authority.
const std = @import("std");
const effects = @import("../effect.zig");
const outcomes = @import("../outcome.zig");
const events = @import("event.zig");
const journal = @import("event_journal.zig");
const projections = @import("projection.zig");
const Issue = @import("../schema/schema_issue.zig").Issue;
const Parsed = @import("../schema.zig").Parsed;
const parser = @import("../schema/internal/parser.zig");
const inference = @import("../dsl/inference.zig");
const roles = @import("../dsl/roles.zig");
const ownership = @import("../dsl/ownership.zig");
const ast = @import("../schema/schema_ast.zig");
const codecType = @import("../dsl/declarations.zig").codecType;

pub fn case(comptime handler: anytype, comptime fold: anytype) Case(handler, fold, null, null) {
    return .{};
}
fn Case(comptime selected: anytype, comptime callback: anytype, comptime decode_environment: anytype, comptime duplicates: ?ast.DuplicateFields) type {
    const H = @TypeOf(selected);
    if (@typeInfo(H) != .@"struct" or !@hasDecl(H, "EventDefinition") or !@hasDecl(H, "Environment") or H != events.Handler(H.EventDefinition, H.Environment)) @compileError("event case requires a completed native event handler");
    selected.definition.validate();
    return struct {
        pub const handler = selected;
        pub const fold = callback;
        pub const decoder_environment = decode_environment;
        pub const duplicate_fields = duplicates;
        pub const projection_case = struct {
            pub const definition = selected;
            pub const operation = callback;
            pub const projection = decode_environment;
            pub const duplicate_fields = duplicates;
        };
    };
}
fn requireCase(comptime T: type) void {
    if (@typeInfo(T) != .@"struct" or !@hasDecl(T, "projection_case")) @compileError("projection requires generated typed event cases");
    const C = T.projection_case;
    if (T != Case(C.definition, C.operation, C.projection, C.duplicate_fields)) @compileError("projection rejects foreign cases and copied markers");
}
fn handlers(comptime cases: anytype) Handlers(cases) {
    var result: Handlers(cases) = undefined;
    inline for (cases, 0..) |item, i| {
        requireCase(@TypeOf(item));
        result[i] = @TypeOf(item).handler;
    }
    return result;
}
fn Handlers(comptime cases: anytype) type {
    if (cases.len == 0) @compileError("projection requires a nonempty tuple of typed cases");
    var types: [cases.len]type = undefined;
    for (cases, 0..) |item, i| {
        requireCase(@TypeOf(item));
        types[i] = @TypeOf(@TypeOf(item).handler);
    }
    return @Tuple(&types);
}
fn FoldError(comptime State: type, comptime cases: anytype) type {
    var Failure: ?type = null;
    for (cases) |item| {
        const C = @TypeOf(item);
        const Payload = @TypeOf(C.handler).EventDefinition.Payload;
        const Result = inference.result(C.fold, &.{ *State, *const Payload }, "projection.case.fold");
        if (Result == void) continue;
        if (@typeInfo(Result) != .@"union" or !@hasField(Result, "err") or Result != outcomes.Result(void, @FieldType(Result, "err"))) @compileError("projection.case.fold requires void or exact Result(void,FoldError), never an Effect");
        if (Failure) |F| {
            if (F != @FieldType(Result, "err")) @compileError("projection cases have incompatible fold error types");
        } else Failure = @FieldType(Result, "err");
    }
    return Failure orelse void;
}
fn caseFor(comptime handler: anytype, comptime cases: anytype) type {
    for (cases) |item| {
        const C = @TypeOf(item);
        if (@TypeOf(C.handler) == @TypeOf(handler) and std.mem.eql(u8, C.handler.definition.tag, handler.definition.tag) and C.handler.definition.version == handler.definition.version) return C;
    }
    @compileError("projection cannot fold an event outside its typed cases");
}

pub fn projection(comptime supplied: anytype) type {
    roles.validate(roles.recordType(supplied), "projection", &.{ "state", "cases", "limits" }, &.{});
    const Schema = codecType(roles.get("state", supplied));
    const State = Schema.Decoded;
    if (@import("../cluster/entity_resource_plan.zig").containsResource(State)) @compileError("projection state cannot contain live activation resources");
    const cases = roles.get("cases", supplied);
    const declared_handlers = handlers(cases);
    const Group = events.Group(declared_handlers);
    const R = Group.Environment;
    const F = FoldError(State, cases);
    const limits = @import("../dsl/parsing.zig").checkedLimits(roles.get("limits", supplied), "projection");
    return struct {
        pub const StateType = State;
        pub const StateSchema = Schema;
        pub const schema = @import("../dsl/declarations.zig").codec(roles.get("state", supplied));
        pub const Error = projections.Error(F);
        pub const Environment = R;
        pub const Result = effects.Effect(projections.Replayed(State), Error, R);
        pub const resolve = Group.resolve;
        fn payloadOwner(comptime handler: anytype) type {
            const D = @TypeOf(handler).EventDefinition;
            const C = D.PayloadSchema;
            if (@hasDecl(C, "parseBytesDirect") or (@hasDecl(C, "Representation") and C.Representation == std.json.Value)) return Parsed(D.Payload);
            if (@hasDecl(C, "Representation") and C.Representation == []const u8) return ownership.Capture(D.Payload);
            @compileError("projection codec must use its bounded JSON schema or a byte representation");
        }
        fn duplicates(comptime handler: anytype) ast.DuplicateFields {
            const C = @TypeOf(handler).EventDefinition.PayloadSchema;
            const CaseType = caseFor(handler, cases);
            const shape: ?*const ast.Node = if (@hasDecl(C, "encodedNode")) C.encodedNode() else if (@hasField(C, "encoded_shape")) handler.definition.payload.encoded_shape else null;
            const known = if (shape) |node| duplicatePolicy(node) else null;
            if (CaseType.duplicate_fields) |selected| {
                if (known) |policy| if (selected != policy) @compileError("event case duplicate policy conflicts with its codec definition");
                return selected;
            }
            return known orelse @compileError("event case needs its JSON duplicate policy: use caseWith(.duplicate_fields) when codec metadata does not determine it");
        }
        fn decode(comptime handler: anytype, env: *const R, bytes: []const u8) outcomes.Exit(payloadOwner(handler), Issue) {
            const D = @TypeOf(handler).EventDefinition;
            const C = D.PayloadSchema;
            if (comptime @hasDecl(C, "parseBytesDirect")) return C.parseBytesDirect(env, bytes, limits);
            if (comptime C.Representation == []const u8) return switch (handler.definition.payload.decodeDirect(&bytes)) {
                .ok => |value| .{ .success = .{ .value = value } },
                .err => |problem| .{ .fail = problem },
            };
            return parser.parseWith(D.Payload, env, bytes, duplicates(handler), limits, struct {
                fn decodeRepresentation(_: *const R, representation: *const std.json.Value) outcomes.Exit(D.Payload, Issue) {
                    return switch (handler.definition.payload.decodeDirect(representation)) {
                        .ok => |value| .{ .success = value },
                        .err => |problem| .{ .fail = problem },
                    };
                }
            }.decodeRepresentation);
        }
        fn projectDecoder(comptime handler: anytype, described: anytype, env: *const R) effects.Effect(@typeInfo(@TypeOf(described)).pointer.child.Success, Issue, R) {
            const Source = @typeInfo(@TypeOf(described)).pointer.child;
            if (comptime Source.Environment == R) {
                const moved = described.*;
                described.* = undefined;
                return moved;
            }
            if (comptime Source.Environment == void) return described.provideSomeAllocated(env.allocator, R, struct {
                fn project(_: *const R) *const void {
                    return &{};
                }
            }.project);
            const CaseType = caseFor(handler, cases);
            if (comptime @TypeOf(CaseType.decoder_environment) == @TypeOf(null)) @compileError("event case needs an explicit directional environment projection in caseWith");
            return described.provideSomeAllocated(env.allocator, R, CaseType.decoder_environment);
        }
        fn decodeEffect(comptime handler: anytype, env: *const R, bytes: []const u8) effects.Effect(payloadOwner(handler), Issue, R) {
            const D = @TypeOf(handler).EventDefinition;
            const C = D.PayloadSchema;
            const Out = effects.Effect(payloadOwner(handler), Issue, R);
            if (comptime @hasDecl(C, "parseBytesDirect") or @hasDecl(C, "decodeDirect")) return Out.fromOwned(env, bytes, struct {
                fn native(input: *const []const u8, current: *const R) outcomes.Exit(payloadOwner(handler), Issue) {
                    return decode(handler, current, input.*);
                }
            }.native);
            if (comptime C.Representation == []const u8) {
                var seed = effects.Effect(void, Issue, R).succeed({});
                return seed.flatMapCapturedOwned(env, payloadOwner(handler), bytes, struct {
                    fn decodeBytes(input: *const []const u8, _: void, current: *const R) Out {
                        var described = handler.definition.payload.decode(input);
                        var projected = projectDecoder(handler, &described, current);
                        return projected.mapOwned(current, struct {
                            fn wrap(value: D.Payload) payloadOwner(handler) {
                                return .{ .value = value };
                            }
                        }.wrap);
                    }
                }.decodeBytes);
            }
            const Document = Parsed(std.json.Value);
            const Parse = effects.Effect(Document, Issue, R);
            var parsing = Parse.fromOwned(env, bytes, struct {
                fn document(input: *const []const u8, current: *const R) outcomes.Exit(Document, Issue) {
                    return parser.parseWith(std.json.Value, current, input.*, duplicates(handler), limits, struct {
                        fn identity(_: *const R, value: *const std.json.Value) outcomes.Exit(std.json.Value, Issue) {
                            return .{ .success = value.* };
                        }
                    }.identity);
                }
            }.document);
            return parsing.flatMapOwned(env, struct {
                fn parsed(document: Document, current: *const R) Out {
                    var seed = effects.Effect(void, Issue, R).succeed({});
                    return seed.flatMapCapturedOwned(current, payloadOwner(handler), document, struct {
                        fn decodeDocument(stored: *const Document, _: void, latest: *const R) Out {
                            var described = handler.definition.payload.decode(&stored.value);
                            var projected = projectDecoder(handler, &described, latest);
                            return projected.flatMapCapturedOwned(latest, payloadOwner(handler), stored, struct {
                                fn completed(pointer: *const *const Document, value: D.Payload, newest: *const R) Out {
                                    return Out.fromOwnedSuccess(newest, pointer.*.retainValue(value));
                                }
                            }.completed);
                        }
                    }.decodeDocument);
                }
            }.parsed);
        }
        fn fold(comptime handler: anytype, state: *State, payload: *const payloadOwner(handler)) outcomes.Result(void, F) {
            const C = caseFor(handler, cases);
            if (comptime inference.result(C.fold, &.{ *State, *const @TypeOf(handler).EventDefinition.Payload }, "projection.case.fold") == void) {
                C.fold(state, &payload.value);
                return .{ .ok = {} };
            }
            return C.fold(state, &payload.value);
        }
        const Inputs = struct { initial: *const State, after: journal.Cursor, page: *const journal.ReadPage };
        fn replay(input: *const Inputs, env: *const R) outcomes.Exit(projections.Replayed(State), Error) {
            return projections.replayPage(State, F, R, declared_handlers, env, input.initial, input.after, input.page, decode, fold);
        }
        /// Borrow the initial state/page through this lazy Effect. Native replay
        /// clones the state, preserves ordering, and advances only on success.
        pub fn replayPage(env: *const R, initial: *const State, after: journal.Cursor, page: *const journal.ReadPage) Result {
            const suspended = comptime blk: {
                for (declared_handlers) |handler| {
                    const C = @TypeOf(handler).EventDefinition.PayloadSchema;
                    if (!@hasDecl(C, "parseBytesDirect") and !@hasDecl(C, "decodeDirect")) break :blk true;
                }
                break :blk false;
            };
            if (comptime suspended) return projections.replayPageEffect(State, F, R, declared_handlers, env, initial, after, page, decodeEffect, fold);
            return Result.fromOwned(env, Inputs{ .initial = initial, .after = after, .page = page }, replay);
        }
    };
}

fn duplicatePolicy(comptime node: *const ast.Node) ?ast.DuplicateFields {
    return switch (node.*) {
        .primitive, .exact_number, .enumeration => .reject,
        .record => |record| record.duplicate_fields,
        .nullable => |child| duplicatePolicy(child),
        .constrained => |rule| duplicatePolicy(rule.base),
        else => null,
    };
}
fn CaseWith(comptime handler: anytype, comptime supplied: anytype) type {
    roles.validate(roles.recordType(supplied), "event.caseWith", &.{"fold"}, &.{ "decode_environment", "duplicate_fields" });
    const selected_projection = if (roles.has(roles.recordType(supplied), "decode_environment")) roles.get("decode_environment", supplied) else null;
    const duplicates: ?ast.DuplicateFields = if (roles.has(roles.recordType(supplied), "duplicate_fields")) roles.get("duplicate_fields", supplied) else null;
    return Case(handler, roles.get("fold", supplied), selected_projection, duplicates);
}
pub fn caseWith(comptime handler: anytype, comptime supplied: anytype) CaseWith(handler, supplied) {
    return .{};
}
