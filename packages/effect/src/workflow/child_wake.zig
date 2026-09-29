//! Child completion and parent wake composition. The child store retains the
//! encoded Exit independently of the wake attempt. A failed or lost wake is
//! reconciled from pending completions; acknowledgement is a separate step
//! performed only after the parent consumes the recorded reason.
const outcomes = @import("../outcome.zig");
const effects = @import("../effect.zig");
const children = @import("child_store.zig");
const workflow = @import("workflow_engine.zig");
const types = @import("workflow_types.zig");

pub fn WakeReport(comptime EngineError: type) type {
    return struct {
        const Self = @This();
        pub const effect_owned_payload = true;
        completion: children.CompleteResult,
        wake: ?outcomes.Exit(types.ControlResult, EngineError),

        pub fn deinit(self: *Self) void {
            outcomes.discardPayload(&self.wake);
            self.* = undefined;
        }

        pub fn clone(self: *const Self) error{OutOfMemory}!Self {
            if (self.wake) |value| {
                const copy = value.clone();
                if (copy == .die and copy.die.kind == .out_of_memory and
                    !(value == .die and value.die.kind == .out_of_memory))
                    return error.OutOfMemory;
                return .{ .completion = self.completion, .wake = copy };
            }
            return .{ .completion = self.completion, .wake = null };
        }
    };
}

pub fn ReconcileReport(comptime EngineError: type) type {
    return struct {
        const Self = @This();
        pub const effect_owned_payload = true;
        pending_count: usize,
        wake: ?outcomes.Exit(types.ControlResult, EngineError),

        pub fn deinit(self: *Self) void {
            outcomes.discardPayload(&self.wake);
            self.* = undefined;
        }

        pub fn clone(self: *const Self) error{OutOfMemory}!Self {
            if (self.wake) |value| {
                const copy = value.clone();
                if (copy == .die and copy.die.kind == .out_of_memory and
                    !(value == .die and value.die.kind == .out_of_memory))
                    return error.OutOfMemory;
                return .{ .pending_count = self.pending_count, .wake = copy };
            }
            return .{ .pending_count = self.pending_count, .wake = null };
        }
    };
}

pub fn completeAndWake(
    comptime R: type,
    comptime ChildProviderError: type,
    comptime EngineProviderError: type,
    comptime child_guarantee: workflow.Guarantee,
    comptime engine_guarantee: workflow.Guarantee,
    comptime select_children: fn (*const R) *const children.Store(R, ChildProviderError, child_guarantee),
    comptime select_engine: fn (*const R) *const workflow.WorkflowEngine(R, EngineProviderError, engine_guarantee),
    comptime select_link: fn (*const R) children.Link,
    comptime select_exit: fn (*const R) []const u8,
) effects.Effect(WakeReport(workflow.Error(EngineProviderError)), children.Error(ChildProviderError), R) {
    const Child = children.Store(R, ChildProviderError, child_guarantee);
    const Engine = workflow.WorkflowEngine(R, EngineProviderError, engine_guarantee);
    const Report = WakeReport(Engine.Failure);
    const Out = effects.Effect(Report, Child.Failure, R);
    return Out.deferEffect(struct {
        fn start(env: *const R) Out {
            var recorded = Child.complete(select_children, select_link, select_exit);
            return recorded.flatMapOwned(env, struct {
                fn after(completion: children.CompleteResult, current: *const R) Out {
                    if (completion != .recorded and completion != .already_recorded)
                        return Out.fromOwnedSuccess(current, .{ .completion = completion, .wake = null });
                    var waking = Engine.resumeExecution(select_engine, struct {
                        fn parent(supplied: *const R) types.ExecutionId {
                            return select_link(supplied).parent;
                        }
                    }.parent);
                    var settled = waking.materializeOwned(current);
                    var aligned = settled.mapErrorOwned(current, struct {
                        fn impossible(_: void) Child.Failure {
                            unreachable;
                        }
                    }.impossible);
                    return aligned.flatMapCapturedOwned(current, Report, completion, struct {
                        fn result(stored: *const children.CompleteResult, wake: outcomes.Exit(types.ControlResult, Engine.Failure), supplied: *const R) Out {
                            return Out.fromOwnedSuccess(supplied, .{ .completion = stored.*, .wake = wake });
                        }
                    }.result);
                }
            }.after);
        }
    }.start);
}

pub fn reconcile(
    comptime R: type,
    comptime ChildProviderError: type,
    comptime EngineProviderError: type,
    comptime child_guarantee: workflow.Guarantee,
    comptime engine_guarantee: workflow.Guarantee,
    comptime select_children: fn (*const R) *const children.Store(R, ChildProviderError, child_guarantee),
    comptime select_engine: fn (*const R) *const workflow.WorkflowEngine(R, EngineProviderError, engine_guarantee),
    comptime select_parent: fn (*const R) types.ExecutionId,
) effects.Effect(ReconcileReport(workflow.Error(EngineProviderError)), children.Error(ChildProviderError), R) {
    const Child = children.Store(R, ChildProviderError, child_guarantee);
    const Engine = workflow.WorkflowEngine(R, EngineProviderError, engine_guarantee);
    const Report = ReconcileReport(Engine.Failure);
    const Out = effects.Effect(Report, Child.Failure, R);
    return Out.deferEffect(struct {
        fn start(env: *const R) Out {
            var reading = Child.pending(select_children, select_parent);
            return reading.flatMapOwned(env, struct {
                fn after(pending: outcomes.OwnedSlice(children.Pending), current: *const R) Out {
                    var owned = pending;
                    const count = owned.items.len;
                    owned.deinit();
                    if (count == 0) return Out.fromOwnedSuccess(current, .{ .pending_count = 0, .wake = null });
                    var waking = Engine.resumeExecution(select_engine, select_parent);
                    var settled = waking.materializeOwned(current);
                    var aligned = settled.mapErrorOwned(current, struct {
                        fn impossible(_: void) Child.Failure {
                            unreachable;
                        }
                    }.impossible);
                    return aligned.flatMapCapturedOwned(current, Report, count, struct {
                        fn result(stored: *const usize, wake: outcomes.Exit(types.ControlResult, Engine.Failure), supplied: *const R) Out {
                            return Out.fromOwnedSuccess(supplied, .{ .pending_count = stored.*, .wake = wake });
                        }
                    }.result);
                }
            }.after);
        }
    }.start);
}
