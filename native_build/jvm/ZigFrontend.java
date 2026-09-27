package org.openjdk.jextract.impl;

import org.openjdk.jextract.*;
import org.openjdk.jextract.json.parser.*;
import java.nio.file.*;
import java.util.*;
import static org.openjdk.jextract.impl.DeclarationImpl.*;

/** Converts Zig's compiler AST into upstream jextract declarations. */
public final class ZigFrontend {
    private final List<String> compiler;
    private final Path output;
    private final List<String> constants;
    private final Map<String, JSONObject> declarations = new LinkedHashMap<>();
    private final Map<String, String> recordNames = new HashMap<>();
    private final Map<String, Declaration.Scoped> records = new HashMap<>();
    private final Map<String, Type> aliases = new HashMap<>();
    private final Map<String, JSONObject> probes = new HashMap<>();
    private final Map<String, Long> values = new HashMap<>();
    private final Map<String, Integer> recordIndices = new HashMap<>();
    private final Map<String, Integer> functionIndices = new HashMap<>();

    private ZigFrontend(List<String> compiler, Path output, List<String> constants) { this.compiler = compiler; this.output = output; this.constants = constants; }
    static String text(JSONObject n, String key) { return n.contains(key) ? n.get(key).asString() : ""; }
    static List<JSONObject> children(JSONObject n) {
        return n.contains("inner") ? n.get("inner").asArray().stream().map(JSONValue::asObject).toList() : List.of();
    }
    static boolean flag(JSONObject n, String key) { return n.contains(key) && n.get(key).asBoolean(); }
    static JSONObject typeChild(JSONObject n) {
        return children(n).stream().filter(c -> text(c,"kind").endsWith("Type")).reduce((a,b)->b)
            .orElseThrow(() -> new IllegalArgumentException("No compiler type in " + n));
    }
    private JSONObject parse(Path input) throws Exception {
        var command = new ArrayList<>(compiler); command.add("-ast-dump=json"); command.add(input.toString());
        Path ast = output.resolve("ast.json"), diagnostics = output.resolve("diagnostics.txt");
        var pb = new ProcessBuilder(command).redirectOutput(ast.toFile()).redirectError(diagnostics.toFile());
        for (String key : List.of("CPATH","C_INCLUDE_PATH","CPLUS_INCLUDE_PATH","OBJC_INCLUDE_PATH","SDKROOT","LIBRARY_PATH")) pb.environment().remove(key);
        int status = pb.start().waitFor();
        if (status != 0) throw new IllegalStateException("Zig frontend exit " + status + ": " + Files.readString(diagnostics));
        return new JSONParser().parse(Files.readString(ast)).asObject();
    }
    private void index(JSONObject ast) {
        declarations.clear(); recordNames.clear();
        for (JSONObject n : children(ast)) {
            String kind = text(n,"kind"), name = text(n,"name");
            if (kind.equals("RecordDecl") || kind.equals("EnumDecl") || kind.equals("TypedefDecl")) declarations.put(text(n,"id"),n);
            if (kind.equals("RecordDecl") && !name.isEmpty()) recordNames.put(text(n,"id"),text(n,"tagUsed") + " " + name);
        }
        for (JSONObject n : children(ast)) if (text(n,"kind").equals("TypedefDecl") && !flag(n,"isImplicit")) {
            JSONObject t = typeChild(n);
            while (List.of("ElaboratedType","ParenType","AttributedType").contains(text(t,"kind"))) t = typeChild(t);
            if (text(t,"kind").equals("RecordType")) recordNames.putIfAbsent(text(t.get("decl").asObject(),"id"),text(n,"name"));
        }
    }
    private static Long constant(JSONObject n) {
        if (text(n,"kind").equals("ConstantExpr") && n.contains("value")) return text(n,"value").startsWith("-") ? Long.parseLong(text(n,"value")) : Long.parseUnsignedLong(text(n,"value"));
        for (JSONObject c : children(n)) { Long value=constant(c); if(value!=null)return value; }
        return null;
    }
    private static void measure(StringBuilder code, String key, String expression) {
        code.append("enum { ").append(key).append(" = ").append(expression).append(" };\n");
    }
    private JSONObject prepare(Path header) throws Exception {
        JSONObject ast=parse(header); index(ast);
        StringBuilder code=new StringBuilder("#include \"").append(header.toString().replace("\\","\\\\").replace("\"","\\\"")).append("\"\n");
        for (int c = 0; c < constants.size(); c++) {
            String name = constants.get(c);
            if (!name.matches("[A-Za-z_][A-Za-z0-9_]*")) throw new IllegalArgumentException("Invalid constant name " + name);
            measure(code, "JX_C_" + c, name);
            code.append("typedef __typeof__(").append(name).append(") JX_CT_").append(c).append(";\n");
        }
        int index=0;
        for(JSONObject n:children(ast)) {
            if(text(n,"kind").equals("RecordDecl") && flag(n,"completeDefinition") && !text(n,"name").startsWith("__")) {
                String cname=recordNames.get(text(n,"id"));
                if(cname==null) throw new IllegalArgumentException("Anonymous record requires a reachable typedef: " + n);
                int r=index++; recordIndices.put(cname,r);
                measure(code,"JX_S_"+r,"sizeof("+cname+")"); measure(code,"JX_A_"+r,"_Alignof("+cname+")");
                int f=0;
                for(JSONObject field:children(n)) if(text(field,"kind").equals("FieldDecl")) {
                    String name=text(field,"name");
                    if(name.isEmpty() || flag(field,"isBitfield")) throw new IllegalArgumentException("Unimplemented anonymous/bitfield layout at " + field);
                    String key="JX_F_"+r+"_"+f++, expr="(("+cname+"*)0)->"+name;
                    code.append("typedef __typeof__(").append(expr).append(") ").append(key).append(";\n");
                    measure(code,key+"_S","sizeof("+expr+")");measure(code,key+"_A","_Alignof(__typeof__("+expr+"))");
                    measure(code,key+"_O","__builtin_offsetof("+cname+","+name+")");
                }
            }
            if(text(n,"kind").equals("FunctionDecl") && !text(n,"storageClass").equals("static")) {
                int f=functionIndices.size();functionIndices.put(text(n,"name"),f);
                code.append("typedef __typeof__(").append(text(n,"name")).append(") JX_FN_").append(f).append(";\n");
            }
        }
        Path input=output.resolve("declarations.h");Files.writeString(input,code);ast=parse(input);index(ast);
        for(JSONObject n:children(ast)) {
            if(text(n,"kind").equals("TypedefDecl") && text(n,"name").startsWith("JX_")) probes.put(text(n,"name"),typeChild(n));
            if(text(n,"kind").equals("EnumDecl")) for(JSONObject c:children(n)) if(text(c,"name").startsWith("JX_")) values.put(text(c,"name"),Objects.requireNonNull(constant(c)));
        }
        return ast;
    }
    private Declaration.Scoped record(String id) {
        if(records.containsKey(id))return records.get(id);
        JSONObject node=Objects.requireNonNull(declarations.get(id),"Missing record "+id);
        String name=text(node,"name"), cname=recordNames.get(id);
        if(name.isEmpty())name=cname;
        var members=new ArrayList<Declaration>();
        var result=new ScopedImpl(text(node,"tagUsed").equals("union")?Declaration.Scoped.Kind.UNION:Declaration.Scoped.Kind.STRUCT,members,name,Position.NO_POSITION);
        records.put(id,result);
        if(!flag(node,"completeDefinition"))return result;
        int r=Objects.requireNonNull(recordIndices.get(cname),"Missing layout for "+cname), f=0;
        ClangSizeOf.with(result,values.get("JX_S_"+r)*8);ClangAlignOf.with(result,values.get("JX_A_"+r)*8);
        for(JSONObject field:children(node)) if(text(field,"kind").equals("FieldDecl")) {
            String key="JX_F_"+r+"_"+f++;
            var value=Declaration.field(Position.NO_POSITION,text(field,"name"),type(probes.get(key)));
            ClangSizeOf.with(value,values.get(key+"_S")*8);ClangAlignOf.with(value,values.get(key+"_A")*8);ClangOffsetOf.with(value,values.get(key+"_O")*8);members.add(value);
        }
        return result;
    }
    private Type type(JSONObject n) {
        String kind=text(n,"kind");
        return switch(kind) {
            case "TypeOfExprType","ElaboratedType","ParenType","AttributedType","DecayedType","AdjustedType","QualType" -> type(typeChild(n));
            case "BuiltinType" -> builtin(text(n.get("type").asObject(),"qualType"));
            case "PointerType" -> Type.pointer(() -> type(typeChild(n)));
            case "RecordType" -> Type.declared(record(text(n.get("decl").asObject(),"id")));
            case "EnumType" -> Type.primitive(Type.Primitive.Kind.Int);
            case "TypedefType" -> {
                String id=text(n.get("decl").asObject(),"id");
                if(!aliases.containsKey(id))aliases.put(id,type(typeChild(declarations.get(id))));
                yield aliases.get(id);
            }
            case "FunctionProtoType","FunctionNoProtoType" -> {
                var types=children(n).stream().filter(c->text(c,"kind").endsWith("Type")).map(this::type).toList();
                yield Type.function(flag(n,"variadic"),types.getFirst(),types.subList(1,types.size()).toArray(Type[]::new));
            }
            case "ConstantArrayType" -> Type.array(n.get("size").asLong(),type(typeChild(n)));
            case "IncompleteArrayType" -> Type.array(type(typeChild(n)));
            default -> throw new IllegalArgumentException("Unsupported Zig compiler type " + kind + ": " + n);
        };
    }
    private static Type builtin(String name) {
        boolean unsigned=name.startsWith("unsigned ");String base=name.replace("unsigned ","").replace("signed ","");
        var kind=switch(base) {
            case "void" -> Type.Primitive.Kind.Void; case "_Bool","bool" -> Type.Primitive.Kind.Bool;
            case "char" -> Type.Primitive.Kind.Char; case "short" -> Type.Primitive.Kind.Short; case "int" -> Type.Primitive.Kind.Int;
            case "long" -> Type.Primitive.Kind.Long; case "long long" -> Type.Primitive.Kind.LongLong;
            case "float" -> Type.Primitive.Kind.Float;case "double" -> Type.Primitive.Kind.Double;
            case "long double" -> Type.Primitive.Kind.LongDouble;case "__int128" -> Type.Primitive.Kind.Int128;
            default -> throw new IllegalArgumentException("Unsupported builtin "+name);
        };
        Type t=Type.primitive(kind);return unsigned?Type.qualified(Type.Delegated.Kind.UNSIGNED,t):t;
    }
    private void generate(JSONObject ast,String packageName,String className) throws Exception {
        var result=new ArrayList<Declaration>();
        for(JSONObject n:children(ast)) {
            String kind=text(n,"kind"), name=text(n,"name");
            if(name.startsWith("JX_") || name.startsWith("__") || flag(n,"isImplicit"))continue;
            switch(kind) {
                case "RecordDecl" -> { if(flag(n,"completeDefinition"))result.add(record(text(n,"id"))); }
                case "TypedefDecl" -> result.add(Declaration.typedef(Position.NO_POSITION,name,type(typeChild(n))));
                case "FunctionDecl" -> {
                    if(text(n,"storageClass").equals("static"))continue;
                    Type.Function ft=(Type.Function)type(probes.get("JX_FN_"+functionIndices.get(name)));
                    var params=new ArrayList<Declaration.Variable>();int i=0;
                    for(JSONObject c:children(n))if(text(c,"kind").equals("ParmVarDecl"))params.add(Declaration.parameter(Position.NO_POSITION,text(c,"name"),ft.argumentTypes().get(i++)));
                    result.add(Declaration.function(Position.NO_POSITION,name,ft,params.toArray(Declaration.Variable[]::new)));
                }
                case "EnumDecl" -> {
                    long value=0;
                    for(JSONObject c:children(n))if(text(c,"kind").equals("EnumConstantDecl")) {
                        Long explicit=constant(c);if(explicit!=null)value=explicit;
                        result.add(Declaration.constant(Position.NO_POSITION,text(c,"name"),value++,Type.primitive(Type.Primitive.Kind.Int)));
                    }
                }
                default -> { }
            }
        }
        for (int c = 0; c < constants.size(); c++) result.add(Declaration.constant(Position.NO_POSITION,
            constants.get(c), values.get("JX_C_" + c), type(probes.get("JX_CT_" + c))));
        Files.writeString(output.resolve("exports.txt"), String.join("\n", result.stream()
            .filter(d -> d instanceof Declaration.Function).map(Declaration::name).toList()) + "\n");
        ForeignMetadata.write(output.resolve("reachability-metadata.json"), result, packageName);
        for (Declaration d : result) annotate(d);
        JextractTool.write(output.resolve("java"),JextractTool.generate(Declaration.toplevel(Position.NO_POSITION,result.toArray(Declaration[]::new)),className,packageName,List.of(),false));
    }
    private static void annotate(Declaration d) {
        DeclarationString.with(d, d.name());
        if(d instanceof Declaration.Scoped scoped) scoped.members().forEach(ZigFrontend::annotate);
        if(d instanceof Declaration.Function function) function.parameters().forEach(ZigFrontend::annotate);
    }
    public static void main(String[] args) throws Exception {
        if(Runtime.version().feature()!=26)throw new IllegalStateException("JDK 26 required");
        // Output, package, header class, header, followed by the exact Zig frontend command.
        Path output=Path.of(args[0]).toAbsolutePath();Files.createDirectories(output);
        var tool=new ZigFrontend(List.of(args).subList(5,args.length),output,args[4].isEmpty() ? List.of() : List.of(args[4].split(",")));
        tool.generate(tool.prepare(Path.of(args[3]).toAbsolutePath()),args[1],args[2]);
    }
}
