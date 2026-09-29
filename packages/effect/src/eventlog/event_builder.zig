//! Checked, staged event construction. A handler can only be attached after
//! all three codecs have been supplied; the codec types determine the handler
//! signature at comptime.
const effects = @import("../effect.zig");
const events = @import("event.zig");
const journal = @import("event_journal.zig");
const projections = @import("projection.zig");
const outcomes = @import("../outcome.zig");
const codec_contract = @import("../schema/internal/codec_contract.zig");

/// Begins a stable event tag/version definition; payload, result, error, and handler follow.
///
/// Parameters:
/// - `tag: comptime []const u8`: Compile-time stable, nonempty identity component validated at definition time.
/// - `version: comptime u32`: Compile-time stable version used to reject incompatible encoded or replayed data.
/// Returns (`type`): versioned event declaration stage requiring payload, result, and failure codecs before its handler.
pub fn event(comptime tag: []const u8, comptime version: u32) type {
    return struct {
        /// Supplies the codec for the event payload.
        ///
        /// Parameters:
        /// - `payload_codec: comptime anytype`: Compile-time typed codec/schema fixing the corresponding decoded and
        ///   encoded value types.
        /// Returns (`type`): event stage retaining the payload codec and requiring the result codec.
        pub fn payload(comptime payload_codec: anytype) type {
            const PayloadCodec = @TypeOf(payload_codec);
            return struct {
                /// Supplies the codec for the event handler result.
                ///
                /// Parameters:
                /// - `success_codec: comptime anytype`: Compile-time typed codec/schema fixing the corresponding
                ///   decoded and encoded value types.
                /// Returns (`type`): event stage retaining payload and result codecs and requiring the failure codec.
                pub fn success(comptime success_codec: anytype) type {
                    const SuccessCodec = @TypeOf(success_codec);
                    return struct {
                        /// Supplies the codec for the event handler expected error.
                        ///
                        /// Parameters:
                        /// - `failure_codec: comptime anytype`: Compile-time typed codec/schema fixing the
                        ///   corresponding decoded and encoded value types.
                        /// Returns (`type`): completed event definition stage ready for a matching typed handler.
                        pub fn failure(comptime failure_codec: anytype) type {
                            const ErrorCodec = @TypeOf(failure_codec);
                            const Definition = events.Event(PayloadCodec, SuccessCodec, ErrorCodec);
                            const defined = Definition.define(tag, version, payload_codec, success_codec, failure_codec);
                            return struct {
                                /// Completed typed event definition.
                                pub const EventDefinition = Definition;
                                /// Completed definition carried by this builder or handler.
                                pub const definition = defined;

                                /// Completes the event with a typed Effect handler.
                                ///
                                /// Parameters:
                                /// - `R: comptime type`: Compile-time caller-supplied environment type used to
                                ///   specialize the returned API.
                                /// - `handler: comptime fn (*const Definition.Payload, *const R) effects.Effect(Definition.Success, Definition.DomainError, R)`: Compile-time typed callback; its signature is
                                ///   checked at comptime where static.
                                /// Returns (`events.Handler(Definition, R)`): A completed typed handler binding its operation to the selected environment.
                                pub fn handledBy(
                                    comptime R: type,
                                    comptime handler: fn (*const Definition.Payload, *const R) effects.Effect(Definition.Success, Definition.DomainError, R),
                                ) events.Handler(Definition, R) {
                                    return defined.handledBy(R, handler);
                                }
                            };
                        }
                    };
                }
            };
        }
    };
}

/// A group can contain only completed typed handlers. Event.Group checks
/// shared environment and duplicate (tag, version) identities at comptime.
///
/// Parameters:
/// - `handlers: comptime anytype`: Compile-time statically inferred input checked when this operation is
///   instantiated.
/// Returns (`type`): checked group type for the supplied completed events.
pub fn group(comptime handlers: anytype) type {
    const Checked = events.Group(handlers);
    return struct {
        /// Exact caller environment type required when the description runs.
        pub const Environment = Checked.Environment;

        /// Finds a completed event by tag/version without invoking its handler.
        ///
        /// Parameters:
        /// - `tag: []const u8`: Stable, nonempty identity component validated at definition time.
        /// - `version: u32`: Stable version used to reject incompatible encoded or replayed data.
        /// Returns (`outcomes.Result(usize, events.ResolveError)`): An ok/err Result; owned payloads must be released according to their type contract.
        pub fn resolve(tag: []const u8, version: u32) outcomes.Result(usize, events.ResolveError) {
            return Checked.resolve(tag, version);
        }

        /// Dispatches an encoded event only to its matching typed handler.
        ///
        /// Parameters:
        /// - `Response: comptime type`: Response payload type produced by the selected operation.
        /// - `tag: []const u8`: Stable, nonempty identity component validated at definition time.
        /// - `version: u32`: Stable version used to reject incompatible encoded or replayed data.
        /// - `env: *const Environment`: Borrowed caller environment supplying allocator, I/O, and selected
        ///   capabilities.
        /// - `encoded: []const u8`: Borrowed input; keep its owner alive until the operation has copied or
        ///   joined it.
        /// - `visitor: comptime anytype`: Compile-time typed callback; its signature is checked at comptime where
        ///   static.
        /// Returns (`outcomes.Result(Response, events.ResolveError)`): An ok/err Result; owned payloads must be released according to their type contract.
        pub fn dispatch(
            comptime Response: type,
            tag: []const u8,
            version: u32,
            env: *const Environment,
            encoded: []const u8,
            comptime visitor: anytype,
        ) outcomes.Result(Response, events.ResolveError) {
            return Checked.dispatch(Response, tag, version, env, encoded, visitor);
        }

        /// The completed group fixes the event set and environment once.
        /// Replay callers provide only a page and initial state; codec and
        /// fold relationships remain attached to this checked description.
        ///
        /// Parameters:
        /// - `state_codec: comptime anytype`: Compile-time typed codec/schema fixing the corresponding decoded and
        ///   encoded value types.
        /// - `FoldError: comptime type`: Compile-time typed projection-fold failure.
        /// - `decode: comptime anytype`: Compile-time typed callback; its signature is checked at comptime where
        ///   static.
        /// - `fold: comptime anytype`: Compile-time typed callback; its signature is checked at comptime where
        ///   static.
        /// Returns (`type`): typed projection definition over the selected event group.
        pub fn projection(
            comptime state_codec: anytype,
            comptime FoldError: type,
            comptime decode: anytype,
            comptime fold: anytype,
        ) type {
            const StateCodec = @TypeOf(state_codec);
            codec_contract.require(StateCodec);
            const State = StateCodec.Decoded;
            return struct {
                /// Application state type folded by this event definition.
                pub const StateType = State;
                /// StateSchema contract selected by this definition; its concrete type is `StateCodec`.
                pub const StateSchema = StateCodec;
                /// Schema selected for this definition’s state or result payload.
                pub const schema = state_codec;

                /// Folds a journal page into state and checkpoint without relying on live notifications.
                ///
                /// Parameters:
                /// - `env: *const Environment`: Borrowed caller environment supplying allocator, I/O, and selected
                ///   capabilities.
                /// - `initial: *const State`: Borrowed state from which event replay begins.
                /// - `after: journal.Cursor`: Caller-supplied time or cursor boundary used for ordering and
                ///   retention.
                /// - `page: *const journal.ReadPage`: Borrowed journal page processed by this operation.
                /// Returns (`outcomes.Exit(projections.Replayed(State), projections.Error(FoldError))`): Carries `projections.Replayed(State)` on `.success` or `projections.Error(FoldError)` on `.fail`; `.die`, `.interrupt`, and `.cleanup` preserve other terminal causes.
                pub fn replayPage(
                    env: *const Environment,
                    initial: *const State,
                    after: journal.Cursor,
                    page: *const journal.ReadPage,
                ) outcomes.Exit(projections.Replayed(State), projections.Error(FoldError)) {
                    return projections.replayPage(State, FoldError, Environment, handlers, env, initial, after, page, decode, fold);
                }
            };
        }
    };
}
