//! Checked workflow-body syntax built on the same Effect continuation kernel.
//! This adds no second scheduler, error channel or environment convention.
const effects = @import("../effect.zig");
const workflows = @import("workflow.zig");
const types = @import("workflow_types.zig");
const activity_execution = @import("activity_execution.zig");
const engine_module = @import("workflow_engine.zig");
const outcomes = @import("../outcome.zig");
const deferred = @import("durable_deferred.zig");
const clock = @import("durable_clock.zig");
const queue = @import("durable_queue.zig");
const persisted_queue = @import("../persistence/persisted_queue.zig");
const parallel = @import("durable_parallel.zig");
const compensation = @import("compensation.zig");
const children = @import("child_store.zig");
const child_wake = @import("child_wake.zig");

pub fn WorkflowBuilder(comptime E: type, comptime R: type) type {
    return struct {
        const Core = effects.EffectBuilder(E, R);

        fn Step(comptime A: type) type {
            return struct {
                pub const Success = A;
                pub const Error = E;
                pub const Environment = R;
                program: effects.Effect(A, E, R),
            };
        }

        fn requireStep(comptime Candidate: type) void {
            if (!@hasDecl(Candidate, "Success") or Candidate != Step(Candidate.Success))
                @compileError("WorkflowBuilder accepts only its described durable steps; place ordinary Effects inside a recorded activity");
        }

        fn nextType(comptime next: anytype) type {
            const Info = @typeInfo(@TypeOf(next));
            if (Info != .@"fn" or Info.@"fn".return_type == null)
                @compileError("WorkflowBuilder continuation must return a described durable step");
            const Next = Info.@"fn".return_type.?;
            requireStep(Next);
            return Next;
        }

        fn mappedStep(comptime operation: anytype, comptime map_error: anytype) Step(@TypeOf(operation).Success) {
            const A = @TypeOf(operation).Success;
            const Out = effects.Effect(A, E, R);
            return .{ .program = Out.deferEffect(struct {
                fn start(env: *const R) Out {
                    var current = operation;
                    return current.mapErrorOwned(env, map_error);
                }
            }.start) };
        }

        pub fn begin(comptime description: types.Definition) type {
            if (!description.valid()) @compileError("workflow definition requires stable name and nonzero definition/schema versions");
            return struct {
                pub fn payload(comptime payload_codec: anytype) type {
                    const PayloadCodec = @TypeOf(payload_codec);
                    return struct {
                        pub fn success(comptime success_codec: anytype) type {
                            const SuccessCodec = @TypeOf(success_codec);
                            return struct {
                                pub fn failure(comptime error_codec: anytype) type {
                                    const ErrorCodec = @TypeOf(error_codec);
                                    if (!@hasDecl(ErrorCodec, "Decoded") or ErrorCodec.Decoded != E)
                                        @compileError("workflow error codec must match the builder's typed error channel");
                                    const Definition = workflows.Workflow(PayloadCodec, SuccessCodec, ErrorCodec);
                                    const defined = Definition.define(
                                        description.name,
                                        description.version,
                                        description.payload_schema_version,
                                        description.success_schema_version,
                                        description.error_schema_version,
                                        payload_codec,
                                        success_codec,
                                        error_codec,
                                    );
                                    return struct {
                                        pub const WorkflowDefinition = Definition;
                                        pub const definition = defined;

                                        pub fn handledBy(
                                            comptime handler: fn (*const Definition.Payload, *const R) effects.Effect(Definition.Success, E, R),
                                        ) workflows.Handler(Definition, R) {
                                            return defined.handledBy(R, handler);
                                        }
                                    };
                                }
                            };
                        }
                    };
                }
            };
        }

        pub fn registry(comptime handlers: anytype) type {
            return workflows.Registry(handlers);
        }
        pub fn succeed(value: anytype) Step(@TypeOf(value)) {
            return .{ .program = Core.succeed(value) };
        }

        /// The selector must return an owned value when A owns payload data.
        pub fn succeedFrom(comptime A: type, comptime select_value: fn (*const R) A) Step(A) {
            return .{ .program = effects.Effect(A, E, R).fromOutcome(struct {
                fn run(env: *const R) outcomes.Exit(A, E) {
                    return .{ .success = select_value(env) };
                }
            }.run) };
        }

        pub fn fail(comptime A: type, comptime err: E) Step(A) {
            return .{ .program = effects.Effect(A, E, R).fromOutcome(struct {
                fn run(_: *const R) outcomes.Exit(A, E) {
                    return .{ .fail = outcomes.clonePayload(err) catch return .{ .die = outcomes.Defect.out_of_memory } };
                }
            }.run) };
        }

        /// The selector must return an owned error when E owns payload data.
        pub fn failFrom(comptime A: type, comptime select_error: fn (*const R) E) Step(A) {
            return .{ .program = effects.Effect(A, E, R).fromOutcome(struct {
                fn run(env: *const R) outcomes.Exit(A, E) {
                    return .{ .fail = select_error(env) };
                }
            }.run) };
        }

        pub fn bind(comptime step: anytype, comptime next: anytype) Step(nextType(next).Success) {
            const Current = @TypeOf(step);
            requireStep(Current);
            const Next = nextType(next);
            return .{ .program = Core.bind(step.program, struct {
                fn advance(value: Current.Success) effects.Effect(Next.Success, E, R) {
                    return next(value).program;
                }
            }.advance) };
        }

        pub fn returnFrom(step: anytype) @TypeOf(step) {
            requireStep(@TypeOf(step));
            return step;
        }

        pub fn delay(comptime A: type, comptime factory: fn (*const R) Step(A)) Step(A) {
            return .{ .program = effects.Effect(A, E, R).deferEffect(struct {
                fn start(env: *const R) effects.Effect(A, E, R) {
                    return factory(env).program;
                }
            }.start) };
        }

        pub fn combine(comptime first: Step(void), comptime second: anytype) @TypeOf(second) {
            const Next = @TypeOf(second);
            requireStep(Next);
            return bind(first, struct {
                fn advance(_: void) Next {
                    return second;
                }
            }.advance);
        }

        pub fn run(comptime step: anytype) effects.Effect(@TypeOf(step).Success, E, R) {
            requireStep(@TypeOf(step));
            return step.program;
        }

        pub fn activity(
            comptime handler: anytype,
            comptime ProviderError: type,
            comptime CodecError: type,
            comptime guarantee: engine_module.Guarantee,
            comptime select_engine: anytype,
            comptime select_activation: anytype,
            comptime select_activity: anytype,
            comptime select_payload: anytype,
            comptime encode_exit: anytype,
            comptime decode_exit: anytype,
        ) Step(activity_execution.Recorded(@TypeOf(handler).ActivityDefinition.Success, @TypeOf(handler).ActivityDefinition.DomainError)) {
            const program = activity_execution.runRecorded(
                handler,
                R,
                ProviderError,
                CodecError,
                guarantee,
                select_engine,
                select_activation,
                select_activity,
                select_payload,
                encode_exit,
                decode_exit,
            );
            if (@TypeOf(program).Error != E)
                @compileError("recorded activity error must match WorkflowBuilder error; map it explicitly before composition");
            return .{ .program = program };
        }

        pub fn activityMapped(
            comptime handler: anytype,
            comptime ProviderError: type,
            comptime CodecError: type,
            comptime guarantee: engine_module.Guarantee,
            comptime select_engine: anytype,
            comptime select_activation: anytype,
            comptime select_activity: anytype,
            comptime select_payload: anytype,
            comptime encode_exit: anytype,
            comptime decode_exit: anytype,
            comptime map_error: fn (activity_execution.RunError(engine_module.WorkflowEngine(R, ProviderError, guarantee).Failure, CodecError)) E,
        ) Step(activity_execution.Recorded(@TypeOf(handler).ActivityDefinition.Success, @TypeOf(handler).ActivityDefinition.DomainError)) {
            const Raw = activity_execution.runRecorded(
                handler,
                R,
                ProviderError,
                CodecError,
                guarantee,
                select_engine,
                select_activation,
                select_activity,
                select_payload,
                encode_exit,
                decode_exit,
            );
            return mappedStep(Raw, map_error);
        }

        pub fn suspendExecution(
            comptime ProviderError: type,
            comptime guarantee: engine_module.Guarantee,
            comptime select_engine: fn (*const R) *const engine_module.WorkflowEngine(R, ProviderError, guarantee),
            comptime select_activation: fn (*const R) types.Activation,
            comptime map_error: fn (engine_module.Error(ProviderError)) E,
        ) Step(types.SuspensionResult) {
            const Engine = engine_module.WorkflowEngine(R, ProviderError, guarantee);
            return mappedStep(Engine.suspendExecution(select_engine, select_activation), map_error);
        }

        pub fn abandonActivation(
            comptime ProviderError: type,
            comptime guarantee: engine_module.Guarantee,
            comptime select_engine: fn (*const R) *const engine_module.WorkflowEngine(R, ProviderError, guarantee),
            comptime select_activation: fn (*const R) types.Activation,
            comptime map_error: fn (engine_module.Error(ProviderError)) E,
        ) Step(types.SuspensionResult) {
            const Engine = engine_module.WorkflowEngine(R, ProviderError, guarantee);
            return mappedStep(Engine.abandonActivation(select_engine, select_activation), map_error);
        }

        pub fn resumeExecution(
            comptime ProviderError: type,
            comptime guarantee: engine_module.Guarantee,
            comptime select_engine: fn (*const R) *const engine_module.WorkflowEngine(R, ProviderError, guarantee),
            comptime select_execution: fn (*const R) types.ExecutionId,
            comptime map_error: fn (engine_module.Error(ProviderError)) E,
        ) Step(types.ControlResult) {
            const Engine = engine_module.WorkflowEngine(R, ProviderError, guarantee);
            return mappedStep(Engine.resumeExecution(select_engine, select_execution), map_error);
        }

        pub fn cancelTerminal(
            comptime ProviderError: type,
            comptime guarantee: engine_module.Guarantee,
            comptime select_engine: fn (*const R) *const engine_module.WorkflowEngine(R, ProviderError, guarantee),
            comptime select_execution: fn (*const R) types.ExecutionId,
            comptime map_error: fn (engine_module.Error(ProviderError)) E,
        ) Step(types.ControlResult) {
            const Engine = engine_module.WorkflowEngine(R, ProviderError, guarantee);
            return mappedStep(Engine.cancelTerminal(select_engine, select_execution), map_error);
        }

        /// This may fence a running activation before its work has joined.
        pub fn interruptUnsafe(
            comptime ProviderError: type,
            comptime guarantee: engine_module.Guarantee,
            comptime select_engine: fn (*const R) *const engine_module.WorkflowEngine(R, ProviderError, guarantee),
            comptime select_execution: fn (*const R) types.ExecutionId,
            comptime map_error: fn (engine_module.Error(ProviderError)) E,
        ) Step(types.ControlResult) {
            const Engine = engine_module.WorkflowEngine(R, ProviderError, guarantee);
            return mappedStep(Engine.interruptUnsafe(select_engine, select_execution), map_error);
        }

        /// The provider atomically registers a waiter and reads a prior
        /// completion. A waiting result remains explicit to the workflow.
        pub fn deferredObserve(
            comptime ProviderError: type,
            comptime guarantee: deferred.Guarantee,
            comptime select_store: fn (*const R) *const deferred.Store(R, ProviderError, guarantee),
            comptime select_identity: fn (*const R) deferred.Identity,
            comptime map_error: fn (deferred.Store(R, ProviderError, guarantee).Failure) E,
        ) Step(deferred.Observed) {
            const Store = deferred.Store(R, ProviderError, guarantee);
            return mappedStep(Store.observe(select_store, select_identity), map_error);
        }

        pub fn signalObserve(
            comptime SignalType: type,
            comptime ProviderError: type,
            comptime guarantee: deferred.Guarantee,
            comptime select_store: fn (*const R) *const deferred.Store(R, ProviderError, guarantee),
            comptime select_token: fn (*const R) SignalType.Token,
            comptime map_error: fn (deferred.Store(R, ProviderError, guarantee).Failure) E,
        ) Step(deferred.Observed) {
            if (!@hasDecl(SignalType, "durable_signal") or !SignalType.durable_signal or
                !@hasDecl(SignalType, "Result") or !@hasDecl(SignalType, "ResultSchema"))
                @compileError("signalObserve requires a typed durable signal");
            return deferredObserve(ProviderError, guarantee, select_store, struct {
                fn identity(env: *const R) deferred.Identity {
                    return select_token(env).identity;
                }
            }.identity, map_error);
        }

        pub fn deferredComplete(
            comptime ProviderError: type,
            comptime guarantee: deferred.Guarantee,
            comptime select_store: fn (*const R) *const deferred.Store(R, ProviderError, guarantee),
            comptime select_identity: fn (*const R) deferred.Identity,
            comptime select_encoded_result: fn (*const R) []const u8,
            comptime map_error: fn (deferred.Store(R, ProviderError, guarantee).Failure) E,
        ) Step(deferred.Completion) {
            const Store = deferred.Store(R, ProviderError, guarantee);
            return mappedStep(Store.complete(select_store, select_identity, select_encoded_result), map_error);
        }

        pub fn signalComplete(
            comptime SignalType: type,
            comptime ProviderError: type,
            comptime guarantee: deferred.Guarantee,
            comptime select_store: fn (*const R) *const deferred.Store(R, ProviderError, guarantee),
            comptime select_token: fn (*const R) SignalType.Token,
            comptime select_encoded_result: fn (*const R) []const u8,
            comptime map_error: fn (deferred.Store(R, ProviderError, guarantee).Failure) E,
        ) Step(deferred.Completion) {
            if (!@hasDecl(SignalType, "durable_signal") or !SignalType.durable_signal or
                !@hasDecl(SignalType, "Result") or !@hasDecl(SignalType, "ResultSchema"))
                @compileError("signalComplete requires a typed durable signal");
            return deferredComplete(ProviderError, guarantee, select_store, struct {
                fn identity(env: *const R) deferred.Identity {
                    return select_token(env).identity;
                }
            }.identity, select_encoded_result, map_error);
        }

        pub fn deferredReleaseWait(
            comptime ProviderError: type,
            comptime guarantee: deferred.Guarantee,
            comptime select_store: fn (*const R) *const deferred.Store(R, ProviderError, guarantee),
            comptime select_registration: fn (*const R) deferred.RegistrationView,
            comptime map_error: fn (deferred.Store(R, ProviderError, guarantee).Failure) E,
        ) Step(void) {
            const Store = deferred.Store(R, ProviderError, guarantee);
            return mappedStep(Store.releaseWait(select_store, select_registration), map_error);
        }

        pub fn clockSchedule(
            comptime ProviderError: type,
            comptime guarantee: clock.Guarantee,
            comptime select_store: fn (*const R) *const clock.Store(R, ProviderError, guarantee),
            comptime select_schedule: fn (*const R) clock.Schedule,
            comptime map_error: fn (clock.Store(R, ProviderError, guarantee).Error) E,
        ) Step(clock.ScheduleResult) {
            const Store = clock.Store(R, ProviderError, guarantee);
            return mappedStep(Store.schedule(select_store, select_schedule), map_error);
        }

        pub fn clockCancel(
            comptime ProviderError: type,
            comptime guarantee: clock.Guarantee,
            comptime select_store: fn (*const R) *const clock.Store(R, ProviderError, guarantee),
            comptime select_identity: fn (*const R) clock.Identity,
            comptime map_error: fn (clock.Store(R, ProviderError, guarantee).Error) E,
        ) Step(clock.CancelResult) {
            const Store = clock.Store(R, ProviderError, guarantee);
            return mappedStep(Store.cancel(select_store, select_identity), map_error);
        }

        pub fn clockClaimDue(
            comptime ProviderError: type,
            comptime guarantee: clock.Guarantee,
            comptime select_store: fn (*const R) *const clock.Store(R, ProviderError, guarantee),
            comptime select_policy: fn (*const R) clock.ClaimPolicy,
            comptime map_error: fn (clock.Store(R, ProviderError, guarantee).Error) E,
        ) Step(clock.ClaimResult) {
            const Store = clock.Store(R, ProviderError, guarantee);
            return mappedStep(Store.claimDue(select_store, select_policy), map_error);
        }

        pub fn clockAcknowledge(
            comptime ProviderError: type,
            comptime guarantee: clock.Guarantee,
            comptime select_store: fn (*const R) *const clock.Store(R, ProviderError, guarantee),
            comptime select_token: fn (*const R) clock.Token,
            comptime map_error: fn (clock.Store(R, ProviderError, guarantee).Error) E,
        ) Step(clock.Acknowledgement) {
            const Store = clock.Store(R, ProviderError, guarantee);
            return mappedStep(Store.acknowledge(select_store, select_token), map_error);
        }

        pub fn queueOfferAndObserve(
            comptime QueueProviderError: type,
            comptime DeferredProviderError: type,
            comptime queue_guarantee: persisted_queue.Guarantee,
            comptime deferred_guarantee: deferred.Guarantee,
            comptime select_queue: fn (*const R) *const persisted_queue.Store(R, QueueProviderError, queue_guarantee),
            comptime select_deferred: fn (*const R) *const deferred.Store(R, DeferredProviderError, deferred_guarantee),
            comptime select_submission: fn (*const R) persisted_queue.Submission,
            comptime select_identity: fn (*const R) deferred.Identity,
            comptime map_error: fn (queue.Error(persisted_queue.Store(R, QueueProviderError, queue_guarantee).Failure, deferred.Store(R, DeferredProviderError, deferred_guarantee).Failure)) E,
        ) Step(queue.Submission) {
            const Raw = queue.offerAndObserve(R, QueueProviderError, DeferredProviderError, queue_guarantee, deferred_guarantee, select_queue, select_deferred, select_submission, select_identity);
            return mappedStep(Raw, map_error);
        }

        pub fn recordedBranch(
            comptime A: type,
            comptime branch: Step(A),
            comptime ProviderError: type,
            comptime CodecError: type,
            comptime guarantee: parallel.Guarantee,
            comptime select_journal: fn (*const R) *const parallel.Journal(R, ProviderError, guarantee),
            comptime select_branch: fn (*const R) parallel.Branch,
            comptime select_attempt: fn (*const R) u64,
            comptime encode_exit: fn (*const outcomes.Exit(A, E), *const R) outcomes.Exit(outcomes.OwnedSlice(u8), CodecError),
            comptime decode_exit: fn ([]const u8, *const R) outcomes.Exit(outcomes.Exit(A, E), CodecError),
            comptime map_error: fn (parallel.RunError(parallel.Journal(R, ProviderError, guarantee).Failure, CodecError)) E,
        ) Step(parallel.Recorded(A, E)) {
            const Raw = parallel.runRecorded(A, E, R, ProviderError, CodecError, guarantee, branch.program, select_journal, select_branch, select_attempt, encode_exit, decode_exit);
            return mappedStep(Raw, map_error);
        }

        pub fn retryBranch(comptime A: type, comptime policy: parallel.Policy, comptime branch: Step(A)) Step(A) {
            return .{ .program = parallel.retryBranch(A, E, R, policy, branch.program) };
        }

        pub fn durableRace(
            comptime A: type,
            comptime left: Step(A),
            comptime right: Step(A),
            comptime ProviderError: type,
            comptime CodecError: type,
            comptime guarantee: parallel.Guarantee,
            comptime select_journal: fn (*const R) *const parallel.Journal(R, ProviderError, guarantee),
            comptime select_execution: fn (*const R) types.ExecutionId,
            comptime select_race_name: fn (*const R) []const u8,
            comptime select_left: fn (*const R) parallel.Branch,
            comptime select_right: fn (*const R) parallel.Branch,
            comptime select_attempt: fn (*const R) u64,
            comptime encode_exit: fn (*const outcomes.Exit(A, E), *const R) outcomes.Exit(outcomes.OwnedSlice(u8), CodecError),
            comptime decode_exit: fn ([]const u8, *const R) outcomes.Exit(outcomes.Exit(A, E), CodecError),
            comptime map_error: fn (parallel.RaceError(parallel.RunError(parallel.Journal(R, ProviderError, guarantee).Failure, CodecError), parallel.Journal(R, ProviderError, guarantee).Failure, CodecError)) E,
        ) Step(parallel.RaceOutcome(A, E)) {
            const Raw = parallel.durableRace(A, E, R, ProviderError, CodecError, guarantee, left.program, right.program, select_journal, select_execution, select_race_name, select_left, select_right, select_attempt, encode_exit, decode_exit);
            return mappedStep(Raw, map_error);
        }

        pub fn recordedCompensation(
            comptime body: Step(void),
            comptime ProviderError: type,
            comptime CodecError: type,
            comptime guarantee: compensation.Guarantee,
            comptime select_journal: fn (*const R) *const compensation.Journal(R, ProviderError, guarantee),
            comptime select_attempt: fn (*const R) compensation.Attempt,
            comptime encode_exit: fn (*const outcomes.Exit(void, E), *const R) outcomes.Exit(outcomes.OwnedSlice(u8), CodecError),
            comptime decode_exit: fn ([]const u8, *const R) outcomes.Exit(outcomes.Exit(void, E), CodecError),
            comptime map_error: fn (compensation.RunError(compensation.Journal(R, ProviderError, guarantee).Failure, CodecError)) E,
        ) Step(compensation.Completed(E)) {
            const Raw = compensation.runRecorded(E, R, ProviderError, CodecError, guarantee, body.program, select_journal, select_attempt, encode_exit, decode_exit);
            return mappedStep(Raw, map_error);
        }

        pub fn registerCompensation(
            comptime ProviderError: type,
            comptime guarantee: compensation.Guarantee,
            comptime select_journal: fn (*const R) *const compensation.Journal(R, ProviderError, guarantee),
            comptime select_identity: fn (*const R) compensation.Identity,
            comptime map_error: fn (compensation.Journal(R, ProviderError, guarantee).Failure) E,
        ) Step(compensation.Registration) {
            const Journal = compensation.Journal(R, ProviderError, guarantee);
            return mappedStep(Journal.register(select_journal, select_identity), map_error);
        }

        pub fn runCompensations(
            comptime ProviderError: type,
            comptime CodecError: type,
            comptime guarantee: compensation.Guarantee,
            comptime select_journal: fn (*const R) *const compensation.Journal(R, ProviderError, guarantee),
            comptime select_execution: fn (*const R) types.ExecutionId,
            comptime dispatch: fn (*const compensation.Pending, *const R) effects.Effect(compensation.Completed(E), compensation.RunError(compensation.Journal(R, ProviderError, guarantee).Failure, CodecError), R),
            comptime map_error: fn (compensation.RunError(compensation.Journal(R, ProviderError, guarantee).Failure, CodecError)) E,
        ) Step(outcomes.Exit(void, E)) {
            const Raw = compensation.runAllPending(E, R, ProviderError, CodecError, guarantee, select_journal, select_execution, dispatch);
            return mappedStep(Raw, map_error);
        }

        pub fn childCompleteAndWake(
            comptime ChildProviderError: type,
            comptime EngineProviderError: type,
            comptime child_guarantee: engine_module.Guarantee,
            comptime engine_guarantee: engine_module.Guarantee,
            comptime select_children: fn (*const R) *const children.Store(R, ChildProviderError, child_guarantee),
            comptime select_engine: fn (*const R) *const engine_module.WorkflowEngine(R, EngineProviderError, engine_guarantee),
            comptime select_link: fn (*const R) children.Link,
            comptime select_exit: fn (*const R) []const u8,
            comptime map_error: fn (children.Error(ChildProviderError)) E,
        ) Step(child_wake.WakeReport(engine_module.Error(EngineProviderError))) {
            const Raw = child_wake.completeAndWake(R, ChildProviderError, EngineProviderError, child_guarantee, engine_guarantee, select_children, select_engine, select_link, select_exit);
            return mappedStep(Raw, map_error);
        }

        pub fn childReconcile(
            comptime ChildProviderError: type,
            comptime EngineProviderError: type,
            comptime child_guarantee: engine_module.Guarantee,
            comptime engine_guarantee: engine_module.Guarantee,
            comptime select_children: fn (*const R) *const children.Store(R, ChildProviderError, child_guarantee),
            comptime select_engine: fn (*const R) *const engine_module.WorkflowEngine(R, EngineProviderError, engine_guarantee),
            comptime select_parent: fn (*const R) types.ExecutionId,
            comptime map_error: fn (children.Error(ChildProviderError)) E,
        ) Step(child_wake.ReconcileReport(engine_module.Error(EngineProviderError))) {
            const Raw = child_wake.reconcile(R, ChildProviderError, EngineProviderError, child_guarantee, engine_guarantee, select_children, select_engine, select_parent);
            return mappedStep(Raw, map_error);
        }
    };
}
