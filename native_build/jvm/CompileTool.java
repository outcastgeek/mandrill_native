import java.nio.file.*;
import java.util.*;
import javax.tools.ToolProvider;

/** Builds the pinned extraction module using only the supplied JDK. */
class CompileTool {
    public static void main(String[] args) throws Exception {
        if (Runtime.version().feature() != 26) throw new IllegalStateException("JDK 26 required");
        Path upstream = Path.of(args[0]), helpers = Path.of(args[1]), output = Path.of(args[2]);
        Files.createDirectories(output);
        var options = new ArrayList<>(List.of("--release", "26", "-g:lines,source", "-d", output.toString()));
        for (Path root : List.of(upstream.resolve("src/main/java"), helpers)) {
            try (var paths = Files.walk(root)) {
                paths.filter(p -> (!root.equals(helpers) || p.getParent().equals(helpers)) && p.toString().endsWith(".java") && !p.getFileName().toString().startsWith("Compile"))
                    .sorted().forEach(p -> options.add(p.toString()));
            }
        }
        // JDK 26 javac requires an explicit wildcard type witness for this upstream stream.
        for (int i = 0; i < options.size(); i++) {
            Path file = Path.of(options.get(i));
            if (file.endsWith("org/openjdk/jextract/impl/TypeImpl.java") || file.endsWith("org/openjdk/jextract/impl/HeaderFileBuilder.java")) {
                String source = Files.readString(file);
                if (file.getFileName().toString().equals("TypeImpl.java"))
                    source = source.replace("System.getProperty(\"os.name\").startsWith(\"Windows\")", "Boolean.getBoolean(\"jextract.target.windows\")");
                else source = source.replace("\"static final SymbolLookup SYMBOL_LOOKUP = \", \";\"", "\"public static final nativebuild.ffm.NativeLookup SYMBOL_LOOKUP = new nativebuild.ffm.NativeLookup(\", \");\"");
                Path patched = output.resolve("tool-source/org/openjdk/jextract/impl/" + file.getFileName());
                Files.createDirectories(patched.getParent()); Files.writeString(patched, source); options.set(i, patched.toString());
            }
            if (file.endsWith("org/openjdk/jextract/impl/Utils.java")) {
                String source = Files.readString(file);
                String old = "type.argumentTypes().stream().map(Utils::carrierFor).toList()";
                if (!source.contains(old)) throw new IllegalStateException("Pinned jextract source changed");
                Path patched = output.resolve("tool-source/org/openjdk/jextract/impl/Utils.java");
                Files.createDirectories(patched.getParent());
                Files.writeString(patched, source.replace(old, "type.argumentTypes().stream().<Class<?>>map(Utils::carrierFor).toList()"));
                options.set(i, patched.toString());
            }
        }
        if (ToolProvider.getSystemJavaCompiler().run(null, null, null, options.toArray(String[]::new)) != 0)
            throw new IllegalStateException("jextract compilation failed");
        Path resources = upstream.resolve("src/main/resources");
        try (var paths = Files.walk(resources)) {
            for (Path source : paths.filter(Files::isRegularFile).toList()) {
                Path dest = output.resolve(resources.relativize(source));
                Files.createDirectories(dest.getParent());
                Files.copy(source, dest, StandardCopyOption.REPLACE_EXISTING);
            }
        }
    }
}
