//! Typed queue declarations keep delivery, claim fencing and acknowledgements
//! in the existing PersistedQueue capability and schema-binary envelope.
const std = @import("std");
const effects = @import("../effect.zig");
const outcomes = @import("../outcome.zig");
const queue = @import("../persistence/persisted_queue.zig");
const binary = @import("../encoding/schema_binary.zig");
const deadlines = @import("../deadline.zig");
const Issue = @import("../schema/schema_issue.zig").Issue;
const inference = @import("inference.zig");
const roles = @import("roles.zig");
const ownership = @import("ownership.zig");
const declarations = @import("declarations.zig");
const owners = @import("../internal/capture_owner.zig");
fn Role(comptime callback: anytype, comptime C: type, comptime R: type, comptime operation: []const u8) type {
    return if (inference.info(callback, operation).param_types.len == 1) inference.unary(callback, *const R, operation) else inference.result(callback, &.{ *const C, *const R }, operation);
}
fn call(comptime callback: anytype, input: anytype, env: anytype) Role(callback, @typeInfo(@TypeOf(input)).pointer.child, @typeInfo(@TypeOf(env)).pointer.child, "queue.role") {
    return if (comptime inference.info(callback, "queue.role").param_types.len == 1) callback(env) else callback(input, env);
}
fn Info(comptime C: type, comptime supplied: anytype) type {
    return struct {
        const Type = describe(C, supplied);
    }.Type;
}
fn describe(comptime C: type, comptime supplied: anytype) type {
    roles.validate(roles.recordType(supplied), "queue", &.{ "store", "codec", "limits", "key", "offer_key", "visible_at", "retention_until", "owner", "generation", "claim_policy", "body" }, &.{});
    const selector = roles.get("store", supplied);
    const Signature = inference.info(selector, "queue.store");
    const Pointer = Signature.return_type orelse @compileError("queue.store needs a typed provider return to infer its context");
    if (@typeInfo(Pointer) != .pointer or @typeInfo(Pointer).pointer.size != .one) @compileError("queue.store requires a pointer to PersistedQueue");
    const Store = @typeInfo(Pointer).pointer.child;
    if (@typeInfo(Store) != .@"struct" or !@hasDecl(Store, "OfferEffect") or !@hasDecl(Store, "storage_guarantee")) @compileError("queue.store requires the existing PersistedQueue capability");
    const R = Store.OfferEffect.Environment;
    const Failure = Store.Failure;
    const ProviderError = @FieldType(Failure, "domain");
    if (Store != queue.Store(R, ProviderError, Store.storage_guarantee)) @compileError("queue.store rejects foreign providers and fabricated durability guarantees");
    if (Role(selector, C, R, "queue.store") != Pointer) @compileError("queue.store callback mismatch");
    const selected = roles.get("codec", supplied);
    const Wire = if (@TypeOf(selected) == type) selected else @TypeOf(selected);
    if (@typeInfo(Wire) != .@"struct" or !@hasDecl(Wire, "binary_schema")) @compileError("queue.codec requires the existing versioned PureSchemaBinary adapter");
    const Shape = Wire.binary_schema;
    if (Wire != binary.Pure(Shape.codec, Shape.name)) @compileError("queue.codec rejects copied binary schema markers");
    const A = Wire.Decoded;
    if (@import("../cluster/entity_resource_plan.zig").containsResource(A)) @compileError("queue payload cannot persist live activation resources");
    const supplied_limits = roles.get("limits", supplied);
    const limits: binary.Limits = if (@TypeOf(supplied_limits) == binary.Limits) supplied_limits else blk: {
        roles.validate(@TypeOf(supplied_limits), "queue.limits", &.{ "max_schema_name_bytes", "max_payload_bytes", "max_frame_bytes" }, &.{});
        break :blk .{ .max_schema_name_bytes = supplied_limits.max_schema_name_bytes, .max_payload_bytes = supplied_limits.max_payload_bytes, .max_frame_bytes = supplied_limits.max_frame_bytes };
    };
    if (!limits.valid()) @compileError("queue requires valid existing schema-binary limits");
    if (Role(roles.get("key", supplied), C, R, "queue.key") != []const u8 or Role(roles.get("owner", supplied), C, R, "queue.owner") != []const u8 or Role(roles.get("generation", supplied), C, R, "queue.generation") != u64) @compileError("queue key/owner/generation roles must match their native contracts");
    if (inference.result(roles.get("offer_key", supplied), &.{ *const C, *const A, *const R }, "queue.offer_key") != []const u8) @compileError("queue.offer_key requires encoded key bytes");
    if (Role(roles.get("visible_at", supplied), C, R, "queue.visible_at") != deadlines.WallInstant or Role(roles.get("retention_until", supplied), C, R, "queue.retention_until") != deadlines.Deadline) @compileError("queue visibility/retention roles must use native clock/deadline policy");
    const policy = roles.get("claim_policy", supplied);
    const dynamic = @typeInfo(@TypeOf(policy)) == .@"fn" or (@typeInfo(@TypeOf(policy)) == .pointer and @typeInfo(@typeInfo(@TypeOf(policy)).pointer.child) == .@"fn");
    const static_policy: ?queue.ClaimPolicy = if (dynamic) null else blk: {
        const typed_policy: queue.ClaimPolicy = if (@TypeOf(policy) == queue.ClaimPolicy) policy else .{ .max_items = policy.max_items, .lease_until = policy.lease_until };
        if (!typed_policy.valid()) @compileError("queue static claim policy requires a positive bound and finite lease");
        break :blk typed_policy;
    };
    if (dynamic and Role(policy, C, R, "queue.claim_policy") != queue.ClaimPolicy) @compileError("queue.claim_policy must return the existing ClaimPolicy");
    // Composite ownership disposes fields in reverse order: the decoded
    // payload may borrow the claimed representation through its destructor.
    const Fields = struct { item: queue.ClaimedItem, value: A };
    const Owner = owners.Owner(Fields);
    const Decoded = struct {
        pub const effect_owned_payload = true;
        owner: Owner,
        pub fn value(self: *const @This()) *const A {
            return &self.owner.borrow().value;
        }
        pub fn token(self: *const @This()) queue.ClaimToken {
            return self.owner.borrow().item.token();
        }
        pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
            return .{ .owner = self.owner.retain() };
        }
        pub fn deinit(self: *@This()) void {
            self.owner.deinit();
        }
    };
    const Unit = effects.Effect(void, Failure, R);
    return struct {
        pub const Environment = R;
        pub const View = struct {
            const Self = @This();
            pub const Payload = A;
            pub const DecodedClaim = Decoded;
            pub const Error = Failure;
            input: C,
            fn mapIssue(issue: Issue) Failure {
                return .{ .schema = issue };
            }
            pub fn offer(self: *const Self, env: *const R, payload: A) Store.OfferEffect {
                const Inputs = struct {
                    pub const effect_owned_payload = true;
                    view: *const Self,
                    value: ownership.Capture(A),
                    pub fn deinit(owned: *@This()) void {
                        owned.value.deinit();
                    }
                };
                const Encoded = effects.Effect(outcomes.OwnedSlice(u8), Failure, R);
                var seed = Unit.succeed({});
                return seed.flatMapCapturedOwned(env, queue.OfferResult, Inputs{ .view = self, .value = .{ .value = payload } }, struct {
                    fn start(input: *const Inputs, _: void, current: *const R) Store.OfferEffect {
                        var encoding = Encoded.fromBorrowed(Inputs, input, struct {
                            fn encode(value: *const Inputs, latest: *const R) outcomes.Exit(outcomes.OwnedSlice(u8), Failure) {
                                return outcomes.mapError(Failure, Wire.encodeDirect(latest, &value.value.value, limits), latest.allocator, mapIssue);
                            }
                        }.encode);
                        return encoding.acquireReleaseCapturedOwned(current, input, struct {
                            fn use(pointer: *const *const Inputs, bytes: outcomes.OwnedSlice(u8), latest: *const R) Store.OfferEffect {
                                const input_value = pointer.*;
                                const config = &input_value.view.input;
                                const provider = call(selector, config, latest);
                                return provider.offerValue(latest, .{ .queue_key = call(roles.get("key", supplied), config, latest), .offer_key = roles.get("offer_key", supplied)(config, &input_value.value.value, latest), .encoded_payload = bytes.items, .visible_at = call(roles.get("visible_at", supplied), config, latest), .retention_until = call(roles.get("retention_until", supplied), config, latest) });
                            }
                        }.use, struct {
                            fn release(_: *const *const Inputs, bytes: outcomes.OwnedSlice(u8), _: outcomes.PrimaryOutcome(Failure), _: *const R) Unit {
                                var owned = bytes;
                                owned.deinit();
                                return Unit.succeed({});
                            }
                        }.release);
                    }
                }.start);
            }
            pub fn claim(self: *const Self, env: *const R) Store.ClaimEffect {
                var seed = Unit.succeed({});
                return seed.flatMapCapturedOwned(env, queue.ClaimResult, self, struct {
                    fn start(pointer: *const *const Self, _: void, current: *const R) Store.ClaimEffect {
                        const config = &pointer.*.input;
                        const provider = call(selector, config, current);
                        const selected_policy = if (comptime dynamic) call(policy, config, current) else static_policy.?;
                        return provider.claimValue(current, call(roles.get("key", supplied), config, current), call(roles.get("owner", supplied), config, current), call(roles.get("generation", supplied), config, current), selected_policy);
                    }
                }.start);
            }
            /// Consume the claimed item into a shared typed result. Borrowed
            /// decoded payload views remain valid through the claim owner.
            pub fn decode(_: *const Self, env: *const R, item: queue.ClaimedItem) effects.Effect(Decoded, Failure, R) {
                const Out = effects.Effect(Decoded, Failure, R);
                var source = effects.Effect(queue.ClaimedItem, Failure, R).fromOwnedSuccess(env, item);
                return source.flatMapOwned(env, struct {
                    fn decoded(value: queue.ClaimedItem, current: *const R) Out {
                        const parsed = Wire.decodeDirect(current, value.encoded_payload.items, limits);
                        if (parsed != .success) {
                            var abandoned = value;
                            abandoned.deinit();
                            return Out.fromOwnedOutcome(current, outcomes.mapError(Failure, outcomes.castFailure(Decoded, parsed), current.allocator, mapIssue));
                        }
                        const owner = Owner.create(current.allocator, Fields{ .value = parsed.success, .item = value }) catch return Out.die(outcomes.Defect.out_of_memory);
                        return Out.fromOwnedSuccess(current, Decoded{ .owner = owner });
                    }
                }.decoded);
            }
            pub fn acknowledge(self: *const Self, env: *const R, value: Decoded) Store.AckEffect {
                const Input = struct {
                    pub const effect_owned_payload = true;
                    view: *const Self,
                    value: Decoded,
                    pub fn deinit(owned: *@This()) void {
                        owned.value.deinit();
                    }
                };
                var seed = Unit.succeed({});
                return seed.flatMapCapturedOwned(env, queue.AckResult, Input{ .view = self, .value = value }, struct {
                    fn start(input: *const Input, _: void, current: *const R) Store.AckEffect {
                        return call(selector, &input.view.input, current).acknowledgeValue(current, input.value.token());
                    }
                }.start);
            }
            pub fn retry(self: *const Self, env: *const R, value: Decoded, visible_at: deadlines.WallInstant) Store.RetryEffect {
                const Input = struct {
                    pub const effect_owned_payload = true;
                    view: *const Self,
                    value: Decoded,
                    at: deadlines.WallInstant,
                    pub fn deinit(owned: *@This()) void {
                        owned.value.deinit();
                    }
                };
                var seed = Unit.succeed({});
                return seed.flatMapCapturedOwned(env, queue.RetryResult, Input{ .view = self, .value = value, .at = visible_at }, struct {
                    fn start(input: *const Input, _: void, current: *const R) Store.RetryEffect {
                        return call(selector, &input.view.input, current).retryValue(current, input.value.token(), input.at);
                    }
                }.start);
            }
        };
    };
}
fn Result(comptime C: type, comptime supplied: anytype) type {
    const Selected = Info(C, supplied);
    const Out = inference.result(roles.get("body", supplied), &.{ *const Selected.View, *const Selected.Environment }, "queue.body");
    inference.requireEffect(Out, "queue.body");
    if (Out.Environment != Selected.Environment) @compileError("queue.body environment mismatch");
    return Out;
}
pub fn scoped(env: anytype, capture: anytype, comptime supplied: anytype) Result(@TypeOf(capture), supplied) {
    const Selected = Info(@TypeOf(capture), supplied);
    const Out = Result(@TypeOf(capture), supplied);
    const Capture = ownership.Capture(Selected.View);
    return Out.scopedCaptured(env, Capture{ .value = .{ .input = capture } }, struct {
        fn body(stored: *const Capture, _: *@import("../scope.zig").Scope(Out.Error), current: *const Selected.Environment) Out {
            return roles.get("body", supplied)(&stored.value, current);
        }
    }.body);
}
pub fn declared(env: anytype, input: anytype) Result(declarations.Info(if (@TypeOf(input) == type) input else @TypeOf(input), "queue.declared").Inputs, declarations.Info(if (@TypeOf(input) == type) input else @TypeOf(input), "queue.declared").definition) {
    const Decl = declarations.Info(if (@TypeOf(input) == type) input else @TypeOf(input), "queue.declared");
    return scoped(env, roles.get("inputs", input), Decl.definition);
}
