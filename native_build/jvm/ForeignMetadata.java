package org.openjdk.jextract.impl;

import org.openjdk.jextract.*;
import java.nio.file.*;
import java.util.*;
import static org.openjdk.jextract.impl.DeclarationImpl.*;

/** Native Image descriptors derived from the same compiler declarations as jextract. */
final class ForeignMetadata {
    static String layout(Type type) {
        if (type instanceof Type.Primitive p) return p.kind() == Type.Primitive.Kind.Bool ? "bool" : p.kind().typeName();
        if (type instanceof Type.Delegated d) return d.kind() == Type.Delegated.Kind.POINTER ? "void*" : layout(d.type());
        if (type instanceof Type.Array a) return "sequence(" + a.elementCount().orElseThrow() + ", " + layout(a.elementType()) + ")";
        if (type instanceof Type.Declared d) {
            var fields = new ArrayList<String>(); long offset = 0;
            boolean union = d.tree().kind() == Declaration.Scoped.Kind.UNION;
            for (Declaration member : d.tree().members()) if (member instanceof Declaration.Variable v) {
                long start = ClangOffsetOf.getOrThrow(v) / 8;
                if (!union && start > offset) fields.add("padding(" + (start - offset) + ")");
                fields.add(layout(v.type()));
                offset = union ? Math.max(offset, ClangSizeOf.getOrThrow(v) / 8) : start + ClangSizeOf.getOrThrow(v) / 8;
            }
            long size = ClangSizeOf.getOrThrow(d.tree()) / 8;
            if (!union && size > offset) fields.add("padding(" + (size - offset) + ")");
            return (union ? "union(" : "struct(") + String.join(", ", fields) + ")";
        }
        throw new IllegalArgumentException("Foreign descriptor cannot represent " + type);
    }
    static String descriptor(Type.Function function) {
        return "{\"returnType\":\"" + layout(function.returnType()) + "\",\"parameterTypes\":[" +
            String.join(",", function.argumentTypes().stream().map(t -> "\"" + layout(t) + "\"").toList()) + "]}";
    }
    static void write(Path path, List<Declaration> declarations, String packageName) throws Exception {
        var down = new LinkedHashSet<String>(); var up = new LinkedHashSet<String>(); var reflection = new ArrayList<String>();
        down.add("{\"returnType\":\"void*\",\"parameterTypes\":[\"void*\",\"int\"]}");
        down.add("{\"returnType\":\"void*\",\"parameterTypes\":[\"void*\",\"void*\"]}");
        down.add("{\"returnType\":\"void*\",\"parameterTypes\":[\"void*\"]}");
        for (Declaration declaration : declarations) {
            if (declaration instanceof Declaration.Function f && !f.type().varargs()) down.add(descriptor(f.type()));
            if (declaration instanceof Declaration.Typedef alias) {
                Type t = alias.type();
                while (t instanceof Type.Delegated d) t = d.type();
                if (t instanceof Type.Function f) {
                    up.add(descriptor(f)); down.add(descriptor(f));
                    reflection.add("{\"type\":\"" + packageName + "." + alias.name() + "$Function\",\"allDeclaredMethods\":true}");
                }
            }
        }
        Files.writeString(path, "{\"foreign\":{\"downcalls\":[" + String.join(",\n", down) + "],\"upcalls\":[" + String.join(",\n", up) + "]},\"reflection\":[" + String.join(",", reflection) + "]}\n");
    }
}
