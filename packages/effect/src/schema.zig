const std = @import("std");
const outcomes = @import("outcome.zig");
const effects = @import("effect.zig");

const Document = struct {
    source: std.json.Parsed(std.json.Value),
    allocator: std.mem.Allocator,
    references: std.atomic.Value(usize) = .init(1),
};

pub fn Parsed(comptime A: type) type {
    return struct {
        pub const effect_owned_payload = true;
        value: A,
        document: *Document,

        /// Takes ownership of an already parsed JSON document. On allocation
        /// failure the caller still owns both arguments and must release them.
        pub fn init(allocator: std.mem.Allocator, source: std.json.Parsed(std.json.Value), value: A) error{OutOfMemory}!@This() {
            const document = try allocator.create(Document);
            document.* = .{ .source = source, .allocator = allocator };
            return .{ .value = value, .document = document };
        }

        /// Decoder views into JSON remain valid until this owner is destroyed.
        /// Independently allocated application payloads keep their own contract.
        pub fn deinit(self: *@This()) void {
            outcomes.discardPayload(&self.value);
            if (self.document.references.fetchSub(1, .acq_rel) == 1) {
                self.document.source.deinit();
                self.document.allocator.destroy(self.document);
            }
            self.* = undefined;
        }

        pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
            const value = try outcomes.clonePayload(self.value);
            _ = self.document.references.fetchAdd(1, .monotonic);
            return .{ .value = value, .document = self.document };
        }

        pub fn json(self: *const @This()) std.json.Value {
            return self.document.source.value;
        }
    };
}

pub fn Schema(comptime A: type, comptime E: type) type {
    return struct {
        const Self = @This();
        pub const Success = A;
        pub const Error = E;
        decoder: *const fn (std.json.Value) outcomes.Exit(A, E),

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

        pub fn fromOutcomeDecoder(comptime decoder: fn (std.json.Value) outcomes.Exit(A, E)) Self {
            return .{ .decoder = decoder };
        }

        pub fn fromOptionDecoder(comptime decoder: fn (std.json.Value) ?A, comptime on_invalid: fn (std.json.Value) E) Self {
            return fromDecoder(struct {
                fn decode(value: std.json.Value) outcomes.Result(A, E) {
                    return if (decoder(value)) |a| .{ .ok = a } else .{ .err = on_invalid(value) };
                }
            }.decode);
        }

        /// Borrows the JSON value and its document through decoder execution.
        pub fn decode(comptime self: Self, comptime R: type, json: *const std.json.Value) effects.Effect(A, E, R) {
            return effects.Effect(A, E, R).fromBorrowed(std.json.Value, json, struct {
                fn decode(value: *const std.json.Value, _: *const R) outcomes.Exit(A, E) {
                    return self.decoder(value.*);
                }
            }.decode);
        }

        /// Text lives in R. Parsed strings are copied into the returned document,
        /// making their lifetime explicit without borrowing a temporary buffer.
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
