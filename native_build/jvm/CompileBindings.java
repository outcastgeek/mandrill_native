import java.nio.file.*;
import java.util.*;
import javax.tools.ToolProvider;

/** Compiles generated FFM sources with the supplied Java 26 compiler. */
class CompileBindings {
    public static void main(String[] args) throws Exception {
        if(Runtime.version().feature()!=26)throw new IllegalStateException("JDK 26 required");
        Path output=Path.of(args[0]);Files.createDirectories(output);
        var options=new ArrayList<>(List.of("--release","26","-g:lines,source","-d",output.toString()));
        for(int i=1;i<args.length;i++)try(var files=Files.walk(Path.of(args[i]))) {
            files.filter(p->p.toString().endsWith(".java")).sorted().forEach(p->options.add(p.toString()));
        }
        if(ToolProvider.getSystemJavaCompiler().run(null,null,null,options.toArray(String[]::new))!=0)
            throw new IllegalStateException("FFM binding compilation failed");
    }
}
