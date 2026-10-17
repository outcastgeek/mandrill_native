//! Collection descriptions delegate ordering, bounded admission and draining
//! to the existing traversal authority. Runtime rows only retain typed inputs.
const std = @import("std");
const effects = @import("../effect.zig");
const outcomes = @import("../outcome.zig");
const inference = @import("inference.zig");
const roles = @import("roles.zig");
const ownership = @import("ownership.zig");

fn Item(comptime T: type) type {
    return switch (@typeInfo(T)) {
        .array => |a| a.child,
        .pointer => |p| if (p.size == .slice) p.child else if (p.size == .one and @typeInfo(p.child) == .array) @typeInfo(p.child).array.child else @compileError("collection requires a slice or array borrow"),
        else => @compileError("collection requires a slice or array borrow"),
    };
}
fn Program(comptime T: type) type {
    const P = switch (@typeInfo(T)) {
        .@"struct" => |s| if (s.is_tuple and s.field_types.len != 0) s.field_types[0] else @compileError("all needs a nonempty typed tuple or one explicit Effect anchor via allOf"),
        else => Item(T),
    };
    inference.requireEffect(P, "all.item");
    if (@typeInfo(T) == .@"struct") inline for (@typeInfo(T).@"struct".field_types) |F| {
        if (F != P) @compileError("all tuple members must share exact A/E/R; use explicit mapping and projection before collection");
    };
    return P;
}
fn Collected(comptime P: type) type {
    inference.requireEffect(P, "all.item");
    return effects.Effect(outcomes.OwnedSlice(P.Success), P.Error, P.Environment);
}

pub fn all(comptime programs: anytype) Collected(Program(@TypeOf(programs))) {
    return allOf(Program(@TypeOf(programs)), programs);
}
pub fn allOf(comptime P: type, comptime programs: anytype) Collected(P) {
    const stored = comptime blk: {
        var values: [programs.len]P = undefined;
        for (programs, 0..) |program, index| {
            if (@TypeOf(program) != P) @compileError("allOf members must match the explicit Effect anchor");
            values[index] = program;
        }
        break :blk values;
    };
    return P.all(&stored);
}
pub fn allBorrowed(programs: anytype) Collected(Program(@TypeOf(programs))) {
    return Program(@TypeOf(programs)).all(programs);
}
pub fn allConcurrent(comptime programs: anytype, concurrency: usize) Collected(Program(@TypeOf(programs))) {
    const P = Program(@TypeOf(programs));
    const stored = comptime blk: {
        var values: [programs.len]P = undefined;
        for (programs, 0..) |program, index| values[index] = program;
        break :blk values;
    };
    return P.allConcurrent(concurrency, &stored);
}
pub fn allConcurrentStatic(comptime programs: anytype, comptime concurrency: usize) Collected(Program(@TypeOf(programs))) {
    if (concurrency == 0) @compileError("allConcurrent concurrency must be positive");
    return allConcurrent(programs, concurrency);
}
pub fn allConcurrentBorrowed(programs: anytype, concurrency: usize) Collected(Program(@TypeOf(programs))) {
    return Program(@TypeOf(programs)).allConcurrent(concurrency, programs);
}

/// Consume both an owning descriptions slice and each description it stores.
/// Borrowed all keeps these owners with its caller instead.
fn OwnedPrograms(comptime T: type) type {
    if (@typeInfo(T) != .pointer or @typeInfo(T).pointer.size != .one or @typeInfo(T).pointer.attrs.@"const") @compileError("allOwned requires a mutable consuming source pointer; use allBorrowed for caller-owned descriptions");
    const Owner = @typeInfo(T).pointer.child;
    if (@typeInfo(Owner) != .@"struct" or !@hasField(Owner, "items")) @compileError("allOwned requires an owning descriptions slice");
    const P = Program(@FieldType(Owner, "items"));
    if (Owner != outcomes.OwnedSlice(P)) @compileError("allOwned requires an owning descriptions slice");
    return P;
}
pub fn allOwned(env: anytype, source: anytype) Collected(OwnedPrograms(@TypeOf(source))) {
    const P = OwnedPrograms(@TypeOf(source));
    const programs = source.*;
    source.* = undefined;
    const Owner = struct {
        pub const effect_owned_payload = true;
        allocator: std.mem.Allocator,
        items: []P,
        pub fn deinit(self: *@This()) void {
            for (self.items) |*program| program.deinit();
            self.allocator.free(self.items);
        }
    };
    return forEachOwned(env, Owner{ .allocator = programs.allocator, .items = programs.items }, @as(void, {}), .{ .body = struct {
        fn run(_: *const void, item: *const P, _: *const P.Environment) P {
            return item.borrow();
        }
    }.run });
}

fn Body(comptime T: type, comptime Input: type, comptime R: type, comptime supplied: anytype) type {
    roles.validate(roles.recordType(supplied), "forEach", &.{"body"}, &.{});
    const P = inference.result(roles.get("body", supplied), &.{ *const T, *const Input, *const R }, "forEach.body");
    inference.requireEffect(P, "forEach.body");
    if (P.Environment != R) @compileError("forEach.body environment mismatch; use an explicit projection");
    return P;
}

fn PlainBody(comptime Input: type, comptime R: type, comptime supplied: anytype) type {
    roles.validate(roles.recordType(supplied), "forEach", &.{"body"}, &.{});
    const P = inference.result(roles.get("body", supplied), &.{ *const Input, *const R }, "forEach.body");
    inference.requireEffect(P, "forEach.body");
    if (P.Environment != R) @compileError("forEach.body environment mismatch; use an explicit projection");
    return P;
}

pub fn forEach(env: anytype, values: anytype, comptime supplied: anytype) Collected(PlainBody(Item(@TypeOf(values)), @typeInfo(@TypeOf(env)).pointer.child, supplied)) {
    const R = @typeInfo(@TypeOf(env)).pointer.child;
    const Input = Item(@TypeOf(values));
    const P = PlainBody(Input, R, supplied);
    return forEachCaptured(env, values, @as(void, {}), .{ .body = struct {
        fn run(_: *const void, item: *const Input, current: *const R) P {
            return roles.get("body", supplied)(item, current);
        }
    }.run });
}
pub fn forEachConcurrent(env: anytype, values: anytype, concurrency: usize, comptime supplied: anytype) Collected(PlainBody(Item(@TypeOf(values)), @typeInfo(@TypeOf(env)).pointer.child, supplied)) {
    const R = @typeInfo(@TypeOf(env)).pointer.child;
    const Input = Item(@TypeOf(values));
    const P = PlainBody(Input, R, supplied);
    return forEachConcurrentCaptured(env, values, @as(void, {}), concurrency, .{ .body = struct {
        fn run(_: *const void, item: *const Input, current: *const R) P {
            return roles.get("body", supplied)(item, current);
        }
    }.run });
}

fn build(env: anytype, values: anytype, capture: anytype, owned_values: anytype, concurrency: ?usize, comptime concurrent: bool, comptime supplied: anytype) Collected(Body(@TypeOf(capture), Item(@TypeOf(values)), @typeInfo(@TypeOf(env)).pointer.child, supplied)) {
    const T = @TypeOf(capture);
    const Input = Item(@TypeOf(values));
    const InputOwner = @TypeOf(owned_values);
    const R = @typeInfo(@TypeOf(env)).pointer.child;
    const P = Body(T, Input, R, supplied);
    const Out = Collected(P);
    const Storage = struct {
        const Self = @This();
        const Row = struct { storage: *Self, index: usize };
        allocator: std.mem.Allocator,
        inputs: ownership.Capture(T),
        owned_items: InputOwner,
        values: []const Input,
        rows: []Row,
        concurrency: ?usize,
        fn destroy(self: *@This()) void {
            self.allocator.free(self.rows);
            self.inputs.deinit();
            ownership.discard(&self.owned_items);
            self.allocator.destroy(self);
        }
    };
    const Owner = struct {
        pub const effect_owned_payload = true;
        storage: *Storage,
        pub fn deinit(self: *@This()) void {
            self.storage.destroy();
        }
    };
    const Adapter = struct {
        fn body(row: *const Storage.Row, current: *const R) P {
            const storage = row.storage;
            return roles.get("body", supplied)(&storage.inputs.value, &storage.values[row.index], current);
        }
        fn transform(row: Storage.Row) P {
            return P.deferBorrowed(Storage.Row, &row.storage.rows[row.index], body);
        }
        fn collection(owner: *const Owner, _: *const R) Out {
            const storage = owner.storage;
            if (comptime concurrent) return P.forEachConcurrent(Storage.Row, storage.concurrency.?, storage.rows, transform);
            return P.forEach(Storage.Row, storage.rows, transform);
        }
    };
    var inputs = ownership.Capture(T){ .value = capture };
    var owned_items = owned_values;
    if (comptime concurrent) if (concurrency.? == 0) {
        inputs.deinit();
        ownership.discard(&owned_items);
        return Out.die(.{ .kind = .invalid_configuration, .message = @import("../internal/diagnostics.zig").concurrency_must_be_positive });
    };
    if (comptime concurrent) {
        var permitted = ownership.concurrentValue(&capture);
        for (values) |*item| permitted = ownership.concurrentValue(item) and permitted;
        if (!permitted) {
            inputs.deinit();
            ownership.discard(&owned_items);
            return Out.die(.{ .kind = .invalid_configuration, .message = "forEachConcurrent single_thread retained capture cannot cross a concurrent worker boundary" });
        }
    }
    const storage = env.allocator.create(Storage) catch {
        inputs.deinit();
        ownership.discard(&owned_items);
        return Out.die(outcomes.Defect.out_of_memory);
    };
    const rows = env.allocator.alloc(Storage.Row, values.len) catch {
        inputs.deinit();
        ownership.discard(&owned_items);
        env.allocator.destroy(storage);
        return Out.die(outcomes.Defect.out_of_memory);
    };
    storage.* = .{ .allocator = env.allocator, .inputs = inputs, .owned_items = owned_items, .values = values, .rows = rows, .concurrency = concurrency };
    for (rows, 0..) |*row, index| row.* = .{ .storage = storage, .index = index };
    return @import("../dsl.zig").owned(env, Owner{ .storage = storage }, Adapter.collection);
}

/// Input pointers are borrows. The supplied collection outlives every run,
/// including suspended item bodies and their finalizers.
pub fn forEachCaptured(env: anytype, values: anytype, capture: anytype, comptime supplied: anytype) Collected(Body(@TypeOf(capture), Item(@TypeOf(values)), @typeInfo(@TypeOf(env)).pointer.child, supplied)) {
    return build(env, values, capture, @as(void, {}), null, false, supplied);
}
pub fn forEachConcurrentCaptured(env: anytype, values: anytype, capture: anytype, concurrency: usize, comptime supplied: anytype) Collected(Body(@TypeOf(capture), Item(@TypeOf(values)), @typeInfo(@TypeOf(env)).pointer.child, supplied)) {
    comptime ownership.requireConcurrent(@TypeOf(capture), "forEachConcurrent");
    comptime ownership.requireConcurrent(Item(@TypeOf(values)), "forEachConcurrent.item");
    return build(env, values, capture, @as(void, {}), concurrency, true, supplied);
}
pub fn forEachConcurrentCapturedStatic(env: anytype, values: anytype, capture: anytype, comptime concurrency: usize, comptime supplied: anytype) Collected(Body(@TypeOf(capture), Item(@TypeOf(values)), @typeInfo(@TypeOf(env)).pointer.child, supplied)) {
    if (concurrency == 0) @compileError("forEachConcurrent concurrency must be positive");
    return forEachConcurrentCaptured(env, values, capture, concurrency, supplied);
}

fn ownedItem(comptime T: type) type {
    if (@typeInfo(T) != .@"struct" or !@hasField(T, "items") or !outcomes.ownsPayload(T)) @compileError("owned collection requires an explicit owning items slice and public deinit evidence");
    ownership.validate(T);
    const Items = @FieldType(T, "items");
    if (@typeInfo(Items) != .pointer or @typeInfo(Items).pointer.size != .slice) @compileError("owned collection items must be a stable slice");
    return @typeInfo(Items).pointer.child;
}
pub fn forEachOwned(env: anytype, values: anytype, capture: anytype, comptime supplied: anytype) Collected(Body(@TypeOf(capture), ownedItem(@TypeOf(values)), @typeInfo(@TypeOf(env)).pointer.child, supplied)) {
    return build(env, values.items, capture, values, null, false, supplied);
}
pub fn forEachConcurrentOwned(env: anytype, values: anytype, capture: anytype, concurrency: usize, comptime supplied: anytype) Collected(Body(@TypeOf(capture), ownedItem(@TypeOf(values)), @typeInfo(@TypeOf(env)).pointer.child, supplied)) {
    comptime ownership.requireConcurrent(@TypeOf(capture), "forEachConcurrent");
    comptime ownership.requireConcurrent(ownedItem(@TypeOf(values)), "forEachConcurrent.item");
    return build(env, values.items, capture, values, concurrency, true, supplied);
}

fn Factory(comptime T: type, comptime R: type, comptime supplied: anytype) type {
    roles.validate(roles.recordType(supplied), "forEachFactory", &.{ "items", "body" }, &.{});
    const Acquire = inference.result(roles.get("items", supplied), &.{ *const T, *const R }, "forEachFactory.items");
    inference.requireEffect(Acquire, "forEachFactory.items");
    const Input = ownedItem(Acquire.Success);
    const P = Body(T, Input, R, .{ .body = roles.get("body", supplied) });
    if (Acquire.Environment != R or Acquire.Error != P.Error) @compileError("forEachFactory items/body E/R mismatch; adapt explicitly");
    return struct {
        pub const Items = Acquire.Success;
        pub const Out = Collected(P);
        fn acquire(stored: *const T, env: *const R) Acquire {
            return roles.get("items", supplied)(stored, env);
        }
        fn body(stored: *const T, items: Acquire.Success, env: *const R) Out {
            return forEachCaptured(env, items.items, stored, .{ .body = struct {
                fn run(pointer: *const *const T, item: *const Input, current: *const R) P {
                    return roles.get("body", supplied)(pointer.*, item, current);
                }
            }.run });
        }
        fn parallel(stored: *const T, items: Acquire.Success, env: *const R, concurrency: usize) Out {
            return forEachConcurrentCaptured(env, items.items, stored, concurrency, .{ .body = struct {
                fn run(pointer: *const *const T, item: *const Input, current: *const R) P {
                    return roles.get("body", supplied)(pointer.*, item, current);
                }
            }.run });
        }
        fn release(_: *const T, items: Acquire.Success, _: outcomes.PrimaryOutcome(P.Error), env: *const R) effects.Effect(void, P.Error, R) {
            const Unit = effects.Effect(void, P.Error, R);
            return Unit.fromOwned(env, items, struct {
                fn done(_: *const Acquire.Success, _: *const R) outcomes.Exit(void, P.Error) {
                    return .{ .success = {} };
                }
            }.done);
        }
    };
}
/// Each run acquires a fresh owning collection and brackets every item body
/// and cleanup before releasing it. There is no intermediate manual owner.
pub fn forEachFactory(env: anytype, capture: anytype, comptime supplied: anytype) Factory(@TypeOf(capture), @typeInfo(@TypeOf(env)).pointer.child, supplied).Out {
    const Spec = Factory(@TypeOf(capture), @typeInfo(@TypeOf(env)).pointer.child, supplied);
    return @import("resources.zig").bracket(env, capture, .{ .acquire = Spec.acquire, .body = Spec.body, .release = Spec.release });
}

pub fn forEachFactoryConcurrent(env: anytype, capture: anytype, concurrency: usize, comptime supplied: anytype) Factory(@TypeOf(capture), @typeInfo(@TypeOf(env)).pointer.child, supplied).Out {
    const T = @TypeOf(capture);
    const R = @typeInfo(@TypeOf(env)).pointer.child;
    const Spec = Factory(T, R, supplied);
    comptime ownership.requireConcurrent(T, "forEachFactoryConcurrent");
    if (concurrency == 0) {
        var abandoned = ownership.Capture(T){ .value = capture };
        abandoned.deinit();
        return Spec.Out.die(.{ .kind = .invalid_configuration, .message = @import("../internal/diagnostics.zig").concurrency_must_be_positive });
    }
    const Capture = struct { inputs: T, concurrency: usize };
    const Adapter = struct {
        fn acquire(stored: *const Capture, current: *const R) effects.Effect(Spec.Items, Spec.Out.Error, R) {
            return Spec.acquire(&stored.inputs, current);
        }
        fn body(stored: *const Capture, items: Spec.Items, current: *const R) Spec.Out {
            return Spec.parallel(&stored.inputs, items, current, stored.concurrency);
        }
        fn release(stored: *const Capture, items: Spec.Items, primary: outcomes.PrimaryOutcome(Spec.Out.Error), current: *const R) effects.Effect(void, Spec.Out.Error, R) {
            return Spec.release(&stored.inputs, items, primary, current);
        }
    };
    return @import("resources.zig").bracket(env, Capture{ .inputs = capture, .concurrency = concurrency }, .{ .acquire = Adapter.acquire, .body = Adapter.body, .release = Adapter.release });
}
