//! COMP-09/10: ordinary Effect composition across the existing domain boundaries.
const std = @import("std");
const fx = @import("effect");
const invoices = @import("workflow.zig");
const prices = @import("atoms.zig");
const projections = @import("activation_projection.zig");
const withdrawals = @import("withdrawal.zig");
const rpc = @import("rpc_session.zig");
const Error = enum { schema, transaction, workflow, atom, activation, rpc };
const R = struct {
    allocator: std.mem.Allocator,
    io: std.Io,
    workflow: invoices.R,
    atom: prices.R,
    activation: projections.R,
    transaction: withdrawals.Environment,
    primary: *fx.TransactionRef(u32),
    reserve: *fx.TransactionRef(u32),
    audit: *fx.TransactionRef(u32),
    rpc: rpc.Root,
};
const Out = fx.Effect(i64, Error, R);
const Record = fx.dsl.schemaRecord(R, .{ .fields = .{ .amount = .{ .codec = fx.JsonCodec.signed_integer, .presence = .required } }, .policy = .{ .unknown_fields = .reject, .duplicate_fields = .reject, .issues = .first } });
const Roles = struct {
    fn text(input: anytype, _: *const R) []const u8 {
        return input.text;
    }
    fn schemaError(problem: fx.SchemaIssue.Issue) Error {
        var owned = problem;
        defer owned.deinit();
        return .schema;
    }
    fn parse(input: anytype, env: *const R) fx.Effect(fx.Parsed(Record.Decoded), Error, R) {
        return fx.dsl.parseSchema(env, .{ .text = input.text }, .{ .schema = Record, .text = text, .limits = .{ .max_document_bytes = 128, .max_depth = 4, .max_value_bytes = 32 }, .map_error = schemaError });
    }
    fn transactionEnv(env: *const R) *const withdrawals.Environment {
        return &env.transaction;
    }
    fn workflowEnv(env: *const R) *const invoices.R {
        return &env.workflow;
    }
    fn atomEnv(env: *const R) *const prices.R {
        return &env.atom;
    }
    fn activationEnv(env: *const R) *const projections.R {
        return &env.activation;
    }
    fn rpcEnv(env: *const R) *const rpc.Root {
        return &env.rpc;
    }
    fn anyTransactionError(problem: anytype) Error {
        var owned = problem;
        defer owned.deinit();
        return .transaction;
    }
    fn workflowError(_: invoices.E) Error {
        return .workflow;
    }
    fn atomError(_: anytype) Error {
        return .atom;
    }
    fn activationError(_: projections.E) Error {
        return .activation;
    }
    fn rpcError(problem: anytype) Error {
        var owned = problem;
        defer owned.deinit();
        return .rpc;
    }
    fn withdraw(parsed: *const fx.Parsed(Record.Decoded), env: *const R) fx.Effect(u32, Error, R) {
        const amount = Record.get("amount", &parsed.value).*;
        if (amount < 0 or amount > std.math.maxInt(u32)) return fx.Effect(u32, Error, R).fail(.schema);
        var described = withdrawals.withdrawal(&env.transaction, .{ .primary = env.primary, .reserve = env.reserve, .audit = env.audit, .amount = @as(u32, @intCast(amount)) });
        var projected = described.provideSomeAllocated(env.allocator, R, transactionEnv);
        return projected.mapErrorOwned(env, anyTransactionError);
    }
    fn recorded(_: *const u32, env: *const R) fx.Effect(fx.OwnedSlice(fx.WorkflowRecordedActivity(i64, void)), Error, R) {
        var described = invoices.invoice(&env.workflow, .{ .rows = &.{.{ .id = "integration-line", .amount = 9 }} });
        var projected = described.provideSomeAllocated(env.allocator, R, workflowEnv);
        return projected.mapErrorOwned(env, workflowError);
    }
    fn atom(_: *const fx.OwnedSlice(fx.WorkflowRecordedActivity(i64, void)), env: *const R) fx.Effect(fx.AtomProgram.Published(u32), Error, R) {
        var described = prices.price();
        var projected = described.provideSomeAllocated(env.allocator, R, atomEnv);
        return projected.mapErrorOwned(env, atomError);
    }
    fn projection(input: anytype, _: *const fx.AtomProgram.Published(u32), env: *const R) Out {
        var described = projections.project(&env.activation, .{ .id = input.id, .generation = env.activation.generation, .history = input.page });
        var projected = described.provideSomeAllocated(env.allocator, R, activationEnv);
        return projected.mapErrorOwned(env, activationError);
    }
};
pub fn composed(env: *const R, input: anytype) Out {
    const p = fx.dsl.program;
    return p.captured(env, input, .{ p.binding("parsed", .{"inputs"}, Roles.parse), p.binding("withdrawn", .{"parsed"}, Roles.withdraw), p.binding("recorded", .{"withdrawn"}, Roles.recorded), p.binding("tracked", .{"recorded"}, Roles.atom), p.binding("persisted", .{ "inputs", "tracked" }, Roles.projection) });
}
fn check(allocator: std.mem.Allocator, io: std.Io) !void {
    var arena = std.heap.ArenaAllocator.init(allocator);
    defer arena.deinit();
    var activity_calls: usize = 0;
    var undos: usize = 0;
    var acquired: usize = 0;
    var released: usize = 0;
    const runtime = .{ .allocator = allocator, .io = io };
    var domain = fx.TransactionDomain.init(&runtime);
    defer domain.deinit();
    var primary = fx.TransactionRef(u32).init(&domain, 20);
    var reserve = fx.TransactionRef(u32).init(&domain, 30);
    var audit = fx.TransactionRef(u32).init(&domain, 0);
    var registry_result = fx.AtomRegistry.create(&runtime, .{ .max_nodes = 8, .max_dependencies_per_node = 4 }, 8);
    if (registry_result != .success) {
        registry_result.deinit();
        return error.OutOfMemory;
    }
    const registry = registry_result.success;
    defer registry.deinit();
    var keys: [5]fx.OwnedSlice(u8) = undefined;
    var initialized: usize = 0;
    defer for (keys[0..initialized]) |*key| key.deinit();
    for ([_][]const u8{ "workflow", "owner", "selector", "retail", "wholesale" }, 0..) |name, i| {
        var encoded = fx.PrimaryKey.encodeDirect(&runtime, "composition", &.{.{ .text = name }});
        if (encoded != .success) {
            encoded.deinit();
            return error.OutOfMemory;
        }
        keys[i] = encoded.success;
        initialized += 1;
    }
    var workflow_env: invoices.R = .{ .allocator = allocator, .io = io, .calls = &activity_calls, .undos = &undos, .replay_arena = &arena, .execution = .{ .encoded = keys[0].items } };
    var engine_memory = fx.MemoryWorkflowEngine(invoices.R, invoices.Calls.now).init(&workflow_env);
    defer engine_memory.deinit();
    const engine = engine_memory.capability();
    workflow_env.engine = &engine;
    var submitting = engine.vtable.submit(engine.userdata, &workflow_env, .{ .definition = .{ .name = "composition", .version = 1, .payload_schema_version = 1, .success_schema_version = 1, .error_schema_version = 1 }, .execution = workflow_env.execution, .encoded_payload = "9" });
    defer submitting.deinit();
    var submitted = fx.run(&workflow_env, submitting);
    defer submitted.deinit();
    try std.testing.expect(submitted == .success);
    var activating = engine.vtable.activate(engine.userdata, &workflow_env, workflow_env.execution, "worker", .{ .lease_until = .{ .at = .{ .unix_nanoseconds = 1000 } } });
    defer activating.deinit();
    var activated = fx.run(&workflow_env, activating);
    defer activated.deinit();
    try std.testing.expect(activated == .success and activated.success == .activated);
    workflow_env.activation = activated.success.activated.activation();
    var activation_env: projections.R = .{ .allocator = allocator, .io = io, .acquired = &acquired, .released = &released };
    const Resource = fx.ClusterEntityResource(*projections.Live, projections.E, projections.R);
    const resource = Resource.fromProvider(null, &.{ .acquire = projections.Calls.acquire, .release = projections.Calls.release });
    activation_env.resource = &resource;
    var checkpoints = fx.MemoryProjectionCheckpoint(projections.R).init(&activation_env);
    defer checkpoints.deinit();
    const checkpoint_store = checkpoints.capability();
    activation_env.checkpoints = &checkpoint_store;
    var counts: rpc.Counts = .{};
    var executor: fx.NativeIoExecutor = .{ .allocator = allocator, .io = io, .capabilities = .{ .independent = true, .cpu = true, .blocking = true }, .maximum_owned = 4 };
    defer executor.deinit();
    const rpc_env: rpc.Root = .{ .allocator = allocator, .io = io, .async_engine = executor.executor(), .counts = &counts };
    activation_env.rpc = &rpc_env;
    const env: R = .{ .allocator = allocator, .io = io, .workflow = workflow_env, .activation = activation_env, .atom = .{ .allocator = allocator, .io = io, .registry = registry, .owner = keys[1].items, .selection = keys[2].items, .retail = keys[3].items, .wholesale = keys[4].items, .bulk = false }, .transaction = .{ .allocator = allocator, .io = io, .domain = &domain }, .primary = &primary, .reserve = &reserve, .audit = &audit, .rpc = .{ .allocator = allocator, .io = io, .async_engine = executor.executor(), .counts = &counts } };
    var page = try projections.history(allocator, 1, "9");
    defer page.deinit();
    var description = composed(&env, .{ .text = @as([]const u8, "{\"amount\":9}"), .id = keys[1].items, .page = &page });
    comptime if (@TypeOf(description) != Out) @compileError("cross-domain composition requires the existing Effect");
    try std.testing.expectEqual(@as(usize, 0), activity_calls);
    try std.testing.expectEqual(@as(usize, 0), acquired);
    var result = fx.run(&env, description);
    description.deinit();
    defer result.deinit();
    try std.testing.expect(result == .success);
    try std.testing.expectEqual(@as(i64, 9), result.success);
    try std.testing.expectEqual(@as(u32, 11), primary.value);
    try std.testing.expectEqual(@as(usize, 1), activity_calls);
    try std.testing.expectEqual(acquired, released);
    try std.testing.expectEqual(@as(usize, 1), counts.reader_closed);
}
pub fn main(init: std.process.Init) !void {
    try check(init.gpa, init.io);
}
test "COMP-09 COMP-10 schema transaction recorded workflow tracked atom activation RPC stream and checkpoint share ordinary Effect composition" {
    try check(std.testing.allocator, std.testing.io);
}
