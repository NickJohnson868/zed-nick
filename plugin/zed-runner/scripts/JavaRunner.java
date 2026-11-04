import java.io.IOException;
import java.nio.channels.FileChannel;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardOpenOption;
import java.security.MessageDigest;
import java.time.Duration;
import java.util.ArrayList;
import java.util.HexFormat;
import java.util.List;
import java.util.Map;
import java.util.TreeMap;
import java.util.UUID;
import java.util.concurrent.TimeUnit;
import java.util.jar.Attributes;
import java.util.jar.JarOutputStream;
import java.util.jar.Manifest;

/** Uses the project's JDK directly; no PowerShell, Python, or Node runtime is required. */
public class JavaRunner {
    private static volatile Process application;
    private static volatile Process build;
    private static final boolean WINDOWS = System.getProperty("os.name").startsWith("Windows");

    public static void main(String[] arguments) {
        Runtime.getRuntime().addShutdownHook(new Thread(() -> {
            stop(build);
            stop(application);
        }));
        try {
            initializeWindowsConsole();
            run();
        } catch (Exception exception) {
            System.err.println("Java 热更新失败：" + exception.getMessage());
            System.exit(1);
        }
    }

    static void initializeWindowsConsole() throws Exception {
        if (!WINDOWS) return;
        String root = System.getenv("SystemRoot");
        Path cmd = Path.of(root == null ? "C:\\Windows" : root, "System32", "cmd.exe");
        // UTF-8 JVM output also needs a UTF-8 shared console; otherwise ConPTY
        // transcodes those bytes using the inherited legacy Windows code page.
        // A detached process has no console, so a nonzero chcp exit is harmless.
        new ProcessBuilder(cmd.toString(), "/d", "/c", "chcp 65001 >nul")
            .inheritIO().redirectError(ProcessBuilder.Redirect.DISCARD).start().waitFor();
    }

    private static Path configuredPath(String name) throws IOException {
        String value = System.getenv(name);
        if (value == null || value.isBlank()) throw new IOException("缺少运行配置：" + name);
        Path path = Path.of(value);
        if (!path.isAbsolute()) throw new IOException("运行路径必须是绝对路径：" + name);
        return path.toRealPath();
    }

    private static List<String> arguments(String name) throws IOException {
        byte[] bytes = System.getenv().getOrDefault(name, "").getBytes(StandardCharsets.UTF_8);
        List<String> result = new ArrayList<>();
        for (int offset = 0; offset < bytes.length;) {
            int colon = offset;
            while (colon < bytes.length && bytes[colon] != ':') colon++;
            if (colon == bytes.length) throw new IOException("运行参数编码无效");
            int length = Integer.parseInt(new String(bytes, offset, colon - offset, StandardCharsets.US_ASCII));
            offset = colon + 1;
            if (length < 0 || length > bytes.length - offset) throw new IOException("运行参数长度无效");
            result.add(new String(bytes, offset, length, StandardCharsets.UTF_8));
            offset += length;
        }
        return result;
    }

    private static void run() throws Exception {
        Path root = configuredPath("ZED_JAVA_NATIVE_ROOT");
        Path module = configuredPath("ZED_JAVA_NATIVE_MODULE");
        Path maven = configuredPath("ZED_JAVA_NATIVE_MAVEN");
        Path javaHome = configuredPath("JAVA_HOME");
        if (!module.startsWith(root)) throw new IOException("启动模块必须位于项目内");
        String main = System.getenv("ZED_JAVA_NATIVE_MAIN");
        if (main == null || main.isBlank()) throw new IOException("缺少 Java Main 类");
        String mode = System.getenv().getOrDefault("ZED_JAVA_NATIVE_MODE", "hot");
        boolean hot = mode.equals("hot");
        String relativeTarget = hot ? "target/dev-hot-reload" : "target";
        Path target = module.resolve(relativeTarget);
        Path classes = target.resolve("classes");
        Path classpathFile = target.resolve("native-classpath.txt");
        Path lockDirectory = root.resolve("target/dev-hot-reload");
        Files.createDirectories(lockDirectory);
        String lockName = HexFormat.of().formatHex(MessageDigest.getInstance("SHA-256").digest(module.toString().getBytes(StandardCharsets.UTF_8)));
        try (FileChannel channel = FileChannel.open(lockDirectory.resolve("native-run-" + lockName + ".lock"), StandardOpenOption.CREATE, StandardOpenOption.WRITE)) {
            var lock = channel.tryLock();
            if (lock == null) throw new IOException("该项目已启动热更新，请先停止原进程");
            try (lock) {
                List<String> buildArguments = new ArrayList<>();
                if (Files.isRegularFile(root.resolve("pom.xml")) && !module.equals(root)) {
                    buildArguments.addAll(List.of("-f", root.resolve("pom.xml").toString(), "-pl", root.relativize(module).toString().replace('\\', '/'), "-am"));
                } else {
                    buildArguments.addAll(List.of("-f", module.resolve("pom.xml").toString()));
                }
                buildArguments.addAll(List.of("-B", "-DskipTests"));
                if (hot) buildArguments.add("-Pdev-hot-reload");
                if (!compile(root, maven, javaHome, buildArguments, relativeTarget, true)) throw new IOException("构建失败，未启动应用");
                if (mode.equals("build")) { System.out.println("运行前构建完成。"); return; }
                List<Path> classpath = new ArrayList<>();
                classpath.add(classes);
                for (String entry : Files.readString(classpathFile).trim().split(java.util.regex.Pattern.quote(java.io.File.pathSeparator))) {
                    if (!entry.isBlank()) classpath.add(Path.of(entry).toRealPath());
                }
                if (hot && classpath.stream().noneMatch(path -> path.getFileName().toString().startsWith("spring-boot-devtools-"))) {
                    throw new IOException("dev-hot-reload profile 未提供 DevTools");
                }
                List<Path> modules = classpath.stream().filter(Files::isDirectory)
                    .filter(path -> path.endsWith(Path.of("target/dev-hot-reload/classes")))
                    .map(path -> path.getParent().getParent().getParent()).filter(path -> path.startsWith(root)).distinct().toList();
                Path trigger = classes.resolve(".dev-reload-trigger");
                Files.writeString(trigger, UUID.randomUUID().toString());
                Path classpathJar = target.resolve("native-classpath.jar");
                Manifest manifest = new Manifest();
                manifest.getMainAttributes().put(Attributes.Name.MANIFEST_VERSION, "1.0");
                manifest.getMainAttributes().put(Attributes.Name.CLASS_PATH,
                    String.join(" ", classpath.stream().map(path -> path.toUri().toASCIIString()).toList()));
                try (var jar = new JarOutputStream(Files.newOutputStream(classpathJar), manifest)) { }
                List<String> command = new ArrayList<>();
                command.add(javaHome.resolve(WINDOWS ? "bin/java.exe" : "bin/java").toString());
                command.addAll(arguments("ZED_JAVA_NATIVE_VM_ARGS"));
                command.addAll(List.of("--enable-native-access=ALL-UNNAMED", "-Dfile.encoding=UTF-8", "-Dstdout.encoding=UTF-8", "-Dstderr.encoding=UTF-8"));
                if (hot) command.addAll(List.of("-Dspring.devtools.add-properties=false", "-Dspring.devtools.livereload.enabled=false",
                    "-Dspring.devtools.restart.trigger-file=.dev-reload-trigger",
                    "-Dspring.devtools.restart.poll-interval=2s", "-Dspring.devtools.restart.quiet-period=1s"));
                command.addAll(List.of("-cp", classpathJar.toString(), main));
                command.addAll(arguments("ZED_JAVA_NATIVE_ARGS"));
                String ambiguous = System.getProperty("jdk.lang.Process.allowAmbiguousCommands");
                try {
                    // Java's legacy Windows quoting loses embedded quotes in application arguments.
                    if (WINDOWS) System.setProperty("jdk.lang.Process.allowAmbiguousCommands", "false");
                    application = new ProcessBuilder(command).directory(root.toFile()).inheritIO().start();
                } finally {
                    if (WINDOWS) {
                        if (ambiguous == null) System.clearProperty("jdk.lang.Process.allowAmbiguousCommands");
                        else System.setProperty("jdk.lang.Process.allowAmbiguousCommands", ambiguous);
                    }
                }
                if (!hot) {
                    int code = application.waitFor();
                    if (code != 0) throw new IOException("应用退出，退出码：" + code);
                    return;
                }
                Map<String, String> stamp = snapshot(root, modules);
                System.out.println("开发热更新已启动：使用 JDK/Maven，无需 PowerShell。Ctrl+C 停止。");
                while (application.isAlive()) {
                    Thread.sleep(Duration.ofSeconds(2));
                    Map<String, String> current = snapshot(root, modules);
                    if (current.equals(stamp)) continue;
                    Thread.sleep(Duration.ofSeconds(1));
                    stamp = snapshot(root, modules);
                    if (compile(root, maven, javaHome, buildArguments, relativeTarget, false)) {
                        Files.writeString(trigger, UUID.randomUUID().toString());
                        System.out.println("编译成功，已通知 DevTools 重启。");
                    } else {
                        System.out.println("编译失败，未触发重启；修正并保存后重试。");
                    }
                }
                int code = application.waitFor();
                if (code != 0) throw new IOException("应用退出，退出码：" + code);
            }
        }
    }

    private static boolean compile(Path root, Path maven, Path javaHome, List<String> options, String target, boolean classpath) throws Exception {
        List<String> command = new ArrayList<>();
        command.add(maven.toString());
        command.addAll(options);
        command.add("compile");
        if (classpath) command.addAll(List.of("dependency:build-classpath", "-DincludeScope=runtime",
            "-Dmdep.outputFile=" + target + "/native-classpath.txt", "-Dmdep.regenerateFile=true"));
        if (WINDOWS && (maven.toString().endsWith(".cmd") || maven.toString().endsWith(".bat"))) {
            String line = String.join(" ", command.stream().map(value -> "\"" + value + "\"").toList());
            command = List.of(System.getenv().getOrDefault("COMSPEC", "cmd.exe"), "/d", "/s", "/c", "\"" + line + "\"");
        }
        ProcessBuilder builder = new ProcessBuilder(command).directory(root.toFile()).inheritIO();
        builder.environment().put("JAVA_HOME", javaHome.toString());
        String javaBin = javaHome.resolve("bin").toString();
        String pathKey = builder.environment().keySet().stream().filter(key -> key.equalsIgnoreCase("PATH")).findFirst().orElse("PATH");
        builder.environment().put(pathKey, javaBin + java.io.File.pathSeparator + builder.environment().getOrDefault(pathKey, ""));
        build = builder.start();
        int result = build.waitFor();
        build = null;
        return result == 0;
    }

    private static Map<String, String> snapshot(Path root, List<Path> modules) throws Exception {
        Map<String, String> result = new TreeMap<>();
        for (Path module : modules) {
            for (Path parent = module; parent != null && parent.startsWith(root); parent = parent.getParent()) {
                fingerprint(parent.resolve("pom.xml"), result);
            }
            Path source = module.resolve("src/main");
            if (Files.isDirectory(source)) {
                try (var files = Files.walk(source)) {
                    for (Path file : files.filter(Files::isRegularFile).sorted().toList()) fingerprint(file, result);
                }
            }
        }
        return result;
    }

    private static void fingerprint(Path path, Map<String, String> result) throws Exception {
        try {
            if (Files.isRegularFile(path)) result.put(path.toString(), HexFormat.of().formatHex(MessageDigest.getInstance("SHA-256").digest(Files.readAllBytes(path))));
        } catch (java.nio.file.NoSuchFileException exception) {
            // Editors may replace a source file atomically while it is being sampled.
        }
    }

    private static void stop(Process process) {
        if (process == null || !process.isAlive()) return;
        process.destroy();
        try {
            if (!process.waitFor(10, TimeUnit.SECONDS)) process.destroyForcibly();
        } catch (InterruptedException exception) {
            Thread.currentThread().interrupt();
            process.destroyForcibly();
        }
    }
}
