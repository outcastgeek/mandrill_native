//! Checked, staged event construction. A handler can only be attached after
//! all three codecs have been supplied; the codec types determine the handler
//! signature at comptime.
const effects = @import("../effect.zig");
const events = @import("event.zig");
const journal = @import("event_journal.zig");
const projections = @import("projection.zig");
const outcomes = @import("../outcome.zig");
const codec_contract = @import("../schema/internal/codec_contract.zig");

pub fn event(comptime tag: []const u8, comptime version: u32) type {
    return struct {
        pub fn payload(comptime payload_codec: anytype) type {
            const PayloadCodec = @TypeOf(payload_codec);
            return struct {
                pub fn success(comptime success_codec: anytype) type {
                    const SuccessCodec = @TypeOf(success_codec);
                    return struct {
                        pub fn failure(comptime failure_codec: anytype) type {
                            const ErrorCodec = @TypeOf(failure_codec);
                            const Definition = events.Event(PayloadCodec, SuccessCodec, ErrorCodec);
                            const defined = Definition.define(tag, version, payload_codec, success_codec, failure_codec);
                            return struct {
                                pub const EventDefinition = Definition;
                                pub const definition = defined;

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
pub fn group(comptime handlers: anytype) type {
    const Checked = events.Group(handlers);
    return struct {
        pub const Environment = Checked.Environment;

        pub fn resolve(tag: []const u8, version: u32) outcomes.Result(usize, events.ResolveError) {
            return Checked.resolve(tag, version);
        }

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
                pub const StateType = State;
                pub const StateSchema = StateCodec;
                pub const schema = state_codec;

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
