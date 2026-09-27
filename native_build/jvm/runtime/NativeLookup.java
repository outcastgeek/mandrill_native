package nativebuild.ffm;

import java.lang.foreign.*;
import java.lang.invoke.MethodHandle;
import java.util.Optional;
import java.util.Objects;
import java.util.concurrent.atomic.AtomicReference;

/** A binding's caller-selected lookup becomes immutable at its first symbol resolution. */
public final class NativeLookup implements SymbolLookup {
    private record State(SymbolLookup lookup, boolean resolved) {}
    private final AtomicReference<State> state;
    public NativeLookup(SymbolLookup fallback) { state = new AtomicReference<>(new State(fallback, false)); }
    public void configure(SymbolLookup lookup) {
        Objects.requireNonNull(lookup);
        State current = state.get();
        for (;;) {
            if (current.resolved()) throw new IllegalStateException("bindings already resolved a native symbol");
            if (state.compareAndSet(current, new State(lookup, false))) return;
            current = state.get();
        }
    }
    public Optional<MemorySegment> find(String symbol) {
        State current = state.get();
        while (!current.resolved()) {
            State resolved = new State(current.lookup(), true);
            if (state.compareAndSet(current, resolved)) { current = resolved; break; }
            current = state.get();
        }
        return current.lookup().find(symbol);
    }
    /** Resolve exported symbols from the current executable, including statically linked archives. */
    public static SymbolLookup process() {
        Linker linker = Linker.nativeLinker();
        boolean windows = System.getProperty("os.name").startsWith("Windows");
        SymbolLookup system = windows ? SymbolLookup.libraryLookup("kernel32.dll", Arena.global()) : linker.defaultLookup();
        var pointer = ValueLayout.ADDRESS;
        MethodHandle open = linker.downcallHandle(system.findOrThrow(windows ? "GetModuleHandleW" : "dlopen"),
            windows ? FunctionDescriptor.of(pointer, pointer) : FunctionDescriptor.of(pointer, pointer, ValueLayout.JAVA_INT));
        MethodHandle find = linker.downcallHandle(system.findOrThrow(windows ? "GetProcAddress" : "dlsym"),
            FunctionDescriptor.of(pointer, pointer, pointer));
        final MemorySegment process;
        try { process = windows ? (MemorySegment)open.invokeExact(MemorySegment.NULL) : (MemorySegment)open.invokeExact(MemorySegment.NULL, 1); }
        catch (Throwable error) { throw new IllegalStateException("opening process symbol lookup", error); }
        if (process.equals(MemorySegment.NULL)) throw new IllegalStateException("process symbol lookup returned NULL");
        // One process-lifetime loader reference, matching the lifetime of generated handles.
        return symbol -> {
            if (symbol.indexOf('\0') >= 0) throw new IllegalArgumentException("NUL in symbol");
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment address = (MemorySegment)find.invokeExact(process, arena.allocateFrom(symbol));
                return address.equals(MemorySegment.NULL) ? Optional.empty() : Optional.of(address);
            } catch (Throwable error) { throw new IllegalStateException("resolving " + symbol, error); }
        };
    }
}
