//! Volatile branch and race replay authority. Branch outcomes and winners are
//! immutable once recorded, so reruns observe the same full encoded Exit.
const std = @import("std");
const outcomes = @import("../outcome.zig");
const types = @import("workflow_types.zig");
const parallel = @import("durable_parallel.zig");

/// Construct an in-memory parallel journal provider with caller-owned runtime resources.
/// Parameters:
/// - `R: comptime type`: Caller-defined environment type that supplies dependencies and runtime capabilities when work executes.
/// Returns (`type`): in-memory provider for durable parallel branch records.
pub fn MemoryParallelJournal(comptime R: type) type {
    return struct {
        const Self = @This();
        const Journal = parallel.Journal(R, void, .@"volatile");
        const Failure = Journal.Failure;
        const Bytes = outcomes.OwnedSlice(u8);
        const BranchItem = struct {
            execution: Bytes,
            name: Bytes,
            version: u32,
            attempt: u64,
            encoded_exit: Bytes,

            fn deinit(self: *@This()) void {
                self.execution.deinit();
                self.name.deinit();
                self.encoded_exit.deinit();
                self.* = undefined;
            }
        };
        const WinnerItem = struct {
            execution: Bytes,
            race_name: Bytes,
            branch_name: Bytes,
            branch_version: u32,
            encoded_exit: Bytes,

            fn deinit(self: *@This()) void {
                self.execution.deinit();
                self.race_name.deinit();
                self.branch_name.deinit();
                self.encoded_exit.deinit();
                self.* = undefined;
            }
        };
        const BranchCapture = struct { owner: *Self, branch: parallel.Branch };
        const CommitBranchCapture = struct { owner: *Self, branch: parallel.Branch, attempt: u64, encoded_exit: []const u8 };
        const RaceCapture = struct { owner: *Self, execution: types.ExecutionId, race_name: []const u8 };
        const CommitWinnerCapture = struct { owner: *Self, execution: types.ExecutionId, race_name: []const u8, branch: parallel.Branch, encoded_exit: []const u8 };

        allocator: std.mem.Allocator,
        io: std.Io,
        mutex: std.Io.Mutex = .init,
        branches: std.array_list.Managed(BranchItem),
        winners: std.array_list.Managed(WinnerItem),
        closed: bool = false,

        /// Create an in-memory parallel-work journal using the caller environment.
        /// Parameters:
        /// - `env: *const R`: Borrowed caller environment supplying allocator, I/O, and selected
        ///   capabilities.
        /// Returns (`Self`): a parallel-work journal bound to the supplied environment.
        pub fn init(env: *const R) Self {
            if (R == void or !@hasField(R, "allocator") or
                @FieldType(R, "allocator") != std.mem.Allocator)
                @compileError("memory parallel journal requires R.allocator");
            if (R == void or !@hasField(R, "io") or @FieldType(R, "io") != std.Io)
                @compileError("memory parallel journal requires R.io");
            return .{ .allocator = env.allocator, .io = env.io, .branches = std.array_list.Managed(BranchItem).init(env.allocator), .winners = std.array_list.Managed(WinnerItem).init(env.allocator) };
        }

        /// Expose this implementation through its capability interface.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// Returns (`Journal`): A capability adapter referencing the selected implementation; keep its backing state alive while in use.
        pub fn capability(self: *Self) Journal {
            return Journal.fromProvider(self, &.{
                .lookup_branch = lookupBranchEffect,
                .commit_branch = commitBranchEffect,
                .lookup_winner = lookupWinnerEffect,
                .commit_winner = commitWinnerEffect,
            });
        }

        fn copy(allocator: std.mem.Allocator, input: []const u8) error{OutOfMemory}!Bytes {
            return .{ .items = try allocator.dupe(u8, input), .allocator = allocator };
        }

        fn findBranch(self: *Self, branch: parallel.Branch) ?*BranchItem {
            for (self.branches.items) |*item| {
                if (item.version == branch.version and
                    std.mem.eql(u8, item.execution.items, branch.execution.encoded) and
                    std.mem.eql(u8, item.name.items, branch.name)) return item;
            }
            return null;
        }

        fn findWinner(self: *Self, execution: types.ExecutionId, race_name: []const u8) ?*WinnerItem {
            for (self.winners.items) |*item| {
                if (std.mem.eql(u8, item.execution.items, execution.encoded) and
                    std.mem.eql(u8, item.race_name.items, race_name)) return item;
            }
            return null;
        }

        fn lookupBranchEffect(data: ?*anyopaque, env: *const R, branch: parallel.Branch) Journal.LookupBranchEffect {
            const owner: *Self = @ptrCast(@alignCast(data.?));
            return Journal.LookupBranchEffect.fromOwned(env, BranchCapture{
                .owner = owner,
                .branch = branch,
            }, struct {
                fn run(capture: *const BranchCapture, supplied: *const R) outcomes.Exit(?parallel.BranchResult, Failure) {
                    return capture.owner.lookupBranchNow(supplied.allocator, capture.branch);
                }
            }.run);
        }

        fn lookupBranchNow(self: *Self, allocator: std.mem.Allocator, branch: parallel.Branch) outcomes.Exit(?parallel.BranchResult, Failure) {
            if (!branch.valid()) return .{ .fail = .{ .operation = .invalid_branch } };
            self.mutex.lockUncancelable(self.io);
            defer self.mutex.unlock(self.io);
            if (self.closed) return .{ .fail = .{ .operation = .unavailable } };
            const item = self.findBranch(branch) orelse return .{ .success = null };
            return .{ .success = .{ .attempt = item.attempt, .encoded_exit = copy(allocator, item.encoded_exit.items) catch
                return .{ .die = outcomes.Defect.out_of_memory } } };
        }

        fn commitBranchEffect(data: ?*anyopaque, env: *const R, branch: parallel.Branch, attempt: u64, encoded_exit: []const u8) Journal.CommitEffect {
            const owner: *Self = @ptrCast(@alignCast(data.?));
            return Journal.CommitEffect.fromOwned(env, CommitBranchCapture{
                .owner = owner,
                .branch = branch,
                .attempt = attempt,
                .encoded_exit = encoded_exit,
            }, struct {
                fn run(capture: *const CommitBranchCapture, _: *const R) outcomes.Exit(parallel.Commit, Failure) {
                    return capture.owner.commitBranchNow(capture.branch, capture.attempt, capture.encoded_exit);
                }
            }.run);
        }

        fn commitBranchNow(self: *Self, branch: parallel.Branch, attempt: u64, encoded_exit: []const u8) outcomes.Exit(parallel.Commit, Failure) {
            if (!branch.valid() or attempt == 0)
                return .{ .fail = .{ .operation = .invalid_branch } };
            var execution = copy(self.allocator, branch.execution.encoded) catch
                return .{ .die = outcomes.Defect.out_of_memory };
            var keep_execution = false;
            defer if (!keep_execution) execution.deinit();
            var name = copy(self.allocator, branch.name) catch
                return .{ .die = outcomes.Defect.out_of_memory };
            var keep_name = false;
            defer if (!keep_name) name.deinit();
            var encoded = copy(self.allocator, encoded_exit) catch
                return .{ .die = outcomes.Defect.out_of_memory };
            var keep_encoded = false;
            defer if (!keep_encoded) encoded.deinit();
            self.mutex.lockUncancelable(self.io);
            defer self.mutex.unlock(self.io);
            if (self.closed) return .{ .fail = .{ .operation = .unavailable } };
            if (self.findBranch(branch)) |existing| {
                if (attempt < existing.attempt) return .{ .success = .stale_attempt };
                if (attempt == existing.attempt and
                    std.mem.eql(u8, existing.encoded_exit.items, encoded_exit))
                    return .{ .success = .already_recorded };
                return .{ .success = .conflicting_result };
            }
            self.branches.append(.{ .execution = execution, .name = name, .version = branch.version, .attempt = attempt, .encoded_exit = encoded }) catch
                return .{ .die = outcomes.Defect.out_of_memory };
            keep_execution = true;
            keep_name = true;
            keep_encoded = true;
            return .{ .success = .recorded };
        }

        fn lookupWinnerEffect(data: ?*anyopaque, env: *const R, execution: types.ExecutionId, race_name: []const u8) Journal.LookupWinnerEffect {
            const owner: *Self = @ptrCast(@alignCast(data.?));
            return Journal.LookupWinnerEffect.fromOwned(env, RaceCapture{
                .owner = owner,
                .execution = execution,
                .race_name = race_name,
            }, struct {
                fn run(capture: *const RaceCapture, supplied: *const R) outcomes.Exit(?parallel.Winner, Failure) {
                    return capture.owner.lookupWinnerNow(supplied.allocator, capture.execution, capture.race_name);
                }
            }.run);
        }

        fn validRace(execution: types.ExecutionId, race_name: []const u8) bool {
            return execution.validate() == .ok and race_name.len > 0 and
                std.unicode.utf8ValidateSlice(race_name);
        }

        fn lookupWinnerNow(self: *Self, allocator: std.mem.Allocator, execution: types.ExecutionId, race_name: []const u8) outcomes.Exit(?parallel.Winner, Failure) {
            if (!validRace(execution, race_name))
                return .{ .fail = .{ .operation = .invalid_branch } };
            self.mutex.lockUncancelable(self.io);
            defer self.mutex.unlock(self.io);
            if (self.closed) return .{ .fail = .{ .operation = .unavailable } };
            const item = self.findWinner(execution, race_name) orelse
                return .{ .success = null };
            var branch_name = copy(allocator, item.branch_name.items) catch
                return .{ .die = outcomes.Defect.out_of_memory };
            const encoded = copy(allocator, item.encoded_exit.items) catch {
                branch_name.deinit();
                return .{ .die = outcomes.Defect.out_of_memory };
            };
            return .{ .success = .{ .branch_name = branch_name, .branch_version = item.branch_version, .encoded_exit = encoded } };
        }

        fn commitWinnerEffect(data: ?*anyopaque, env: *const R, execution: types.ExecutionId, race_name: []const u8, branch: parallel.Branch, encoded_exit: []const u8) Journal.CommitEffect {
            const owner: *Self = @ptrCast(@alignCast(data.?));
            return Journal.CommitEffect.fromOwned(env, CommitWinnerCapture{
                .owner = owner,
                .execution = execution,
                .race_name = race_name,
                .branch = branch,
                .encoded_exit = encoded_exit,
            }, struct {
                fn run(capture: *const CommitWinnerCapture, _: *const R) outcomes.Exit(parallel.Commit, Failure) {
                    return capture.owner.commitWinnerNow(capture.execution, capture.race_name, capture.branch, capture.encoded_exit);
                }
            }.run);
        }

        fn commitWinnerNow(self: *Self, execution: types.ExecutionId, race_name: []const u8, branch: parallel.Branch, encoded_exit: []const u8) outcomes.Exit(parallel.Commit, Failure) {
            if (!validRace(execution, race_name) or !branch.valid() or
                !std.mem.eql(u8, branch.execution.encoded, execution.encoded))
                return .{ .fail = .{ .operation = .invalid_branch } };
            var execution_copy = copy(self.allocator, execution.encoded) catch
                return .{ .die = outcomes.Defect.out_of_memory };
            var keep_execution = false;
            defer if (!keep_execution) execution_copy.deinit();
            var race_copy = copy(self.allocator, race_name) catch
                return .{ .die = outcomes.Defect.out_of_memory };
            var keep_race = false;
            defer if (!keep_race) race_copy.deinit();
            var branch_copy = copy(self.allocator, branch.name) catch
                return .{ .die = outcomes.Defect.out_of_memory };
            var keep_branch = false;
            defer if (!keep_branch) branch_copy.deinit();
            var encoded = copy(self.allocator, encoded_exit) catch
                return .{ .die = outcomes.Defect.out_of_memory };
            var keep_encoded = false;
            defer if (!keep_encoded) encoded.deinit();
            self.mutex.lockUncancelable(self.io);
            defer self.mutex.unlock(self.io);
            if (self.closed) return .{ .fail = .{ .operation = .unavailable } };
            if (self.findWinner(execution, race_name)) |existing| {
                if (existing.branch_version == branch.version and
                    std.mem.eql(u8, existing.branch_name.items, branch.name) and
                    std.mem.eql(u8, existing.encoded_exit.items, encoded_exit))
                    return .{ .success = .already_recorded };
                return .{ .success = .conflicting_result };
            }
            self.winners.append(.{ .execution = execution_copy, .race_name = race_copy, .branch_name = branch_copy, .branch_version = branch.version, .encoded_exit = encoded }) catch
                return .{ .die = outcomes.Defect.out_of_memory };
            keep_execution = true;
            keep_race = true;
            keep_branch = true;
            keep_encoded = true;
            return .{ .success = .recorded };
        }

        /// Stop the capability and release its retained resources.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// Returns (`void`): no value.
        pub fn shutdown(self: *Self) void {
            self.mutex.lockUncancelable(self.io);
            self.closed = true;
            self.mutex.unlock(self.io);
        }

        /// Release resources owned by this value.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// Returns (`void`): no value.
        pub fn deinit(self: *Self) void {
            self.shutdown();
            for (self.branches.items) |*item| item.deinit();
            for (self.winners.items) |*item| item.deinit();
            self.branches.deinit();
            self.winners.deinit();
            self.* = undefined;
        }
    };
}
