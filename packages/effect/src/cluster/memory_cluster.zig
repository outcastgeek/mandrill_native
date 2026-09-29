//! Volatile logical ownership implementation. The message store is a
//! separate capability; this provider only fences entity activations.
const std = @import("std");
const outcomes = @import("../outcome.zig");
const deadlines = @import("../deadline.zig");
const capabilities = @import("cluster_capabilities.zig");
const messages = @import("message_store.zig");

pub fn MemoryOwnership(comptime R: type, comptime now: fn (*const R) deadlines.WallInstant) type {
    return struct {
        const Self = @This();
        const Service = capabilities.Ownership(R, void, .@"volatile");
        const Failure = Service.Failure;
        const Bytes = messages.Bytes;
        const Item = struct {
            group: Bytes,
            entity_key: Bytes,
            owner_id: Bytes,
            generation: u64,
            until: deadlines.Deadline,
            active: bool,

            fn deinit(self: *Item) void {
                self.group.deinit();
                self.entity_key.deinit();
                self.owner_id.deinit();
                self.* = undefined;
            }
        };
        const AcquireCapture = struct {
            owner: *Self,
            address: capabilities.EntityAddress,
            owner_id: []const u8,
            policy: capabilities.LeasePolicy,
        };
        const RenewCapture = struct { owner: *Self, token: capabilities.Token, policy: capabilities.LeasePolicy };
        const ReleaseCapture = struct { owner: *Self, token: capabilities.Token };
        const ReadCapture = struct { owner: *Self, address: capabilities.EntityAddress };

        allocator: std.mem.Allocator,
        io: std.Io,
        mutex: std.Io.Mutex = .init,
        items: std.array_list.Managed(Item),
        closed: bool = false,

        pub fn init(env: *const R) Self {
            if (R == void or !@hasField(R, "allocator") or @FieldType(R, "allocator") != std.mem.Allocator)
                @compileError("memory cluster ownership requires caller allocator in R");
            if (R == void or !@hasField(R, "io") or @FieldType(R, "io") != std.Io)
                @compileError("memory cluster ownership requires caller I/O in R");
            return .{ .allocator = env.allocator, .io = env.io, .items = std.array_list.Managed(Item).init(env.allocator) };
        }

        pub fn capability(self: *Self) Service {
            return Service.fromProvider(self, &.{ .acquire = acquireEffect, .renew = renewEffect, .release = releaseEffect, .read = readEffect });
        }

        /// Borrowed by a message store. This owner must outlive that store;
        /// every held guard is released before either provider is destroyed.
        pub fn fence(self: *Self) capabilities.Fence {
            return .{ .userdata = self, .enter = enterFence };
        }

        fn enterFence(data: ?*anyopaque, token: capabilities.Token, instant: deadlines.WallInstant) capabilities.Fence.Enter {
            const self: *Self = @ptrCast(@alignCast(data.?));
            self.mutex.lockUncancelable(self.io);
            if (self.closed) {
                self.mutex.unlock(self.io);
                return .unavailable;
            }
            const item = self.find(token.address) orelse {
                self.mutex.unlock(self.io);
                return .stale;
            };
            if (!current(item, token, instant)) {
                self.mutex.unlock(self.io);
                return .stale;
            }
            return .{ .held = .{ .userdata = self, .until = item.until, .release_fn = releaseFence } };
        }

        fn releaseFence(data: ?*anyopaque) void {
            const self: *Self = @ptrCast(@alignCast(data.?));
            self.mutex.unlock(self.io);
        }

        fn copy(allocator: std.mem.Allocator, value: []const u8) error{OutOfMemory}!Bytes {
            return .{ .items = try allocator.dupe(u8, value), .allocator = allocator };
        }

        fn find(self: *Self, address: capabilities.EntityAddress) ?*Item {
            for (self.items.items) |*item| {
                if (std.mem.eql(u8, item.group.items, address.group) and
                    std.mem.eql(u8, item.entity_key.items, address.entity_key)) return item;
            }
            return null;
        }

        fn current(item: *const Item, token: capabilities.Token, instant: deadlines.WallInstant) bool {
            return item.active and item.generation == token.generation and
                std.mem.eql(u8, item.owner_id.items, token.owner_id) and
                !item.until.isDue(instant);
        }

        fn lease(allocator: std.mem.Allocator, item: *const Item) error{OutOfMemory}!capabilities.Lease {
            var group = try copy(allocator, item.group.items);
            errdefer group.deinit();
            var entity_key = try copy(allocator, item.entity_key.items);
            errdefer entity_key.deinit();
            return .{ .group = group, .entity_key = entity_key, .owner_id = try copy(allocator, item.owner_id.items), .generation = item.generation, .until = item.until };
        }

        fn acquireEffect(data: ?*anyopaque, env: *const R, address: capabilities.EntityAddress, owner_id: []const u8, policy: capabilities.LeasePolicy) Service.AcquireEffect {
            const owner: *Self = @ptrCast(@alignCast(data.?));
            return Service.AcquireEffect.fromOwned(env, AcquireCapture{ .owner = owner, .address = address, .owner_id = owner_id, .policy = policy }, struct {
                fn run(capture: *const AcquireCapture, supplied: *const R) outcomes.Exit(capabilities.Acquisition, Failure) {
                    return capture.owner.acquireNow(supplied.allocator, capture.*, now(supplied));
                }
            }.run);
        }

        fn acquireNow(self: *Self, allocator: std.mem.Allocator, request: AcquireCapture, instant: deadlines.WallInstant) outcomes.Exit(capabilities.Acquisition, Failure) {
            if (!request.address.valid() or request.owner_id.len == 0)
                return .{ .fail = .{ .operation = .invalid_identity } };
            if (!request.policy.valid() or request.policy.until.isDue(instant))
                return .{ .fail = .{ .operation = .invalid_deadline } };
            self.mutex.lockUncancelable(self.io);
            defer self.mutex.unlock(self.io);
            if (self.closed) return .{ .fail = .{ .operation = .unavailable } };
            if (self.find(request.address)) |item| {
                if (item.active and !item.until.isDue(instant)) {
                    if (!std.mem.eql(u8, item.owner_id.items, request.owner_id))
                        return .{ .success = .unavailable };
                    const returned = lease(allocator, item) catch return .{ .die = outcomes.Defect.out_of_memory };
                    return .{ .success = .{ .acquired = returned } };
                }
                const generation = std.math.add(u64, item.generation, 1) catch
                    return .{ .fail = .{ .operation = .generation_exhausted } };
                var stored_owner = copy(self.allocator, request.owner_id) catch
                    return .{ .die = outcomes.Defect.out_of_memory };
                var returned_group = copy(allocator, item.group.items) catch {
                    stored_owner.deinit();
                    return .{ .die = outcomes.Defect.out_of_memory };
                };
                var returned_key = copy(allocator, item.entity_key.items) catch {
                    stored_owner.deinit();
                    returned_group.deinit();
                    return .{ .die = outcomes.Defect.out_of_memory };
                };
                const returned_owner = copy(allocator, request.owner_id) catch {
                    stored_owner.deinit();
                    returned_group.deinit();
                    returned_key.deinit();
                    return .{ .die = outcomes.Defect.out_of_memory };
                };
                item.owner_id.deinit();
                item.owner_id = stored_owner;
                item.generation = generation;
                item.until = request.policy.until;
                item.active = true;
                return .{ .success = .{ .acquired = .{ .group = returned_group, .entity_key = returned_key, .owner_id = returned_owner, .generation = generation, .until = request.policy.until } } };
            }
            const first_generation: u64 = 1;
            var group = copy(self.allocator, request.address.group) catch return .{ .die = outcomes.Defect.out_of_memory };
            var entity_key = copy(self.allocator, request.address.entity_key) catch {
                group.deinit();
                return .{ .die = outcomes.Defect.out_of_memory };
            };
            const stored_owner = copy(self.allocator, request.owner_id) catch {
                group.deinit();
                entity_key.deinit();
                return .{ .die = outcomes.Defect.out_of_memory };
            };
            var item = Item{ .group = group, .entity_key = entity_key, .owner_id = stored_owner, .generation = first_generation, .until = request.policy.until, .active = true };
            const returned = lease(allocator, &item) catch {
                item.deinit();
                return .{ .die = outcomes.Defect.out_of_memory };
            };
            self.items.append(item) catch {
                var discarded = returned;
                discarded.deinit();
                item.deinit();
                return .{ .die = outcomes.Defect.out_of_memory };
            };
            return .{ .success = .{ .acquired = returned } };
        }

        fn renewEffect(data: ?*anyopaque, env: *const R, token: capabilities.Token, policy: capabilities.LeasePolicy) Service.RenewEffect {
            const owner: *Self = @ptrCast(@alignCast(data.?));
            return Service.RenewEffect.fromOwned(env, RenewCapture{ .owner = owner, .token = token, .policy = policy }, struct {
                fn run(capture: *const RenewCapture, supplied: *const R) outcomes.Exit(capabilities.Renewal, Failure) {
                    return capture.owner.renewNow(capture.token, capture.policy, now(supplied));
                }
            }.run);
        }

        fn renewNow(self: *Self, token: capabilities.Token, policy: capabilities.LeasePolicy, instant: deadlines.WallInstant) outcomes.Exit(capabilities.Renewal, Failure) {
            if (!token.address.valid() or token.owner_id.len == 0 or token.generation == 0)
                return .{ .fail = .{ .operation = .invalid_identity } };
            if (!policy.valid() or policy.until.isDue(instant))
                return .{ .fail = .{ .operation = .invalid_deadline } };
            self.mutex.lockUncancelable(self.io);
            defer self.mutex.unlock(self.io);
            if (self.closed) return .{ .fail = .{ .operation = .unavailable } };
            const item = self.find(token.address) orelse return .{ .success = .stale };
            if (!current(item, token, instant)) return .{ .success = .stale };
            item.until = policy.until;
            return .{ .success = .renewed };
        }

        fn releaseEffect(data: ?*anyopaque, env: *const R, token: capabilities.Token) Service.ReleaseEffect {
            const owner: *Self = @ptrCast(@alignCast(data.?));
            return Service.ReleaseEffect.fromOwned(env, ReleaseCapture{ .owner = owner, .token = token }, struct {
                fn run(capture: *const ReleaseCapture, _: *const R) outcomes.Exit(capabilities.Release, Failure) {
                    return capture.owner.releaseNow(capture.token);
                }
            }.run);
        }

        fn releaseNow(self: *Self, token: capabilities.Token) outcomes.Exit(capabilities.Release, Failure) {
            if (!token.address.valid() or token.owner_id.len == 0 or token.generation == 0)
                return .{ .fail = .{ .operation = .invalid_identity } };
            self.mutex.lockUncancelable(self.io);
            defer self.mutex.unlock(self.io);
            if (self.closed) return .{ .fail = .{ .operation = .unavailable } };
            const item = self.find(token.address) orelse return .{ .success = .stale };
            if (item.generation != token.generation or
                !std.mem.eql(u8, item.owner_id.items, token.owner_id))
                return .{ .success = .stale };
            if (!item.active) return .{ .success = .already_released };
            item.active = false;
            return .{ .success = .released };
        }

        fn readEffect(data: ?*anyopaque, env: *const R, address: capabilities.EntityAddress) Service.ReadEffect {
            const owner: *Self = @ptrCast(@alignCast(data.?));
            return Service.ReadEffect.fromOwned(env, ReadCapture{ .owner = owner, .address = address }, struct {
                fn run(capture: *const ReadCapture, supplied: *const R) outcomes.Exit(?capabilities.Lease, Failure) {
                    return capture.owner.readNow(supplied.allocator, capture.address, now(supplied));
                }
            }.run);
        }

        fn readNow(self: *Self, allocator: std.mem.Allocator, address: capabilities.EntityAddress, instant: deadlines.WallInstant) outcomes.Exit(?capabilities.Lease, Failure) {
            if (!address.valid()) return .{ .fail = .{ .operation = .invalid_identity } };
            self.mutex.lockUncancelable(self.io);
            defer self.mutex.unlock(self.io);
            if (self.closed) return .{ .fail = .{ .operation = .unavailable } };
            const item = self.find(address) orelse return .{ .success = null };
            if (!item.active or item.until.isDue(instant)) return .{ .success = null };
            return .{ .success = lease(allocator, item) catch
                return .{ .die = outcomes.Defect.out_of_memory } };
        }

        pub fn shutdown(self: *Self) void {
            self.mutex.lockUncancelable(self.io);
            self.closed = true;
            self.mutex.unlock(self.io);
        }

        pub fn deinit(self: *Self) void {
            self.shutdown();
            for (self.items.items) |*item| item.deinit();
            self.items.deinit();
            self.* = undefined;
        }
    };
}
