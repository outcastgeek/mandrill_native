//! Journal-authoritative replica import. Stable event IDs make append retry
//! safe after uncertain acknowledgements; source progress advances only after
//! every event in the batch is known committed.
const std = @import("std");
const outcomes = @import("../outcome.zig");
const effects = @import("../effect.zig");
const journal = @import("event_journal.zig");
const message = @import("event_log_message.zig");
const remote = @import("event_log_remote.zig");

pub fn Failure(comptime JournalError: type, comptime CursorError: type) type {
    return union(enum) {
        const Self = @This();
        pub const effect_owned_payload = true;
        journal: JournalError,
        cursor: CursorError,
        invalid_batch,
        wrong_destination,
        rejected_append,

        pub fn deinit(self: *Self) void {
            if (self.* == .journal) outcomes.discardPayload(&self.journal);
            if (self.* == .cursor) outcomes.discardPayload(&self.cursor);
            self.* = undefined;
        }

        pub fn clone(self: *const Self) error{OutOfMemory}!Self {
            return switch (self.*) {
                .journal => |value| .{ .journal = try outcomes.clonePayload(value) },
                .cursor => |value| .{ .cursor = try outcomes.clonePayload(value) },
                else => self.*,
            };
        }
    };
}

pub fn Importer(
    comptime R: type,
    comptime JournalProviderError: type,
    comptime CursorProviderError: type,
    comptime select_journal: fn (*const R) *const journal.Journal(R, JournalProviderError),
    comptime select_cursor: fn (*const R) *const remote.CursorStore(R, CursorProviderError),
) type {
    return struct {
        const Self = @This();
        const Journal = journal.Journal(R, JournalProviderError);
        const CursorStore = remote.CursorStore(R, CursorProviderError);
        const Error = Failure(Journal.Failure, CursorStore.Failure);
        const Out = effects.Effect(message.ImportResult, Error, R);

        fn journalFailure(problem: Journal.Failure) Error {
            return .{ .journal = problem };
        }
        fn cursorFailure(problem: CursorStore.Failure) Error {
            return .{ .cursor = problem };
        }

        fn result(env: *const R, value: message.ImportResult) Out {
            return Out.fromOwnedSuccess(env, value);
        }

        pub fn importBatch(
            comptime select_batch: fn (*const R) *const message.Batch,
            comptime select_local: fn (*const R) []const u8,
            comptime select_bounds: fn (*const R) message.Bounds,
        ) Out {
            return Out.deferEffect(struct {
                fn start(env: *const R) Out {
                    const batch = select_batch(env);
                    if (!batch.valid(select_bounds(env))) return Out.fail(.invalid_batch);
                    if (!std.mem.eql(u8, batch.destination.items, select_local(env))) return Out.fail(.wrong_destination);
                    const store = select_cursor(env);
                    var reading = store.vtable.read(store.userdata, env, batch.source.items);
                    var mapped = reading.mapErrorOwned(env, cursorFailure);
                    return mapped.flatMapCapturedOwned(env, message.ImportResult, batch, struct {
                        fn next(captured: *const *const message.Batch, current: journal.Cursor, supplied: *const R) Out {
                            const input = captured.*;
                            if (current.sequence < input.after.sequence) return result(supplied, .{ .gap = .{
                                .expected_after = current,
                                .received_after = input.after,
                            } });
                            if (current.sequence > input.after.sequence) {
                                if (current.sequence < input.page.next.sequence)
                                    return result(supplied, .{ .conflicting_history = current });
                                return verifyIndex(input, 0, current, supplied);
                            }
                            return appendIndex(input, 0, current, supplied);
                        }
                    }.next);
                }
            }.start);
        }

        const Position = struct { batch: *const message.Batch, index: usize, source_cursor: journal.Cursor };

        fn verifyIndex(batch: *const message.Batch, index: usize, source_cursor: journal.Cursor, env: *const R) Out {
            if (index == batch.page.entries.items.len) return result(env, .{ .duplicate = source_cursor });
            const entry = &batch.page.entries.items[index];
            const provider = select_journal(env);
            var finding = provider.vtable.find_event(provider.userdata, env, entry.event_id.items);
            var mapped = finding.mapErrorOwned(env, journalFailure);
            return mapped.flatMapCapturedOwned(env, message.ImportResult, Position{
                .batch = batch,
                .index = index,
                .source_cursor = source_cursor,
            }, struct {
                fn next(position: *const Position, existing: ?journal.Entry, supplied: *const R) Out {
                    var owned = existing orelse return result(supplied, .{ .conflicting_history = position.source_cursor });
                    defer owned.deinit();
                    const expected = &position.batch.page.entries.items[position.index];
                    if (!std.mem.eql(u8, owned.tag.items, expected.tag.items) or
                        owned.schema_version != expected.schema_version or
                        !std.mem.eql(u8, owned.encoded.items, expected.encoded.items))
                        return result(supplied, .{ .conflicting_history = position.source_cursor });
                    return verifyIndex(position.batch, position.index + 1, position.source_cursor, supplied);
                }
            }.next);
        }

        fn appendIndex(batch: *const message.Batch, index: usize, source_cursor: journal.Cursor, env: *const R) Out {
            if (index == batch.page.entries.items.len) return advance(batch, source_cursor, env);
            const entry = &batch.page.entries.items[index];
            const provider = select_journal(env);
            var appending = provider.vtable.append(provider.userdata, env, .{
                .tag = entry.tag.items,
                .event_id = entry.event_id.items,
                .encoded = entry.encoded.items,
                .schema_version = entry.schema_version,
            });
            var mapped = appending.mapErrorOwned(env, journalFailure);
            return mapped.flatMapCapturedOwned(env, message.ImportResult, Position{
                .batch = batch,
                .index = index,
                .source_cursor = source_cursor,
            }, struct {
                fn next(position: *const Position, acknowledgement: journal.AppendAcknowledgement, supplied: *const R) Out {
                    return switch (acknowledgement) {
                        .committed => appendIndex(position.batch, position.index + 1, position.source_cursor, supplied),
                        .accepted, .uncertain => result(supplied, .{ .uncertain = position.source_cursor }),
                        .rejected => Out.fail(.rejected_append),
                    };
                }
            }.next);
        }

        fn advance(batch: *const message.Batch, source_cursor: journal.Cursor, env: *const R) Out {
            const store = select_cursor(env);
            var advancing = store.vtable.compare_and_advance(store.userdata, env, batch.source.items, batch.after, batch.page.next);
            var mapped = advancing.mapErrorOwned(env, cursorFailure);
            return mapped.flatMapCapturedOwned(env, message.ImportResult, Position{
                .batch = batch,
                .index = batch.page.entries.items.len,
                .source_cursor = source_cursor,
            }, struct {
                fn next(position: *const Position, status: remote.Advance, supplied: *const R) Out {
                    return switch (status) {
                        .committed => |cursor| result(supplied, .{ .applied = cursor }),
                        .already_advanced => |cursor| if (cursor.sequence == position.batch.page.next.sequence)
                            result(supplied, .{ .duplicate = cursor })
                        else
                            result(supplied, .{ .conflicting_history = cursor }),
                        .conflict => |cursor| result(supplied, .{ .conflicting_history = cursor }),
                        .possibly_committed => result(supplied, .{ .uncertain = position.source_cursor }),
                    };
                }
            }.next);
        }
    };
}
