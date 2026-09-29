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

/// Defines versioned workflows and composes described durable steps. Ordinary
/// nondeterministic Effect work belongs inside a recorded activity; replay,
/// signals, deadlines, parallel branches, and compensation use injected
/// engine/capability projections from `R`.
///
/// Parameters:
/// - `E: comptime type`: Expected application-failure type; defects and interruption remain separate outcomes.
/// - `R: comptime type`: Caller-defined environment type that supplies dependencies and runtime capabilities when work executes.
/// Returns (`type`): workflow definition and durable composition API fixed to `E` and `R`.
pub fn WorkflowBuilder(comptime E: type, comptime R: type) type {
    return struct {
        const Core = effects.EffectBuilder(E, R);

        fn Step(comptime A: type) type {
            return struct {
                /// Value type carried by the successful channel.
                pub const Success = A;
                /// Expected workflow failure type `E` retained across builder stages.
                pub const Error = E;
                /// Exact caller environment type required when the description runs.
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

        /// Begins a versioned workflow definition after validating its identity and schema versions.
        ///
        /// Parameters:
        /// - `description: comptime types.Definition`: Completed typed definition selected at comptime.
        /// Returns (`type`): versioned workflow stage requiring payload, success, and failure codecs.
        pub fn begin(comptime description: types.Definition) type {
            if (!description.valid()) @compileError("workflow definition requires stable name and nonzero definition/schema versions");
            return struct {
                /// Supplies the workflow input codec and fixes the handler input type.
                ///
                /// Parameters:
                /// - `payload_codec: comptime anytype`: Compile-time typed codec/schema fixing the corresponding
                ///   decoded and encoded value types.
                /// Returns (`type`): workflow stage retaining its payload codec and requiring the success codec.
                pub fn payload(comptime payload_codec: anytype) type {
                    const PayloadCodec = @TypeOf(payload_codec);
                    return struct {
                        /// Supplies the workflow success codec and fixes the handler result type.
                        ///
                        /// Parameters:
                        /// - `success_codec: comptime anytype`: Compile-time typed codec/schema fixing the
                        ///   corresponding decoded and encoded value types.
                        /// Returns (`type`): workflow stage retaining payload and success codecs and requiring the failure codec.
                        pub fn success(comptime success_codec: anytype) type {
                            const SuccessCodec = @TypeOf(success_codec);
                            return struct {
                                /// Supplies a codec whose decoded error type must equal E.
                                ///
                                /// Parameters:
                                /// - `error_codec: comptime anytype`: Compile-time typed codec/schema fixing the
                                ///   corresponding decoded and encoded value types.
                                /// Returns (`type`): completed workflow definition stage ready for its typed body.
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
                                        /// Completed typed workflow definition.
                                        pub const WorkflowDefinition = Definition;
                                        /// Completed definition carried by this builder or handler.
                                        pub const definition = defined;

                                        /// Completes the workflow with a handler returning Effect(Success, E, R).
                                        ///
                                        /// Parameters:
                                        /// - `handler: comptime fn (*const Definition.Payload, *const R) effects.Effect(Definition.Success, E, R)`: Compile-time typed callback; its signature
                                        ///   is checked at comptime where static.
                                        /// Returns (`workflows.Handler(Definition, R)`): A completed typed handler binding its operation to the selected environment.
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

        /// Closes completed workflow handlers into a static registry.
        ///
        /// Parameters:
        /// - `handlers: comptime anytype`: Compile-time statically inferred input checked when this operation is
        ///   instantiated.
        /// Returns (`type`): registry type containing the completed workflow definitions.
        pub fn registry(comptime handlers: anytype) type {
            return workflows.Registry(handlers);
        }
        /// Creates a described durable step that succeeds with a typed value.
        ///
        /// Parameters:
        /// - `value: anytype`: success value returned by this workflow description.
        /// Returns (`Step(@TypeOf(value))`): A typed workflow step description; execution is deferred to the engine.
        pub fn succeed(value: anytype) Step(@TypeOf(value)) {
            return .{ .program = Core.succeed(value) };
        }

        /// The selector must return an owned value when A owns payload data.
        ///
        /// Parameters:
        /// - `A: comptime type`: Value type produced by a successful computation.
        /// - `select_value: comptime fn (*const R) A`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// Returns (`Step(A)`): A typed workflow step description; execution is deferred to the engine.
        pub fn succeedFrom(comptime A: type, comptime select_value: fn (*const R) A) Step(A) {
            return .{ .program = effects.Effect(A, E, R).fromOutcome(struct {
                fn run(env: *const R) outcomes.Exit(A, E) {
                    return .{ .success = select_value(env) };
                }
            }.run) };
        }

        /// Creates a described durable step that fails in the expected E channel.
        ///
        /// Parameters:
        /// - `A: comptime type`: Value type produced by a successful computation.
        /// - `err: comptime E`: Typed error value propagated through the failure channel.
        /// Returns (`Step(A)`): A typed workflow step description; execution is deferred to the engine.
        pub fn fail(comptime A: type, comptime err: E) Step(A) {
            return .{ .program = effects.Effect(A, E, R).fromOutcome(struct {
                fn run(_: *const R) outcomes.Exit(A, E) {
                    return .{ .fail = outcomes.clonePayload(err) catch return .{ .die = outcomes.Defect.out_of_memory } };
                }
            }.run) };
        }

        /// The selector must return an owned error when E owns payload data.
        ///
        /// Parameters:
        /// - `A: comptime type`: Value type produced by a successful computation.
        /// - `select_error: comptime fn (*const R) E`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// Returns (`Step(A)`): A typed workflow step description; execution is deferred to the engine.
        pub fn failFrom(comptime A: type, comptime select_error: fn (*const R) E) Step(A) {
            return .{ .program = effects.Effect(A, E, R).fromOutcome(struct {
                fn run(env: *const R) outcomes.Exit(A, E) {
                    return .{ .fail = select_error(env) };
                }
            }.run) };
        }

        /// Sequences described durable steps; an ordinary Effect is not accepted here.
        ///
        /// Parameters:
        /// - `step: comptime anytype`: Compile-time statically inferred input checked when this operation is
        ///   instantiated.
        /// - `next: comptime anytype`: Compile-time typed callback; its signature is checked at comptime where
        ///   static.
        /// Returns (`Step(nextType(next).Success)`): A typed workflow step description; execution is deferred to the engine.
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

        /// Lift an existing typed description into this builder.
        ///
        /// Parameters:
        /// - `step: anytype`: Statically inferred input checked when this operation is instantiated.
        /// Returns (`@TypeOf(step)`): The value or description with the exact type inferred from the supplied input.
        pub fn returnFrom(step: anytype) @TypeOf(step) {
            requireStep(@TypeOf(step));
            return step;
        }

        /// Defers creation of a described step until the current R is available.
        ///
        /// Parameters:
        /// - `A: comptime type`: Value type produced by a successful computation.
        /// - `factory: comptime fn (*const R) Step(A)`: Compile-time typed callback; its signature is checked at comptime where
        ///   static.
        /// Returns (`Step(A)`): A typed workflow step description; execution is deferred to the engine.
        pub fn delay(comptime A: type, comptime factory: fn (*const R) Step(A)) Step(A) {
            return .{ .program = effects.Effect(A, E, R).deferEffect(struct {
                fn start(env: *const R) effects.Effect(A, E, R) {
                    return factory(env).program;
                }
            }.start) };
        }

        /// Sequences a void durable step before another described step.
        ///
        /// Parameters:
        /// - `first: comptime Step(void)`: First typed description, executed or evaluated before the next.
        /// - `second: comptime anytype`: Compile-time statically inferred input checked when this operation is
        ///   instantiated.
        /// Returns (`@TypeOf(second)`): The value or description with the exact type inferred from the supplied input.
        pub fn combine(comptime first: Step(void), comptime second: anytype) @TypeOf(second) {
            const Next = @TypeOf(second);
            requireStep(Next);
            return bind(first, struct {
                fn advance(_: void) Next {
                    return second;
                }
            }.advance);
        }

        /// Exposes a described durable step as an Effect to execute with R.
        ///
        /// Parameters:
        /// - `step: comptime anytype`: Compile-time statically inferred input checked when this operation is
        ///   instantiated.
        /// Returns (`effects.Effect(@TypeOf(step).Success, E, R)`): typed execution of the completed workflow definition through the selected engine.
        pub fn run(comptime step: anytype) effects.Effect(@TypeOf(step).Success, E, R) {
            requireStep(@TypeOf(step));
            return step.program;
        }

        /// Runs a typed activity through the engine and records its completed Exit for replay.
        ///
        /// Parameters:
        /// - `handler: comptime anytype`: Compile-time typed callback; its signature is checked at comptime where
        ///   static.
        /// - `ProviderError: comptime type`: Provider failure type retained in the typed result.
        /// - `CodecError: comptime type`: Failure type returned by the selected encoder or decoder.
        /// - `guarantee: comptime engine_module.Guarantee`: Compile-time volatility or recoverability promised by this provider; durable compositions check it before compiling.
        /// - `select_engine: comptime anytype`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_activation: comptime anytype`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// - `select_activity: comptime anytype`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// - `select_payload: comptime anytype`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// - `encode_exit: comptime anytype`: Compile-time statically inferred input checked when this operation is
        ///   instantiated.
        /// - `decode_exit: comptime anytype`: Compile-time statically inferred input checked when this operation is
        ///   instantiated.
        /// Returns (`Step(activity_execution.Recorded(@TypeOf(handler).ActivityDefinition.Success, @TypeOf(handler).ActivityDefinition.DomainError))`): A typed workflow step description; execution is deferred to the engine.
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

        /// Runs a recorded activity while explicitly mapping provider failure into E.
        ///
        /// Parameters:
        /// - `handler: comptime anytype`: Compile-time typed callback; its signature is checked at comptime where
        ///   static.
        /// - `ProviderError: comptime type`: Provider failure type retained in the typed result.
        /// - `CodecError: comptime type`: Failure type returned by the selected encoder or decoder.
        /// - `guarantee: comptime engine_module.Guarantee`: Compile-time volatility or recoverability promised by this provider; durable compositions check it before compiling.
        /// - `select_engine: comptime anytype`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_activation: comptime anytype`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// - `select_activity: comptime anytype`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// - `select_payload: comptime anytype`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// - `encode_exit: comptime anytype`: Compile-time statically inferred input checked when this operation is
        ///   instantiated.
        /// - `decode_exit: comptime anytype`: Compile-time statically inferred input checked when this operation is
        ///   instantiated.
        /// - `map_error: comptime fn (activity_execution.RunError(engine_module.WorkflowEngine(R, ProviderError, guarantee).Failure, CodecError)) E`: Compile-time typed function selected at comptime; no runtime callback
        ///   lookup is required.
        /// Returns (`Step(activity_execution.Recorded(@TypeOf(handler).ActivityDefinition.Success, @TypeOf(handler).ActivityDefinition.DomainError))`): A typed workflow step description; execution is deferred to the engine.
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

        /// Records suspension so a later owner can resume the execution.
        ///
        /// Parameters:
        /// - `ProviderError: comptime type`: Provider failure type retained in the typed result.
        /// - `guarantee: comptime engine_module.Guarantee`: Compile-time volatility or recoverability promised by this provider; durable compositions check it before compiling.
        /// - `select_engine: comptime fn (*const R) *const engine_module.WorkflowEngine(R, ProviderError, guarantee)`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_activation: comptime fn (*const R) types.Activation`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// - `map_error: comptime fn (engine_module.Error(ProviderError)) E`: Compile-time typed function selected at comptime; no runtime callback
        ///   lookup is required.
        /// Returns (`Step(types.SuspensionResult)`): A typed workflow step description; execution is deferred to the engine.
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

        /// Releases local activation ownership without declaring terminal cancellation.
        ///
        /// Parameters:
        /// - `ProviderError: comptime type`: Provider failure type retained in the typed result.
        /// - `guarantee: comptime engine_module.Guarantee`: Compile-time volatility or recoverability promised by this provider; durable compositions check it before compiling.
        /// - `select_engine: comptime fn (*const R) *const engine_module.WorkflowEngine(R, ProviderError, guarantee)`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_activation: comptime fn (*const R) types.Activation`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// - `map_error: comptime fn (engine_module.Error(ProviderError)) E`: Compile-time typed function selected at comptime; no runtime callback
        ///   lookup is required.
        /// Returns (`Step(types.SuspensionResult)`): A typed workflow step description; execution is deferred to the engine.
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

        /// Requests resumption of a retained workflow execution.
        ///
        /// Parameters:
        /// - `ProviderError: comptime type`: Provider failure type retained in the typed result.
        /// - `guarantee: comptime engine_module.Guarantee`: Compile-time volatility or recoverability promised by this provider; durable compositions check it before compiling.
        /// - `select_engine: comptime fn (*const R) *const engine_module.WorkflowEngine(R, ProviderError, guarantee)`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_execution: comptime fn (*const R) types.ExecutionId`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// - `map_error: comptime fn (engine_module.Error(ProviderError)) E`: Compile-time typed function selected at comptime; no runtime callback
        ///   lookup is required.
        /// Returns (`Step(types.ControlResult)`): A typed workflow step description; execution is deferred to the engine.
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

        /// Records terminal cancellation as a workflow control request.
        ///
        /// Parameters:
        /// - `ProviderError: comptime type`: Provider failure type retained in the typed result.
        /// - `guarantee: comptime engine_module.Guarantee`: Compile-time volatility or recoverability promised by this provider; durable compositions check it before compiling.
        /// - `select_engine: comptime fn (*const R) *const engine_module.WorkflowEngine(R, ProviderError, guarantee)`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_execution: comptime fn (*const R) types.ExecutionId`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// - `map_error: comptime fn (engine_module.Error(ProviderError)) E`: Compile-time typed function selected at comptime; no runtime callback
        ///   lookup is required.
        /// Returns (`Step(types.ControlResult)`): A typed workflow step description; execution is deferred to the engine.
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
        ///
        /// Parameters:
        /// - `ProviderError: comptime type`: Provider failure type retained in the typed result.
        /// - `guarantee: comptime engine_module.Guarantee`: Compile-time volatility or recoverability promised by this provider; durable compositions check it before compiling.
        /// - `select_engine: comptime fn (*const R) *const engine_module.WorkflowEngine(R, ProviderError, guarantee)`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_execution: comptime fn (*const R) types.ExecutionId`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// - `map_error: comptime fn (engine_module.Error(ProviderError)) E`: Compile-time typed function selected at comptime; no runtime callback
        ///   lookup is required.
        /// Returns (`Step(types.ControlResult)`): A typed workflow step description; execution is deferred to the engine.
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
        ///
        /// Parameters:
        /// - `ProviderError: comptime type`: Provider failure type retained in the typed result.
        /// - `guarantee: comptime deferred.Guarantee`: Compile-time volatility or recoverability promised by this provider; durable compositions check it before compiling.
        /// - `select_store: comptime fn (*const R) *const deferred.Store(R, ProviderError, guarantee)`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_identity: comptime fn (*const R) deferred.Identity`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// - `map_error: comptime fn (deferred.Store(R, ProviderError, guarantee).Failure) E`: Compile-time typed function selected at comptime; no runtime callback
        ///   lookup is required.
        /// Returns (`Step(deferred.Observed)`): A typed workflow step description; execution is deferred to the engine.
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

        /// Observes a typed durable signal, preserving its result schema.
        ///
        /// Parameters:
        /// - `SignalType: comptime type`: Compile-time signal payload type.
        /// - `ProviderError: comptime type`: Provider failure type retained in the typed result.
        /// - `guarantee: comptime deferred.Guarantee`: Compile-time volatility or recoverability promised by this provider; durable compositions check it before compiling.
        /// - `select_store: comptime fn (*const R) *const deferred.Store(R, ProviderError, guarantee)`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_token: comptime fn (*const R) SignalType.Token`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `map_error: comptime fn (deferred.Store(R, ProviderError, guarantee).Failure) E`: Compile-time typed function selected at comptime; no runtime callback
        ///   lookup is required.
        /// Returns (`Step(deferred.Observed)`): A typed workflow step description; execution is deferred to the engine.
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

        /// Records completion of a durable deferred and wakes observers.
        ///
        /// Parameters:
        /// - `ProviderError: comptime type`: Provider failure type retained in the typed result.
        /// - `guarantee: comptime deferred.Guarantee`: Compile-time volatility or recoverability promised by this provider; durable compositions check it before compiling.
        /// - `select_store: comptime fn (*const R) *const deferred.Store(R, ProviderError, guarantee)`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_identity: comptime fn (*const R) deferred.Identity`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// - `select_encoded_result: comptime fn (*const R) []const u8`: Compile-time typed selector from the current environment;
        ///   its result must remain valid through use.
        /// - `map_error: comptime fn (deferred.Store(R, ProviderError, guarantee).Failure) E`: Compile-time typed function selected at comptime; no runtime callback
        ///   lookup is required.
        /// Returns (`Step(deferred.Completion)`): A typed workflow step description; execution is deferred to the engine.
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

        /// Completes a typed signal using its stable identity and result schema.
        ///
        /// Parameters:
        /// - `SignalType: comptime type`: Compile-time signal payload type.
        /// - `ProviderError: comptime type`: Provider failure type retained in the typed result.
        /// - `guarantee: comptime deferred.Guarantee`: Compile-time volatility or recoverability promised by this provider; durable compositions check it before compiling.
        /// - `select_store: comptime fn (*const R) *const deferred.Store(R, ProviderError, guarantee)`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_token: comptime fn (*const R) SignalType.Token`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_encoded_result: comptime fn (*const R) []const u8`: Compile-time typed selector from the current environment;
        ///   its result must remain valid through use.
        /// - `map_error: comptime fn (deferred.Store(R, ProviderError, guarantee).Failure) E`: Compile-time typed function selected at comptime; no runtime callback
        ///   lookup is required.
        /// Returns (`Step(deferred.Completion)`): A typed workflow step description; execution is deferred to the engine.
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

        /// Releases a durable wait registration after observation or cancellation.
        ///
        /// Parameters:
        /// - `ProviderError: comptime type`: Provider failure type retained in the typed result.
        /// - `guarantee: comptime deferred.Guarantee`: Compile-time volatility or recoverability promised by this provider; durable compositions check it before compiling.
        /// - `select_store: comptime fn (*const R) *const deferred.Store(R, ProviderError, guarantee)`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_registration: comptime fn (*const R) deferred.RegistrationView`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// - `map_error: comptime fn (deferred.Store(R, ProviderError, guarantee).Failure) E`: Compile-time typed function selected at comptime; no runtime callback
        ///   lookup is required.
        /// Returns (`Step(void)`): A typed workflow step description; execution is deferred to the engine.
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

        /// Records a named wall-clock deadline through the injected clock store.
        ///
        /// Parameters:
        /// - `ProviderError: comptime type`: Provider failure type retained in the typed result.
        /// - `guarantee: comptime clock.Guarantee`: Compile-time volatility or recoverability promised by this provider; durable compositions check it before compiling.
        /// - `select_store: comptime fn (*const R) *const clock.Store(R, ProviderError, guarantee)`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_schedule: comptime fn (*const R) clock.Schedule`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// - `map_error: comptime fn (clock.Store(R, ProviderError, guarantee).Error) E`: Compile-time typed function selected at comptime; no runtime callback
        ///   lookup is required.
        /// Returns (`Step(clock.ScheduleResult)`): A typed workflow step description; execution is deferred to the engine.
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

        /// Cancels a named deadline without altering unrelated timers.
        ///
        /// Parameters:
        /// - `ProviderError: comptime type`: Provider failure type retained in the typed result.
        /// - `guarantee: comptime clock.Guarantee`: Compile-time volatility or recoverability promised by this provider; durable compositions check it before compiling.
        /// - `select_store: comptime fn (*const R) *const clock.Store(R, ProviderError, guarantee)`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_identity: comptime fn (*const R) clock.Identity`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// - `map_error: comptime fn (clock.Store(R, ProviderError, guarantee).Error) E`: Compile-time typed function selected at comptime; no runtime callback
        ///   lookup is required.
        /// Returns (`Step(clock.CancelResult)`): A typed workflow step description; execution is deferred to the engine.
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

        /// Claims due deadlines according to the provider ownership contract.
        ///
        /// Parameters:
        /// - `ProviderError: comptime type`: Provider failure type retained in the typed result.
        /// - `guarantee: comptime clock.Guarantee`: Compile-time volatility or recoverability promised by this provider; durable compositions check it before compiling.
        /// - `select_store: comptime fn (*const R) *const clock.Store(R, ProviderError, guarantee)`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_policy: comptime fn (*const R) clock.ClaimPolicy`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `map_error: comptime fn (clock.Store(R, ProviderError, guarantee).Error) E`: Compile-time typed function selected at comptime; no runtime callback
        ///   lookup is required.
        /// Returns (`Step(clock.ClaimResult)`): A typed workflow step description; execution is deferred to the engine.
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

        /// Acknowledges a claimed deadline after its wakeup is recorded.
        ///
        /// Parameters:
        /// - `ProviderError: comptime type`: Provider failure type retained in the typed result.
        /// - `guarantee: comptime clock.Guarantee`: Compile-time volatility or recoverability promised by this provider; durable compositions check it before compiling.
        /// - `select_store: comptime fn (*const R) *const clock.Store(R, ProviderError, guarantee)`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_token: comptime fn (*const R) clock.Token`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `map_error: comptime fn (clock.Store(R, ProviderError, guarantee).Error) E`: Compile-time typed function selected at comptime; no runtime callback
        ///   lookup is required.
        /// Returns (`Step(clock.Acknowledgement)`): A typed workflow step description; execution is deferred to the engine.
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

        /// Offers work through PersistedQueue and observes its durable deferred result.
        ///
        /// Parameters:
        /// - `QueueProviderError: comptime type`: Compile-time error type returned by the queue provider.
        /// - `DeferredProviderError: comptime type`: Compile-time error type returned by the deferred provider.
        /// - `queue_guarantee: comptime persisted_queue.Guarantee`: Compile-time the queue guarantee input required by this typed
        ///   operation.
        /// - `deferred_guarantee: comptime deferred.Guarantee`: Compile-time the deferred guarantee input required by this
        ///   typed operation.
        /// - `select_queue: comptime fn (*const R) *const persisted_queue.Store(R, QueueProviderError, queue_guarantee)`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_deferred: comptime fn (*const R) *const deferred.Store(R, DeferredProviderError, deferred_guarantee)`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// - `select_submission: comptime fn (*const R) persisted_queue.Submission`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// - `select_identity: comptime fn (*const R) deferred.Identity`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// - `map_error: comptime fn (queue.Error(persisted_queue.Store(R, QueueProviderError, queue_guarantee).Failure, deferred.Store(R, DeferredProviderError, deferred_guarantee).Failure)) E`: Compile-time typed function selected at comptime; no runtime callback
        ///   lookup is required.
        /// Returns (`Step(queue.Submission)`): A typed workflow step description; execution is deferred to the engine.
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

        /// Runs a named branch and records its Exit before replay can reuse it.
        ///
        /// Parameters:
        /// - `A: comptime type`: Value type produced by a successful computation.
        /// - `branch: comptime Step(A)`: Typed workflow branch to record or compose.
        /// - `ProviderError: comptime type`: Provider failure type retained in the typed result.
        /// - `CodecError: comptime type`: Failure type returned by the selected encoder or decoder.
        /// - `guarantee: comptime parallel.Guarantee`: Compile-time volatility or recoverability promised by this provider; durable compositions check it before compiling.
        /// - `select_journal: comptime fn (*const R) *const parallel.Journal(R, ProviderError, guarantee)`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// - `select_branch: comptime fn (*const R) parallel.Branch`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_attempt: comptime fn (*const R) u64`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// - `encode_exit: comptime fn (*const outcomes.Exit(A, E), *const R) outcomes.Exit(outcomes.OwnedSlice(u8), CodecError)`: Compile-time typed function selected at comptime; no runtime callback
        ///   lookup is required.
        /// - `decode_exit: comptime fn ([]const u8, *const R) outcomes.Exit(outcomes.Exit(A, E), CodecError)`: Compile-time typed function selected at comptime; no runtime callback
        ///   lookup is required.
        /// - `map_error: comptime fn (parallel.RunError(parallel.Journal(R, ProviderError, guarantee).Failure, CodecError)) E`: Compile-time typed function selected at comptime; no runtime callback
        ///   lookup is required.
        /// Returns (`Step(parallel.Recorded(A, E))`): A typed workflow step description; execution is deferred to the engine.
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

        /// Applies an explicit retry policy to a described durable branch.
        ///
        /// Parameters:
        /// - `A: comptime type`: Value type produced by a successful computation.
        /// - `policy: comptime parallel.Policy`: Compile-time caller-selected, validated behavior or resource bounds; no
        ///   hidden default is installed.
        /// - `branch: comptime Step(A)`: Typed workflow branch to record or compose.
        /// Returns (`Step(A)`): A typed workflow step description; execution is deferred to the engine.
        pub fn retryBranch(comptime A: type, comptime policy: parallel.Policy, comptime branch: Step(A)) Step(A) {
            return .{ .program = parallel.retryBranch(A, E, R, policy, branch.program) };
        }

        /// Records a winning branch outcome before reporting race completion.
        ///
        /// Parameters:
        /// - `A: comptime type`: Value type produced by a successful computation.
        /// - `left: comptime Step(A)`: Left-hand typed description in this composition.
        /// - `right: comptime Step(A)`: Right-hand typed description in this composition.
        /// - `ProviderError: comptime type`: Provider failure type retained in the typed result.
        /// - `CodecError: comptime type`: Failure type returned by the selected encoder or decoder.
        /// - `guarantee: comptime parallel.Guarantee`: Compile-time volatility or recoverability promised by this provider; durable compositions check it before compiling.
        /// - `select_journal: comptime fn (*const R) *const parallel.Journal(R, ProviderError, guarantee)`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// - `select_execution: comptime fn (*const R) types.ExecutionId`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// - `select_race_name: comptime fn (*const R) []const u8`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// - `select_left: comptime fn (*const R) parallel.Branch`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_right: comptime fn (*const R) parallel.Branch`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_attempt: comptime fn (*const R) u64`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// - `encode_exit: comptime fn (*const outcomes.Exit(A, E), *const R) outcomes.Exit(outcomes.OwnedSlice(u8), CodecError)`: Compile-time typed function selected at comptime; no runtime callback
        ///   lookup is required.
        /// - `decode_exit: comptime fn ([]const u8, *const R) outcomes.Exit(outcomes.Exit(A, E), CodecError)`: Compile-time typed function selected at comptime; no runtime callback
        ///   lookup is required.
        /// - `map_error: comptime fn (parallel.RaceError(parallel.RunError(parallel.Journal(R, ProviderError, guarantee).Failure, CodecError), parallel.Journal(R, ProviderError, guarantee).Failure, CodecError)) E`: Compile-time typed function selected at comptime; no runtime callback
        ///   lookup is required.
        /// Returns (`Step(parallel.RaceOutcome(A, E))`): A typed workflow step description; execution is deferred to the engine.
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

        /// Runs a named compensation and records its completion outcome.
        ///
        /// Parameters:
        /// - `body: comptime Step(void)`: Compile-time typed callback; its signature is checked at comptime where
        ///   static.
        /// - `ProviderError: comptime type`: Provider failure type retained in the typed result.
        /// - `CodecError: comptime type`: Failure type returned by the selected encoder or decoder.
        /// - `guarantee: comptime compensation.Guarantee`: Compile-time volatility or recoverability promised by this provider; durable compositions check it before compiling.
        /// - `select_journal: comptime fn (*const R) *const compensation.Journal(R, ProviderError, guarantee)`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// - `select_attempt: comptime fn (*const R) compensation.Attempt`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// - `encode_exit: comptime fn (*const outcomes.Exit(void, E), *const R) outcomes.Exit(outcomes.OwnedSlice(u8), CodecError)`: Compile-time typed function selected at comptime; no runtime callback
        ///   lookup is required.
        /// - `decode_exit: comptime fn ([]const u8, *const R) outcomes.Exit(outcomes.Exit(void, E), CodecError)`: Compile-time typed function selected at comptime; no runtime callback
        ///   lookup is required.
        /// - `map_error: comptime fn (compensation.RunError(compensation.Journal(R, ProviderError, guarantee).Failure, CodecError)) E`: Compile-time typed function selected at comptime; no runtime callback
        ///   lookup is required.
        /// Returns (`Step(compensation.Completed(E))`): A typed workflow step description; execution is deferred to the engine.
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

        /// Registers a stable compensation identity for reverse-order recovery.
        ///
        /// Parameters:
        /// - `ProviderError: comptime type`: Provider failure type retained in the typed result.
        /// - `guarantee: comptime compensation.Guarantee`: Compile-time volatility or recoverability promised by this provider; durable compositions check it before compiling.
        /// - `select_journal: comptime fn (*const R) *const compensation.Journal(R, ProviderError, guarantee)`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// - `select_identity: comptime fn (*const R) compensation.Identity`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// - `map_error: comptime fn (compensation.Journal(R, ProviderError, guarantee).Failure) E`: Compile-time typed function selected at comptime; no runtime callback
        ///   lookup is required.
        /// Returns (`Step(compensation.Registration)`): A typed workflow step description; execution is deferred to the engine.
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

        /// Runs pending compensations while preserving original and cleanup failures.
        ///
        /// Parameters:
        /// - `ProviderError: comptime type`: Provider failure type retained in the typed result.
        /// - `CodecError: comptime type`: Failure type returned by the selected encoder or decoder.
        /// - `guarantee: comptime compensation.Guarantee`: Compile-time volatility or recoverability promised by this provider; durable compositions check it before compiling.
        /// - `select_journal: comptime fn (*const R) *const compensation.Journal(R, ProviderError, guarantee)`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// - `select_execution: comptime fn (*const R) types.ExecutionId`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// - `dispatch: comptime fn (*const compensation.Pending, *const R) effects.Effect(compensation.Completed(E), compensation.RunError(compensation.Journal(R, ProviderError, guarantee).Failure, CodecError), R)`: Compile-time typed function selected at comptime; no runtime callback
        ///   lookup is required.
        /// - `map_error: comptime fn (compensation.RunError(compensation.Journal(R, ProviderError, guarantee).Failure, CodecError)) E`: Compile-time typed function selected at comptime; no runtime callback
        ///   lookup is required.
        /// Returns (`Step(outcomes.Exit(void, E))`): Carries no value on `.success` or `E)` on `.fail`; `.die`, `.interrupt`, and `.cleanup` preserve other terminal causes.
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

        /// Records a child result and wakes its parent under stable identities.
        ///
        /// Parameters:
        /// - `ChildProviderError: comptime type`: Compile-time error type returned by the child provider.
        /// - `EngineProviderError: comptime type`: Compile-time error type returned by the selected engine provider.
        /// - `child_guarantee: comptime engine_module.Guarantee`: Compile-time the child guarantee input required by this typed
        ///   operation.
        /// - `engine_guarantee: comptime engine_module.Guarantee`: Compile-time the engine guarantee input required by this typed
        ///   operation.
        /// - `select_children: comptime fn (*const R) *const children.Store(R, ChildProviderError, child_guarantee)`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// - `select_engine: comptime fn (*const R) *const engine_module.WorkflowEngine(R, EngineProviderError, engine_guarantee)`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_link: comptime fn (*const R) children.Link`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_exit: comptime fn (*const R) []const u8`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `map_error: comptime fn (children.Error(ChildProviderError)) E`: Compile-time typed function selected at comptime; no runtime callback
        ///   lookup is required.
        /// Returns (`Step(child_wake.WakeReport(engine_module.Error(EngineProviderError)))`): A typed workflow step description; execution is deferred to the engine.
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

        /// Reconciles retained child completions after a lost wake notification.
        ///
        /// Parameters:
        /// - `ChildProviderError: comptime type`: Compile-time error type returned by the child provider.
        /// - `EngineProviderError: comptime type`: Compile-time error type returned by the selected engine provider.
        /// - `child_guarantee: comptime engine_module.Guarantee`: Compile-time the child guarantee input required by this typed
        ///   operation.
        /// - `engine_guarantee: comptime engine_module.Guarantee`: Compile-time the engine guarantee input required by this typed
        ///   operation.
        /// - `select_children: comptime fn (*const R) *const children.Store(R, ChildProviderError, child_guarantee)`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// - `select_engine: comptime fn (*const R) *const engine_module.WorkflowEngine(R, EngineProviderError, engine_guarantee)`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_parent: comptime fn (*const R) types.ExecutionId`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `map_error: comptime fn (children.Error(ChildProviderError)) E`: Compile-time typed function selected at comptime; no runtime callback
        ///   lookup is required.
        /// Returns (`Step(child_wake.ReconcileReport(engine_module.Error(EngineProviderError)))`): A typed workflow step description; execution is deferred to the engine.
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
