//! Provider-neutral, fenced local delivery of accepted Cluster messages.
//! Bounded batches use Effect's injected concurrent traversal; this runner
//! owns no scheduler, thread pool or network host.
const outcomes = @import("../outcome.zig");
const effects = @import("../effect.zig");
const store_module = @import("message_store.zig");
const types = @import("cluster_types.zig");

pub fn Error(comptime StoreError: type, comptime CodecError: type) type {
    return union(enum) {
        const Self = @This();
        pub const effect_owned_payload = outcomes.ownsPayload(StoreError) or outcomes.ownsPayload(CodecError);
        store: StoreError,
        codec: CodecError,
        invalid_claim,

        pub fn deinit(self: *Self) void {
            switch (self.*) {
                .store => outcomes.discardPayload(&self.store),
                .codec => outcomes.discardPayload(&self.codec),
                .invalid_claim => {},
            }
            self.* = undefined;
        }

        pub fn clone(self: *const Self) error{OutOfMemory}!Self {
            return switch (self.*) {
                .store => |value| .{ .store = try outcomes.clonePayload(value) },
                .codec => |value| .{ .codec = try outcomes.clonePayload(value) },
                .invalid_claim => .invalid_claim,
            };
        }
    };
}

pub const BatchProgress = struct {
    completed: usize,
    stopped: enum { drained, ownership_lost, stale_claim, uncertain },
};

/// Claim at most the caller's policy limit, run each accepted request, and
/// stop at a fenced or uncertain completion. The claimed page is retained by
/// structured acquisition until every handler and store operation has joined.
/// Use `serveClaims` when a caller wants bounded parallel execution of a page.
pub fn claimAndServe(
    comptime A: type,
    comptime E: type,
    comptime R: type,
    comptime ProviderError: type,
    comptime CodecError: type,
    comptime guarantee: store_module.Guarantee,
    comptime select_store: fn (*const R) *const store_module.MessageStore(R, ProviderError, guarantee),
    comptime select_address: fn (*const R) types.EntityAddress,
    comptime select_owner: fn (*const R) types.Ownership,
    comptime select_policy: fn (*const R) store_module.ClaimPolicy,
    comptime handler: fn (*const store_module.ClaimedMessage, *const R) effects.Effect(A, E, R),
    comptime encode_exit: fn (*const outcomes.Exit(A, E), *const R) outcomes.Exit(outcomes.OwnedSlice(u8), CodecError),
) effects.Effect(BatchProgress, Error(store_module.MessageStore(R, ProviderError, guarantee).Failure, CodecError), R) {
    const Store = store_module.MessageStore(R, ProviderError, guarantee);
    const Failure = Error(Store.Failure, CodecError);
    const Out = effects.Effect(BatchProgress, Failure, R);
    const Unit = effects.Effect(void, Failure, R);
    const Adapter = struct {
        fn storeError(problem: Store.Failure) Failure {
            return .{ .store = problem };
        }

        fn step(items: []const store_module.ClaimedMessage, index: usize, env: *const R) Out {
            if (index == items.len) return Out.succeed(.{ .completed = index, .stopped = .drained });
            const Position = struct { items: []const store_module.ClaimedMessage, index: usize };
            var serving = serveAt(A, E, R, ProviderError, CodecError, guarantee, select_store, handler, encode_exit, &items[index], env);
            return serving.flatMapCapturedOwned(env, BatchProgress, Position{ .items = items, .index = index }, struct {
                fn next(position: *const Position, completion: store_module.Completion, supplied: *const R) Out {
                    return switch (completion) {
                        .committed, .already_committed => step(position.items, position.index + 1, supplied),
                        .stale_claim => Out.succeed(.{ .completed = position.index, .stopped = .stale_claim }),
                        .possibly_committed => Out.succeed(.{ .completed = position.index, .stopped = .uncertain }),
                    };
                }
            }.next);
        }

        fn use(result: store_module.ClaimResult, env: *const R) Out {
            return switch (result) {
                .claimed => |batch| step(batch.items, 0, env),
                .none_available => Out.succeed(.{ .completed = 0, .stopped = .drained }),
                .ownership_lost => Out.succeed(.{ .completed = 0, .stopped = .ownership_lost }),
            };
        }

        fn release(result: store_module.ClaimResult, _: outcomes.PrimaryOutcome(Failure)) Unit {
            var owned = result;
            owned.deinit();
            return Unit.succeed({});
        }
    };
    const acquire = comptime Store.claim(select_store, select_address, select_owner, select_policy).mapError(Adapter.storeError);
    return acquire.acquireRelease(Adapter.use, Adapter.release);
}

/// Run a typed handler and save its full Exit under the claim's ownership
/// fence. A stale or uncertain completion remains an explicit store result.
/// The claim selected from R must remain alive until this Effect completes.
pub fn serveClaim(
    comptime A: type,
    comptime E: type,
    comptime R: type,
    comptime ProviderError: type,
    comptime CodecError: type,
    comptime guarantee: store_module.Guarantee,
    comptime select_store: fn (*const R) *const store_module.MessageStore(R, ProviderError, guarantee),
    comptime select_claim: fn (*const R) *const store_module.ClaimedMessage,
    comptime handler: fn (*const store_module.ClaimedMessage, *const R) effects.Effect(A, E, R),
    comptime encode_exit: fn (*const outcomes.Exit(A, E), *const R) outcomes.Exit(outcomes.OwnedSlice(u8), CodecError),
) effects.Effect(store_module.Completion, Error(store_module.MessageStore(R, ProviderError, guarantee).Failure, CodecError), R) {
    const Out = effects.Effect(store_module.Completion, Error(store_module.MessageStore(R, ProviderError, guarantee).Failure, CodecError), R);
    return Out.deferEffect(struct {
        fn start(env: *const R) Out {
            return serveAt(A, E, R, ProviderError, CodecError, guarantee, select_store, handler, encode_exit, select_claim(env), env);
        }
    }.start);
}

/// The caller retains the claimed batch and pointer array until the returned
/// Effect and all its children have joined. Concurrency is supplied by the
/// caller and admitted through Effect's selected execution capabilities.
/// Each output corresponds to the same input position, including stale and
/// uncertain fenced completions.
pub fn serveClaims(
    comptime A: type,
    comptime E: type,
    comptime R: type,
    comptime ProviderError: type,
    comptime CodecError: type,
    comptime guarantee: store_module.Guarantee,
    concurrency: usize,
    claims: []const *const store_module.ClaimedMessage,
    comptime select_store: fn (*const R) *const store_module.MessageStore(R, ProviderError, guarantee),
    comptime handler: fn (*const store_module.ClaimedMessage, *const R) effects.Effect(A, E, R),
    comptime encode_exit: fn (*const outcomes.Exit(A, E), *const R) outcomes.Exit(outcomes.OwnedSlice(u8), CodecError),
) effects.Effect(outcomes.OwnedSlice(store_module.Completion), Error(store_module.MessageStore(R, ProviderError, guarantee).Failure, CodecError), R) {
    const Failure = Error(store_module.MessageStore(R, ProviderError, guarantee).Failure, CodecError);
    const One = effects.Effect(store_module.Completion, Failure, R);
    return One.forEachConcurrent(*const store_module.ClaimedMessage, concurrency, claims, struct {
        fn run(claim: *const store_module.ClaimedMessage) One {
            return One.deferBorrowed(store_module.ClaimedMessage, claim, struct {
                fn start(selected: *const store_module.ClaimedMessage, env: *const R) One {
                    return serveAt(A, E, R, ProviderError, CodecError, guarantee, select_store, handler, encode_exit, selected, env);
                }
            }.start);
        }
    }.run);
}

fn serveAt(
    comptime A: type,
    comptime E: type,
    comptime R: type,
    comptime ProviderError: type,
    comptime CodecError: type,
    comptime guarantee: store_module.Guarantee,
    comptime select_store: fn (*const R) *const store_module.MessageStore(R, ProviderError, guarantee),
    comptime handler: fn (*const store_module.ClaimedMessage, *const R) effects.Effect(A, E, R),
    comptime encode_exit: fn (*const outcomes.Exit(A, E), *const R) outcomes.Exit(outcomes.OwnedSlice(u8), CodecError),
    selected_claim: *const store_module.ClaimedMessage,
    initial_env: *const R,
) effects.Effect(store_module.Completion, Error(store_module.MessageStore(R, ProviderError, guarantee).Failure, CodecError), R) {
    const Store = store_module.MessageStore(R, ProviderError, guarantee);
    const Failure = Error(Store.Failure, CodecError);
    const Out = effects.Effect(store_module.Completion, Failure, R);
    const Encoded = outcomes.OwnedSlice(u8);
    const Owner = struct {
        pub const effect_owned_payload = true;
        claim: *const store_module.ClaimedMessage,
        bytes: Encoded,

        pub fn deinit(self: *@This()) void {
            self.bytes.deinit();
            self.* = undefined;
        }

        pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
            return .{ .claim = self.claim, .bytes = try self.bytes.clone() };
        }
    };
    const Adapter = struct {
        fn storeError(problem: Store.Failure) Failure {
            return .{ .store = problem };
        }
        fn codecError(problem: CodecError) Failure {
            return .{ .codec = problem };
        }
        fn impossible(_: void) Failure {
            unreachable;
        }

        fn committed(_: *const Owner, value: store_module.Completion, env: *const R) Out {
            return Out.fromOwnedSuccess(env, value);
        }

        fn handled(claim: *const *const store_module.ClaimedMessage, result: outcomes.Exit(A, E), env: *const R) Out {
            var owned = result;
            const encoded = encode_exit(&owned, env);
            owned.deinit();
            const bytes = switch (encoded) {
                .success => |value| value,
                else => return Out.fromOwnedOutcome(env, outcomes.mapError(Failure, outcomes.castFailure(store_module.Completion, encoded), env.allocator, codecError)),
            };
            const store = select_store(env);
            const capture: Owner = .{ .claim = claim.*, .bytes = bytes };
            var completing = store.vtable.complete(store.userdata, env, claim.*.token(), bytes.items);
            var mapped = completing.mapErrorOwned(env, storeError);
            return mapped.flatMapCapturedOwned(env, store_module.Completion, capture, committed);
        }
    };
    const token = selected_claim.token();
    if ((types.Address{ .group = token.address.group, .entity_key = token.address.entity_key, .request_key = token.request_key }).validate() == .err or
        token.owner_id.len == 0 or token.generation == 0 or token.delivery_attempt == 0 or
        selected_claim.definition_version == 0)
        return Out.fromOwnedOutcome(initial_env, .{ .fail = .invalid_claim });
    var running = handler(selected_claim, initial_env);
    var settled = running.materializeOwned(initial_env);
    var aligned = settled.mapErrorOwned(initial_env, Adapter.impossible);
    return aligned.flatMapCapturedOwned(initial_env, store_module.Completion, selected_claim, Adapter.handled);
}
