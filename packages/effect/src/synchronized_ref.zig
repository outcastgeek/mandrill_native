//! Effectful, exclusive updates to one value. An update holds a structured
//! semaphore permit while its Effect runs, without holding a native mutex
//! across suspension. Successful updates commit exactly once; failure,
//! interruption and defects leave the stored value unchanged.
const outcomes = @import("outcome.zig");
const effects = @import("effect.zig");
const semaphores = @import("semaphore.zig");

pub fn SynchronizedRef(comptime T: type) type {
    return struct {
        const Self = @This();
        const exclusive_permit_count: usize = 1;
        pub const AdmissionError = semaphores.Error;

        permit: semaphores.Semaphore,
        value: T,

        /// On an initialization error, ownership of `initial` remains with
        /// the caller. A successful result transfers it to this reference.
        pub fn init(env: anytype, initial: T) outcomes.Result(Self, AdmissionError) {
            return switch (semaphores.Semaphore.init(env, exclusive_permit_count)) {
                .ok => |permit| .{ .ok = .{ .permit = permit, .value = initial } },
                .err => |problem| .{ .err = problem },
            };
        }

        /// The read is serialized with updates. Owned T is cloned using its
        /// normal owner contract before the permit is released.
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

        pub fn shutdown(self: *Self) void {
            self.permit.shutdown();
        }

        /// Call after all operations have joined and all permits returned.
        pub fn deinit(self: *Self) void {
            self.permit.shutdown();
            self.permit.deinit();
            outcomes.discardPayload(&self.value);
            self.* = undefined;
        }
    };
}
