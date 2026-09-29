//! Persisted atom composition keeps storage acknowledgement distinct from
//! local invalidation. A local notice is only a hint; persisted state remains
//! authoritative after ambiguous writes or lost notifications.
const effects = @import("../effect.zig");
const outcomes = @import("../outcome.zig");
const persistence = @import("../persistence/persistable.zig");
const storage = @import("../persistence/key_value_store.zig");
const persistence_error = @import("../persistence/persistence_error.zig");
const registry = @import("atom_registry.zig");
const invalidation = @import("invalidation_registry.zig");
const query = @import("query.zig");
const services = @import("../services.zig");

/// Outcome of persisting an atom revision and notifying observers.
pub const SaveReport = struct {
    /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements `deinit` and
    /// must be released after use.
    pub const effect_owned_payload = true;
    acknowledgement: storage.WriteAcknowledgement,
    /// Null means the store could not establish that the write committed.
    invalidation: ?outcomes.Exit(invalidation.InvalidateResult, registry.Error),

    /// Release resources owned by this value.
    /// Parameters:
    /// - `self: *@This()`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
    /// Returns (`void`): no value.
    pub fn deinit(self: *@This()) void {
        outcomes.discardPayload(&self.invalidation);
        self.* = undefined;
    }

    /// Duplicate this value; marked owning payloads are cloned, while ordinary
    /// borrowed fields retain their original lifetime.
    /// Parameters:
    /// - `self: *const @This()`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
    /// Returns (`error{OutOfMemory}!@This()`): copied value on success; `OutOfMemory` if owned storage could not be cloned. The source remains valid.
    pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
        if (self.invalidation) |value| {
            const copy = value.clone();
            if (copy == .die and copy.die.kind == .out_of_memory and
                !(value == .die and value.die.kind == .out_of_memory))
                return error.OutOfMemory;
            return .{ .acknowledgement = self.acknowledgement, .invalidation = copy };
        }
        return .{ .acknowledgement = self.acknowledgement, .invalidation = null };
    }
};

/// Refresh the atom from its persistence provider.
/// Parameters:
/// - `A: comptime type`: Value type produced by a successful computation.
/// - `E: comptime type`: Expected application-failure type; defects and interruption remain separate outcomes.
/// - `R: comptime type`: Caller-defined environment type that supplies dependencies and runtime capabilities when work executes.
/// - `ProviderError: comptime type`: Failure type reported by the selected provider and retained in the typed error channel.
/// - `DecodeR: comptime type`: Environment type passed to the decoding side of the codec.
/// - `EncodeR: comptime type`: Environment type passed to the encoding side of the codec.
/// - `definition: comptime persistence.Persistable(A, E, R, ProviderError, DecodeR, EncodeR)`: Typed definition whose codec, error, and environment members are preserved.
/// - `select_state: comptime fn (*const R) *query.State(persistence.ReadResult(A, E), persistence_error.Error(ProviderError))`: Compile-time typed selector from the current environment; its result must
///   remain valid through use.
/// Returns (`effects.Effect(void, query.Error(persistence_error.Error(ProviderError)), R)`): decoded persisted atom state published to subscribers.
pub fn refresh(
    comptime A: type,
    comptime E: type,
    comptime R: type,
    comptime ProviderError: type,
    comptime DecodeR: type,
    comptime EncodeR: type,
    comptime definition: persistence.Persistable(A, E, R, ProviderError, DecodeR, EncodeR),
    comptime select_state: fn (*const R) *query.State(persistence.ReadResult(A, E), persistence_error.Error(ProviderError)),
) effects.Effect(void, query.Error(persistence_error.Error(ProviderError)), R) {
    const Failure = persistence_error.Error(ProviderError);
    return query.refresh(persistence.ReadResult(A, E), Failure, R, select_state, struct {
        fn evaluate(_: *const R) effects.Effect(persistence.ReadResult(A, E), Failure, R) {
            return definition.read();
        }
    }.evaluate);
}

/// Refresh only when the atom or query is stale.
/// Parameters:
/// - `A: comptime type`: Value type produced by a successful computation.
/// - `E: comptime type`: Expected application-failure type; defects and interruption remain separate outcomes.
/// - `R: comptime type`: Caller-defined environment type that supplies dependencies and runtime capabilities when work executes.
/// - `ProviderError: comptime type`: Failure type reported by the selected provider and retained in the typed error channel.
/// - `DecodeR: comptime type`: Environment type passed to the decoding side of the codec.
/// - `EncodeR: comptime type`: Environment type passed to the encoding side of the codec.
/// - `definition: comptime persistence.Persistable(A, E, R, ProviderError, DecodeR, EncodeR)`: Typed definition whose codec, error, and environment members are preserved.
/// - `select_state: comptime fn (*const R) *query.State(persistence.ReadResult(A, E), persistence_error.Error(ProviderError))`: Compile-time typed selector from the current environment; its result must
///   remain valid through use.
/// - `select_clock: comptime fn (*const R) services.Clock`: Compile-time typed selector from the current environment; its result must
///   remain valid through use.
/// - `force: comptime bool`: Compile-time choice to bypass normal freshness or reuse checks.
/// Returns (`effects.Effect(void, query.Error(persistence_error.Error(ProviderError)), R)`): refresh result only when the current revision is stale.
pub fn refreshIfStale(
    comptime A: type,
    comptime E: type,
    comptime R: type,
    comptime ProviderError: type,
    comptime DecodeR: type,
    comptime EncodeR: type,
    comptime definition: persistence.Persistable(A, E, R, ProviderError, DecodeR, EncodeR),
    comptime select_state: fn (*const R) *query.State(persistence.ReadResult(A, E), persistence_error.Error(ProviderError)),
    comptime select_clock: fn (*const R) services.Clock,
    comptime force: bool,
) effects.Effect(void, query.Error(persistence_error.Error(ProviderError)), R) {
    const Failure = persistence_error.Error(ProviderError);
    return query.refreshIfStale(persistence.ReadResult(A, E), Failure, R, select_state, select_clock, struct {
        fn evaluate(_: *const R) effects.Effect(persistence.ReadResult(A, E), Failure, R) {
            return definition.read();
        }
    }.evaluate, force);
}

/// Only a proven committed write triggers local invalidation. A possibly
/// committed write returns its status for read-back/reconciliation.
///
/// Parameters:
/// - `A: comptime type`: Value type produced by a successful computation.
/// - `E: comptime type`: Expected application-failure type; defects and interruption remain separate outcomes.
/// - `R: comptime type`: Caller-defined environment type that supplies dependencies and runtime capabilities when work executes.
/// - `ProviderError: comptime type`: Failure type reported by the selected provider and retained in the typed error channel.
/// - `DecodeR: comptime type`: Environment type passed to the decoding side of the codec.
/// - `EncodeR: comptime type`: Environment type passed to the encoding side of the codec.
/// - `definition: comptime persistence.Persistable(A, E, R, ProviderError, DecodeR, EncodeR)`: Typed definition whose codec, error, and environment members are preserved.
/// - `select_outcome: comptime fn (*const R) *const outcomes.Exit(A, E)`: Compile-time typed selector from the current environment; its result must
///   remain valid through use.
/// - `select_registry: comptime fn (*const R) *registry.Registry`: Compile-time typed selector from the current environment; its result must
///   remain valid through use.
/// - `select_key: comptime fn (*const R) []const u8`: Compile-time typed selector from the current environment; its result must
///   remain valid through use.
/// Returns (`effects.Effect(SaveReport, persistence_error.Error(ProviderError), R)`): persisted atom revision and notification report.
pub fn save(
    comptime A: type,
    comptime E: type,
    comptime R: type,
    comptime ProviderError: type,
    comptime DecodeR: type,
    comptime EncodeR: type,
    comptime definition: persistence.Persistable(A, E, R, ProviderError, DecodeR, EncodeR),
    comptime select_outcome: fn (*const R) *const outcomes.Exit(A, E),
    comptime select_registry: fn (*const R) *registry.Registry,
    comptime select_key: fn (*const R) []const u8,
) effects.Effect(SaveReport, persistence_error.Error(ProviderError), R) {
    const Failure = persistence_error.Error(ProviderError);
    const Out = effects.Effect(SaveReport, Failure, R);
    return Out.deferEffect(struct {
        fn start(env: *const R) Out {
            var writing = definition.write(select_outcome);
            return writing.flatMapOwned(env, struct {
                fn after(acknowledgement: storage.WriteAcknowledgement, current: *const R) Out {
                    return Out.fromOwnedSuccess(current, .{
                        .acknowledgement = acknowledgement,
                        .invalidation = if (acknowledgement == .committed)
                            select_registry(current).invalidate(select_key(current))
                        else
                            null,
                    });
                }
            }.after);
        }
    }.start);
}
