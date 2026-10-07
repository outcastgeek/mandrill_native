//! Fixed-arity tuples compose pure codecs without allocating or erasing any
//! element's decoded, represented, or ownership type.
const std = @import("std");
const outcomes = @import("../outcome.zig");
const Issue = @import("schema_issue.zig").Issue;
const PureCodec = @import("codec.zig").PureCodec;

/// Compute the tuple value type decoded by the supplied element codecs.
/// Parameters:
/// - `elements: comptime anytype`: Compile-time statically inferred input checked when this operation is
///   instantiated.
/// Returns (`type`): tuple type produced by decoding each supplied element codec.
pub fn Values(comptime elements: anytype) type {
    const Info = @typeInfo(@TypeOf(elements));
    if (Info != .array or Info.array.child != type)
        @compileError("tuple value elements must be a comptime array of types");
    const Items = @Tuple(&elements);
    const owns = comptime blk: {
        for (elements) |Element| if (outcomes.ownsPayload(Element)) break :blk true;
        break :blk false;
    };
    return struct {
        const Self = @This();
        /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements `deinit`
        /// and must be released after use.
        pub const effect_owned_payload = owns;
        items: Items,

        /// Release resources owned by this value.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// Returns (`void`): no value.
        pub fn deinit(self: *Self) void {
            inline for (elements, 0..) |_, index| outcomes.discardPayload(&self.items[index]);
            self.* = undefined;
        }

        /// Duplicate this value; marked owning payloads are cloned, while ordinary
        /// borrowed fields retain their original lifetime.
        /// Parameters:
        /// - `self: *const Self`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
        /// Returns (`error{OutOfMemory}!Self`): copied value on success; `OutOfMemory` if owned storage could not be cloned. The source remains valid.
        pub fn clone(self: *const Self) error{OutOfMemory}!Self {
            var copy: Items = undefined;
            var initialized: usize = 0;
            errdefer {
                inline for (elements, 0..) |_, index| {
                    if (index < initialized) outcomes.discardPayload(&copy[index]);
                }
            }
            inline for (elements, 0..) |_, index| {
                copy[index] = try outcomes.clonePayload(self.items[index]);
                initialized += 1;
            }
            return .{ .items = copy };
        }
    };
}

fn elementTypes(comptime codecs: anytype, comptime declaration: []const u8) [codecs.len]type {
    const info = @typeInfo(@TypeOf(codecs));
    if (info != .@"struct" or !info.@"struct".is_tuple or codecs.len == 0)
        @compileError("tuple codec needs a nonempty tuple of pure codecs");
    var result: [codecs.len]type = undefined;
    inline for (codecs, 0..) |codec, index| {
        const C = @TypeOf(codec);
        if (!@hasDecl(C, "Decoded") or !@hasDecl(C, "Representation") or
            !@hasDecl(C, "decodeDirect") or !@hasDecl(C, "encodeDirect") or
            !@hasDecl(C, "DecodingEnvironment") or !@hasDecl(C, "EncodingEnvironment") or
            C.DecodingEnvironment != void or C.EncodingEnvironment != void)
            @compileError("tuple elements require pure codecs");
        result[index] = @field(C, declaration);
    }
    return result;
}

fn valueType(comptime codecs: anytype, comptime declaration: []const u8) type {
    const elements = elementTypes(codecs, declaration);
    return Values(elements);
}

/// Compose a codec over a fixed tuple of element codecs.
/// Parameters:
/// - `codecs: comptime anytype`: Compile-time typed codec/schema fixing the corresponding decoded and encoded value
///   types.
/// Returns (`PureCodec( valueType(codecs, "Decoded"), valueType(codecs, "Representation"), )`): A typed codec or schema preserving the declared decoded value, representation, and environments.
pub fn tuple(comptime codecs: anytype) PureCodec(
    valueType(codecs, "Decoded"),
    valueType(codecs, "Representation"),
) {
    const Decoded = valueType(codecs, "Decoded");
    const Encoded = valueType(codecs, "Representation");
    return PureCodec(Decoded, Encoded).init(struct {
        fn decode(input: *const Encoded) outcomes.Result(Decoded, Issue) {
            var values: @FieldType(Decoded, "items") = undefined;
            var initialized: usize = 0;
            inline for (codecs, 0..) |codec, index| {
                const decoded = codec.decodeDirect(&input.items[index]);
                switch (decoded) {
                    .ok => |value| {
                        values[index] = value;
                        initialized += 1;
                    },
                    .err => |issue| {
                        inline for (codecs, 0..) |_, prior| {
                            if (prior < initialized) outcomes.discardPayload(&values[prior]);
                        }
                        return .{ .err = issue };
                    },
                }
            }
            return .{ .ok = .{ .items = values } };
        }
    }.decode, struct {
        fn encode(input: *const Decoded) outcomes.Result(Encoded, Issue) {
            var values: @FieldType(Encoded, "items") = undefined;
            var initialized: usize = 0;
            inline for (codecs, 0..) |codec, index| {
                const encoded = codec.encodeDirect(&input.items[index]);
                switch (encoded) {
                    .ok => |value| {
                        values[index] = value;
                        initialized += 1;
                    },
                    .err => |issue| {
                        inline for (codecs, 0..) |_, prior| {
                            if (prior < initialized) outcomes.discardPayload(&values[prior]);
                        }
                        return .{ .err = issue };
                    },
                }
            }
            return .{ .ok = .{ .items = values } };
        }
    }.encode);
}
