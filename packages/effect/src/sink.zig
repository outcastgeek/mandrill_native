//! A typed, environment-aware consumer of a scoped Stream. Construction is
//! comptime, so running a sink introduces no dispatch layer or second runtime.
const effects = @import("effect.zig");

/// Specialize a typed stream consumer that folds elements into one result.
/// Parameters:
/// - `A: comptime type`: Value type produced by a successful computation.
/// - `E: comptime type`: Expected application-failure type; defects and interruption remain separate outcomes.
/// - `R: comptime type`: Caller-defined environment type that supplies dependencies and runtime capabilities when work executes.
/// Returns (`type`): typed stream consumer whose callback folds each `A` through an `Effect`.
pub fn Sink(comptime A: type, comptime E: type, comptime R: type) type {
    return struct {
        const Self = @This();
        consume: *const fn (A, *const R) effects.Effect(void, E, R),

        /// Create a sink that runs the typed consumer effect for each stream element.
        /// Parameters:
        /// - `consume: comptime fn (A, *const R) effects.Effect(void, E, R)`: Compile-time typed function selected at comptime; no runtime callback
        ///   lookup is required.
        /// Returns (`Self`): a sink borrowing the comptime consumer callback.
        pub fn init(comptime consume: fn (A, *const R) effects.Effect(void, E, R)) Self {
            return .{ .consume = consume };
        }

        /// Execute the description using its environment.
        /// Parameters:
        /// - `self: comptime Self`: Compile-time current value passed by value; ownership follows this
        ///   operation’s contract.
        /// - `source: comptime effects.Stream(A, E, R)`: Compile-time typed input value; move or clone owning payloads according to
        ///   its type contract.
        /// Returns (`effects.Effect(void, E, R)`): typed completion of consuming the supplied stream through this sink.
        pub fn run(comptime self: Self, comptime source: effects.Stream(A, E, R)) effects.Effect(void, E, R) {
            return source.runForEach(self.consume);
        }
    };
}
