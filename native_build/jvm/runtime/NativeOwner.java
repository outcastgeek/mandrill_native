package nativebuild.ffm;

import java.lang.foreign.MemorySegment;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.CompletionStage;
import java.util.concurrent.atomic.AtomicReference;
import java.util.function.Function;

/** Prevents owner close from racing native submission, without waiting for borrowers. */
public final class NativeOwner {
    public sealed interface CloseResult {
        record Accepted(CompletionStage<Void> completion) implements CloseResult {}
        record Rejected(Throwable error) implements CloseResult {}
    }
    private sealed interface State {}
    private record Open(int users) implements State {}
    private record Closing(int users, CompletableFuture<Void> completion) implements State {}
    private enum Closed implements State { INSTANCE }
    private final AtomicReference<State> state = new AtomicReference<>(new Open(0));
    private final MemorySegment handle;
    private final Function<MemorySegment, CloseResult> closer;

    private NativeOwner(MemorySegment handle, Function<MemorySegment, CloseResult> closer) {
        if (handle.equals(MemorySegment.NULL)) throw new IllegalArgumentException("null native owner");
        this.handle = handle; this.closer = closer;
    }
    /** Binding implementers call this only for a newly transferred native ownership reference. */
    public static NativeOwner adopt(MemorySegment handle, Function<MemorySegment, CloseResult> closer) {
        return new NativeOwner(handle, closer);
    }
    /** The action must submit/copy/retain before returning; it must not escape this borrowed handle. */
    public <T> T submit(Function<MemorySegment, T> action) {
        State current = state.get();
        for (;;) {
            if (!(current instanceof Open open)) throw new IllegalStateException("native owner is closing or closed");
            if (state.compareAndSet(current, new Open(Math.incrementExact(open.users())))) break;
            current = state.get();
        }
        try { return action.apply(handle); }
        finally { releaseBorrow(); }
    }
    public CompletionStage<Void> closeAsync() {
        State current = state.get();
        for (;;) {
            if (current instanceof Closing closing) return closing.completion().minimalCompletionStage();
            if (current instanceof Closed) return CompletableFuture.<Void>completedFuture(null).minimalCompletionStage();
            Open open = (Open)current;
            Closing closing = new Closing(open.users(), new CompletableFuture<>());
            if (state.compareAndSet(current, closing)) {
                if (closing.users() == 0) closeNative(closing);
                return closing.completion().minimalCompletionStage();
            }
            current = state.get();
        }
    }
    private void releaseBorrow() {
        State current = state.get();
        for (;;) {
            State next = switch (current) {
                case Open open -> new Open(open.users() - 1);
                case Closing closing -> new Closing(closing.users() - 1, closing.completion());
                case Closed ignored -> throw new IllegalStateException("owner closed while borrowed");
            };
            if (state.compareAndSet(current, next)) {
                if (next instanceof Closing closing && closing.users() == 0) closeNative(closing);
                return;
            }
            current = state.get();
        }
    }
    private void closeNative(Closing closing) {
        try {
            CloseResult result = closer.apply(handle);
            if (result instanceof CloseResult.Rejected rejected) {
                state.set(new Open(0)); closing.completion().completeExceptionally(rejected.error());
                return;
            }
            ((CloseResult.Accepted)result).completion().whenComplete((ignored, error) -> {
                // Accepted close consumes ownership even if native cleanup reports an error.
                state.set(Closed.INSTANCE);
                if (error == null) closing.completion().complete(null);
                else closing.completion().completeExceptionally(error);
            });
        } catch (Throwable error) {
            // An adapter that throws cannot establish whether ownership transferred.
            // Never make that pointer usable again; rejection must be explicit above.
            state.set(Closed.INSTANCE); closing.completion().completeExceptionally(error);
        }
    }
}
