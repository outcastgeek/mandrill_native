package nativebuild.ffm;

import java.lang.foreign.MemorySegment;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.CompletionStage;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.ArrayBlockingQueue;
import java.util.concurrent.ThreadPoolExecutor;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.RejectedExecutionException;
import java.util.concurrent.atomic.AtomicInteger;
import java.util.concurrent.atomic.AtomicLong;
import java.util.function.Function;

/** Ownership of one accepted callback operation, independent of any native project. */
public final class NativeOperation<T> {
    /** Bind these three methods directly to the native cancellation ABI. */
    public interface Cancellation {
        MemorySegment create();
        boolean request(MemorySegment token);
        void release(MemorySegment token);
    }

    @FunctionalInterface public interface Submission {
        /** Zero accepts exactly one callback; nonzero rejects without a callback. */
        int submit(MemorySegment token, MemorySegment context);
    }

    public sealed interface Outcome<T> {
        record Success<T>(T value) implements Outcome<T> {}
        record NativeFailure<T>(int status) implements Outcome<T> {}
        record CallbackFailure<T>(Throwable error) implements Outcome<T> {}
    }

    /** Shared admission and delivery capacity. Never runs continuations on native threads. */
    public static final class Dispatcher {
        private final int capacity;
        private final AtomicInteger admitted = new AtomicInteger();
        private final ThreadPoolExecutor executor;
        public Dispatcher(int parallelism, int capacity) {
            if (parallelism < 1 || capacity < 1) throw new IllegalArgumentException("positive capacity required");
            this.capacity = capacity;
            executor = new ThreadPoolExecutor(parallelism, parallelism, 0, TimeUnit.SECONDS,
                new ArrayBlockingQueue<>(capacity), runnable -> {
                    Thread thread = new Thread(runnable, "native-ffm-completion");
                    thread.setDaemon(true);
                    return thread;
                }, new ThreadPoolExecutor.AbortPolicy());
            executor.prestartAllCoreThreads();
        }
        private void admit() {
            int count = admitted.get();
            do {
                if (count >= capacity) throw new RejectedExecutionException("native completion capacity reached");
            } while (!admitted.compareAndSet(count, count + 1) && (count = admitted.get()) >= 0);
        }
    }

    private static final AtomicLong NEXT = new AtomicLong();
    private static final ConcurrentHashMap<Long, NativeOperation<?>> PENDING = new ConcurrentHashMap<>();
    private final CompletableFuture<Outcome<T>> completion = new CompletableFuture<>();
    private final Cancellation cancellation;
    private final Function<MemorySegment, T> capture;
    private final MemorySegment token;
    // One owner reference plus any cancellation callers currently using the pointer.
    private final AtomicInteger tokenUsers = new AtomicInteger(1);
    private final Dispatcher dispatcher;
    private int submissionStatus = -1;

    private NativeOperation(Cancellation cancellation, Dispatcher dispatcher, Function<MemorySegment, T> capture) {
        this.capture = capture;
        this.cancellation = cancellation;
        this.dispatcher = dispatcher;
        token = cancellation.create();
        if (token.equals(MemorySegment.NULL)) throw new OutOfMemoryError("native cancellation token");
    }

    /** Submission is a native asynchronous downcall, never a Java worker wrapping sync work. */
    public static <T> NativeOperation<T> submit(Cancellation cancellation, Dispatcher dispatcher, Function<MemorySegment, T> capture, Submission submission) {
        dispatcher.admit();
        NativeOperation<T> operation;
        try { operation = new NativeOperation<>(cancellation, dispatcher, capture); }
        catch (Throwable error) { dispatcher.admitted.decrementAndGet(); throw error; }
        long id = NEXT.incrementAndGet();
        if (id <= 0) {
            operation.releaseToken(); dispatcher.admitted.decrementAndGet();
            throw new IllegalStateException("native callback identity space exhausted");
        }
        PENDING.put(id, operation);
        // The callback is allowed to arrive before the downcall returns.
        try {
            int status = submission.submit(operation.token, MemorySegment.ofAddress(id));
            operation.submissionStatus = status;
            if (status != 0 && PENDING.remove(id, operation)) operation.finish(new Outcome.NativeFailure<>(status));
        } catch (Throwable error) {
            if (PENDING.remove(id, operation)) operation.finish(new Outcome.CallbackFailure<>(error));
        }
        return operation;
    }

    /** A cancellation request does not end the callback or native-memory lifetime. */
    public boolean cancel() {
        int count = tokenUsers.get();
        while (count != 0) {
            if (tokenUsers.compareAndSet(count, count + 1)) {
                try { return cancellation.request(token); }
                finally { releaseToken(); }
            }
            count = tokenUsers.get();
        }
        return false;
    }

    /** Native acceptance is established before this operation is returned to its caller. */
    public boolean accepted() { return submissionStatus == 0; }

    /** Consumers cannot complete/cancel the internal future independently of native ownership. */
    public CompletionStage<Outcome<T>> completion() { return completion.minimalCompletionStage(); }

    /** Called by a stable upcall stub. Copy borrowed native data in capture before returning. */
    public static void complete(MemorySegment context, int status, MemorySegment payload) {
        NativeOperation<?> operation = PENDING.remove(context.address());
        if (operation == null) return;
        operation.capture(status, payload);
    }

    private void capture(int status, MemorySegment payload) {
        Outcome<T> outcome;
        try {
            outcome = status == 0 ? new Outcome.Success<>(capture.apply(payload)) : new Outcome.NativeFailure<>(status);
        } catch (Throwable error) { outcome = new Outcome.CallbackFailure<>(error); }
        finish(outcome);
    }

    private void releaseToken() {
        if (tokenUsers.decrementAndGet() == 0) cancellation.release(token);
    }

    private void finish(Outcome<T> outcome) {
        releaseToken();
        // Each admitted operation contributes at most one queued completion. Admission
        // is released after delivery, so the bounded queue cannot reject this enqueue.
        dispatcher.executor.execute(() -> {
            try { completion.complete(outcome); }
            finally { dispatcher.admitted.decrementAndGet(); }
        });
    }
}
