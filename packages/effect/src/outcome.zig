//! Typed terminal outcomes. No operation here introduces a second error channel.
const std = @import("std");
const diagnostics = @import("internal/diagnostics.zig");

/// Unexpected failure reported by `Exit.die`. It is distinct from the
/// application error type `E`; allocation failure uses `out_of_memory`.
pub const Defect = struct {
    kind: Kind,
    message: []const u8,

    /// `Kind` cases: `explicit`, `out_of_memory`, `native_failure`, `provider_unavailable`,
    /// `invalid_configuration`.
    pub const Kind = enum { explicit, out_of_memory, native_failure, provider_unavailable, invalid_configuration };

    /// Create an explicit defect with a caller-owned diagnostic message.
    /// Parameters:
    /// - `message: []const u8`: borrowed text; keep it alive while any resulting
    ///   `Exit.die` may be inspected. This constructor does not copy it.
    /// Returns (`Defect`): defect with `kind = .explicit` and the given message.
    pub fn init(message: []const u8) Defect {
        return .{ .kind = .explicit, .message = message };
    }

    /// Canonical Defect for allocation failure.
    pub const out_of_memory: Defect = .{ .kind = .out_of_memory, .message = diagnostics.allocation_failed };
};

/// Two-channel result for pure operations. Use `Exit` for effect execution,
/// where interruption, defects, and failed finalization must remain visible.
///
/// Parameters:
/// - `A: comptime type`: Value type produced by a successful computation.
/// - `E: comptime type`: Expected application-failure type; defects and interruption remain separate outcomes.
/// Returns (`type`): `ok: A` or `err: E` union for pure code; use `Exit`
/// for work that can also be interrupted or fail during cleanup.
pub fn Result(comptime A: type, comptime E: type) type {
    return union(enum) {
        /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements `deinit`
        /// and must be released after use.
        pub const effect_owned_payload = ownsPayload(A) or ownsPayload(E);
        ok: A,
        err: E,

        /// Release the active payload only when it declares the Effect-owned
        /// payload contract, then invalidate this result.
        /// Parameters:
        /// - `self: *@This()`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// Returns (`void`): no value; the receiver is invalid afterward.
        pub fn deinit(self: *@This()) void {
            switch (self.*) {
                .ok => |*a| discardPayload(a),
                .err => |*e| discardPayload(e),
            }
            self.* = undefined;
        }
        /// Clone the active payload when it owns storage; otherwise copy it by value.
        /// Parameters:
        /// - `self: *const @This()`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
        /// Returns (`error{OutOfMemory}!@This()`): independent result or
        /// `OutOfMemory` if cloning an owned payload needs storage and fails.
        pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
            return switch (self.*) {
                .ok => |a| .{ .ok = try clonePayload(a) },
                .err => |e| .{ .err = try clonePayload(e) },
            };
        }
    };
}

/// Describe the outcome that existed before finalizer failures were attached.
/// Parameters:
/// - `E: comptime type`: Expected application-failure type; defects and interruption remain separate outcomes.
/// Returns (`type`): union distinguishing success, typed `E` failure, defect,
/// and interruption without erasing that primary result during cleanup.
pub fn PrimaryOutcome(comptime E: type) type {
    return union(enum) { succeeded, failed: E, defected: Defect, was_interrupted };
}

/// Describe one failure from a resource finalizer.
/// Parameters:
/// - `E: comptime type`: Expected application-failure type; defects and interruption remain separate outcomes.
/// Returns (`type`): union holding a release-time `E`, defect, or interruption.
pub fn CleanupFailure(comptime E: type) type {
    return union(enum) { release_failure: E, release_defect: Defect, release_interrupted };
}

const PayloadDeclaration = enum { effect_owned_payload };

/// Check whether a type declares the Effect-owned payload contract.
/// Parameters:
/// - `T: comptime type`: type to inspect at compilation, including optional payloads.
/// Returns (`bool`): `true` if `T` declares `effect_owned_payload = true`.
pub fn ownsPayload(comptime T: type) bool {
    return switch (@typeInfo(T)) {
        .optional => |optional| ownsPayload(optional.child),
        .@"struct", .@"union" => @hasDecl(T, @tagName(PayloadDeclaration.effect_owned_payload)) and T.effect_owned_payload,
        else => false,
    };
}

/// Explicit witness for imported owning application payloads.
pub fn requireOwned(comptime T: type) type {
    if (!ownsPayload(T) or !@hasDecl(T, "clone") or !@hasDecl(T, "deinit")) @compileError("owned payload requires public ownership, clone and deinit evidence");
    return T;
}

/// Library-generated owners and consumer owners can declare this comptime
/// contract. Unmarked values keep their normal Zig borrowing/ownership rules.
///
/// Parameters:
/// - `value: anytype`: mutable pointer to a payload. An optional payload is
///   unwrapped; an owning type must expose `deinit(*T) void`.
/// Returns (`void`): no value; calls `deinit` only for marked owned payloads.
pub fn discardPayload(value: anytype) void {
    const T = @typeInfo(@TypeOf(value)).pointer.child;
    if (comptime @typeInfo(T) == .optional) {
        if (value.*) |*payload| discardPayload(payload);
    } else if (comptime ownsPayload(T)) {
        if (@TypeOf(T.deinit) != fn (*T) void) @compileError(diagnostics.owned_effect_payload_deinit_must_accept_self_and_return_void);
        value.deinit();
    }
}

/// Clone a payload according to its declared ownership contract.
/// Parameters:
/// - `value: anytype`: value to duplicate according to its ownership marker;
///   unmarked values are copied and optionals recurse into their payload.
/// Returns (`error{OutOfMemory}!@TypeOf(value)`): copied or independently
/// cloned value; allocation failure is explicit for owning payloads.
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
/// Store a cleanup failure in a reference-counted chain node. Each node
/// remembers the allocator or external owner that must release its storage.
/// Parameters:
/// - `E: comptime type`: Expected application-failure type; defects and interruption remain separate outcomes.
/// Returns (`type`): node type carrying one `CleanupFailure(E)` and its successor.
pub fn CleanupNode(comptime E: type) type {
    return struct {
        const Self = @This();
        failure: CleanupFailure(E),
        next: ?*Self = null,
        allocator: std.mem.Allocator,
        owner: ?*anyopaque = null,
        release_owner: ?*const fn (*anyopaque) void = null,
        references: std.atomic.Value(usize) = .init(1),

        /// Release this owned node and its payload.
        /// Parameters:
        /// - `self: *Self`: node whose reference count is decremented once.
        /// Returns (`void`): no value; the last owner releases node storage.
        pub fn destroy(self: *Self) void {
            if (self.references.fetchSub(1, .acq_rel) != 1) return;
            if (self.release_owner) |release| release(self.owner.?) else self.allocator.destroy(self);
        }
    };
}

/// Retain the primary result together with ordered finalizer failures.
/// Parameters:
/// - `E: comptime type`: Expected application-failure type; defects and interruption remain separate outcomes.
/// Returns (`type`): failure record whose `first` entry is inline and whose
/// remaining entries form an owned `CleanupNode(E)` chain.
pub fn Finalization(comptime E: type) type {
    return struct {
        primary: PrimaryOutcome(E),
        first: CleanupFailure(E),
        rest: ?*CleanupNode(E) = null,
        tail: ?*CleanupNode(E) = null,

        /// Create an iterator over the retained entries.
        /// Parameters:
        /// - `self: *const @This()`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
        /// Returns (`Iterator`): An iterator borrowing the collection it traverses.
        pub fn iterator(self: *const @This()) Iterator {
            return .{ .first = self.first, .next = self.rest };
        }

        /// Borrowed cursor over this finalization's failures, starting with
        /// `first` and then following `next`. Keep its owner alive while iterating.
        pub const Iterator = struct {
            first: ?CleanupFailure(E),
            next: ?*const CleanupNode(E),

            /// Advance to the next finalizer failure.
            /// Parameters:
            /// - `self: *Iterator`: cursor advanced past each returned failure.
            /// Returns (`?CleanupFailure(E)`): next borrowed failure value,
            /// or `null` after the chain is exhausted.
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

/// Terminal outcome of `Effect(A, E, R)`: success, expected failure, defect,
/// interruption, or failed finalization retaining the primary outcome.
/// Owning payloads and cleanup metadata must be released with `deinit`.
///
/// Parameters:
/// - `A: comptime type`: Value type produced by a successful computation.
/// - `E: comptime type`: Expected application-failure type; defects and interruption remain separate outcomes.
/// Returns (`type`): five-case terminal union. It does not expose an error
/// union at the Effect execution boundary; callers switch on its tag.
pub fn Exit(comptime A: type, comptime E: type) type {
    // A byte-sized discriminant avoids sub-byte tag load/store conversions.
    // This allows the optimizer to eliminate known outcome construction.
    return union(enum(u8)) {
        const Self = @This();
        /// Value type carried by the successful channel.
        pub const Success = A;
        /// Expected failure payload type `E` carried by `.fail`.
        pub const Error = E;
        /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements `deinit`
        /// and must be released after use.
        pub const effect_owned_payload = true;
        success: A,
        fail: E,
        interrupted,
        die: Defect,
        finalization_failed: Finalization(E),

        /// Extract the primary outcome without erasing cleanup information.
        /// Parameters:
        /// - `self: Self`: Receiver passed by value: the struct itself is copied, while any referenced storage keeps its original ownership and lifetime.
        /// Returns (`PrimaryOutcome(E)`): The primary success/failure/defect/interruption classification.
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
        ///
        /// Parameters:
        /// - `self: Self`: Receiver passed by value: the struct itself is copied, while any referenced storage keeps its original ownership and lifetime.
        /// Returns (`Self`): A separate value that the caller owns.
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

        /// Typed observation record; release owned state with `deinit`.
        pub const Observation = struct { exit: Self, copied: bool };

        /// A failed copy is an observer defect, not observation of the source.
        /// Controllers retain the original diagnostics until a real observer
        /// receives them or their owning scope closes.
        ///
        /// Parameters:
        /// - `self: Self`: Receiver passed by value: the struct itself is copied, while any referenced storage keeps its original ownership and lifetime.
        /// Returns (`Observation`): A subscription view of the current value and revision.
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
        ///
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// Returns (`void`): no value.
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

/// Map the typed failure payload while preserving other Exit variants.
/// Parameters:
/// - `B: comptime type`: success type required by the destination API; this
///   function accepts only non-success input and never constructs a `B`.
/// - `value: anytype`: non-success Exit whose `E` case and cleanup metadata are preserved.
/// Returns (`Exit(B, @TypeOf(value).Error)`): the same failure, defect,
/// interruption, or cleanup failure under the new success type.
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
///
/// Parameters:
/// - `T: comptime type`: Value type stored or transported by the returned component.
/// Returns (`type`): slice owner retaining the allocator that must free the
/// array. Element cleanup follows each element's declared payload contract.
pub fn OwnedSlice(comptime T: type) type {
    return struct {
        /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements `deinit`
        /// and must be released after use.
        pub const effect_owned_payload = true;
        items: []T,
        allocator: std.mem.Allocator,

        /// Release marked owning elements, free the array with its original
        /// allocator, and invalidate this owner.
        /// Parameters:
        /// - `self: *@This()`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// Returns (`void`): no value; `items` is invalid after this call.
        pub fn deinit(self: *@This()) void {
            for (self.items) |*item| discardPayload(item);
            self.allocator.free(self.items);
            self.* = undefined;
        }

        /// Allocate a new array and clone each element according to its
        /// declared ownership contract.
        /// Parameters:
        /// - `self: *const @This()`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
        /// Returns (`error{OutOfMemory}!@This()`): independently owned array,
        /// or `OutOfMemory` after partially cloned elements have been released.
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

/// Pair two values while preserving their individual payload ownership rules.
/// Parameters:
/// - `A: comptime type`: Value type produced by a successful computation.
/// - `B: comptime type`: type of the right-hand value.
/// Returns (`type`): record with `left: A` and `right: B`; its cleanup and
/// cloning visit both marked owning payloads.
pub fn Pair(comptime A: type, comptime B: type) type {
    return struct {
        /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements `deinit`
        /// and must be released after use.
        pub const effect_owned_payload = ownsPayload(A) or ownsPayload(B);
        left: A,
        right: B,

        /// Release marked owning left and right values, then invalidate the pair.
        /// Parameters:
        /// - `self: *@This()`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// Returns (`void`): no value; both payload lifetimes end here.
        pub fn deinit(self: *@This()) void {
            discardPayload(&self.left);
            discardPayload(&self.right);
            self.* = undefined;
        }

        /// Clone both sides, releasing the first clone if the second fails.
        /// Parameters:
        /// - `self: *const @This()`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
        /// Returns (`error{OutOfMemory}!@This()`): new pair with independently
        /// owned marked payloads, or `OutOfMemory` with no leaked partial copy.
        pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
            var left = try clonePayload(self.left);
            errdefer discardPayload(&left);
            return .{ .left = left, .right = try clonePayload(self.right) };
        }
    };
}

/// Transform the typed error channel.
/// Parameters:
/// - `F: comptime type`: Failure type after this operation maps the original error.
/// - `value: anytype`: Exit whose typed error and cleanup failures are mapped to `F`.
/// - `allocator: ?std.mem.Allocator`: Caller-owned allocator; allocations and retained values stay in this allocation
///   domain.
/// - `transform: comptime fn (@TypeOf(value).Error) F`: Compile-time typed callback; its signature is checked at comptime where static.
/// Returns (`Exit(@TypeOf(value).Success, F)`): Carries `@TypeOf(value).Success` on `.success` or `F` on `.fail`; `.die`, `.interrupt`, and `.cleanup` preserve other terminal causes.
pub fn mapError(comptime F: type, value: anytype, allocator: ?std.mem.Allocator, comptime transform: fn (@TypeOf(value).Error) F) Exit(@TypeOf(value).Success, F) {
    return mapErrorCaptured(F, value, allocator, @as(void, {}), struct {
        fn apply(_: void, err: @TypeOf(value).Error) F {
            return transform(err);
        }
    }.apply);
}

/// Map an error and its cleanup records with explicitly borrowed context.
/// Context is used synchronously; every mapped owning error transfers to the
/// returned Exit through the same metadata conversion as mapError.
pub fn mapErrorCaptured(comptime F: type, value: anytype, allocator: ?std.mem.Allocator, capture: anytype, comptime transform: fn (@TypeOf(capture), @TypeOf(value).Error) F) Exit(@TypeOf(value).Success, F) {
    const A = @TypeOf(value).Success;
    const Mapping = struct {
        fn primary(context: @TypeOf(capture), source: PrimaryOutcome(@TypeOf(value).Error)) PrimaryOutcome(F) {
            return switch (source) {
                .succeeded => .succeeded,
                .failed => |err| .{ .failed = transform(context, err) },
                .defected => |defect| .{ .defected = defect },
                .was_interrupted => .was_interrupted,
            };
        }

        fn cleanup(context: @TypeOf(capture), source: CleanupFailure(@TypeOf(value).Error)) CleanupFailure(F) {
            return switch (source) {
                .release_failure => |err| .{ .release_failure = transform(context, err) },
                .release_defect => |defect| .{ .release_defect = defect },
                .release_interrupted => .release_interrupted,
            };
        }
    };
    return switch (value) {
        .success => |result| .{ .success = result },
        .fail => |err| .{ .fail = transform(capture, err) },
        .die => |defect| .{ .die = defect },
        .interrupted => .interrupted,
        .finalization_failed => |details| blk: {
            var mapped: Exit(A, F) = .{ .finalization_failed = .{
                .primary = Mapping.primary(capture, details.primary),
                .first = Mapping.cleanup(capture, details.first),
            } };
            var remaining = details.rest;
            var failed = false;
            while (remaining) |node| {
                remaining = node.next;
                // Conversion is eager, including when metadata allocation has
                // failed. Never defer application mapper calls to iteration.
                var failure = Mapping.cleanup(capture, node.failure);
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
///
/// Parameters:
/// - `E: comptime type`: Expected application-failure type; defects and interruption remain separate outcomes.
/// Returns (`type`): cleanup storage with reserved inline nodes. Finalizers
/// can append failures without allocating during resource release.
pub fn CleanupStorage(comptime E: type) type {
    return struct {
        const Prefix = @Tuple(&.{ PrimaryOutcome(E), CleanupFailure(E) });
        nodes: [@typeInfo(Prefix).@"struct".field_names.len]CleanupNode(E) = undefined,
        references: std.atomic.Value(usize) = .init(0),
        owner: *anyopaque,
        release: *const fn (*anyopaque) void,
        allocator: std.mem.Allocator,

        fn releaseNode(data: *anyopaque) void {
            const self: *@This() = @ptrCast(@alignCast(data));
            if (self.references.fetchSub(1, .acq_rel) == 1) self.release(self.owner);
        }

        /// Add one finalizer failure to an `Exit`, preserving its primary outcome.
        /// Parameters:
        /// - `self: *@This()`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// - `target: anytype`: mutable `Exit` that receives the failure chain.
        /// - `failure: CleanupFailure(E)`: release failure moved into that chain.
        /// Returns (`void`): no value; `target` retains the original primary
        /// result and the new cleanup record.
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

        /// Append terminal cleanup information to an Exit.
        /// Parameters:
        /// - `self: *@This()`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// - `target: anytype`: mutable primary `Exit` receiving failures.
        /// - `cleanup: Exit(void, E)`: cleanup outcome whose non-success
        ///   cases and existing failure chain are transferred to `target`.
        /// Returns (`void`): no value; all cleanup failures remain observable.
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

        /// Append finalizer failures in execution order.
        /// Parameters:
        /// - `self: *@This()`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// - `target: anytype`: mutable `Exit` receiving retained failures.
        /// - `source: anytype`: failed outcome whose cleanup chain is consumed;
        ///   its primary owning failure is discarded as appropriate.
        /// Returns (`void`): no value; failures from `source` are attached to `target`.
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

        /// Retain the finalization failure chain for later inspection.
        /// Parameters:
        /// - `self: *const @This()`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
        /// Returns (`bool`): `true` while retained cleanup nodes still refer
        /// to this storage owner.
        pub fn retained(self: *const @This()) bool {
            return self.references.load(.acquire) != 0;
        }
    };
}

/// Join two finalizer failure chains.
/// Parameters:
/// - `E: comptime type`: Expected application-failure type; defects and interruption remain separate outcomes.
/// - `details: *Finalization(E)`: Owned finalization failure details.
/// - `head: ?*CleanupNode(E)`: Head pointer of the cleanup or description chain.
/// - `tail: ?*CleanupNode(E)`: Tail pointer of the cleanup or description chain.
/// Returns (`void`): no value; `details` now owns the appended chain.
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
