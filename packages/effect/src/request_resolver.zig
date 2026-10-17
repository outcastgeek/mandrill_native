//! Provider-neutral batched request resolution. Results are positionally
//! aligned with submitted jobs, and every item has its own typed Exit.
const outcomes = @import("outcome.zig");
const effects = @import("effect.zig");
const keys = @import("primary_key.zig");
const std = @import("std");

/// `BatchError` is a tagged choice.
pub const BatchError = union(enum) {
    invalid_key,
    empty_batch,
    response_count_mismatch,
    unavailable,
};

/// Specialize a batched resolver with typed job, success, and provider-error channels.
/// Parameters:
/// - `Definition: comptime type`: Compile-time typed definition used to derive this API.
/// - `R: comptime type`: Caller-defined environment type that supplies dependencies and runtime capabilities when work executes.
/// - `ProviderError: comptime type`: Failure type reported by the selected provider and retained in the typed error channel.
/// Returns (`type`): batched resolver type carrying job, result, provider failure, and environment contracts.
pub fn Resolver(comptime Definition: type, comptime R: type, comptime ProviderError: type) type {
    comptime {
        if (!@hasDecl(Definition, "Input") or !@hasDecl(Definition, "Success") or
            !@hasDecl(Definition, "DomainError"))
            @compileError("resolver requires a typed request definition");
    }
    return struct {
        const Self = @This();
        pub const RequestDefinition = Definition;
        pub const Environment = R;
        pub const ProviderFailure = ProviderError;
        /// Typed request or input value decoded by this definition.
        pub const Input = Definition.Input;
        /// Value type carried by the successful channel.
        pub const Success = Definition.Success;
        /// Domain-specific error type carried by the operation.
        pub const DomainError = Definition.DomainError;
        /// Typed terminal outcome for item exit; its success and failure types are fixed by
        /// `outcomes.Exit(Success, DomainError)`.
        pub const ItemExit = outcomes.Exit(Success, DomainError);
        /// Owned results buffer paired with its allocator; release it with `deinit`.
        pub const Results = outcomes.OwnedSlice(ItemExit);
        /// Typed failure channel combining domain and provider errors.
        pub const Failure = union(enum) {
            const Own = @This();
            /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements
            /// `deinit` and must be released after use.
            pub const effect_owned_payload = outcomes.ownsPayload(ProviderError);
            operation: BatchError,
            provider: ProviderError,

            /// Release resources owned by this value.
            /// Parameters:
            /// - `self: *Own`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
            /// Returns (`void`): no value.
            pub fn deinit(self: *Own) void {
                if (self.* == .provider) outcomes.discardPayload(&self.provider);
                self.* = undefined;
            }

            /// Duplicate this value; marked owning payloads are cloned, while ordinary
            /// borrowed fields retain their original lifetime.
            /// Parameters:
            /// - `self: *const Own`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
            /// Returns (`error{OutOfMemory}!Own`): A `Own` on success; `OutOfMemory` if required allocation fails.
            pub fn clone(self: *const Own) error{OutOfMemory}!Own {
                return switch (self.*) {
                    .provider => |value| .{ .provider = try outcomes.clonePayload(value) },
                    else => self.*,
                };
            }
        };
        /// Lazy Effect for batch effect; success, error, and environment types are fixed by
        /// `effects.Effect(Results, Failure, R)`.
        pub const BatchEffect = effects.Effect(Results, Failure, R);
        /// One resolver request together with its typed completion target and batching identity.
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
        /// Typed function-pointer interface for this capability. Implement every
        /// hook with the exact signature below; keep the table valid while the
        /// capability or pending work can invoke it.
        pub const VTable = struct {
            /// Resolve a batch in input order.
            /// Parameters (callback order):
            /// - `1: ?*anyopaque`: opaque provider or handle context; keep it live until the hook and any retained work complete.
            /// - `2: *const R`: borrowed generic environment `R`, including caller-supplied runtime services.
            /// - `3: []const Job`: borrowed input slice; keep it valid through deferred use or copy before retaining it.
            /// Returns (`BatchEffect`): lazy typed Effect; run it with its declared environment to obtain Exit.
            resolve_batch: *const fn (?*anyopaque, *const R, []const Job) BatchEffect,
        };

        /// Wrap a caller-supplied provider implementation.
        /// Parameters:
        /// - `data: ?*anyopaque`: Borrowed provider context passed unchanged to every hook.
        ///   Its owner keeps it alive through any pending effect or operation.
        /// - `vtable: *const VTable`: Borrowed table of exactly typed callbacks; keep it
        ///   alive for every use of this capability, including deferred execution.
        /// Returns (`Self`): A non-owning adapter referencing `data` and `vtable`; no
        ///   provider state is allocated or transferred by this constructor.
        pub fn fromProvider(data: ?*anyopaque, vtable: *const VTable) Self {
            return .{ .userdata = data, .vtable = vtable };
        }

        /// Resolve the requested jobs as one typed batch.
        /// Parameters:
        /// - `select_resolver: comptime fn (*const R) *const Self`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// - `select_jobs: comptime fn (*const R) []const Job`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// Returns (`BatchEffect`): typed results for one bounded batch of jobs.
        pub fn resolveBatch(comptime select_resolver: fn (*const R) *const Self, comptime select_jobs: fn (*const R) []const Job) BatchEffect {
            return batch(void, &{}, struct {
                fn select(_: *const void, env: *const R) Self {
                    return select_resolver(env).*;
                }
            }.select, struct {
                fn select(_: *const void, env: *const R) []const Job {
                    return select_jobs(env);
                }
            }.select);
        }

        fn batch(comptime Inputs: type, inputs: *const Inputs, comptime select_resolver: fn (*const Inputs, *const R) Self, comptime select_jobs: fn (*const Inputs, *const R) []const Job) BatchEffect {
            return BatchEffect.deferBorrowed(Inputs, inputs, struct {
                fn start(capture: *const Inputs, env: *const R) BatchEffect {
                    const jobs = select_jobs(capture, env);
                    if (jobs.len == 0) return BatchEffect.fail(.{ .operation = .empty_batch });
                    for (jobs) |job| if (keys.inspectEncoded(job.key) == .err)
                        return BatchEffect.fail(.{ .operation = .invalid_key });
                    const resolver = select_resolver(capture, env);
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

        fn CapturedProvider(comptime Inputs: type, comptime resolve_batch: fn (*const Inputs, []const Job, *const R) BatchEffect) type {
            return struct {
                fn select(inputs: *const Inputs, _: *const R) Self {
                    return fromProvider(@constCast(inputs), &.{ .resolve_batch = invoke });
                }
                fn invoke(data: ?*anyopaque, env: *const R, jobs: []const Job) BatchEffect {
                    const inputs: *const Inputs = @ptrCast(@alignCast(data.?));
                    return resolve_batch(inputs, jobs, env);
                }
            };
        }

        /// Retain generated input ownership through provider completion and
        /// result validation on the same batch authority.
        pub fn resolveBatchCaptured(env: *const R, inputs: anytype, comptime select_jobs: fn (*const @TypeOf(inputs), *const R) []const Job, comptime resolve_batch: fn (*const @TypeOf(inputs), []const Job, *const R) BatchEffect) BatchEffect {
            const Inputs = @TypeOf(inputs);
            return @import("dsl.zig").owned(env, inputs, struct {
                fn body(stored: *const Inputs, _: *const R) BatchEffect {
                    return batch(Inputs, stored, CapturedProvider(Inputs, resolve_batch).select, select_jobs);
                }
            }.body);
        }

        /// Partition one bounded input batch by a caller-selected stable
        /// grouping key. Each provider call sees only jobs in one group, while
        /// the returned Exits retain the original input order. The caller's
        /// jobs and their inputs remain borrowed until this Effect settles.
        ///
        /// Parameters:
        /// - `select_resolver: comptime fn (*const R) *const Self`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// - `select_jobs: comptime fn (*const R) []const Job`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_group: comptime fn (*const Job) []const u8`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// Returns (`BatchEffect`): typed results after partitioning the input batch by stable caller-selected group.
        pub fn resolveGrouped(
            comptime select_resolver: fn (*const R) *const Self,
            comptime select_jobs: fn (*const R) []const Job,
            comptime select_group: fn (*const Job) []const u8,
        ) BatchEffect {
            const Adapter = Grouped(void, struct {
                fn select(_: *const void, env: *const R) Self {
                    return select_resolver(env).*;
                }
            }.select, struct {
                fn select(_: *const void, env: *const R) []const Job {
                    return select_jobs(env);
                }
            }.select, struct {
                fn select(_: *const void, job: *const Job, _: *const R) []const u8 {
                    return select_group(job);
                }
            }.select);
            const acquire = comptime effects.Effect(*Adapter.FrameState, Failure, R).fromOutcome(Adapter.prepareDefault);
            return acquire.acquireRelease(Adapter.use, Adapter.release);
        }

        pub fn resolveGroupedCaptured(env: *const R, inputs: anytype, comptime select_jobs: fn (*const @TypeOf(inputs), *const R) []const Job, comptime select_group: fn (*const @TypeOf(inputs), *const Job, *const R) []const u8, comptime resolve_batch: fn (*const @TypeOf(inputs), []const Job, *const R) BatchEffect) BatchEffect {
            const Inputs = @TypeOf(inputs);
            const Adapter = Grouped(Inputs, CapturedProvider(Inputs, resolve_batch).select, select_jobs, select_group);
            const Acquire = effects.Effect(*Adapter.FrameState, Failure, R);
            return @import("dsl.zig").owned(env, inputs, struct {
                fn body(stored: *const Inputs, current: *const R) BatchEffect {
                    var acquire = Acquire.fromBorrowed(Inputs, stored, Adapter.prepare);
                    return acquire.acquireReleaseOwned(current, Adapter.use, Adapter.release);
                }
            }.body);
        }

        fn Grouped(comptime Inputs: type, comptime select_resolver: fn (*const Inputs, *const R) Self, comptime select_jobs: fn (*const Inputs, *const R) []const Job, comptime select_group: fn (*const Inputs, *const Job, *const R) []const u8) type {
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
                inputs: *const Inputs,
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
                pub const FrameState = State;
                fn prepareDefault(env: *const R) outcomes.Exit(*State, Failure) {
                    return prepare(&{}, env);
                }
                fn prepare(inputs: *const Inputs, env: *const R) outcomes.Exit(*State, Failure) {
                    const jobs = select_jobs(inputs, env);
                    if (jobs.len == 0) return .{ .fail = .{ .operation = .empty_batch } };
                    const state = env.allocator.create(State) catch return .{ .die = outcomes.Defect.out_of_memory };
                    state.* = .{
                        .inputs = inputs,
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
                        const group_key = select_group(inputs, &job, env);
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
                    const resolver = select_resolver(state.inputs, env);
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
            return Adapter;
        }
    };
}
