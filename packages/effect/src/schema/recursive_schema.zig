//! Lazy boxed structural composition for recursive Zig application types.
//! A schema factory is evaluated at comptime only when an operation uses it;
//! the box breaks the otherwise infinite type-level expansion.
const std = @import("std");
const outcomes = @import("../outcome.zig");
const effects = @import("../effect.zig");
const Issue = @import("schema_issue.zig").Issue;
const structural = @import("internal/structural.zig");
const parser = @import("internal/parser.zig");
const legacy = @import("../schema.zig");
const ast = @import("schema_ast.zig");

/// Specialize an owned box or recursive schema with a checked depth bound.
/// Parameters:
/// - `T: comptime type`: Value type stored or transported by the returned component.
/// Returns (`type`): owned recursive node whose payload follows the Effect ownership contract.
pub fn OwnedBox(comptime T: type) type {
    return struct {
        const Self = @This();
        /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements `deinit`
        /// and must be released after use.
        pub const effect_owned_payload = true;
        value: *T,
        allocator: std.mem.Allocator,

        /// Consumes `item`, including when the caller allocator cannot retain
        /// it. The allocator is taken from R, never passed separately.
        ///
        /// Parameters:
        /// - `env: anytype`: Borrowed caller environment supplying allocator, I/O, and selected
        ///   capabilities.
        /// - `item: T`: Typed item inserted into the collection.
        /// Returns (`outcomes.Exit(Self, Issue)`): Carries `Self` on `.success` or `Issue` on `.fail`; `.die`, `.interrupt`, and `.cleanup` preserve other terminal causes.
        pub fn init(env: anytype, item: T) outcomes.Exit(Self, Issue) {
            const R = @typeInfo(@TypeOf(env)).pointer.child;
            if (!@hasField(R, "allocator") or @FieldType(R, "allocator") != std.mem.Allocator)
                @compileError("OwnedBox requires R.allocator");
            const storage = env.allocator.create(T) catch {
                var discarded = item;
                outcomes.discardPayload(&discarded);
                return .{ .die = outcomes.Defect.out_of_memory };
            };
            storage.* = item;
            return .{ .success = .{ .value = storage, .allocator = env.allocator } };
        }

        /// Release resources owned by this value.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// Returns (`void`): no value.
        pub fn deinit(self: *Self) void {
            outcomes.discardPayload(self.value);
            self.allocator.destroy(self.value);
            self.* = undefined;
        }

        /// Duplicate this value; marked owning payloads are cloned, while ordinary
        /// borrowed fields retain their original lifetime.
        /// Parameters:
        /// - `self: *const Self`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
        /// Returns (`error{OutOfMemory}!Self`): copied value on success; `OutOfMemory` if owned storage could not be cloned. The source remains valid.
        pub fn clone(self: *const Self) error{OutOfMemory}!Self {
            const copied = try self.allocator.create(T);
            errdefer self.allocator.destroy(copied);
            copied.* = try outcomes.clonePayload(self.value.*);
            return .{ .value = copied, .allocator = self.allocator };
        }
    };
}

/// `max_depth` is an explicit caller-authored schema policy. Each box consumes
/// one depth unit in invocation-local state, so cycles fail as typed issues.
///
/// Parameters:
/// - `T: comptime type`: Value type stored or transported by the returned component.
/// - `R: comptime type`: Caller-defined environment type that supplies dependencies and runtime capabilities when work executes.
/// - `max_depth: comptime usize`: Compile-time maximum recursive decoding depth.
/// - `reference_name: comptime []const u8`: Compile-time reference identity for recursive schemas.
/// - `schema: comptime fn () type`: Compile-time typed codec/schema fixing the corresponding decoded and encoded value
///   types.
/// Returns (`type`): recursive schema with explicit depth bound and typed child representation.
pub fn Box(comptime T: type, comptime R: type, comptime max_depth: usize, comptime reference_name: []const u8, comptime schema: fn () type) type {
    comptime {
        if (max_depth == 0) @compileError("recursive schema depth must permit at least one box");
        if (reference_name.len == 0 or !std.unicode.utf8ValidateSlice(reference_name))
            @compileError("recursive schema requires a stable named reference");
        if (!@hasField(R, "allocator") or @FieldType(R, "allocator") != std.mem.Allocator)
            @compileError("recursive schema requires R.allocator");
    }
    return struct {
        const Self = @This();
        /// Application value type produced by this codec.
        pub const Decoded = OwnedBox(T);
        /// Exact caller environment type required when the description runs.
        pub const Environment = R;

        /// Return the structural representation node for this schema.
        /// Returns (`*const ast.Node`): A borrowed read-only pointer; keep its owner alive while using it.
        pub fn encodedNode() *const ast.Node {
            const Metadata = struct {
                const node: ast.Node = .{ .reference = reference_name };
            };
            return &Metadata.node;
        }

        fn child() type {
            const C = schema();
            if (!@hasDecl(C, "Decoded") or C.Decoded != T or
                !@hasDecl(C, "Environment") or C.Environment != R or
                !@hasDecl(C, "decodeWithin") or !@hasDecl(C, "encodeWithin"))
                @compileError("recursive schema factory must return a structural schema for T in R");
            return C;
        }

        /// Decode within the caller-supplied depth or size limits.
        /// Parameters:
        /// - `env: *const R`: Borrowed caller environment supplying allocator, I/O, and selected
        ///   capabilities.
        /// - `json: *const std.json.Value`: JSON value or document to decode, encode, or validate.
        /// - `state: *structural.DecodeState`: Mutable operation state, retained for the duration of this call.
        /// Returns (`outcomes.Exit(Decoded, Issue)`): Carries `Decoded` on `.success` or `Issue` on `.fail`; `.die`, `.interrupt`, and `.cleanup` preserve other terminal causes.
        pub fn decodeWithin(env: *const R, json: *const std.json.Value, state: *structural.DecodeState) outcomes.Exit(Decoded, Issue) {
            if (!state.enter(max_depth)) return .{ .fail = Issue.init(env, .decode, .depth_limit) };
            defer state.leave();
            const C = comptime child();
            const decoded = C.decodeWithin(env, json, state);
            const value = switch (decoded) {
                .success => |item| item,
                else => return outcomes.castFailure(Decoded, decoded),
            };
            return OwnedBox(T).init(env, value);
        }

        /// Decode immediately and return the typed validation outcome.
        /// Parameters:
        /// - `env: *const R`: Borrowed caller environment supplying allocator, I/O, and selected
        ///   capabilities.
        /// - `json: *const std.json.Value`: JSON value or document to decode, encode, or validate.
        /// Returns (`outcomes.Exit(Decoded, Issue)`): Carries `Decoded` on `.success` or `Issue` on `.fail`; `.die`, `.interrupt`, and `.cleanup` preserve other terminal causes.
        pub fn decodeDirect(env: *const R, json: *const std.json.Value) outcomes.Exit(Decoded, Issue) {
            var state: structural.DecodeState = .{};
            return decodeWithin(env, json, &state);
        }

        /// Decode a value from its wire representation.
        /// Parameters:
        /// - `json: *const std.json.Value`: JSON value or document to decode, encode, or validate.
        /// Returns (`effects.Effect(Decoded, Issue, R)`): decoded recursive value, bounded by the caller-selected maximum depth.
        pub fn decode(json: *const std.json.Value) effects.Effect(Decoded, Issue, R) {
            return effects.Effect(Decoded, Issue, R).fromBorrowed(std.json.Value, json, struct {
                fn run(input: *const std.json.Value, env: *const R) outcomes.Exit(Decoded, Issue) {
                    return decodeDirect(env, input);
                }
            }.run);
        }

        /// Encode within the caller-supplied depth or size limits.
        /// Parameters:
        /// - `env: *const R`: Borrowed caller environment supplying allocator, I/O, and selected
        ///   capabilities.
        /// - `value: *const Decoded`: Borrowed input; keep its owner alive until the operation has copied or
        ///   joined it.
        /// - `state: *structural.EncodeState`: Mutable operation state, retained for the duration of this call.
        /// Returns (`outcomes.Exit(std.json.Value, Issue)`): Carries `std.json.Value` on `.success` or `Issue` on `.fail`; `.die`, `.interrupt`, and `.cleanup` preserve other terminal causes.
        pub fn encodeWithin(env: *const R, value: *const Decoded, state: *structural.EncodeState) outcomes.Exit(std.json.Value, Issue) {
            if (!state.enter(max_depth)) return .{ .fail = Issue.init(env, .encode, .depth_limit) };
            defer state.leave();
            const C = comptime child();
            return C.encodeWithin(env, value.value, state);
        }

        /// Encode directly into owned bytes.
        /// Parameters:
        /// - `env: *const R`: Borrowed caller environment supplying allocator, I/O, and selected
        ///   capabilities.
        /// - `value: *const Decoded`: Borrowed input; keep its owner alive until the operation has copied or
        ///   joined it.
        /// Returns (`outcomes.Exit(outcomes.OwnedSlice(u8), Issue)`): Carries `outcomes.OwnedSlice(u8)` on `.success` or `Issue` on `.fail`; `.die`, `.interrupt`, and `.cleanup` preserve other terminal causes.
        pub fn encodeBytesDirect(env: *const R, value: *const Decoded) outcomes.Exit(outcomes.OwnedSlice(u8), Issue) {
            var arena = std.heap.ArenaAllocator.init(env.allocator);
            defer arena.deinit();
            var state: structural.EncodeState = .{ .scratch = arena.allocator() };
            const encoded = encodeWithin(env, value, &state);
            const json = switch (encoded) {
                .success => |item| item,
                else => return outcomes.castFailure(outcomes.OwnedSlice(u8), encoded),
            };
            const bytes = std.json.Stringify.valueAlloc(env.allocator, json, .{}) catch
                return .{ .die = outcomes.Defect.out_of_memory };
            return .{ .success = .{ .items = bytes, .allocator = env.allocator } };
        }

        /// Validate input immediately against this schema.
        /// Parameters:
        /// - `env: *const R`: Borrowed caller environment supplying allocator, I/O, and selected
        ///   capabilities.
        /// - `value: *const Decoded`: Borrowed input; keep its owner alive until the operation has copied or
        ///   joined it.
        /// Returns (`outcomes.Exit(void, Issue)`): Carries no value on `.success` or `Issue` on `.fail`; `.die`, `.interrupt`, and `.cleanup` preserve other terminal causes.
        pub fn validateDirect(env: *const R, value: *const Decoded) outcomes.Exit(void, Issue) {
            var arena = std.heap.ArenaAllocator.init(env.allocator);
            defer arena.deinit();
            var state: structural.EncodeState = .{ .scratch = arena.allocator() };
            const checked = encodeWithin(env, value, &state);
            return switch (checked) {
                .success => .{ .success = {} },
                else => outcomes.castFailure(void, checked),
            };
        }

        /// Validate the value against its declared contract.
        /// Parameters:
        /// - `value: *const Decoded`: Borrowed input; keep its owner alive until the operation has copied or
        ///   joined it.
        /// Returns (`effects.Effect(void, Issue, R)`): unit success for a recursive value within the depth and child-schema constraints.
        pub fn validate(value: *const Decoded) effects.Effect(void, Issue, R) {
            return effects.Effect(void, Issue, R).fromBorrowed(Decoded, value, struct {
                fn run(input: *const Decoded, env: *const R) outcomes.Exit(void, Issue) {
                    return validateDirect(env, input);
                }
            }.run);
        }

        /// Describe encoding to owned bytes as a lazy Effect.
        /// Parameters:
        /// - `value: *const Decoded`: Borrowed input; keep its owner alive until the operation has copied or
        ///   joined it.
        /// Returns (`effects.Effect(outcomes.OwnedSlice(u8), Issue, R)`): owned encoded bytes for a recursive value within the depth bound.
        pub fn encodeBytes(value: *const Decoded) effects.Effect(outcomes.OwnedSlice(u8), Issue, R) {
            return effects.Effect(outcomes.OwnedSlice(u8), Issue, R).fromBorrowed(Decoded, value, struct {
                fn run(input: *const Decoded, env: *const R) outcomes.Exit(outcomes.OwnedSlice(u8), Issue) {
                    return encodeBytesDirect(env, input);
                }
            }.run);
        }

        /// Parse encoded bytes immediately under declared limits.
        /// Parameters:
        /// - `env: *const R`: Borrowed caller environment supplying allocator, I/O, and selected
        ///   capabilities.
        /// - `bytes: []const u8`: Borrowed input; keep its owner alive until the operation has copied or
        ///   joined it.
        /// - `limits: parser.Limits`: Caller-selected, validated behavior or resource bounds; no hidden default
        ///   is installed.
        /// Returns (`outcomes.Exit(legacy.Parsed(Decoded), Issue)`): Carries `legacy.Parsed(Decoded)` on `.success` or `Issue` on `.fail`; `.die`, `.interrupt`, and `.cleanup` preserve other terminal causes.
        pub fn parseBytesDirect(env: *const R, bytes: []const u8, limits: parser.Limits) outcomes.Exit(legacy.Parsed(Decoded), Issue) {
            return parser.parseWith(Decoded, env, bytes, .reject, limits, decodeDirect);
        }
    };
}
