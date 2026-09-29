//! Compatibility decoder API and parsed-document ownership for typed schemas.
const std = @import("std");
const outcomes = @import("outcome.zig");
const effects = @import("effect.zig");

const Document = struct {
    source: std.json.Parsed(std.json.Value),
    allocator: std.mem.Allocator,
    references: std.atomic.Value(usize) = .init(1),
};

/// Specialize a decoded value that retains its parsed source and allocator ownership.
/// Parameters:
/// - `A: comptime type`: Value type produced by a successful computation.
/// Returns (`type`): decoded value owner retaining the parsed JSON document and its allocator.
pub fn Parsed(comptime A: type) type {
    return struct {
        /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements `deinit`
        /// and must be released after use.
        pub const effect_owned_payload = true;
        value: A,
        document: *Document,

        /// Takes ownership of an already parsed JSON document. On allocation
        /// failure the caller still owns both arguments and must release them.
        ///
        /// Parameters:
        /// - `allocator: std.mem.Allocator`: Caller-owned allocator; allocations and retained values stay in this
        ///   allocation domain.
        /// - `source: std.json.Parsed(std.json.Value)`: parsed JSON document whose storage is moved into the returned owner.
        /// - `value: A`: decoded value retained alongside the parsed source.
        /// Returns (`error{OutOfMemory}!@This()`): copied value on success; `OutOfMemory` if owned storage could not be cloned. The source remains valid.
        pub fn init(allocator: std.mem.Allocator, source: std.json.Parsed(std.json.Value), value: A) error{OutOfMemory}!@This() {
            const document = try allocator.create(Document);
            document.* = .{ .source = source, .allocator = allocator };
            return .{ .value = value, .document = document };
        }

        /// Decoder views into JSON remain valid until this owner is destroyed.
        /// Independently allocated application payloads keep their own contract.
        ///
        /// Parameters:
        /// - `self: *@This()`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// Returns (`void`): no value.
        pub fn deinit(self: *@This()) void {
            outcomes.discardPayload(&self.value);
            if (self.document.references.fetchSub(1, .acq_rel) == 1) {
                self.document.source.deinit();
                self.document.allocator.destroy(self.document);
            }
            self.* = undefined;
        }

        /// Duplicate this value; marked owning payloads are cloned, while ordinary
        /// borrowed fields retain their original lifetime.
        /// Parameters:
        /// - `self: *const @This()`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
        /// Returns (`error{OutOfMemory}!@This()`): copied value on success; `OutOfMemory` if owned storage could not be cloned. The source remains valid.
        pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
            const value = try outcomes.clonePayload(self.value);
            _ = self.document.references.fetchAdd(1, .monotonic);
            return .{ .value = value, .document = self.document };
        }

        /// Construct a JSON-backed typed schema.
        /// Parameters:
        /// - `self: *const @This()`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
        /// Returns (`std.json.Value`): The JSON representation produced by this codec.
        pub fn json(self: *const @This()) std.json.Value {
            return self.document.source.value;
        }
    };
}

/// Define a typed schema with its compile-time value and representation types.
/// Parameters:
/// - `A: comptime type`: Value type produced by a successful computation.
/// - `E: comptime type`: Expected application-failure type; defects and interruption remain separate outcomes.
/// Returns (`type`): typed schema that decodes JSON into `A` with expected failure `E`.
pub fn Schema(comptime A: type, comptime E: type) type {
    return struct {
        const Self = @This();
        /// Value type carried by the successful channel.
        pub const Success = A;
        /// Expected decoding failure type `E` selected for this Schema.
        pub const Error = E;
        decoder: *const fn (std.json.Value) outcomes.Exit(A, E),

        /// Build a schema from a typed decoding callback.
        /// Parameters:
        /// - `decoder: comptime fn (std.json.Value) outcomes.Result(A, E)`: Compile-time typed function selected at comptime; no runtime callback
        ///   lookup is required.
        /// Returns (`Self`): schema that turns a JSON value into `Result(A, E)` at execution.
        pub fn fromDecoder(comptime decoder: fn (std.json.Value) outcomes.Result(A, E)) Self {
            return .{ .decoder = struct {
                fn decode(value: std.json.Value) outcomes.Exit(A, E) {
                    return switch (decoder(value)) {
                        .ok => |a| .{ .success = a },
                        .err => |e| .{ .fail = e },
                    };
                }
            }.decode };
        }

        /// Build a schema from a callback returning a terminal Exit.
        /// Parameters:
        /// - `decoder: comptime fn (std.json.Value) outcomes.Exit(A, E)`: Compile-time typed function selected at comptime; no runtime callback
        ///   lookup is required.
        /// Returns (`Self`): schema preserving all five `Exit(A, E)` outcomes from the decoder.
        pub fn fromOutcomeDecoder(comptime decoder: fn (std.json.Value) outcomes.Exit(A, E)) Self {
            return .{ .decoder = decoder };
        }

        /// Build a schema from an optional-value decoder.
        /// Parameters:
        /// - `decoder: comptime fn (std.json.Value) ?A`: Compile-time typed function selected at comptime; no runtime callback
        ///   lookup is required.
        /// - `on_invalid: comptime fn (std.json.Value) E`: Compile-time typed callback; its signature is checked at comptime where
        ///   static.
        /// Returns (`Self`): schema mapping a missing decoded value to a fresh typed `E` failure.
        pub fn fromOptionDecoder(comptime decoder: fn (std.json.Value) ?A, comptime on_invalid: fn (std.json.Value) E) Self {
            return fromDecoder(struct {
                fn decode(value: std.json.Value) outcomes.Result(A, E) {
                    return if (decoder(value)) |a| .{ .ok = a } else .{ .err = on_invalid(value) };
                }
            }.decode);
        }

        /// Borrows the JSON value and its document through decoder execution.
        ///
        /// Parameters:
        /// - `self: comptime Self`: Compile-time current value passed by value; ownership follows this
        ///   operation’s contract.
        /// - `R: comptime type`: Caller-defined environment type supplying the dependencies needed by this operation.
        /// - `json: *const std.json.Value`: JSON value or document to decode, encode, or validate.
        /// Returns (`effects.Effect(A, E, R)`): typed decoded value or schema issue without an exposed Zig error union.
        pub fn decode(comptime self: Self, comptime R: type, json: *const std.json.Value) effects.Effect(A, E, R) {
            return effects.Effect(A, E, R).fromBorrowed(std.json.Value, json, struct {
                fn decode(value: *const std.json.Value, _: *const R) outcomes.Exit(A, E) {
                    return self.decoder(value.*);
                }
            }.decode);
        }

        /// Text lives in R. Parsed strings are copied into the returned document,
        /// making their lifetime explicit without borrowing a temporary buffer.
        ///
        /// Parameters:
        /// - `self: comptime Self`: Compile-time current value passed by value; ownership follows this
        ///   operation’s contract.
        /// - `R: comptime type`: Caller-defined environment type supplying the dependencies needed by this operation.
        /// - `text: comptime fn (*const R) []const u8`: Compile-time typed function selected at comptime; no runtime callback lookup
        ///   is required.
        /// - `on_syntax: comptime fn (std.json.ParseError(std.json.Scanner)) E`: Compile-time typed callback; its signature is checked at comptime where
        ///   static.
        /// Returns (`effects.Effect(Parsed(A), E, R)`): parsed value retaining its source JSON document and allocator ownership.
        pub fn parse(comptime self: Self, comptime R: type, comptime text: fn (*const R) []const u8, comptime on_syntax: fn (std.json.ParseError(std.json.Scanner)) E) effects.Effect(Parsed(A), E, R) {
            return effects.Effect(Parsed(A), E, R).fromOutcome(struct {
                fn parse(env: *const R) outcomes.Exit(Parsed(A), E) {
                    return self.parseBytes(env, text(env), on_syntax);
                }
            }.parse);
        }

        /// Parses a runtime-owned byte slice without requiring that slice to
        /// be installed into R. The document copies strings and remains owned
        /// by Parsed(A); callers may release the source bytes after this call.
        ///
        /// Parameters:
        /// - `self: comptime Self`: Compile-time current value passed by value; ownership follows this
        ///   operation’s contract.
        /// - `env: anytype`: Borrowed caller environment supplying allocator, I/O, and selected
        ///   capabilities.
        /// - `bytes: []const u8`: Borrowed input; keep its owner alive until the operation has copied or
        ///   joined it.
        /// - `on_syntax: comptime fn (std.json.ParseError(std.json.Scanner)) E`: Compile-time typed callback; its signature is checked at comptime where
        ///   static.
        /// Returns (`outcomes.Exit(Parsed(A), E)`): Carries `Parsed(A)` on `.success` or `E` on `.fail`; `.die`, `.interrupt`, and `.cleanup` preserve other terminal causes.
        pub fn parseBytes(comptime self: Self, env: anytype, bytes: []const u8, comptime on_syntax: fn (std.json.ParseError(std.json.Scanner)) E) outcomes.Exit(Parsed(A), E) {
            var document = std.json.parseFromSlice(std.json.Value, env.allocator, bytes, .{ .allocate = .alloc_always }) catch |err| {
                if (err == error.OutOfMemory) return .{ .die = outcomes.Defect.out_of_memory };
                return .{ .fail = on_syntax(err) };
            };
            const value = self.decoder(document.value);
            if (value == .success) {
                const owner = env.allocator.create(Document) catch {
                    var discarded = value;
                    discarded.deinit();
                    document.deinit();
                    return .{ .die = outcomes.Defect.out_of_memory };
                };
                owner.* = .{ .source = document, .allocator = env.allocator };
                return .{ .success = .{ .value = value.success, .document = owner } };
            }
            // Declared errors must own their data or borrow independent input;
            // a failed decoder cannot return document-backed views.
            document.deinit();
            return outcomes.castFailure(Parsed(A), value);
        }
    };
}
