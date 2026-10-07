//! Generic collection composition with shared-runtime child execution.
const std = @import("std");
const outcomes = @import("../outcome.zig");
const effects = @import("../effect.zig");
const rt = @import("../internal/runtime.zig");
/// S provides Representation, count, at, Builder, init, append, finish,
/// discard; bounds and shape failures use the child's typed error E.
pub fn Sequence(comptime child: anytype, comptime S: type, comptime D: type, comptime W: type, comptime E: type, comptime maximum: usize, comptime capacity_error: fn () E, comptime encode_capacity_error: fn () E) type {
    const C = @TypeOf(child);
    const A = C.Decoded;
    const Rep = S.Representation;
    comptime {
        if (maximum == 0) @compileError("sequence capacity must be positive");
        if ((C.DecodingEnvironment != void and C.DecodingEnvironment != D) or (C.EncodingEnvironment != void and C.EncodingEnvironment != W)) @compileError("sequence codec environments must match child environments");
    }
    return struct {
        /// Application value produced on success; ownership follows the codec’s declared value type.
        pub const Decoded = outcomes.OwnedSlice(A);
        /// Wire or provider representation consumed and produced by this codec.
        pub const Representation = Rep;
        /// Application failure type; defects, interruption and finalization failures remain separate Exit channels.
        pub const Error = E;
        /// Services needed by decoding only; encoding services are not acquired by this direction.
        pub const DecodingEnvironment = D;
        /// Services needed by encoding only; decoding services are not acquired by this direction.
        pub const EncodingEnvironment = W;
        const Decode = effects.Effect(Decoded, E, D);
        const Encode = effects.Effect(Rep, E, W);
        const DecodeFrame = struct {
            parent: *Decode.RuntimeAdapter.Job,
            allocator: std.mem.Allocator,
            input: *const Rep,
            items: []A,
            count: usize = 0,
            child_job: effects.Effect(A, E, C.DecodingEnvironment).RuntimeAdapter.Job = undefined,
            result: outcomes.Exit(A, E) = undefined,
            fn fail(self: *@This(), result: outcomes.Exit(Decoded, E)) rt.Step {
                for (self.items[0..self.count]) |*item| outcomes.discardPayload(item);
                self.allocator.free(self.items);
                const parent = self.parent;
                self.allocator.destroy(self);
                return parent.adapterFinish(result);
            }
            fn complete(state: *rt.State, data: ?*anyopaque) rt.Step {
                const frame: *@This() = @ptrCast(@alignCast(data.?));
                const result = state.publish(frame.result);
                if (result != .success) return frame.fail(outcomes.castFailure(Decoded, result));
                frame.items[frame.count] = result.success;
                frame.count += 1;
                return .{ .data = frame, .advance = advance };
            }
            fn advance(state: *rt.State, data: ?*anyopaque) rt.Step {
                const frame: *@This() = @ptrCast(@alignCast(data.?));
                if (state.cancelled()) return frame.fail(.interrupted);
                if (frame.count == frame.items.len) {
                    const result: Decoded = .{ .items = frame.items, .allocator = frame.allocator };
                    const parent = frame.parent;
                    frame.allocator.destroy(frame);
                    return parent.adapterFinish(.{ .success = result });
                }
                const program = child.decode(S.at(frame.input, frame.count));
                frame.child_job = .{ .program = program, .owned = program.ownership, .env = if (C.DecodingEnvironment == void) &{} else frame.parent.env, .inherited = state.services, .sink = .{ .output = &frame.result, .next_step = .{ .data = frame, .advance = complete } } };
                return frame.child_job.adapterStep();
            }
        };
        fn startDecode(state: *rt.State, parent: *Decode.RuntimeAdapter.Job) rt.Step {
            const input: *const Rep = @ptrCast(@alignCast(parent.program.value.borrowed));
            const count = S.count(input);
            if (count > maximum) return parent.adapterFinish(.{ .fail = capacity_error() });
            const allocator = state.services.allocatorValue() orelse return parent.adapterFinish(.{ .die = outcomes.Defect.out_of_memory });
            const frame = allocator.create(DecodeFrame) catch return parent.adapterFinish(.{ .die = outcomes.Defect.out_of_memory });
            const items = allocator.alloc(A, count) catch {
                allocator.destroy(frame);
                return parent.adapterFinish(.{ .die = outcomes.Defect.out_of_memory });
            };
            frame.* = .{ .parent = parent, .allocator = allocator, .input = input, .items = items };
            return .{ .data = frame, .advance = DecodeFrame.advance };
        }
        /// Construct lazy sequential decoding on the shared trampoline.
        /// Borrow input through join; success owns an OwnedSlice of child values and failure releases partial children.
        pub fn decode(input: *const Rep) Decode {
            return .{ .value = .{ .borrowed = @ptrCast(input) }, .start = startDecode };
        }
        const EncodeFrame = struct {
            parent: *Encode.RuntimeAdapter.Job,
            allocator: std.mem.Allocator,
            input: *const Decoded,
            builder: S.Builder,
            count: usize = 0,
            child_job: effects.Effect(C.Representation, E, C.EncodingEnvironment).RuntimeAdapter.Job = undefined,
            result: outcomes.Exit(C.Representation, E) = undefined,
            fn fail(self: *@This(), result: outcomes.Exit(Rep, E)) rt.Step {
                S.discard(&self.builder);
                const parent = self.parent;
                self.allocator.destroy(self);
                return parent.adapterFinish(result);
            }
            fn complete(state: *rt.State, data: ?*anyopaque) rt.Step {
                const frame: *@This() = @ptrCast(@alignCast(data.?));
                var result = state.publish(frame.result);
                if (result != .success) return frame.fail(outcomes.castFailure(Rep, result));
                const appended = S.append(frame.parent.env, &frame.builder, &result.success);
                outcomes.discardPayload(&result.success);
                if (appended != .success) return frame.fail(outcomes.castFailure(Rep, appended));
                frame.count += 1;
                return .{ .data = frame, .advance = advance };
            }
            fn advance(state: *rt.State, data: ?*anyopaque) rt.Step {
                const frame: *@This() = @ptrCast(@alignCast(data.?));
                if (state.cancelled()) return frame.fail(.interrupted);
                if (frame.count == frame.input.items.len) {
                    const result = S.finish(frame.parent.env, &frame.builder);
                    if (result != .success) return frame.fail(result);
                    const parent = frame.parent;
                    frame.allocator.destroy(frame);
                    return parent.adapterFinish(result);
                }
                const program = child.encode(&frame.input.items[frame.count]);
                frame.child_job = .{ .program = program, .owned = program.ownership, .env = if (C.EncodingEnvironment == void) &{} else frame.parent.env, .inherited = state.services, .sink = .{ .output = &frame.result, .next_step = .{ .data = frame, .advance = complete } } };
                return frame.child_job.adapterStep();
            }
        };
        fn startEncode(state: *rt.State, parent: *Encode.RuntimeAdapter.Job) rt.Step {
            const input: *const Decoded = @ptrCast(@alignCast(parent.program.value.borrowed));
            if (input.items.len > maximum) return parent.adapterFinish(.{ .fail = encode_capacity_error() });
            const initialized = S.init(parent.env, input.items.len);
            if (initialized != .success) return parent.adapterFinish(outcomes.castFailure(Rep, initialized));
            const allocator = state.services.allocatorValue() orelse {
                var builder = initialized.success;
                S.discard(&builder);
                return parent.adapterFinish(.{ .die = outcomes.Defect.out_of_memory });
            };
            const frame = allocator.create(EncodeFrame) catch {
                var builder = initialized.success;
                S.discard(&builder);
                return parent.adapterFinish(.{ .die = outcomes.Defect.out_of_memory });
            };
            frame.* = .{ .parent = parent, .allocator = allocator, .input = input, .builder = initialized.success };
            return .{ .data = frame, .advance = EncodeFrame.advance };
        }
        /// Construct lazy sequential encoding using W and the representation builder.
        /// append borrows each child representation until it copies/retains it, then child owners are discarded.
        /// finish transfers the builder on success; discard handles failure/cancellation.
        pub fn encode(input: *const Decoded) Encode {
            return .{ .value = .{ .borrowed = @ptrCast(input) }, .start = startEncode };
        }
    };
}
