import java.nio.file.*;
import java.util.*;

/** Native Image links from a temporary directory; preserve absolute archive paths. */
class CompileImage {
    public static void main(String[] args) throws Exception {
        var command = new ArrayList<String>();
        for (String argument : args) {
            if (argument.startsWith("--class-path-file=")) {
                command.add("-cp"); command.add(Files.readString(Path.of(argument.substring("--class-path-file=".length()))).strip()); continue;
            }
            String prefix = "-H:NativeLinkerOption=";
            if (argument.startsWith(prefix)) {
                String value = argument.substring(prefix.length());
                if (value.startsWith("-Wl,-force_load,")) value = "-Wl,-force_load," + Path.of(value.substring("-Wl,-force_load,".length())).toAbsolutePath();
                else if (!value.startsWith("-")) value = Path.of(value).toAbsolutePath().toString();
                argument = prefix + value;
            }
            command.add(argument);
        }
        Path log = Path.of(command.get(command.indexOf("-o") + 1)).toAbsolutePath().getParent().resolve("native-image.log");
        Files.createDirectories(log.getParent());
        int status = new ProcessBuilder(command).redirectErrorStream(true).redirectOutput(log.toFile()).start().waitFor();
        if (status != 0) { System.err.print(Files.readString(log)); System.exit(status); }
    }
}
