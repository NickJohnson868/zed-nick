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
import java.util.Properties;
import javax.tools.ToolProvider;
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
        Path lockDirectory = root.resolve("target/dev-hot-reload");
        Files.createDirectories(lockDirectory);
        String lockName = HexFormat.of().formatHex(MessageDigest.getInstance("SHA-256").digest(module.toString().getBytes(StandardCharsets.UTF_8)));
        try (FileChannel channel = FileChannel.open(lockDirectory.resolve("native-run-" + lockName + ".lock"), StandardOpenOption.CREATE, StandardOpenOption.WRITE)) {
            var lock = channel.tryLock();
            if (lock == null) throw new IOException("该项目已启动热更新，请先停止原进程");
            try (lock) {
                List<String> options = new ArrayList<>(List.of("-B", "-DskipTests"));
                if (hot) options.add("-Pdev-hot-reload");
                Path modelFile = lockDirectory.resolve("native-model-" + lockName + ".properties");
                Path extensionDirectory = lockDirectory.resolve("native-extension-" + lockName);
                Files.createDirectories(extensionDirectory);
                Path extension = modelExtension(root, maven, javaHome, extensionDirectory);
                options.addAll(List.of("-Dmaven.ext.class.path=" + extension,
                    "-Dzed.runner.module=" + module, "-Dzed.runner.model=" + modelFile));
                Path buildRoot = findBuildRoot(root, module, maven, javaHome, options, modelFile);
                List<String> buildArguments = new ArrayList<>(options);
                buildArguments.addAll(List.of("-f", buildRoot.resolve("pom.xml").toString()));
                if (!module.equals(buildRoot)) {
                    buildArguments.addAll(List.of("-pl", buildRoot.relativize(module).toString().replace('\\', '/'), "-am"));
                }
                System.out.println("Maven 构建入口：" + buildRoot.resolve("pom.xml"));
                if (!compile(root, maven, javaHome, buildArguments, modelFile)) throw new IOException("构建失败，未启动应用");
                if (mode.equals("build")) { System.out.println("运行前构建完成。"); return; }
                Properties model = new Properties();
                try (var stream = Files.newInputStream(modelFile)) { model.load(stream); }
                Path classes = Path.of(model.getProperty("output")).toRealPath();
                Path target = classes.getParent();
                List<Path> classpath = modelPaths(model, "classpath");
                List<Path> modules = modelPaths(model, "modules");
                if (hot && classpath.stream().noneMatch(path -> path.getFileName().toString().startsWith("spring-boot-devtools-"))) {
                    throw new IOException("dev-hot-reload profile 未提供 DevTools");
                }
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
                    if (compile(root, maven, javaHome, buildArguments, modelFile)) {
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

    private static Path findBuildRoot(Path root, Path module, Path maven, Path javaHome,
                                      List<String> options, Path modelFile) throws Exception {
        var candidates = new ArrayList<Path>();
        for (Path parent = module.getParent(); parent != null && parent.startsWith(root); parent = parent.getParent()) {
            if (Files.isRegularFile(parent.resolve("pom.xml"))) candidates.addFirst(parent);
        }
        // Maven's active reactor, not POM inheritance or directory ancestry, defines aggregation.
        for (Path candidate : candidates) {
            Files.deleteIfExists(modelFile);
            var command = new ArrayList<String>();
            command.add(maven.toString());
            command.addAll(options);
            command.addAll(List.of("-f", candidate.resolve("pom.xml").toString(),
                "-Dzed.runner.discover=true", "validate"));
            if (executeBuild(root, javaHome, command, null) != 0 || !Files.isRegularFile(modelFile)) {
                throw new IOException("无法解析 Maven 聚合关系：" + candidate.resolve("pom.xml"));
            }
            var model = new Properties();
            try (var stream = Files.newInputStream(modelFile)) { model.load(stream); }
            String membership = model.getProperty("contains.module");
            if ("true".equals(membership)) return candidate;
            if (!"false".equals(membership)) throw new IOException("Maven 聚合模型缺少目标模块状态");
        }
        return module;
    }

    private static boolean compile(Path root, Path maven, Path javaHome, List<String> options, Path modelFile) throws Exception {
        List<String> command = new ArrayList<>();
        command.add(maven.toString());
        command.addAll(options);
        Files.deleteIfExists(modelFile);
        command.add("compile");
        return executeBuild(root, javaHome, command, null) == 0 && Files.isRegularFile(modelFile);
    }

    private static int executeBuild(Path root, Path javaHome, List<String> command, Path output) throws Exception {
        Path maven = Path.of(command.getFirst());
        boolean windowsMvnd = WINDOWS && (maven.getFileName().toString().equalsIgnoreCase("mvnd.exe")
            || maven.getFileName().toString().equalsIgnoreCase("mvnd.cmd"));
        if (windowsMvnd) {
            // The native MVND client loses Unicode arguments on Windows; use its Java client.
            maven = maven.resolveSibling("mvnd.cmd");
            if (!Files.isRegularFile(maven)) throw new IOException("MVND 缺少 Java 客户端启动器：" + maven);
            command = new ArrayList<>(command);
            command.set(0, maven.toString());
        }
        if (WINDOWS && (maven.toString().endsWith(".cmd") || maven.toString().endsWith(".bat"))) {
            String line = String.join(" ", command.stream().map(value -> "\"" + value + "\"").toList());
            command = List.of(System.getenv().getOrDefault("COMSPEC", "cmd.exe"), "/d", "/s", "/c", "\"" + line + "\"");
        }
        ProcessBuilder builder = new ProcessBuilder(command).directory(root.toFile()).inheritIO();
        if (output != null) builder.redirectErrorStream(true).redirectOutput(output.toFile());
        builder.environment().put("JAVA_HOME", javaHome.toString());
        if (windowsMvnd) builder.environment().put("MVND_CLIENT", "jvm");
        String javaBin = javaHome.resolve("bin").toString();
        String pathKey = builder.environment().keySet().stream().filter(key -> key.equalsIgnoreCase("PATH")).findFirst().orElse("PATH");
        builder.environment().put(pathKey, javaBin + java.io.File.pathSeparator + builder.environment().getOrDefault(pathKey, ""));
        build = builder.start();
        int result = build.waitFor();
        build = null;
        return result;
    }

    private static List<Path> modelPaths(Properties model, String name) throws IOException {
        List<Path> paths = new ArrayList<>();
        int count = Integer.parseInt(model.getProperty(name + ".count"));
        for (int index = 0; index < count; index++) paths.add(Path.of(model.getProperty(name + "." + index)).toRealPath());
        return paths;
    }

    private static Path modelExtension(Path root, Path maven, Path javaHome, Path directory) throws Exception {
        Path versionFile = directory.resolve("native-maven-version.txt");
        if (executeBuild(root, javaHome, List.of(maven.toString(), "-v"), versionFile) != 0) {
            throw new IOException("无法确定 Maven 安装目录");
        }
        String version = Files.readString(versionFile).replaceAll("\\x1B\\[[0-9;]*m", "");
        String home = version.lines().filter(line -> line.startsWith("Maven home:"))
            .map(line -> line.substring("Maven home:".length()).trim()).findFirst()
            .orElseThrow(() -> new IOException("Maven 未返回安装目录"));
        List<String> libraries;
        try (var files = Files.list(Path.of(home).resolve("lib"))) {
            libraries = files.filter(path -> path.toString().endsWith(".jar")).map(Path::toString).sorted().toList();
        }
        Path source = Path.of(System.getProperty("jdk.launcher.sourcefile")).resolveSibling("NativeMavenModel.java");
        String identity = Files.readString(source) + version;
        String digest = HexFormat.of().formatHex(MessageDigest.getInstance("SHA-256").digest(identity.getBytes(StandardCharsets.UTF_8)));
        Path jarFile = directory.resolve("native-maven-model-" + digest + ".jar");
        // MVND can keep extension jars open after a build; never overwrite a loaded version.
        if (Files.isRegularFile(jarFile)) return jarFile;
        Path classes = directory.resolve("native-maven-extension-classes");
        Files.createDirectories(classes);
        var compiler = ToolProvider.getSystemJavaCompiler();
        if (compiler == null) throw new IOException("运行器需要完整 JDK 编译 Maven 扩展");
        int result = compiler.run(null, System.out, System.err, "--release", "17", "-encoding", "UTF-8",
            "-classpath", String.join(java.io.File.pathSeparator, libraries), "-d", classes.toString(), source.toString());
        if (result != 0) throw new IOException("Maven 模型扩展编译失败");
        Path temporaryJar = directory.resolve("native-maven-model-" + UUID.randomUUID() + ".tmp");
        try (var jar = new JarOutputStream(Files.newOutputStream(temporaryJar))) {
            jar.putNextEntry(new java.util.jar.JarEntry("NativeMavenModel.class"));
            Files.copy(classes.resolve("NativeMavenModel.class"), jar);
            jar.closeEntry();
            jar.putNextEntry(new java.util.jar.JarEntry("META-INF/plexus/components.xml"));
            jar.write(("<component-set><components><component><role>org.apache.maven.eventspy.EventSpy</role>"
                + "<role-hint>zed-runner-model</role-hint><implementation>NativeMavenModel</implementation>"
                + "</component></components></component-set>").getBytes(StandardCharsets.UTF_8));
            jar.closeEntry();
        }
        Files.move(temporaryJar, jarFile);
        return jarFile;
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
