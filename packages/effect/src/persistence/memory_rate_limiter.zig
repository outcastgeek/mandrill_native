//! Process-local atomic rate-limit provider. It obeys the public Store
//! contract but does not claim shared or restart-stable enforcement.
const std = @import("std");
const outcomes = @import("../outcome.zig");
const deadlines = @import("../deadline.zig");
const rate = @import("rate_limiter.zig");
const keys = @import("../primary_key.zig");

pub fn MemoryRateLimiter(comptime R: type) type {
    return struct {
        const Self = @This();
        const Store = rate.Store(R, void);
        const Failure = rate.Error(void);
        const FixedState = struct { started: deadlines.WallInstant, used: u64 };
        const BucketState = struct { last: deadlines.WallInstant, tokens: u64, remainder: u64 };
        const AdaptiveState = struct {
            started: deadlines.WallInstant,
            used: u64,
            capacity: u64,
            last_feedback_sequence: u64 = 0,
        };
        const State = union(enum) {
            fixed_window: FixedState,
            token_bucket: BucketState,
            adaptive: AdaptiveState,
        };
        const Entry = struct {
            key: []u8,
            policy: rate.Policy,
            state: State,
        };
        const Capture = struct {
            owner: *Self,
            key: []const u8,
            policy: rate.Policy,
            cost: u64,
            instant: deadlines.WallInstant,
        };
        const FeedbackCapture = struct {
            owner: *Self,
            key: []const u8,
            policy: rate.Policy,
            feedback: rate.Feedback,
            instant: deadlines.WallInstant,
        };

        allocator: std.mem.Allocator,
        io: std.Io,
        mutex: std.Io.Mutex = .init,
        entries: std.StringHashMap(Entry),
        closed: bool = false,

        pub fn init(env: *const R) Self {
            if (R == void or !@hasField(R, "allocator") or @FieldType(R, "allocator") != std.mem.Allocator)
                @compileError("MemoryRateLimiter requires caller allocator in R");
            if (R == void or !@hasField(R, "io") or @FieldType(R, "io") != std.Io)
                @compileError("MemoryRateLimiter requires caller io in R");
            return .{ .allocator = env.allocator, .io = env.io, .entries = std.StringHashMap(Entry).init(env.allocator) };
        }

        pub fn capability(self: *Self) Store {
            return Store.fromProvider(self, &.{ .take = takeEffect, .feedback = feedbackEffect });
        }

        fn takeEffect(data: ?*anyopaque, env: *const R, key: []const u8, policy: rate.Policy, cost: u64, instant: deadlines.WallInstant) Store.TakeEffect {
            const owner: *Self = @ptrCast(@alignCast(data.?));
            return Store.TakeEffect.fromOwned(env, Capture{
                .owner = owner,
                .key = key,
                .policy = policy,
                .cost = cost,
                .instant = instant,
            }, struct {
                fn run(capture: *const Capture, _: *const R) outcomes.Exit(rate.Decision, Failure) {
                    return capture.owner.takeNow(capture.key, capture.policy, capture.cost, capture.instant);
                }
            }.run);
        }

        fn feedbackEffect(data: ?*anyopaque, env: *const R, key: []const u8, policy: rate.Policy, feedback: rate.Feedback, instant: deadlines.WallInstant) Store.FeedbackEffect {
            const owner: *Self = @ptrCast(@alignCast(data.?));
            return Store.FeedbackEffect.fromOwned(env, FeedbackCapture{
                .owner = owner,
                .key = key,
                .policy = policy,
                .feedback = feedback,
                .instant = instant,
            }, struct {
                fn run(capture: *const FeedbackCapture, _: *const R) outcomes.Exit(rate.FeedbackResult, Failure) {
                    return capture.owner.feedbackNow(capture.key, capture.policy, capture.feedback, capture.instant);
                }
            }.run);
        }

        fn initialState(policy: rate.Policy, instant: deadlines.WallInstant) State {
            return switch (policy.rule) {
                .fixed_window => .{ .fixed_window = .{ .started = instant, .used = 0 } },
                .token_bucket => |policy_bucket| .{ .token_bucket = .{
                    .last = instant,
                    .tokens = policy_bucket.capacity,
                    .remainder = 0,
                } },
                .adaptive => |adaptive_policy| .{ .adaptive = .{
                    .started = instant,
                    .used = 0,
                    .capacity = adaptive_policy.initial_capacity,
                } },
            };
        }

        fn elapsed(now: deadlines.WallInstant, started: deadlines.WallInstant) outcomes.Exit(u128, Failure) {
            if (now.unix_nanoseconds <= started.unix_nanoseconds) return .{ .success = 0 };
            const difference = std.math.sub(i128, now.unix_nanoseconds, started.unix_nanoseconds) catch
                return .{ .fail = .time_overflow };
            return .{ .success = @intCast(difference) };
        }

        fn fixed(policy: rate.FixedWindow, state: *FixedState, cost: u64, instant: deadlines.WallInstant) outcomes.Exit(rate.Decision, Failure) {
            const elapsed_result = elapsed(instant, state.started);
            const age = switch (elapsed_result) {
                .success => |value| value,
                else => return outcomes.castFailure(rate.Decision, elapsed_result),
            };
            if (age >= policy.window.nanoseconds) {
                state.started = instant;
                state.used = 0;
            }
            const available = policy.capacity - state.used;
            if (cost <= available) {
                state.used += cost;
                return .{ .success = .{ .allowed = .{ .remaining = policy.capacity - state.used } } };
            }
            const retry_ns: u64 = @intCast(@as(u128, policy.window.nanoseconds) - @min(age, @as(u128, policy.window.nanoseconds)));
            return .{ .success = .{ .limited = .{ .retry_after = .{ .nanoseconds = retry_ns } } } };
        }

        fn bucket(policy: rate.TokenBucket, state: *BucketState, cost: u64, instant: deadlines.WallInstant) outcomes.Exit(rate.Decision, Failure) {
            const elapsed_result = elapsed(instant, state.last);
            const age = switch (elapsed_result) {
                .success => |value| value,
                else => return outcomes.castFailure(rate.Decision, elapsed_result),
            };
            if (age > 0 and state.tokens < policy.capacity) {
                const missing = policy.capacity - state.tokens;
                const complete_periods = age / policy.refill_every.nanoseconds;
                const periods_to_fill = (@as(u128, missing) + policy.refill_tokens - 1) / policy.refill_tokens;
                if (complete_periods >= periods_to_fill) {
                    state.tokens = policy.capacity;
                    state.remainder = 0;
                } else {
                    const product = std.math.mul(u128, age, policy.refill_tokens) catch std.math.maxInt(u128);
                    const units = std.math.add(u128, product, state.remainder) catch std.math.maxInt(u128);
                    const added = units / policy.refill_every.nanoseconds;
                    if (added >= missing) {
                        state.tokens = policy.capacity;
                        state.remainder = 0;
                    } else {
                        state.tokens += @intCast(added);
                        state.remainder = @intCast(units % policy.refill_every.nanoseconds);
                    }
                }
            } else if (state.tokens == policy.capacity) {
                state.remainder = 0;
            }
            if (instant.unix_nanoseconds > state.last.unix_nanoseconds) state.last = instant;
            if (cost <= state.tokens) {
                state.tokens -= cost;
                return .{ .success = .{ .allowed = .{ .remaining = state.tokens } } };
            }
            const missing = cost - state.tokens;
            const required = @as(u128, missing) * policy.refill_every.nanoseconds - state.remainder;
            const whole = required / policy.refill_tokens;
            const rounded = whole + @intFromBool(required % policy.refill_tokens != 0);
            if (rounded > std.math.maxInt(u64)) return .{ .fail = .time_overflow };
            return .{ .success = .{ .limited = .{ .retry_after = .{ .nanoseconds = @intCast(rounded) } } } };
        }

        fn adaptive(policy: rate.Adaptive, state: *AdaptiveState, cost: u64, instant: deadlines.WallInstant) outcomes.Exit(rate.Decision, Failure) {
            const elapsed_result = elapsed(instant, state.started);
            const age = switch (elapsed_result) {
                .success => |value| value,
                else => return outcomes.castFailure(rate.Decision, elapsed_result),
            };
            if (age >= policy.window.nanoseconds) {
                state.started = instant;
                state.used = 0;
            }
            const available = state.capacity -| state.used;
            if (cost <= available) {
                state.used += cost;
                return .{ .success = .{ .allowed = .{ .remaining = state.capacity - state.used } } };
            }
            const retry_ns: u64 = @intCast(@as(u128, policy.window.nanoseconds) -
                @min(age, @as(u128, policy.window.nanoseconds)));
            return .{ .success = .{ .limited = .{ .retry_after = .{ .nanoseconds = retry_ns } } } };
        }

        fn apply(policy: rate.Policy, state: *State, cost: u64, instant: deadlines.WallInstant) outcomes.Exit(rate.Decision, Failure) {
            return switch (policy.rule) {
                .fixed_window => |rule| fixed(rule, &state.fixed_window, cost, instant),
                .token_bucket => |rule| bucket(rule, &state.token_bucket, cost, instant),
                .adaptive => |rule| adaptive(rule, &state.adaptive, cost, instant),
            };
        }

        fn applyExisting(existing: *Entry, policy: rate.Policy, cost: u64, instant: deadlines.WallInstant) outcomes.Exit(rate.Decision, Failure) {
            if (existing.policy.revision > policy.revision) return .{ .fail = .stale_policy };
            if (existing.policy.revision == policy.revision and !std.meta.eql(existing.policy.rule, policy.rule))
                return .{ .fail = .invalid_policy };
            var next = if (existing.policy.revision == policy.revision) existing.state else initialState(policy, instant);
            const decision = apply(policy, &next, cost, instant);
            if (decision == .success) {
                existing.policy = policy;
                existing.state = next;
            }
            return decision;
        }

        fn takeNow(self: *Self, key: []const u8, policy: rate.Policy, cost: u64, instant: deadlines.WallInstant) outcomes.Exit(rate.Decision, Failure) {
            if (!policy.valid() or cost == 0 or cost > policy.capacity()) return .{ .fail = .invalid_policy };
            if (keys.inspectEncoded(key) == .err) return .{ .fail = .invalid_key };
            self.mutex.lockUncancelable(self.io);
            if (self.closed) {
                self.mutex.unlock(self.io);
                return .{ .fail = .unavailable };
            }
            if (self.entries.getPtr(key)) |existing| {
                const result = applyExisting(existing, policy, cost, instant);
                self.mutex.unlock(self.io);
                return result;
            }
            self.mutex.unlock(self.io);
            const owned_key = self.allocator.dupe(u8, key) catch return .{ .die = outcomes.Defect.out_of_memory };
            var keep_key = false;
            defer if (!keep_key) self.allocator.free(owned_key);
            self.mutex.lockUncancelable(self.io);
            defer self.mutex.unlock(self.io);
            if (self.closed) return .{ .fail = .unavailable };
            if (self.entries.getPtr(key)) |existing| {
                return applyExisting(existing, policy, cost, instant);
            }
            var next = initialState(policy, instant);
            const decision = apply(policy, &next, cost, instant);
            if (decision != .success) return decision;
            self.entries.ensureUnusedCapacity(1) catch return .{ .die = outcomes.Defect.out_of_memory };
            self.entries.putAssumeCapacity(owned_key, .{ .key = owned_key, .policy = policy, .state = next });
            keep_key = true;
            return decision;
        }

        fn feedbackNow(self: *Self, key: []const u8, policy: rate.Policy, feedback: rate.Feedback, instant: deadlines.WallInstant) outcomes.Exit(rate.FeedbackResult, Failure) {
            if (!policy.valid() or policy.rule != .adaptive)
                return .{ .fail = .invalid_policy };
            if (feedback.sequence == 0) return .{ .fail = .invalid_feedback };
            if (keys.inspectEncoded(key) == .err) return .{ .fail = .invalid_key };
            self.mutex.lockUncancelable(self.io);
            if (self.closed) {
                self.mutex.unlock(self.io);
                return .{ .fail = .unavailable };
            }
            if (self.entries.getPtr(key)) |existing| {
                const result = applyFeedback(existing, policy, feedback, instant);
                self.mutex.unlock(self.io);
                return result;
            }
            self.mutex.unlock(self.io);
            const owned_key = self.allocator.dupe(u8, key) catch
                return .{ .die = outcomes.Defect.out_of_memory };
            var keep_key = false;
            defer if (!keep_key) self.allocator.free(owned_key);
            self.mutex.lockUncancelable(self.io);
            defer self.mutex.unlock(self.io);
            if (self.closed) return .{ .fail = .unavailable };
            if (self.entries.getPtr(key)) |existing|
                return applyFeedback(existing, policy, feedback, instant);
            self.entries.ensureUnusedCapacity(1) catch
                return .{ .die = outcomes.Defect.out_of_memory };
            self.entries.putAssumeCapacity(owned_key, .{
                .key = owned_key,
                .policy = policy,
                .state = initialState(policy, instant),
            });
            keep_key = true;
            return applyFeedback(self.entries.getPtr(key).?, policy, feedback, instant);
        }

        fn applyFeedback(entry: *Entry, policy: rate.Policy, feedback: rate.Feedback, instant: deadlines.WallInstant) outcomes.Exit(rate.FeedbackResult, Failure) {
            if (entry.policy.revision > policy.revision) return .{ .fail = .stale_policy };
            if (entry.policy.revision == policy.revision and
                !std.meta.eql(entry.policy.rule, policy.rule))
                return .{ .fail = .invalid_policy };
            if (entry.policy.revision != policy.revision) {
                entry.policy = policy;
                entry.state = initialState(policy, instant);
            }
            const state = &entry.state.adaptive;
            if (feedback.sequence <= state.last_feedback_sequence)
                return .{ .success = .{ .ignored_stale = state.capacity } };
            state.last_feedback_sequence = feedback.sequence;
            const rule = policy.rule.adaptive;
            state.capacity = switch (feedback.signal) {
                .healthy => @min(rule.maximum_capacity, std.math.add(u64, state.capacity, rule.increase_by) catch rule.maximum_capacity),
                .overloaded => @max(rule.minimum_capacity, state.capacity -| rule.decrease_by),
            };
            return .{ .success = .{ .applied = state.capacity } };
        }

        pub fn shutdown(self: *Self) void {
            self.mutex.lockUncancelable(self.io);
            self.closed = true;
            self.mutex.unlock(self.io);
        }

        pub fn deinit(self: *Self) void {
            self.shutdown();
            var iterator = self.entries.iterator();
            while (iterator.next()) |entry| self.allocator.free(entry.value_ptr.key);
            self.entries.deinit();
            self.* = undefined;
        }
    };
}
