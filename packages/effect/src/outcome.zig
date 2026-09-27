//! Typed terminal outcomes. No operation here introduces a second error channel.
const std = @import("std");
const diagnostics = @import("internal/diagnostics.zig");

pub const Defect = struct {
    kind: Kind,
    message: []const u8,

    pub const Kind = enum { explicit, out_of_memory, native_failure, provider_unavailable, invalid_configuration };

    pub fn init(message: []const u8) Defect {
        return .{ .kind = .explicit, .message = message };
    }

    pub const out_of_memory: Defect = .{ .kind = .out_of_memory, .message = diagnostics.allocation_failed };
};

pub fn Result(comptime A: type, comptime E: type) type {
    return union(enum) {
        pub const effect_owned_payload = ownsPayload(A) or ownsPayload(E);
        ok: A,
        err: E,

        pub fn deinit(self: *@This()) void {
            switch (self.*) {
                .ok => |*a| discardPayload(a),
                .err => |*e| discardPayload(e),
            }
            self.* = undefined;
        }
        pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
            return switch (self.*) {
                .ok => |a| .{ .ok = try clonePayload(a) },
                .err => |e| .{ .err = try clonePayload(e) },
            };
        }
    };
}

pub fn PrimaryOutcome(comptime E: type) type {
    return union(enum) { succeeded, failed: E, defected: Defect, was_interrupted };
}

pub fn CleanupFailure(comptime E: type) type {
    return union(enum) { release_failure: E, release_defect: Defect, release_interrupted };
}

const PayloadDeclaration = enum { effect_owned_payload };

pub fn ownsPayload(comptime T: type) bool {
    return switch (@typeInfo(T)) {
        .optional => |optional| ownsPayload(optional.child),
        .@"struct", .@"union" => @hasDecl(T, @tagName(PayloadDeclaration.effect_owned_payload)) and T.effect_owned_payload,
        else => false,
    };
}

/// Library-generated owners and consumer owners can declare this comptime
/// contract. Unmarked values keep their normal Zig borrowing/ownership rules.
pub fn discardPayload(value: anytype) void {
    const T = @typeInfo(@TypeOf(value)).pointer.child;
    if (comptime @typeInfo(T) == .optional) {
        if (value.*) |*payload| discardPayload(payload);
    } else if (comptime ownsPayload(T)) {
        if (@TypeOf(T.deinit) != fn (*T) void) @compileError(diagnostics.owned_effect_payload_deinit_must_accept_self_and_return_void);
        value.deinit();
    }
}

pub fn clonePayload(value: anytype) error{OutOfMemory}!@TypeOf(value) {
    const T = @TypeOf(value);
    if (comptime @typeInfo(T) == .optional) return if (value) |payload| try clonePayload(payload) else null;
    if (comptime ownsPayload(T)) {
        if (comptime @hasDecl(T, "Success") and @hasDecl(T, "Error")) {
            if (comptime T == Exit(T.Success, T.Error)) {
                const copy = value.clone();
                if (copy == .die and copy.die.kind == .out_of_memory and !(value == .die and value.die.kind == .out_of_memory)) return error.OutOfMemory;
                return copy;
            }
        }
        if (@TypeOf(T.clone) != fn (*const T) error{OutOfMemory}!T) @compileError(diagnostics.owned_effect_payload_clone_must_accept_const_self_and_return_error_outofmemory);
        return value.clone();
    }
    return value;
}

fn discardPrimary(comptime E: type, primary: *PrimaryOutcome(E)) void {
    if (primary.* == .failed) discardPayload(&primary.failed);
}

fn discardCleanup(comptime E: type, failure: *CleanupFailure(E)) void {
    if (failure.* == .release_failure) discardPayload(&failure.release_failure);
}

fn clonePrimary(comptime E: type, primary: PrimaryOutcome(E)) error{OutOfMemory}!PrimaryOutcome(E) {
    return switch (primary) {
        .failed => |err| .{ .failed = try clonePayload(err) },
        else => primary,
    };
}

fn cloneCleanup(comptime E: type, failure: CleanupFailure(E)) error{OutOfMemory}!CleanupFailure(E) {
    return switch (failure) {
        .release_failure => |err| .{ .release_failure = try clonePayload(err) },
        else => failure,
    };
}

// Retained cleanup metadata owns its nodes. Each node remembers the allocation
// domain in which it was created; changing an environment cannot change it.
pub fn CleanupNode(comptime E: type) type {
    return struct {
        const Self = @This();
        failure: CleanupFailure(E),
        next: ?*Self = null,
        allocator: std.mem.Allocator,
        owner: ?*anyopaque = null,
        release_owner: ?*const fn (*anyopaque) void = null,
        references: std.atomic.Value(usize) = .init(1),

        pub fn destroy(self: *Self) void {
            if (self.references.fetchSub(1, .acq_rel) != 1) return;
            if (self.release_owner) |release| release(self.owner.?) else self.allocator.destroy(self);
        }
    };
}

pub fn Finalization(comptime E: type) type {
    return struct {
        primary: PrimaryOutcome(E),
        first: CleanupFailure(E),
        rest: ?*CleanupNode(E) = null,
        tail: ?*CleanupNode(E) = null,

        pub fn iterator(self: *const @This()) Iterator {
            return .{ .first = self.first, .next = self.rest };
        }

        pub const Iterator = struct {
            first: ?CleanupFailure(E),
            next: ?*const CleanupNode(E),

            pub fn nextFailure(self: *Iterator) ?CleanupFailure(E) {
                if (self.first) |failure| {
                    self.first = null;
                    return failure;
                }
                const node = self.next orelse return null;
                self.next = node.next;
                return node.failure;
            }
        };
    };
}

pub fn Exit(comptime A: type, comptime E: type) type {
    // A byte-sized discriminant avoids sub-byte tag load/store conversions.
    // This allows the optimizer to eliminate known outcome construction.
    return union(enum(u8)) {
        const Self = @This();
        pub const Success = A;
        pub const Error = E;
        pub const effect_owned_payload = true;
        success: A,
        fail: E,
        interrupted,
        die: Defect,
        finalization_failed: Finalization(E),

        pub fn primary(self: Self) PrimaryOutcome(E) {
            return switch (self) {
                .success => .succeeded,
                .fail => |err| .{ .failed = err },
                .interrupted => .was_interrupted,
                .die => |defect| .{ .defected = defect },
                .finalization_failed => |details| details.primary,
            };
        }

        /// Copies owned cleanup links through their originating allocation
        /// domains. Declared owning payloads use their clone contract; other
        /// payloads remain borrowed copies. Independent owners can append
        /// cleanup without mutating each other.
        /// Allocation failure returns a typed defect and preserves the source.
        pub fn clone(self: Self) Self {
            var copy: Self = switch (self) {
                .success => |value| .{ .success = clonePayload(value) catch return .{ .die = Defect.out_of_memory } },
                .fail => |err| .{ .fail = clonePayload(err) catch return .{ .die = Defect.out_of_memory } },
                .die, .interrupted => self,
                .finalization_failed => |details| blk: {
                    var primary_value = clonePrimary(E, details.primary) catch return .{ .die = Defect.out_of_memory };
                    const first = cloneCleanup(E, details.first) catch {
                        discardPrimary(E, &primary_value);
                        return .{ .die = Defect.out_of_memory };
                    };
                    break :blk .{ .finalization_failed = .{ .primary = primary_value, .first = first } };
                },
            };
            if (self == .finalization_failed) {
                var next = self.finalization_failed.rest;
                while (next) |node| {
                    const cloned = node.allocator.create(CleanupNode(E)) catch {
                        copy.deinit();
                        return .{ .die = Defect.out_of_memory };
                    };
                    const failure = cloneCleanup(E, node.failure) catch {
                        node.allocator.destroy(cloned);
                        copy.deinit();
                        return .{ .die = Defect.out_of_memory };
                    };
                    cloned.* = .{ .failure = failure, .allocator = node.allocator };
                    appendChain(E, &copy.finalization_failed, cloned, cloned);
                    next = node.next;
                }
            }
            return copy;
        }

        pub const Observation = struct { exit: Self, copied: bool };

        /// A failed copy is an observer defect, not observation of the source.
        /// Controllers retain the original diagnostics until a real observer
        /// receives them or their owning scope closes.
        pub fn observe(self: Self) Observation {
            const copy = self.clone();
            const failed = copy == .die and copy.die.kind == .out_of_memory and
                !(self == .die and self.die.kind == .out_of_memory);
            return .{ .exit = copy, .copied = !failed };
        }

        /// Releases cleanup metadata and declared owning success/error payloads.
        /// Unmarked payloads retain their ordinary Zig ownership contract.
        /// An Exit owning metadata must be moved, not independently deinitialized
        /// through several copies. After this call its fields must not be read.
        pub fn deinit(self: *Self) void {
            if (self.* == .success) discardPayload(&self.success);
            if (self.* == .fail) discardPayload(&self.fail);
            if (self.* == .finalization_failed) {
                discardPrimary(E, &self.finalization_failed.primary);
                discardCleanup(E, &self.finalization_failed.first);
                var next = self.finalization_failed.rest;
                while (next) |node| {
                    next = node.next;
                    discardCleanup(E, &node.failure);
                    node.destroy();
                }
            }
            self.* = undefined;
        }
    };
}

pub fn castFailure(comptime B: type, value: anytype) Exit(B, @TypeOf(value).Error) {
    return switch (value) {
        .success => unreachable,
        .fail => |err| .{ .fail = err },
        .die => |defect| .{ .die = defect },
        .interrupted => .interrupted,
        .finalization_failed => |details| .{ .finalization_failed = details },
    };
}

/// Owns the array storage only. Element ownership follows the producing
/// operation's contract; resources belong in scopes rather than raw arrays.
pub fn OwnedSlice(comptime T: type) type {
    return struct {
        pub const effect_owned_payload = true;
        items: []T,
        allocator: std.mem.Allocator,

        pub fn deinit(self: *@This()) void {
            for (self.items) |*item| discardPayload(item);
            self.allocator.free(self.items);
            self.* = undefined;
        }

        pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
            const items = try self.allocator.alloc(T, self.items.len);
            var copied: usize = 0;
            errdefer {
                for (items[0..copied]) |*item| discardPayload(item);
                self.allocator.free(items);
            }
            for (self.items, items) |value, *target| {
                target.* = try clonePayload(value);
                copied += 1;
            }
            return .{ .items = items, .allocator = self.allocator };
        }
    };
}

pub fn Pair(comptime A: type, comptime B: type) type {
    return struct {
        pub const effect_owned_payload = ownsPayload(A) or ownsPayload(B);
        left: A,
        right: B,

        pub fn deinit(self: *@This()) void {
            discardPayload(&self.left);
            discardPayload(&self.right);
            self.* = undefined;
        }

        pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
            var left = try clonePayload(self.left);
            errdefer discardPayload(&left);
            return .{ .left = left, .right = try clonePayload(self.right) };
        }
    };
}

pub fn mapError(comptime F: type, value: anytype, allocator: ?std.mem.Allocator, comptime transform: fn (@TypeOf(value).Error) F) Exit(@TypeOf(value).Success, F) {
    const A = @TypeOf(value).Success;
    const Mapping = struct {
        fn primary(source: PrimaryOutcome(@TypeOf(value).Error)) PrimaryOutcome(F) {
            return switch (source) {
                .succeeded => .succeeded,
                .failed => |err| .{ .failed = transform(err) },
                .defected => |defect| .{ .defected = defect },
                .was_interrupted => .was_interrupted,
            };
        }

        fn cleanup(source: CleanupFailure(@TypeOf(value).Error)) CleanupFailure(F) {
            return switch (source) {
                .release_failure => |err| .{ .release_failure = transform(err) },
                .release_defect => |defect| .{ .release_defect = defect },
                .release_interrupted => .release_interrupted,
            };
        }
    };
    return switch (value) {
        .success => |result| .{ .success = result },
        .fail => |err| .{ .fail = transform(err) },
        .die => |defect| .{ .die = defect },
        .interrupted => .interrupted,
        .finalization_failed => |details| blk: {
            var mapped: Exit(A, F) = .{ .finalization_failed = .{
                .primary = Mapping.primary(details.primary),
                .first = Mapping.cleanup(details.first),
            } };
            var remaining = details.rest;
            var failed = false;
            while (remaining) |node| {
                remaining = node.next;
                // Conversion is eager, including when metadata allocation has
                // failed. Never defer application mapper calls to iteration.
                var failure = Mapping.cleanup(node.failure);
                var transferred = false;
                if (!failed) {
                    const allocated = if (allocator) |a| a.create(CleanupNode(F)) catch null else null;
                    if (allocated) |target| {
                        target.* = .{ .failure = failure, .allocator = allocator.? };
                        if (mapped.finalization_failed.tail) |tail| tail.next = target else mapped.finalization_failed.rest = target;
                        mapped.finalization_failed.tail = target;
                        transferred = true;
                    } else failed = true;
                }
                if (!transferred) discardCleanup(F, &failure);
                node.destroy();
            }
            if (failed) {
                mapped.deinit();
                // All source metadata has been consumed. Allocation failure is
                // an explicit defect, never an unchecked native error union.
                break :blk .{ .die = Defect.out_of_memory };
            }
            break :blk mapped;
        },
    };
}

/// Storage reserved before acquiring work. A structured cleanup can contribute
/// its primary failure and its first record; its remaining links already exist.
pub fn CleanupStorage(comptime E: type) type {
    return struct {
        const Prefix = std.meta.Tuple(&.{ PrimaryOutcome(E), CleanupFailure(E) });
        nodes: [@typeInfo(Prefix).@"struct".fields.len]CleanupNode(E) = undefined,
        references: std.atomic.Value(usize) = .init(0),
        owner: *anyopaque,
        release: *const fn (*anyopaque) void,
        allocator: std.mem.Allocator,

        fn releaseNode(data: *anyopaque) void {
            const self: *@This() = @ptrCast(@alignCast(data));
            if (self.references.fetchSub(1, .acq_rel) == 1) self.release(self.owner);
        }

        pub fn append(self: *@This(), target: anytype, failure: CleanupFailure(E)) void {
            if (target.* != .finalization_failed) {
                const primary = target.primary();
                if (target.* == .success) discardPayload(&target.success);
                target.* = .{ .finalization_failed = .{ .primary = primary, .first = failure } };
                return;
            }
            const index = self.references.fetchAdd(1, .monotonic);
            const node = &self.nodes[index];
            node.* = .{ .failure = failure, .allocator = self.allocator, .owner = self, .release_owner = releaseNode };
            appendChain(E, &target.finalization_failed, node, node);
        }

        pub fn appendExit(self: *@This(), target: anytype, cleanup: Exit(void, E)) void {
            switch (cleanup) {
                .success => {},
                .fail => |err| self.append(target, .{ .release_failure = err }),
                .die => |defect| self.append(target, .{ .release_defect = defect }),
                .interrupted => self.append(target, .release_interrupted),
                .finalization_failed => |details| {
                    switch (details.primary) {
                        .succeeded => {},
                        .failed => |err| self.append(target, .{ .release_failure = err }),
                        .defected => |defect| self.append(target, .{ .release_defect = defect }),
                        .was_interrupted => self.append(target, .release_interrupted),
                    }
                    self.append(target, details.first);
                    appendChain(E, &target.finalization_failed, details.rest, details.tail);
                },
            }
        }

        pub fn appendFailures(self: *@This(), target: anytype, source: anytype) void {
            if (source == .fail) {
                var discarded = source.fail;
                discardPayload(&discarded);
            }
            if (source == .finalization_failed) {
                var primary = source.finalization_failed.primary;
                discardPrimary(E, &primary);
                self.append(target, source.finalization_failed.first);
                appendChain(E, &target.finalization_failed, source.finalization_failed.rest, source.finalization_failed.tail);
            }
        }

        pub fn retained(self: *const @This()) bool {
            return self.references.load(.acquire) != 0;
        }
    };
}

pub fn appendChain(comptime E: type, details: *Finalization(E), head: ?*CleanupNode(E), tail: ?*CleanupNode(E)) void {
    if (head == null) return;
    if (details.rest) |rest| {
        var last = details.tail orelse rest;
        while (last.next) |next| last = next;
        last.next = head;
    } else details.rest = head;
    details.tail = tail;
}

test "five channels preserve rich errors and nonempty cleanup" {
    const E = union(enum) { rejected: struct { field: []const u8, code: u16 } };
    const value: Exit(u32, E) = .{ .fail = .{ .rejected = .{ .field = "port", .code = 9 } } };
    try std.testing.expectEqual(@as(u16, 9), value.fail.rejected.code);
    const converted = castFailure(void, value);
    try std.testing.expectEqualStrings("port", converted.fail.rejected.field);
    const cleanup: Exit(u32, E) = .{ .finalization_failed = .{
        .primary = value.primary(),
        .first = .release_interrupted,
    } };
    try std.testing.expect(cleanup.primary() == .failed);
    var it = cleanup.finalization_failed.iterator();
    try std.testing.expect(it.nextFailure().? == .release_interrupted);
    try std.testing.expect(it.nextFailure() == null);
}

test "metadata destruction is iterative and uses originating allocators" {
    const E = u32;
    var head: ?*CleanupNode(E) = null;
    for (0..100_000) |i| {
        const node = try std.testing.allocator.create(CleanupNode(E));
        node.* = .{ .failure = .{ .release_failure = @intCast(i) }, .next = head, .allocator = std.testing.allocator };
        head = node;
    }
    var value: Exit(void, E) = .{ .finalization_failed = .{
        .primary = .succeeded,
        .first = .release_interrupted,
        .rest = head,
    } };
    value.deinit();
}
