//! Typed replay over authoritative journal pages. Pure folds are separate
//! from event handlers and never publish a checkpoint on partial failure.
const std = @import("std");
const outcomes = @import("../outcome.zig");
const events = @import("event.zig");
const journal = @import("event_journal.zig");
const Issue = @import("../schema/schema_issue.zig").Issue;
const effects = @import("../effect.zig");
const rt = @import("../internal/runtime.zig");

fn selectEntry(comptime Failure: type, comptime Group: type, cursor: journal.Cursor, entry: *const journal.Entry) outcomes.Result(usize, Failure) {
    const expected = std.math.add(u64, cursor.sequence, 1) catch return .{ .err = .history_gap };
    if (entry.cursor.sequence != expected) return .{ .err = .history_gap };
    return switch (Group.resolve(entry.tag.items, entry.schema_version)) {
        .ok => |index| .{ .ok = index },
        .err => |problem| switch (problem) {
            .unknown_event => .{ .err = .unknown_event },
            .incompatible_version => |versions| .{ .err = .{ .incompatible_version = .{ .requested = versions.requested, .supported = versions.supported } } },
        },
    };
}

/// Specialize the typed eventlog error channel from its compile-time error type.
/// Parameters:
/// - `FoldError: comptime type`: Compile-time typed projection-fold failure.
/// Returns (`type`): typed projection failure union retaining reducer and checkpoint errors.
pub fn Error(comptime FoldError: type) type {
    return union(enum) {
        const Self = @This();
        /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements `deinit`
        /// and must be released after use.
        pub const effect_owned_payload = true;
        fold: FoldError,
        schema: Issue,
        unknown_event,
        incompatible_version: struct { requested: u32, supported: u32 },
        history_gap,
        malformed_page,

        /// Release resources owned by this value.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// Returns (`void`): no value.
        pub fn deinit(self: *Self) void {
            if (self.* == .fold) outcomes.discardPayload(&self.fold);
            if (self.* == .schema) self.schema.deinit();
            self.* = undefined;
        }

        /// Duplicate this value; marked owning payloads are cloned, while ordinary
        /// borrowed fields retain their original lifetime.
        /// Parameters:
        /// - `self: *const Self`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
        /// Returns (`error{OutOfMemory}!Self`): copied value on success; `OutOfMemory` if owned storage could not be cloned. The source remains valid.
        pub fn clone(self: *const Self) error{OutOfMemory}!Self {
            return switch (self.*) {
                .fold => |value| .{ .fold = try outcomes.clonePayload(value) },
                .schema => |value| .{ .schema = try value.clone() },
                else => self.*,
            };
        }
    };
}

/// Specialize projection state together with its replay cursor.
/// Parameters:
/// - `State: comptime type`: State value type retained between operations.
/// Returns (`type`): projection state paired with the cursor reached by replay.
pub fn Replayed(comptime State: type) type {
    return struct {
        const Self = @This();
        /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements `deinit`
        /// and must be released after use.
        pub const effect_owned_payload = true;
        state: State,
        cursor: journal.Cursor,

        /// Release resources owned by this value.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// Returns (`void`): no value.
        pub fn deinit(self: *Self) void {
            outcomes.discardPayload(&self.state);
            self.* = undefined;
        }

        /// Duplicate this value; marked owning payloads are cloned, while ordinary
        /// borrowed fields retain their original lifetime.
        /// Parameters:
        /// - `self: *const Self`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
        /// Returns (`error{OutOfMemory}!Self`): copied value on success; `OutOfMemory` if owned storage could not be cloned. The source remains valid.
        pub fn clone(self: *const Self) error{OutOfMemory}!Self {
            return .{ .state = try outcomes.clonePayload(self.state), .cursor = self.cursor };
        }
    };
}

/// `decode` receives a concrete handler and returns Exit(Payload, Issue).
/// `fold` receives that same handler, a private mutable state and a borrowed
/// payload, returning Result(void, FoldError). A failed fold discards the
/// private clone; the caller's prior projection state is untouched.
///
/// Parameters:
/// - `State: comptime type`: State value type retained between operations.
/// - `FoldError: comptime type`: Compile-time typed projection-fold failure.
/// - `R: comptime type`: Caller-defined environment type that supplies dependencies and runtime capabilities when work executes.
/// - `handlers: comptime anytype`: Compile-time statically inferred input checked when this operation is
///   instantiated.
/// - `env: *const R`: Borrowed caller environment supplying allocator, I/O, and selected capabilities.
/// - `initial: *const State`: Borrowed initial projection state; it remains owned by the caller.
/// - `after: journal.Cursor`: Caller-supplied time or cursor boundary used for ordering and retention.
/// - `page: *const journal.ReadPage`: Borrowed journal page processed by this operation.
/// - `decode: comptime anytype`: Compile-time typed callback; its signature is checked at comptime where static.
/// - `fold: comptime anytype`: Compile-time typed callback; its signature is checked at comptime where static.
/// Returns (`outcomes.Exit(Replayed(State), Error(FoldError))`): Carries `Replayed(State)` on `.success` or `Error(FoldError)` on `.fail`; `.die`, `.interrupt`, and `.cleanup` preserve other terminal causes.
pub fn replayPage(
    comptime State: type,
    comptime FoldError: type,
    comptime R: type,
    comptime handlers: anytype,
    env: *const R,
    initial: *const State,
    after: journal.Cursor,
    page: *const journal.ReadPage,
    comptime decode: anytype,
    comptime fold: anytype,
) outcomes.Exit(Replayed(State), Error(FoldError)) {
    const Failure = Error(FoldError);
    const Group = events.Group(handlers);
    if (R == void or !@hasField(R, "allocator") or @FieldType(R, "allocator") != std.mem.Allocator)
        @compileError("projection replay requires caller allocator in R");
    var state = outcomes.clonePayload(initial.*) catch return .{ .die = outcomes.Defect.out_of_memory };
    var keep_state = false;
    defer if (!keep_state) outcomes.discardPayload(&state);
    var cursor = after;
    for (page.entries.items) |*entry| {
        const selected = switch (selectEntry(Failure, Group, cursor, entry)) {
            .ok => |index| index,
            .err => |problem| return .{ .fail = problem },
        };
        inline for (handlers, 0..) |handler, index| {
            if (selected == index) {
                var decoded = decode(handler, env, entry.encoded.items);
                switch (decoded) {
                    .success => {
                        defer decoded.deinit();
                        switch (fold(handler, &state, &decoded.success)) {
                            .ok => {},
                            .err => |problem| return .{ .fail = .{ .fold = problem } },
                        }
                    },
                    .fail => |problem| return .{ .fail = .{ .schema = problem } },
                    .die => |defect| return .{ .die = defect },
                    .interrupted => return .interrupted,
                    .finalization_failed => return outcomes.mapError(Failure, outcomes.castFailure(Replayed(State), decoded), env.allocator, struct {
                        fn map(problem: Issue) Failure {
                            return .{ .schema = problem };
                        }
                    }.map),
                }
            }
        }
        cursor = entry.cursor;
    }
    if (page.next.sequence != cursor.sequence) return .{ .fail = .malformed_page };
    keep_state = true;
    return .{ .success = .{ .state = state, .cursor = cursor } };
}

/// Replay contextual/suspended decoders on the existing Effect kernel. Typed
/// decoder jobs share native replay's ordering, identity and cursor checks;
/// pure folds see a private state clone and a live decoded payload owner.
pub fn replayPageEffect(
    comptime State: type,
    comptime FoldError: type,
    comptime R: type,
    comptime handlers: anytype,
    env: *const R,
    initial: *const State,
    after: journal.Cursor,
    page: *const journal.ReadPage,
    comptime decode: anytype,
    comptime fold: anytype,
) effects.Effect(Replayed(State), Error(FoldError), R) {
    const Failure = Error(FoldError);
    const Out = effects.Effect(Replayed(State), Failure, R);
    const Unit = effects.Effect(void, Failure, R);
    const Group = events.Group(handlers);
    const Inputs = struct { initial: *const State, after: journal.Cursor, page: *const journal.ReadPage };
    const Types = struct {
        fn Decoder(comptime index: usize) type {
            const T = @TypeOf(decode(handlers[index], @as(*const R, undefined), @as([]const u8, undefined)));
            @import("../dsl/inference.zig").requireEffect(T, "projection decoder");
            if (T.Error != Issue or T.Environment != R) @compileError("projection decoder requires exact Effect(PayloadOwner,Issue,R); project its directional environment explicitly");
            return T;
        }
    };
    const Frame = struct {
        const Self = @This();
        allocator: std.mem.Allocator,
        parent: *Out.RuntimeAdapter.Job,
        input: *const Inputs,
        state: State,
        cursor: journal.Cursor,
        index: usize = 0,
        jobs: Jobs = undefined,
        exits: Exits = undefined,
        const Jobs = blk: {
            var types: [handlers.len]type = undefined;
            for (0..handlers.len) |i| types[i] = Types.Decoder(i).RuntimeAdapter.Job;
            break :blk @Tuple(&types);
        };
        const Exits = blk: {
            var types: [handlers.len]type = undefined;
            for (0..handlers.len) |i| types[i] = outcomes.Exit(Types.Decoder(i).Success, Issue);
            break :blk @Tuple(&types);
        };
        fn finish(self: *Self, value: outcomes.Exit(Replayed(State), Failure)) rt.Step {
            if (value != .success) outcomes.discardPayload(&self.state);
            const parent = self.parent;
            self.allocator.destroy(self);
            return parent.adapterFinish(value);
        }
        fn next(active: *rt.State, data: ?*anyopaque) rt.Step {
            const self: *Self = @ptrCast(@alignCast(data.?));
            if (active.cancelled()) return self.finish(.interrupted);
            const entries = self.input.page.entries.items;
            if (self.index == entries.len) {
                if (self.input.page.next.sequence != self.cursor.sequence) return self.finish(.{ .fail = .malformed_page });
                return self.finish(.{ .success = .{ .state = self.state, .cursor = self.cursor } });
            }
            const entry = &entries[self.index];
            const selected = switch (selectEntry(Failure, Group, self.cursor, entry)) {
                .ok => |index| index,
                .err => |problem| return self.finish(.{ .fail = problem }),
            };
            inline for (handlers, 0..) |handler, i| if (selected == i) {
                const described = decode(handler, self.parent.env, entry.encoded.items);
                self.jobs[i] = .{ .program = described, .owned = described.ownership, .env = self.parent.env, .inherited = active.services, .sink = .{ .output = &self.exits[i], .next_step = .{ .data = self, .advance = struct {
                    fn complete(current: *rt.State, ptr: ?*anyopaque) rt.Step {
                        const frame: *Self = @ptrCast(@alignCast(ptr.?));
                        var decoded = current.publish(frame.exits[i]);
                        if (decoded != .success) return frame.finish(outcomes.mapError(Failure, outcomes.castFailure(Replayed(State), decoded), frame.allocator, struct {
                            fn map(issue: Issue) Failure {
                                return .{ .schema = issue };
                            }
                        }.map));
                        defer decoded.deinit();
                        switch (fold(handler, &frame.state, &decoded.success)) {
                            .ok => {},
                            .err => |problem| return frame.finish(.{ .fail = .{ .fold = problem } }),
                        }
                        frame.cursor = frame.input.page.entries.items[frame.index].cursor;
                        frame.index += 1;
                        return .{ .data = frame, .advance = next };
                    }
                }.complete } } };
                return self.jobs[i].adapterStep();
            };
            unreachable;
        }
        fn start(_: *rt.State, parent: *Out.RuntimeAdapter.Job) rt.Step {
            const input: *const Inputs = @ptrCast(@alignCast(parent.program.value.borrowed));
            const frame = parent.env.allocator.create(Self) catch return parent.adapterFinish(.{ .die = outcomes.Defect.out_of_memory });
            const state = outcomes.clonePayload(input.initial.*) catch {
                parent.env.allocator.destroy(frame);
                return parent.adapterFinish(.{ .die = outcomes.Defect.out_of_memory });
            };
            frame.* = .{ .allocator = parent.env.allocator, .parent = parent, .input = input, .state = state, .cursor = input.after };
            return .{ .data = frame, .advance = next };
        }
    };
    var seed = Unit.succeed({});
    return seed.flatMapCapturedOwned(env, Replayed(State), Inputs{ .initial = initial, .after = after, .page = page }, struct {
        fn body(input: *const Inputs, _: void, _: *const R) Out {
            return .{ .value = .{ .borrowed = input }, .start = Frame.start };
        }
    }.body);
}
