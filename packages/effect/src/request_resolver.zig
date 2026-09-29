//! Provider-neutral batched request resolution. Results are positionally
//! aligned with submitted jobs, and every item has its own typed Exit.
const outcomes = @import("outcome.zig");
const effects = @import("effect.zig");
const keys = @import("primary_key.zig");
const std = @import("std");

pub const BatchError = union(enum) {
    invalid_key,
    empty_batch,
    response_count_mismatch,
    unavailable,
};

pub fn Resolver(comptime Definition: type, comptime R: type, comptime ProviderError: type) type {
    comptime {
        if (!@hasDecl(Definition, "Input") or !@hasDecl(Definition, "Success") or
            !@hasDecl(Definition, "DomainError"))
            @compileError("resolver requires a typed request definition");
    }
    return struct {
        const Self = @This();
        pub const Input = Definition.Input;
        pub const Success = Definition.Success;
        pub const DomainError = Definition.DomainError;
        pub const ItemExit = outcomes.Exit(Success, DomainError);
        pub const Results = outcomes.OwnedSlice(ItemExit);
        pub const Failure = union(enum) {
            const Own = @This();
            pub const effect_owned_payload = outcomes.ownsPayload(ProviderError);
            operation: BatchError,
            provider: ProviderError,

            pub fn deinit(self: *Own) void {
                if (self.* == .provider) outcomes.discardPayload(&self.provider);
                self.* = undefined;
            }

            pub fn clone(self: *const Own) error{OutOfMemory}!Own {
                return switch (self.*) {
                    .provider => |value| .{ .provider = try outcomes.clonePayload(value) },
                    else => self.*,
                };
            }
        };
        pub const BatchEffect = effects.Effect(Results, Failure, R);
        pub const Job = struct {
            key: []const u8,
            input: *const Input,
        };

        /// Scoped single-flight resolution for individual jobs. The existing
        /// Effect cache owns and joins the load through its supplied scope;
        /// cancelling one observer only stops that observer's wait. Its key
        /// policy must retain the input for the lifetime of an in-flight load
        /// and compare both identity and any payload semantics that can vary
        /// under the same key. Batch resolution remains available separately.
        pub const SharedPending = effects.EffectCache(Job, Success, DomainError, R);

        userdata: ?*anyopaque,
        vtable: *const VTable,

        /// The provider must preserve input order and return exactly one Exit
        /// per job. It may group jobs using its caller-provided grouping
        /// policy, but it cannot erase per-request domain errors.
        pub const VTable = struct {
            resolve_batch: *const fn (?*anyopaque, *const R, []const Job) BatchEffect,
        };

        pub fn fromProvider(data: ?*anyopaque, vtable: *const VTable) Self {
            return .{ .userdata = data, .vtable = vtable };
        }

        pub fn resolveBatch(comptime select_resolver: fn (*const R) *const Self, comptime select_jobs: fn (*const R) []const Job) BatchEffect {
            return BatchEffect.deferEffect(struct {
                fn start(env: *const R) BatchEffect {
                    const jobs = select_jobs(env);
                    if (jobs.len == 0) return BatchEffect.fail(.{ .operation = .empty_batch });
                    for (jobs) |job| if (keys.inspectEncoded(job.key) == .err)
                        return BatchEffect.fail(.{ .operation = .invalid_key });
                    const resolver = select_resolver(env);
                    var resolved = resolver.vtable.resolve_batch(resolver.userdata, env, jobs);
                    return resolved.flatMapCapturedOwned(env, Results, jobs.len, struct {
                        fn checked(expected_count: *const usize, results: Results, supplied: *const R) BatchEffect {
                            if (results.items.len != expected_count.*) {
                                var discarded = results;
                                discarded.deinit();
                                return BatchEffect.fail(.{ .operation = .response_count_mismatch });
                            }
                            return BatchEffect.fromOwnedSuccess(supplied, results);
                        }
                    }.checked);
                }
            }.start);
        }

        /// Partition one bounded input batch by a caller-selected stable
        /// grouping key. Each provider call sees only jobs in one group, while
        /// the returned Exits retain the original input order. The caller's
        /// jobs and their inputs remain borrowed until this Effect settles.
        pub fn resolveGrouped(
            comptime select_resolver: fn (*const R) *const Self,
            comptime select_jobs: fn (*const R) []const Job,
            comptime select_group: fn (*const Job) []const u8,
        ) BatchEffect {
            const Group = struct {
                key: []const u8,
                jobs: std.array_list.Managed(Job),
                positions: std.array_list.Managed(usize),

                fn deinit(self: *@This()) void {
                    self.jobs.deinit();
                    self.positions.deinit();
                }
            };
            const State = struct {
                allocator: std.mem.Allocator,
                groups: std.array_list.Managed(Group),
                results: []?ItemExit,

                fn deinit(self: *@This()) void {
                    for (self.results) |*result| if (result.*) |*value| value.deinit();
                    self.allocator.free(self.results);
                    for (self.groups.items) |*group| group.deinit();
                    self.groups.deinit();
                    self.allocator.destroy(self);
                }
            };
            const Position = struct { state: *State, group_index: usize };
            const Unit = effects.Effect(void, Failure, R);
            const Adapter = struct {
                fn prepare(env: *const R) outcomes.Exit(*State, Failure) {
                    const jobs = select_jobs(env);
                    if (jobs.len == 0) return .{ .fail = .{ .operation = .empty_batch } };
                    const state = env.allocator.create(State) catch return .{ .die = outcomes.Defect.out_of_memory };
                    state.* = .{
                        .allocator = env.allocator,
                        .groups = std.array_list.Managed(Group).init(env.allocator),
                        .results = env.allocator.alloc(?ItemExit, jobs.len) catch {
                            env.allocator.destroy(state);
                            return .{ .die = outcomes.Defect.out_of_memory };
                        },
                    };
                    @memset(state.results, null);
                    var keep = false;
                    defer if (!keep) state.deinit();
                    for (jobs, 0..) |job, index| {
                        const group_key = select_group(&job);
                        if (keys.inspectEncoded(job.key) == .err or keys.inspectEncoded(group_key) == .err)
                            return .{ .fail = .{ .operation = .invalid_key } };
                        var selected: ?usize = null;
                        for (state.groups.items, 0..) |group, group_index| {
                            if (std.mem.eql(u8, group.key, group_key)) {
                                selected = group_index;
                                break;
                            }
                        }
                        if (selected == null) {
                            state.groups.append(.{
                                .key = group_key,
                                .jobs = std.array_list.Managed(Job).init(env.allocator),
                                .positions = std.array_list.Managed(usize).init(env.allocator),
                            }) catch return .{ .die = outcomes.Defect.out_of_memory };
                            selected = state.groups.items.len - 1;
                        }
                        const group = &state.groups.items[selected.?];
                        group.jobs.append(job) catch return .{ .die = outcomes.Defect.out_of_memory };
                        group.positions.append(index) catch return .{ .die = outcomes.Defect.out_of_memory };
                    }
                    keep = true;
                    return .{ .success = state };
                }

                fn next(position: *const Position, values: Results, env: *const R) BatchEffect {
                    const group = &position.state.groups.items[position.group_index];
                    if (values.items.len != group.jobs.items.len) {
                        var invalid = values;
                        invalid.deinit();
                        return BatchEffect.fromOwnedOutcome(env, .{ .fail = .{ .operation = .response_count_mismatch } });
                    }
                    for (values.items, group.positions.items) |value, original_index| {
                        position.state.results[original_index] = value;
                    }
                    values.allocator.free(values.items);
                    return resolve(position.state, position.group_index + 1, env);
                }

                fn resolve(state: *State, group_index: usize, env: *const R) BatchEffect {
                    if (group_index == state.groups.items.len) {
                        const ordered = env.allocator.alloc(ItemExit, state.results.len) catch
                            return BatchEffect.die(outcomes.Defect.out_of_memory);
                        for (state.results, ordered) |*result, *slot| {
                            slot.* = result.*.?;
                            result.* = null;
                        }
                        return BatchEffect.fromOwnedSuccess(env, .{ .items = ordered, .allocator = env.allocator });
                    }
                    const resolver = select_resolver(env);
                    var running = resolver.vtable.resolve_batch(resolver.userdata, env, state.groups.items[group_index].jobs.items);
                    return running.flatMapCapturedOwned(env, Results, Position{ .state = state, .group_index = group_index }, next);
                }

                fn use(state: *State, env: *const R) BatchEffect {
                    return resolve(state, 0, env);
                }

                fn release(state: *State, _: outcomes.PrimaryOutcome(Failure)) Unit {
                    state.deinit();
                    return Unit.succeed({});
                }
            };
            const acquire = comptime effects.Effect(*State, Failure, R).fromOutcome(Adapter.prepare);
            return acquire.acquireRelease(Adapter.use, Adapter.release);
        }
    };
}
