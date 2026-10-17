//! Named typed bindings lowered directly into the existing iterative kernel.
//! Every run has independent, initialized-slot storage; no callback runs while
//! the description is validated or its frame type is discovered.
const std = @import("std");
const effects = @import("../effect.zig");
const outcomes = @import("../outcome.zig");
const rt = @import("../internal/runtime.zig");
const inference = @import("inference.zig");
const scopes = @import("../scope.zig");
const ownership = @import("ownership.zig");

pub fn binding(comptime name: []const u8, comptime inputs: anytype, comptime run: anytype) Binding(name, inputs, run) {
    return .{};
}
fn Binding(comptime label: []const u8, comptime selected: anytype, comptime callback: anytype) type {
    return struct {
        pub const name = label;
        pub const inputs = selected;
        pub const run = callback;
    };
}

/// An explicitly immediate native binding enables allocation-free lowering.
/// Its result is a value; Effect/Exit/error-union results use their own roles.
pub fn native(comptime name: []const u8, comptime inputs: anytype, comptime callback: anytype) Native(name, inputs, callback) {
    return .{};
}
fn Native(comptime label: []const u8, comptime selected: anytype, comptime callback: anytype) type {
    return struct {
        pub const name = label;
        pub const inputs = selected;
        pub const run = callback;
        pub const native_binding = true;
        pub fn result(comptime E: type, comptime R: type, comptime Args: type) type {
            const A = inference.result(callback, @typeInfo(Args).@"struct".field_types, "program native '" ++ label ++ "'");
            if (inference.isEffect(A) or inference.isExit(A) or @typeInfo(A) == .error_union) @compileError("program native binding requires a native value; use an Effect binding for execution/error/Exit adapters");
            return effects.Effect(ownership.Moved(A), E, R);
        }
        pub fn invoke(comptime E: type, comptime R: type, args: anytype) result(E, R, @TypeOf(args)) {
            const Out = result(E, R, @TypeOf(args));
            const Value = @FieldType(Out, "value");
            return .{ .value = .{ .terminal = .{ .success = ownership.move(@call(.auto, callback, args)) } }, .delivery = .terminal, .ready = struct {
                fn ready(_: *const R, value: Value, _: *rt.State) outcomes.Exit(Out.Success, E) {
                    return value.terminal;
                }
            }.ready };
        }
    };
}
/// Return an earlier binding by generated move when the predicate holds.
/// The selected slot's cleanup flag transfers with the result.
pub fn returnWhen(comptime name: []const u8, comptime inputs: anytype, comptime predicate: anytype, comptime source: []const u8) Early(name, inputs, predicate, source) {
    return .{};
}
fn Early(comptime label: []const u8, comptime selected: anytype, comptime predicate: anytype, comptime source_name: []const u8) type {
    return struct {
        pub const name = label;
        pub const inputs = selected;
        pub const run = predicate;
        pub const return_binding = source_name;
        pub fn result(comptime E: type, comptime R: type, comptime Args: type) type {
            const Out = Native(label, selected, predicate).result(E, R, Args);
            if (Out.Success != bool) @compileError("program returnWhen predicate requires bool");
            return Out;
        }
        pub fn invoke(comptime E: type, comptime R: type, args: anytype) result(E, R, @TypeOf(args)) {
            return Native(label, selected, predicate).invoke(E, R, args);
        }
    };
}
fn isEarly(comptime Node: type) bool {
    return @hasDecl(Node, "return_binding");
}

fn allNative(comptime nodes: anytype) bool {
    for (nodes) |node| if (!@hasDecl(@TypeOf(node), "native_binding") and !isEarly(@TypeOf(node)) and !@hasDecl(@TypeOf(node), "initial_capture")) return false;
    return true;
}

/// Both supplied branches validate before a predicate can execute.
pub fn branch(comptime name: []const u8, comptime inputs: anytype, comptime predicate: anytype, comptime when_true: anytype, comptime when_false: anytype) Conditional(name, inputs, predicate, when_true, when_false) {
    return .{};
}
fn Conditional(comptime label: []const u8, comptime selected: anytype, comptime predicate: anytype, comptime when_true: anytype, comptime when_false: anytype) type {
    return struct {
        pub const name = label;
        pub const inputs = selected;
        pub const context_callbacks = .{ predicate, when_true, when_false };
        pub fn contextResult(comptime R: type, comptime Args: type) type {
            _ = R;
            return inference.result(when_true, @typeInfo(Args).@"struct".field_types, "program branch context '" ++ label ++ "'");
        }
        pub fn result(comptime E: type, comptime R: type, comptime Args: type) type {
            const types = @typeInfo(Args).@"struct".field_types;
            if (inference.result(predicate, types, "program branch '" ++ label ++ "' predicate") != bool) @compileError("program branch predicate must return bool");
            const Yes = inference.result(when_true, types, "program branch '" ++ label ++ "' then");
            const No = inference.result(when_false, types, "program branch '" ++ label ++ "' else");
            @import("../dsl.zig").checkContext(Yes, E, R, "program branch then");
            @import("../dsl.zig").checkContext(No, E, R, "program branch else");
            if (Yes != No) @compileError("program branch '" ++ label ++ "' has incompatible result/ownership joins");
            return Yes;
        }
        pub fn invoke(comptime E: type, comptime R: type, args: anytype) @TypeOf(@call(.auto, when_true, args)) {
            _ = E;
            _ = R;
            return if (@call(.auto, predicate, args)) @call(.auto, when_true, args) else @call(.auto, when_false, args);
        }
    };
}
/// Register one borrowed resource handle in the generated lexical scope. Its
/// lifetime encloses subsequent bindings and ends after their work drains.
pub fn resource(comptime name: []const u8, comptime inputs: anytype, comptime acquire: anytype, comptime release: anytype) Resource(name, inputs, acquire, release) {
    return .{};
}
fn Resource(comptime label: []const u8, comptime selected: anytype, comptime acquire: anytype, comptime release_callback: anytype) type {
    return struct {
        pub const name = label;
        pub const inputs = selected;
        pub const run = acquire;
        pub const release = release_callback;
        pub const context_callbacks = .{ acquire, release_callback };
        pub const context_environment_parameters = .{ true, false };
        pub const resource_binding = true;
    };
}
fn isResource(comptime Node: type) bool {
    return @hasDecl(Node, "resource_binding") and Node.resource_binding;
}
fn resources(comptime nodes: anytype) bool {
    for (nodes) |node| if (isResource(@TypeOf(node))) return true;
    return false;
}

pub const LoopControl = @import("loop.zig").Control;
/// Loop-carried state is initialized once per run. Each body returns an exact
/// Effect(LoopControl(Result),E,R); nested program scopes own iteration locals.
pub fn loop(comptime name: []const u8, comptime inputs: anytype, comptime initialize: anytype, comptime condition: anytype, comptime body: anytype, comptime finish: anytype) @import("loop.zig").Node(name, inputs, initialize, condition, body, finish) {
    return .{};
}

fn find(comptime nodes: anytype, comptime before: usize, comptime name: []const u8) usize {
    return struct {
        const value = findIndex(nodes, before, name);
    }.value;
}
fn findIndex(comptime nodes: anytype, comptime before: usize, comptime name: []const u8) usize {
    inline for (0..before) |i| if (std.mem.eql(u8, @TypeOf(nodes[i]).name, name)) return i;
    @compileError("program binding '" ++ name ++ "' is unknown, forward, or outside its region");
}

fn Arguments(comptime E: type, comptime R: type, comptime nodes: anytype, comptime index: usize) type {
    return struct {
        const Type = makeArguments(E, R, nodes, index);
    }.Type;
}
fn makeArguments(comptime E: type, comptime R: type, comptime nodes: anytype, comptime index: usize) type {
    const Node = @TypeOf(nodes[index]);
    var types: [Node.inputs.len + 1]type = undefined;
    inline for (Node.inputs, 0..) |name, i| {
        const slot = comptime find(nodes, index, name);
        types[i] = *const ArgumentValue(Operation(E, R, nodes, slot).Success);
    }
    types[Node.inputs.len] = *const R;
    return @Tuple(&types);
}
// Only this exact generated slot unwraps into the captured input type.
fn BorrowedInput(comptime C: type) type {
    return struct {
        pub const program_input = C;
        pointer: *const C,
    };
}
fn inputType(comptime T: type) ?type {
    if (@typeInfo(T) != .@"struct" or !@hasDecl(T, "program_input")) return null;
    if (T != BorrowedInput(T.program_input)) @compileError("program rejects forged captured input slots");
    return T.program_input;
}
fn Initial(comptime C: type) type {
    return struct {
        pub const name = "inputs";
        pub const inputs = .{};
        pub const initial_capture = C;
        pub const context_callbacks = .{};
        pub fn result(comptime E: type, comptime R: type, comptime Args: type) type {
            _ = Args;
            return effects.Effect(BorrowedInput(C), E, R);
        }
    };
}
fn withInputs(comptime C: type, comptime nodes: anytype) @TypeOf(.{Initial(C){}} ++ nodes) {
    return .{Initial(C){}} ++ nodes;
}
fn ArgumentValue(comptime T: type) type {
    return inputType(T) orelse (ownership.capturedType(T) orelse T);
}
fn slotArgument(value: anytype) *const ArgumentValue(@typeInfo(@TypeOf(value)).pointer.child) {
    const T = @typeInfo(@TypeOf(value)).pointer.child;
    if (comptime inputType(T) != null) return value.pointer;
    if (comptime ownership.capturedType(T) != null) return &value.value;
    return value;
}
fn Operation(comptime E: type, comptime R: type, comptime nodes: anytype, comptime index: usize) type {
    return struct {
        const Type = operationType(E, R, nodes, index);
    }.Type;
}
fn operationType(comptime E: type, comptime R: type, comptime nodes: anytype, comptime index: usize) type {
    const Node = @TypeOf(nodes[index]);
    const Args = Arguments(E, R, nodes, index);
    const fields = @typeInfo(Args).@"struct".field_types;
    const Out = if (@hasDecl(Node, "result")) Node.result(E, R, Args) else inference.result(Node.run, fields, "program binding '" ++ Node.name ++ "'");
    @import("../dsl.zig").checkContext(Out, E, R, "program binding '" ++ Node.name ++ "'");
    return Out;
}
fn validate(comptime E: type, comptime R: type, comptime nodes: anytype) void {
    if (nodes.len == 0) @compileError("program requires a final result binding");
    inline for (0..nodes.len) |index| {
        _ = struct {
            const checked = validateNode(E, R, nodes, index);
        }.checked;
    }
}
fn validateNode(comptime E: type, comptime R: type, comptime nodes: anytype, comptime i: usize) void {
    const N = @TypeOf(nodes[i]);
    if (N.name.len == 0) @compileError("program binding needs a nonempty name");
    inline for (0..i) |j| if (std.mem.eql(u8, @TypeOf(nodes[j]).name, N.name)) @compileError("program duplicate binding '" ++ N.name ++ "'");
    ownership.validate(Operation(E, R, nodes, i).Success);
    if (i + 1 == nodes.len and @hasDecl(N, "initial_capture")) @compileError("program captured inputs require a result binding");
    if (i + 1 == nodes.len and isResource(N)) @compileError("program resource binding cannot escape its lexical region; return a value or explicitly retained owner");
    if (isEarly(N)) {
        if (i + 1 == nodes.len) @compileError("program returnWhen requires a final result on its continuing path");
        const source = comptime find(nodes, i, N.return_binding);
        if (@hasDecl(@TypeOf(nodes[source]), "initial_capture")) @compileError("program captured input storage cannot escape its scope");
        if (isResource(@TypeOf(nodes[source]))) @compileError("program resource binding cannot escape its lexical region");
        if (ownership.Moved(Operation(E, R, nodes, source).Success) != ownership.Moved(Operation(E, R, nodes, nodes.len - 1).Success)) @compileError("program early return type must match the final result");
    }
}

fn contextCallbacks(comptime N: type) if (@hasDecl(N, "context_callbacks")) @TypeOf(N.context_callbacks) else @TypeOf(.{N.run}) {
    return if (@hasDecl(N, "context_callbacks")) N.context_callbacks else .{N.run};
}
fn ContextEvidence(comptime A: type, comptime E: ?type) type {
    return struct {
        const Success = A;
        const Error = E;
    };
}
fn nodeContext(comptime R: type, comptime nodes: anytype, comptime index: usize) type {
    return struct {
        const Resolved = resolveNodeContext(R, nodes, index);
        const Success = Resolved.Success;
        const Error = Resolved.Error;
    };
}
fn resolveNodeContext(comptime R: type, comptime nodes: anytype, comptime index: usize) type {
    const N = @TypeOf(nodes[index]);
    if (@hasDecl(N, "initial_capture")) return ContextEvidence(BorrowedInput(N.initial_capture), null);
    const Args = comptime blk: {
        var types: [N.inputs.len + 1]type = undefined;
        for (N.inputs, 0..) |name, i| {
            const slot = find(nodes, index, name);
            types[i] = *const ArgumentValue(nodeContext(R, nodes, slot).Success);
        }
        types[N.inputs.len] = *const R;
        break :blk @Tuple(&types);
    };
    if (@hasDecl(N, "native_binding") or isEarly(N)) return ContextEvidence(N.result(void, R, Args).Success, null);
    const Out = if (@hasDecl(N, "contextResult")) N.contextResult(R, Args) else inference.result(N.run, @typeInfo(Args).@"struct".field_types, "program context '" ++ N.name ++ "'");
    inference.requireEffect(Out, "program context '" ++ N.name ++ "'");
    if (Out.Environment != R) @compileError("program callback environment evidence conflicts; use explicit projection");
    return ContextEvidence(Out.Success, Out.Error);
}
fn InferredContext(comptime anchors: anytype, comptime nodes: anytype) type {
    return struct {
        const Resolved = resolveContext(anchors, nodes);
        const Environment = Resolved.Environment;
        const Error = Resolved.Error;
    };
}
fn resolveContext(comptime anchors: anytype, comptime nodes: anytype) type {
    const roles = @import("roles.zig");
    roles.validate(roles.recordType(anchors), "program.describeWith", &.{}, &.{ "environment", "error_type" });
    if (nodes.len == 0) @compileError("program requires a final result binding");
    var environment: ?type = if (roles.has(roles.recordType(anchors), "environment")) roles.get("environment", anchors) else null;
    var failure: ?type = if (roles.has(roles.recordType(anchors), "error_type")) roles.get("error_type", anchors) else null;
    inline for (nodes) |node| inline for (contextCallbacks(@TypeOf(node)), 0..) |callback, callback_index| {
        const f = inference.info(callback, "program context");
        const candidate: ?type = if (f.return_type) |Return|
            if (inference.isEffect(Return)) Return.Environment else null
        else
            null;
        if (f.return_type) |Return| if (inference.isEffect(Return)) {
            if (failure) |known| {
                if (known != Return.Error) @compileError("program callback error evidence conflicts; use explicit error mapping");
            } else failure = Return.Error;
        };
        const has_environment_parameter = if (@hasDecl(@TypeOf(node), "context_environment_parameters")) @TypeOf(node).context_environment_parameters[callback_index] else true;
        const parameter: ?type = if (has_environment_parameter and f.param_types.len != 0) f.param_types[f.param_types.len - 1] else null;
        const selected: ?type = candidate orelse if (parameter) |P|
            if (@typeInfo(P) == .pointer and @typeInfo(P).pointer.size == .one and @typeInfo(P).pointer.attrs.@"const") @typeInfo(P).pointer.child else null
        else
            null;
        if (selected) |R| {
            if (environment) |known| {
                if (known != R) @compileError("program callback environment evidence conflicts; use explicit projection");
            } else environment = R;
        }
    };
    const R = environment orelse @compileError("program.describe needs an environment anchor; use describeWith(.{ .environment = R }, nodes)");
    inline for (0..nodes.len) |index| {
        if (nodeContext(R, nodes, index).Error) |E| {
            if (failure) |known| {
                if (known != E) @compileError("program callback error evidence conflicts; use explicit error mapping");
            } else failure = E;
        }
    }
    const E = failure orelse void;
    return struct {
        const Environment = R;
        const Error = E;
    };
}
/// Infer the available context from callback signatures and binding data flow.
pub fn describe(comptime nodes: anytype) ProgramResult(InferredContext(.{}, nodes).Error, InferredContext(.{}, nodes).Environment, nodes) {
    return describeWith(.{}, nodes);
}
/// Supply only context evidence that is absent from the callback graph.
pub fn describeWith(comptime anchors: anytype, comptime nodes: anytype) ProgramResult(InferredContext(anchors, nodes).Error, InferredContext(anchors, nodes).Environment, nodes) {
    const C = InferredContext(anchors, nodes);
    return build(C.Error, C.Environment, nodes);
}

fn ProgramResult(comptime E: type, comptime R: type, comptime nodes: anytype) type {
    if (nodes.len == 0) @compileError("program requires a final result binding");
    return effects.Effect(ownership.Moved(Operation(E, R, nodes, nodes.len - 1).Success), E, R);
}
pub fn build(comptime E: type, comptime R: type, comptime nodes: anytype) ProgramResult(E, R, nodes) {
    return buildInternal(E, R, nodes, null);
}
fn Entry(comptime C: type, comptime E: type) type {
    return struct { input: *const C, scope: *scopes.Scope(E) };
}
fn Captured(comptime P: type, comptime C: type, comptime nodes: anytype) type {
    if (@typeInfo(P) != .pointer or @typeInfo(P).pointer.size != .one) @compileError("program.captured requires its construction environment pointer");
    const R = @typeInfo(P).pointer.child;
    const expanded = withInputs(C, nodes);
    const Context = InferredContext(.{ .environment = R }, expanded);
    return struct {
        pub const Out = ProgramResult(Context.Error, R, expanded);
        const Capture = ownership.Capture(C);
        fn owned(input: *const C, _: *const R) Out {
            var described = buildInternal(Context.Error, R, expanded, C);
            described.value = .{ .borrowed = input };
            return described;
        }
        fn use(stored: *const Capture, scope: *scopes.Scope(Context.Error), env: *const R) Out {
            var seed = effects.Effect(void, Context.Error, R).succeed({});
            return seed.flatMapCapturedOwned(env, Out.Success, Entry(C, Context.Error){ .input = &stored.value, .scope = scope }, struct {
                fn start(entry: *const Entry(C, Context.Error), _: void, _: *const R) Out {
                    var described = buildInternal(Context.Error, R, expanded, C);
                    described.value = .{ .borrowed = entry };
                    return described;
                }
            }.start);
        }
    };
}
/// Move runtime inputs into one generated lexical scope. Bindings select the
/// reserved earlier name "inputs"; their callbacks receive *const Input.
/// Pointers returned by native callbacks retain their ordinary borrow contract.
pub fn captured(env: anytype, input: anytype, comptime nodes: anytype) Captured(@TypeOf(env), @TypeOf(input), nodes).Out {
    const Description = Captured(@TypeOf(env), @TypeOf(input), nodes);
    return if (comptime resources(nodes)) Description.Out.scopedCaptured(env, Description.Capture{ .value = input }, Description.use) else @import("../dsl.zig").owned(env, input, Description.owned);
}
fn buildInternal(comptime E: type, comptime R: type, comptime nodes: anytype, comptime Input: ?type) ProgramResult(E, R, nodes) {
    comptime validate(E, R, nodes);
    const Out = ProgramResult(E, R, nodes);
    if (comptime allNative(nodes)) {
        const Immediate = struct {
            const Slots = blk: {
                var types: [nodes.len]type = undefined;
                for (0..nodes.len) |i| types[i] = Operation(E, R, nodes, i).Success;
                break :blk @Tuple(&types);
            };
            fn run(env: *const R, captured_value: @FieldType(Out, "value"), state: *rt.State) outcomes.Exit(Out.Success, E) {
                const input: if (Input) |C| *const C else void = if (comptime Input != null) @ptrCast(@alignCast(captured_value.borrowed)) else {};
                var slots: Slots = undefined;
                var initialized: [nodes.len]bool = @splat(false);
                defer if (comptime ownership.containsOwned(Slots)) inline for (0..nodes.len) |i| {
                    const index = nodes.len - i - 1;
                    if (initialized[index]) ownership.discard(&slots[index]);
                };
                inline for (nodes, 0..) |node, index| {
                    if (state.cancelled()) return .interrupted;
                    const N = @TypeOf(node);
                    var args: Arguments(E, R, nodes, index) = undefined;
                    inline for (N.inputs, 0..) |name, i| {
                        const slot = comptime find(nodes, index, name);
                        args[i] = slotArgument(&slots[slot]);
                    }
                    args[N.inputs.len] = env;
                    slots[index] = if (comptime @hasDecl(N, "initial_capture")) .{ .pointer = input } else ownership.move(@call(.auto, N.run, args));
                    initialized[index] = true;
                    if (comptime isEarly(N)) if (slots[index]) {
                        if (state.cancelled()) return .interrupted;
                        const source = comptime find(nodes, index, N.return_binding);
                        initialized[source] = false;
                        return .{ .success = ownership.move(slots[source]) };
                    };
                }
                if (state.cancelled()) return .interrupted;
                initialized[nodes.len - 1] = false;
                return .{ .success = ownership.move(slots[nodes.len - 1]) };
            }
        };
        return if (comptime Input == null) Out.RuntimeAdapter.specialize(.{ .ready = Immediate.run }) else .{ .ready = Immediate.run };
    }
    const Parent = Out.RuntimeAdapter.Job;
    const Frame = struct {
        const Self = @This();
        allocator: std.mem.Allocator,
        parent: *Parent,
        inherited: rt.Services,
        scope: ?*scopes.Scope(E) = null,
        input: if (Input) |C| *const C else void = undefined,
        frame_finalizer: scopes.Scope(E).Finalizer = undefined,
        slot_finalizers: [nodes.len]scopes.Scope(E).Finalizer = undefined,
        initialized: [nodes.len]bool = @splat(false),
        slots: Slots = undefined,
        jobs: Jobs = undefined,
        exits: Exits = undefined,
        arguments: ArgumentSlots = undefined,
        const Slots = blk: {
            var types: [nodes.len]type = undefined;
            for (0..nodes.len) |i| types[i] = Operation(E, R, nodes, i).Success;
            break :blk @Tuple(&types);
        };
        const Jobs = blk: {
            var types: [nodes.len]type = undefined;
            for (0..nodes.len) |i| types[i] = Operation(E, R, nodes, i).RuntimeAdapter.Job;
            break :blk @Tuple(&types);
        };
        const ArgumentSlots = blk: {
            var types: [nodes.len]type = undefined;
            for (nodes, 0..) |node, i| types[i] = if (@hasDecl(@TypeOf(node), "deferred_inputs")) Arguments(E, R, nodes, i) else void;
            break :blk @Tuple(&types);
        };
        const Exits = blk: {
            var types: [nodes.len]type = undefined;
            for (0..nodes.len) |i| types[i] = outcomes.Exit(Operation(E, R, nodes, i).Success, E);
            break :blk @Tuple(&types);
        };

        fn finish(self: *@This(), value: outcomes.Exit(Out.Success, E)) rt.Step {
            if (comptime resources(nodes)) return self.parent.adapterFinish(value);
            inline for (0..nodes.len) |i| {
                const reverse = nodes.len - i - 1;
                if (comptime !isResource(@TypeOf(nodes[reverse]))) {
                    if (self.initialized[reverse]) ownership.discard(&self.slots[reverse]);
                }
            }
            const parent = self.parent;
            self.allocator.destroy(self);
            return parent.adapterFinish(value);
        }
        fn noDestroy(_: *scopes.Scope(E).Finalizer) void {}
        fn noRelease(_: *anyopaque) void {}
        fn destroyFrame(header: *scopes.Scope(E).Finalizer) void {
            const frame: *Self = @alignCast(@fieldParentPtr("frame_finalizer", header));
            frame.allocator.destroy(frame);
        }
        fn frameDone(_: *scopes.Scope(E).Finalizer, _: *rt.State, sink: rt.Sink(void, E)) rt.Step {
            return sink.deliver(.{ .success = {} });
        }
        fn registerFrame(self: *Self) bool {
            self.frame_finalizer = .{ .start = frameDone, .destroy = destroyFrame, .storage = .{ .allocator = self.allocator, .owner = self, .release = noRelease } };
            return self.scope.?.register(&self.frame_finalizer);
        }
        fn registerSlot(self: *Self, comptime index: usize) void {
            self.slot_finalizers[index] = .{ .start = struct {
                fn release(header: *scopes.Scope(E).Finalizer, _: *rt.State, sink: rt.Sink(void, E)) rt.Step {
                    const base: [*]scopes.Scope(E).Finalizer = @as([*]scopes.Scope(E).Finalizer, @ptrCast(header)) - index;
                    const frame: *Self = @alignCast(@fieldParentPtr("slot_finalizers", @as(*[nodes.len]scopes.Scope(E).Finalizer, @ptrCast(base))));
                    if (frame.initialized[index]) {
                        frame.initialized[index] = false;
                        ownership.discard(&frame.slots[index]);
                    }
                    return sink.deliver(.{ .success = {} });
                }
            }.release, .destroy = noDestroy, .storage = .{ .allocator = self.allocator, .owner = self, .release = noRelease } };
            std.debug.assert(self.scope.?.register(&self.slot_finalizers[index]));
        }
        fn begin(self: *@This(), state: *rt.State, comptime index: usize) rt.Step {
            if (state.cancelled()) return self.finish(.interrupted);
            const Node = @TypeOf(nodes[index]);
            var args: Arguments(E, R, nodes, index) = undefined;
            inline for (Node.inputs, 0..) |name, i| {
                const slot = comptime find(nodes, index, name);
                std.debug.assert(self.initialized[slot]);
                args[i] = slotArgument(&self.slots[slot]);
            }
            args[Node.inputs.len] = self.parent.env;
            if (comptime @hasDecl(Node, "deferred_inputs")) self.arguments[index] = args;
            var selected = if (comptime @hasDecl(Node, "initial_capture")) Operation(E, R, nodes, index).succeed(.{ .pointer = self.input }) else if (comptime @hasDecl(Node, "deferred_inputs")) Node.invoke(E, R, &self.arguments[index]) else if (comptime @hasDecl(Node, "invoke")) Node.invoke(E, R, args) else @call(.auto, Node.run, args);
            const described = if (comptime isResource(Node)) selected.scopeAcquireAllocated(self.allocator, self.scope.?, Node.release) else selected;
            const Success = Operation(E, R, nodes, index).Success;
            if (comptime ownership.Moved(Success) != Success) {
                // The callback has already produced this immediate value. Pin
                // its composite ownership before ordinary child admission can
                // observe cancellation and skip the terminal ready function.
                if (Operation(E, R, nodes, index).RuntimeAdapter.terminalSuccess(&described)) |value| {
                    self.slots[index] = value.*;
                    self.initialized[index] = true;
                    if (comptime resources(nodes) and !isResource(Node)) self.registerSlot(index);
                }
            }
            self.jobs[index] = .{ .program = described, .owned = described.ownership, .inherited = self.inherited, .env = self.parent.env, .sink = .{ .output = &self.exits[index], .next_step = .{ .data = self, .advance = struct {
                fn complete(active: *rt.State, data: ?*anyopaque) rt.Step {
                    const frame: *Self = @ptrCast(@alignCast(data.?));
                    const value = frame.exits[index];
                    if (value != .success) return frame.finish(outcomes.castFailure(Out.Success, active.publish(value)));
                    const staged = frame.initialized[index];
                    frame.slots[index] = value.success;
                    frame.initialized[index] = true;
                    if (comptime resources(nodes) and !isResource(Node)) if (!staged) frame.registerSlot(index);
                    if (!active.committed_delivery and active.cancelled()) return frame.finish(.interrupted);
                    if (comptime isEarly(Node)) if (value.success) {
                        const source = comptime find(nodes, index, Node.return_binding);
                        frame.initialized[source] = false;
                        return frame.finish(.{ .success = ownership.move(frame.slots[source]) });
                    };

                    if (index + 1 == nodes.len) {
                        frame.initialized[index] = false;
                        return frame.finish(.{ .success = ownership.move(frame.slots[index]) });
                    }
                    return frame.begin(active, index + 1);
                }
            }.complete } } };
            return self.jobs[index].adapterStep();
        }
    };
    const Adapter = struct {
        fn start(state: *rt.State, parent: *Parent) rt.Step {
            const allocator = state.services.allocatorValue() orelse return parent.adapterFinish(.{ .die = .{ .kind = .invalid_configuration, .message = "program requires an environment allocator for per-run slots" } });
            const frame = allocator.create(Frame) catch return parent.adapterFinish(.{ .die = outcomes.Defect.out_of_memory });
            const entry: if (Input == null or !resources(nodes)) void else *const Entry(Input.?, E) = if (comptime Input == null or !resources(nodes)) {} else @ptrCast(@alignCast(parent.program.value.borrowed));
            frame.* = .{ .allocator = allocator, .parent = parent, .inherited = state.services, .scope = if (comptime !resources(nodes)) null else if (comptime Input != null) entry.scope else @ptrCast(@alignCast(@constCast(parent.program.value.borrowed))), .input = if (comptime Input == null) {} else if (comptime resources(nodes)) entry.input else @ptrCast(@alignCast(parent.program.value.borrowed)) };
            if (comptime resources(nodes)) if (!frame.registerFrame()) {
                allocator.destroy(frame);
                return parent.adapterFinish(.{ .die = .{ .kind = .invalid_configuration, .message = "program lexical scope closed before frame admission" } });
            };
            return frame.begin(state, 0);
        }
        fn scoped(scope: *scopes.Scope(E)) Out {
            return .{ .value = .{ .borrowed = scope }, .start = start };
        }
    };
    return if (comptime Input == null and resources(nodes)) Out.scoped(Adapter.scoped) else .{ .start = Adapter.start };
}
