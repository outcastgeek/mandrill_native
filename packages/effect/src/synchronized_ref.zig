//! Effectful, exclusive updates to one value. An update holds a structured
//! semaphore permit while its Effect runs, without holding a native mutex
//! across suspension. Successful updates commit exactly once; failure,
//! interruption and defects leave the stored value unchanged.
const outcomes = @import("outcome.zig");
const effects = @import("effect.zig");
const semaphores = @import("semaphore.zig");

/// Specialize a synchronized mutable reference by stored value type.
/// Parameters:
/// - `T: comptime type`: Value type stored or transported by the returned component.
/// Returns (`type`): a synchronized reference to `T` whose reads and writes use the caller I/O capability.
pub fn SynchronizedRef(comptime T: type) type {
    return struct {
        const Self = @This();
        const exclusive_permit_count: usize = 1;
        /// Typed admission error variant or error channel described by `semaphores.Error`.
        pub const AdmissionError = semaphores.Error;

        permit: semaphores.Semaphore,
        value: T,

        /// On an initialization error, ownership of `initial` remains with
        /// the caller. A successful result transfers it to this reference.
        ///
        /// Parameters:
        /// - `env: anytype`: Borrowed caller environment supplying allocator, I/O, and selected
        ///   capabilities.
        /// - `initial: T`: Initial value held by the synchronized reference; ownership transfers to the reference.
        /// Returns (`outcomes.Result(Self, AdmissionError)`): An ok/err Result; owned payloads must be released according to their type contract.
        pub fn init(env: anytype, initial: T) outcomes.Result(Self, AdmissionError) {
            return switch (semaphores.Semaphore.init(env, exclusive_permit_count)) {
                .ok => |permit| .{ .ok = .{ .permit = permit, .value = initial } },
                .err => |problem| .{ .err = problem },
            };
        }

        /// The read is serialized with updates. Owned T is cloned using its
        /// normal owner contract before the permit is released.
        ///
        /// Parameters:
        /// - `R: comptime type`: Caller-defined environment type supplying the dependencies needed by this operation.
        /// - `E: comptime type`: Expected application-failure type; defects and interruption remain separate outcomes.
        /// - `select_ref: comptime fn (*const R) *Self`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `map_admission_error: comptime fn (AdmissionError) E`: Compile-time typed function selected at comptime; no runtime
        ///   callback lookup is required.
        /// Returns (`effects.Effect(T, E, R)`): synchronized observation of the current value, cloning owned `T` under its contract.
        pub fn get(
            comptime R: type,
            comptime E: type,
            comptime select_ref: fn (*const R) *Self,
            comptime map_admission_error: fn (AdmissionError) E,
        ) effects.Effect(T, E, R) {
            const Program = effects.Effect(T, E, R);
            return semaphores.Semaphore.withPermit(R, T, E, struct {
                fn select(env: *const R) *semaphores.Semaphore {
                    return &select_ref(env).permit;
                }
            }.select, map_admission_error, struct {
                fn use(_: void) Program {
                    return Program.fromOutcome(struct {
                        fn read(env: *const R) outcomes.Exit(T, E) {
                            const copied = outcomes.clonePayload(select_ref(env).value) catch
                                return .{ .die = outcomes.Defect.out_of_memory };
                            return .{ .success = copied };
                        }
                    }.read);
                }
            }.use);
        }

        /// `transform` receives a borrowed old value while this reference is
        /// exclusively admitted. It returns a typed Effect whose successful
        /// T is moved into the reference. There is no hidden `try`/`catch`
        /// channel for the domain error E.
        ///
        /// Parameters:
        /// - `R: comptime type`: Caller-defined environment type supplying the dependencies needed by this operation.
        /// - `E: comptime type`: Expected application-failure type; defects and interruption remain separate outcomes.
        /// - `select_ref: comptime fn (*const R) *Self`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `map_admission_error: comptime fn (AdmissionError) E`: Compile-time typed function selected at comptime; no runtime
        ///   callback lookup is required.
        /// - `transform: comptime fn (*const T, *const R) effects.Effect(T, E, R)`: Compile-time typed callback; its signature is checked at comptime where
        ///   static.
        /// Returns (`effects.Effect(void, E, R)`): effect that updates the reference while preserving the callback’s typed outcome.
        pub fn updateEffect(
            comptime R: type,
            comptime E: type,
            comptime select_ref: fn (*const R) *Self,
            comptime map_admission_error: fn (AdmissionError) E,
            comptime transform: fn (*const T, *const R) effects.Effect(T, E, R),
        ) effects.Effect(void, E, R) {
            const Unit = effects.Effect(void, E, R);
            const Value = effects.Effect(T, E, R);
            return semaphores.Semaphore.withPermit(R, void, E, struct {
                fn select(env: *const R) *semaphores.Semaphore {
                    return &select_ref(env).permit;
                }
            }.select, map_admission_error, struct {
                fn use(_: void) Unit {
                    return Unit.deferEffect(struct {
                        fn start(env: *const R) Unit {
                            var candidate: Value = transform(&select_ref(env).value, env);
                            return candidate.flatMapOwned(env, struct {
                                fn commit(next: T, supplied: *const R) Unit {
                                    const reference = select_ref(supplied);
                                    var previous = reference.value;
                                    reference.value = next;
                                    outcomes.discardPayload(&previous);
                                    return Unit.succeed({});
                                }
                            }.commit);
                        }
                    }.start);
                }
            }.use);
        }

        /// Stop the capability and release its retained resources.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// Returns (`void`): no value.
        pub fn shutdown(self: *Self) void {
            self.permit.shutdown();
        }

        /// Call after all operations have joined and all permits returned.
        ///
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// Returns (`void`): no value.
        pub fn deinit(self: *Self) void {
            self.permit.shutdown();
            self.permit.deinit();
            outcomes.discardPayload(&self.value);
            self.* = undefined;
        }
    };
}
