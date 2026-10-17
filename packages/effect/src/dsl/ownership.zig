//! Composite capture ownership follows declared payload evidence, never pointer
//! shape or disposal method names. Plain pointers remain explicit borrows.
const outcomes = @import("../outcome.zig");

// Description lifetime is distinct from success/error payload ownership.
// Recognize the exact existing Effect constructor before descending into its
// implementation fields; those fields are not an application capture record.
fn effectDescription(comptime T: type) bool {
    if (@typeInfo(T) != .@"struct" or !@hasDecl(T, "Success") or !@hasDecl(T, "Error") or !@hasDecl(T, "Environment")) return false;
    if (@TypeOf(T.Success) != type or @TypeOf(T.Error) != type or @TypeOf(T.Environment) != type) return false;
    return T == @import("../effect.zig").Effect(T.Success, T.Error, T.Environment);
}

fn borrowedResourceValue(comptime T: type) bool {
    if (@typeInfo(T) == .@"struct" and @hasDecl(T, "AlreadyOwnedResource")) {
        const A = T.AlreadyOwnedResource;
        if (@TypeOf(A) != type) @compileError("capture rejects forged ResourceValue borrowing evidence");
        if (T != @import("../effect.zig").ResourceValue(A)) @compileError("capture rejects forged ResourceValue borrowing evidence");
        return true;
    }
    return false;
}

pub fn validate(comptime T: type) void {
    if (comptime borrowedResourceValue(T)) return;
    if (comptime effectDescription(T)) return;
    if (comptime !containsOwned(T)) return;
    if (outcomes.ownsPayload(T)) {
        if (@typeInfo(T) == .optional) return validate(@typeInfo(T).optional.child);
        if (!@hasDecl(T, "deinit") or @TypeOf(T.deinit) != fn (*T) void)
            @compileError("capture ownership requires public deinit(*T) void evidence; actual " ++ @typeName(T));
    } else switch (@typeInfo(T)) {
        .@"struct" => |s| inline for (s.field_types) |F| validate(F),
        .optional => |o| validate(o.child),
        .array => |a| validate(a.child),
        .@"union" => |u| {
            if (u.tag_type == null) @compileError("capture ownership requires a tagged union");
            inline for (u.field_types) |F| validate(F);
        },
        else => {},
    }
}

pub fn discard(value: anytype) void {
    const T = @typeInfo(@TypeOf(value)).pointer.child;
    if (comptime borrowedResourceValue(T)) return;
    if (comptime effectDescription(T)) return value.deinit();
    if (comptime !containsOwned(T)) return;
    if (comptime outcomes.ownsPayload(T)) return outcomes.discardPayload(value);
    switch (@typeInfo(T)) {
        .@"struct" => |s| inline for (0..s.field_names.len) |index| {
            const reversed = s.field_names.len - index - 1;
            if (!s.field_attrs[reversed].@"comptime") discard(&@field(value.*, s.field_names[reversed]));
        },
        .optional => if (value.*) |*v| discard(v),
        .array => |a| inline for (0..a.len) |i| discard(&value.*[a.len - i - 1]),
        .@"union" => switch (value.*) {
            inline else => |*v| discard(v),
        },
        else => {},
    }
}

pub fn Capture(comptime T: type) type {
    comptime validate(T);
    return CaptureType(T);
}

// Identity inspection must not validate unrelated custom-owner internals.
// Keep construction validation at Capture(), and compare with this exact
// private constructor before treating a record as generated storage.
fn CaptureType(comptime T: type) type {
    return struct {
        pub const effect_owned_payload = true;
        value: T,
        pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
            return .{ .value = try cloneValue(self.value) };
        }
        pub fn deinit(self: *@This()) void {
            discard(&self.value);
        }
    };
}

pub fn capturedType(comptime T: type) ?type {
    if (@typeInfo(T) != .@"struct" or @typeInfo(T).@"struct".field_names.len != 1 or !@hasField(T, "value")) return null;
    const Inner = @FieldType(T, "value");
    return if (T == CaptureType(Inner)) Inner else null;
}

/// Derive composite ownership only from member contracts. Pointers alone are
/// borrows and do not cause ownership, cloning, or release to be inferred.
pub fn containsOwned(comptime T: type) bool {
    // A directional codec can inspect the same representation several times.
    // Cache structural evidence by its exact type, rather than rewalking it.
    const Shape = struct {
        const contains = ownedMembers(T);
    };
    return Shape.contains;
}

fn ownedMembers(comptime T: type) bool {
    if (borrowedResourceValue(T)) return false;
    if (effectDescription(T)) return true;
    if (outcomes.ownsPayload(T)) return true;
    return switch (@typeInfo(T)) {
        .@"struct" => |s| blk: {
            for (s.field_types) |F| if (containsOwned(F)) break :blk true;
            break :blk false;
        },
        .@"union" => |u| blk: {
            for (u.field_types) |F| if (containsOwned(F)) break :blk true;
            break :blk false;
        },
        .optional => |o| containsOwned(o.child),
        .array => |a| containsOwned(a.child),
        else => false,
    };
}
pub fn cloneValue(value: anytype) error{OutOfMemory}!@TypeOf(value) {
    const T = @TypeOf(value);
    if (comptime borrowedResourceValue(T)) return value;
    if (comptime effectDescription(T)) {
        if (value.ownership) |owner| owner.retain();
        return value;
    }
    if (comptime !containsOwned(T)) return value;
    if (comptime outcomes.ownsPayload(T)) return outcomes.clonePayload(value);
    switch (@typeInfo(T)) {
        .@"struct" => |s| {
            var copied: T = undefined;
            var initialized: [s.field_names.len]bool = @splat(false);
            errdefer inline for (0..s.field_names.len) |i| {
                const index = s.field_names.len - i - 1;
                if (initialized[index]) discard(&@field(copied, s.field_names[index]));
            };
            inline for (s.field_names, 0..) |name, index| {
                if (s.field_attrs[index].@"comptime") continue;
                @field(copied, name) = try cloneValue(@field(value, name));
                initialized[index] = true;
            }
            return copied;
        },
        .optional => return if (value) |payload| try cloneValue(payload) else null,
        .@"union" => return switch (value) {
            inline else => |payload, tag| @unionInit(T, @tagName(tag), try cloneValue(payload)),
        },
        .array => {
            var copied: T = undefined;
            var initialized: usize = 0;
            errdefer for (copied[0..initialized]) |*item| discard(item);
            for (value, 0..) |item, index| {
                copied[index] = try cloneValue(item);
                initialized += 1;
            }
            return copied;
        },
        else => return value,
    }
}
/// A generated ownership adapter is needed only when the outer application
/// record lacks its own contract but its members already declare ownership.
pub fn Moved(comptime T: type) type {
    return if (!outcomes.ownsPayload(T) and containsOwned(T)) Capture(T) else T;
}
pub fn move(value: anytype) Moved(@TypeOf(value)) {
    return if (comptime Moved(@TypeOf(value)) == @TypeOf(value)) value else .{ .value = value };
}

pub const Synchronization = enum { single_thread, thread_safe };
/// Select retained/shared ownership explicitly. The supplied roles constitute
/// the ownership evidence; pointee shape and similarly named methods do not.
pub fn retained(owner: anytype, comptime operations: anytype) Retained(@TypeOf(owner), operations) {
    return .{ .value = operations.retain(owner) };
}
fn Retained(comptime T: type, comptime operations: anytype) type {
    const roles = @import("roles.zig");
    roles.validate(@TypeOf(operations), "retained capture", &.{ "retain", "release", "synchronization" }, &.{});
    if (@TypeOf(operations.retain) != fn (T) T or @TypeOf(operations.release) != fn (*T) void) @compileError("retained capture requires retain(T) T and release(*T) void evidence");
    if (@TypeOf(operations.synchronization) != Synchronization) @compileError("retained capture requires an explicit Synchronization contract");
    const contract = operations;
    return struct {
        pub const effect_owned_payload = true;
        pub const synchronization = operations.synchronization;
        pub const retained_shape = struct {
            pub const owner_type = T;
            pub const retention_contract = contract;
        };
        value: T,
        pub fn deinit(self: *@This()) void {
            operations.release(&self.value);
        }
        pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
            return .{ .value = operations.retain(self.value) };
        }
    };
}

/// Reject only explicitly declared shared modes that cannot cross this
/// boundary. Ordinary pointers and uniquely moved payloads are not classified
/// by guessed pointee ownership or a blanket cloning requirement.
pub fn requireConcurrent(comptime T: type, comptime operation: []const u8) void {
    if (@import("../internal/capture_owner.zig").concurrencyShape(T)) |Shape| {
        requireConcurrent(Shape.Configuration, operation);
        if (Shape.Payload) |Payload| requireConcurrent(Payload, operation);
        return;
    }
    if (@import("owned_codecs.zig").concurrencyShape(T)) |Shape| {
        requireConcurrent(Shape.Configuration, operation);
        if (Shape.Payload) |Payload| requireConcurrent(Payload, operation);
        return;
    }
    if (@typeInfo(T) == .@"struct" and @hasDecl(T, "retained_shape")) {
        const Shape = T.retained_shape;
        if (T != Retained(Shape.owner_type, Shape.retention_contract)) @compileError(operation ++ " rejects forged retained ownership evidence");
        if (T.synchronization != .thread_safe) @compileError(operation ++ " single_thread retained capture cannot cross a concurrent worker boundary; select validated thread_safe retention");
        return;
    }
    switch (@typeInfo(T)) {
        .@"struct" => |s| inline for (s.field_types) |F| requireConcurrent(F, operation),
        .@"union" => |u| inline for (u.field_types) |F| requireConcurrent(F, operation),
        .optional => |o| requireConcurrent(o.child, operation),
        .array => |a| requireConcurrent(a.child, operation),
        else => {},
    }
}

/// Inspect known generated owners and erased Effects before concurrent
/// admission. Native pointers remain borrows; their pointees are not guessed.
pub fn concurrentValue(value: anytype) bool {
    const T = @typeInfo(@TypeOf(value)).pointer.child;
    if (comptime effectDescription(T)) return value.concurrentSafe();
    if (comptime @import("../internal/capture_owner.zig").concurrencyShape(T)) |Shape| {
        if (comptime Shape.Payload != null) return concurrentValue(&value.value) and concurrentValue(&value.owner);
        return concurrentValue(value.borrow());
    }
    if (comptime @import("owned_codecs.zig").concurrencyShape(T)) |Shape| return concurrentType(Shape.Configuration) and (if (Shape.Payload) |Payload| concurrentType(Payload) else true);
    if (comptime @typeInfo(T) == .@"struct" and @hasDecl(T, "retained_shape")) {
        const Shape = T.retained_shape;
        if (T != Retained(Shape.owner_type, Shape.retention_contract)) @compileError("concurrent admission rejects forged retention evidence");
        return T.synchronization == .thread_safe;
    }
    switch (@typeInfo(T)) {
        .@"struct" => |info| inline for (info.field_names, info.field_attrs) |field, attributes| {
            if (!attributes.@"comptime" and !concurrentValue(&@field(value.*, field))) return false;
        },
        .@"union" => |info| if (info.tag_type != null) {
            switch (value.*) {
                inline else => |*payload| return concurrentValue(payload),
            }
        },
        .optional => if (value.*) |*payload| return concurrentValue(payload),
        .array => for (value) |*item| {
            if (!concurrentValue(item)) return false;
        },
        else => {},
    }
    return true;
}
fn concurrentType(comptime T: type) bool {
    if (effectDescription(T)) return true; // Runtime description metadata decides.
    if (@import("../internal/capture_owner.zig").concurrencyShape(T)) |Shape| return concurrentType(Shape.Configuration) and (if (Shape.Payload) |Payload| concurrentType(Payload) else true);
    if (@typeInfo(T) == .@"struct" and @hasDecl(T, "retained_shape")) return T.synchronization == .thread_safe;
    switch (@typeInfo(T)) {
        .@"struct" => |info| inline for (info.field_types) |Field| {
            if (!concurrentType(Field)) return false;
        },
        .@"union" => |info| inline for (info.field_types) |Field| {
            if (!concurrentType(Field)) return false;
        },
        .optional => |info| return concurrentType(info.child),
        .array => |info| return concurrentType(info.child),
        else => {},
    }
    return true;
}
