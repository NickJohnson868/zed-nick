"""Exercise native runners with real JDK/Maven; PowerShell is never launched."""
import importlib.util
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import tempfile
import time

PLUGIN = Path(__file__).resolve().parents[1]


def encode(arguments):
    return "".join(f"{len(argument.encode('utf-8'))}:{argument}" for argument in arguments)


def check_python(root):
    project = root / "python project 中文"
    project.mkdir()
    (project / "requirements.txt").write_text("")
    script = project / "main.py"
    script.write_text('print("NATIVE_PYTHON_OK")\n')
    result = subprocess.run([sys.executable, str(PLUGIN / "scripts/PythonRunner.py"), str(project), str(script)], check=True, capture_output=True, text=True)
    assert "NATIVE_PYTHON_OK" in result.stdout


def check_install(root):
    installer = PLUGIN.parent / "install.py"
    spec = importlib.util.spec_from_file_location("native_install", installer)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    source = root / "package"
    source.mkdir()
    (source / "extension.toml").write_text('id = "native-test"')
    destination = root / "extensions/installed/native-test"
    module.copy_runtime(source, destination)
    assert (destination / "extension.toml").is_file()
    for windows, expected in [(True, "cmd.exe"), (False, "/bin/sh")]:
        variant = root / ("java-windows" if windows else "java-unix")
        module.java_tasks(variant, windows=windows)
        tasks = json.loads((variant / "languages/java/tasks.json").read_text(encoding="utf-8"))
        assert len(tasks) == 5
        assert all(task["shell"]["program"] == expected for task in tasks)
        assert all("$CMD clean " not in task["command"] for task in tasks)
    if os.name != "nt":
        linked = root / "linked"
        linked.symlink_to(destination, target_is_directory=True)
        try:
            module.copy_runtime(source, linked)
        except ValueError:
            pass
        else:
            raise AssertionError("Installer overwrote a development link")


def check_java(root):
    java_home = Path(os.environ["JAVA_HOME"])
    java = java_home / ("bin/java.exe" if os.name == "nt" else "bin/java")
    maven = shutil.which("mvn") or shutil.which("mvnd")
    if not maven:
        raise RuntimeError("Native smoke needs Maven on the build machine")
    project = root / "java project 中文 ' space"
    (project / "src/main/java").mkdir(parents=True)
    (project / "pom.xml").write_text('''<project xmlns="http://maven.apache.org/POM/4.0.0">
<modelVersion>4.0.0</modelVersion><parent><groupId>org.springframework.boot</groupId><artifactId>spring-boot-starter-parent</artifactId><version>4.1.1</version></parent>
<groupId>native.test</groupId><artifactId>native-runner-test</artifactId><version>1.0</version><properties><java.version>17</java.version></properties>
<dependencies><dependency><groupId>org.springframework.boot</groupId><artifactId>spring-boot-starter</artifactId></dependency><dependency><groupId>org.springframework.boot</groupId><artifactId>spring-boot-devtools</artifactId></dependency></dependencies>
<profiles><profile><id>dev-hot-reload</id><build><directory>${project.basedir}/target/dev-hot-reload</directory></build></profile></profiles></project>''')
    main = project / "src/main/java/Main.java"
    main.write_text('public class Main { public static void main(String[] args) { System.out.println("NATIVE_ARGUMENT="+args[0]); } }')
    env = dict(os.environ, ZED_JAVA_NATIVE_ROOT=str(project), ZED_JAVA_NATIVE_MODULE=str(project), ZED_JAVA_NATIVE_MAVEN=maven,
        ZED_JAVA_NATIVE_MAIN="Main", ZED_JAVA_NATIVE_MODE="run", ZED_JAVA_NATIVE_VM_ARGS="", ZED_JAVA_NATIVE_ARGS=encode(['中文 " quoted argument']))
    command = [str(java), "-Dfile.encoding=UTF-8", "-Dstdout.encoding=UTF-8", "-Dstderr.encoding=UTF-8", str(PLUGIN / "scripts/JavaRunner.java")]
    result = subprocess.run(command, env=env, capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=240)
    assert result.returncode == 0, (result.stdout[:1500] + result.stderr[:1500])
    assert 'NATIVE_ARGUMENT=中文 " quoted argument' in result.stdout, repr([line for line in result.stdout.splitlines() if 'NATIVE_ARGUMENT=' in line])
    source = '''import com.sun.net.httpserver.HttpServer;
import java.net.InetSocketAddress;
import org.springframework.boot.SpringApplication;
import org.springframework.boot.WebApplicationType;
import org.springframework.context.annotation.Configuration;
import org.springframework.context.annotation.Import;
import org.springframework.context.annotation.Bean;
import org.springframework.boot.devtools.autoconfigure.LocalDevToolsAutoConfiguration;
@Configuration(proxyBeanMethods=false) @Import(LocalDevToolsAutoConfiguration.class)
public class Main {
  public static void main(String[] args) { SpringApplication app=new SpringApplication(Main.class);app.setWebApplicationType(WebApplicationType.NONE);app.run(args);System.out.println("NATIVE_STARTED"); }
  @Bean(destroyMethod="close") public Server server() throws Exception {return new Server();}
  public static class Server implements AutoCloseable {
    final HttpServer server; Server() throws Exception {server=HttpServer.create(new InetSocketAddress("127.0.0.1",0),0);server.start();}
    public void close() {server.stop(0);}
  }
}'''
    main.write_text(source)
    env.update(ZED_JAVA_NATIVE_MODE="hot", ZED_JAVA_NATIVE_ARGS="")
    log = project / "runtime.log"
    with log.open("w", encoding="utf-8") as output:
        process = subprocess.Popen(command, env=env, stdout=output, stderr=subprocess.STDOUT, start_new_session=os.name != "nt")
        def text():
            return log.read_text(encoding="utf-8", errors="replace")
        def starts():
            return text().count("NATIVE_STARTED")
        def wait(condition):
            deadline = time.monotonic() + 120
            while time.monotonic() < deadline:
                if condition(): return
                if process.poll() is not None: raise AssertionError(text()[-3000:])
                time.sleep(1)
            raise AssertionError("Native hot reload timed out: " + text()[-3000:])
        try:
            wait(lambda: starts() >= 1)
            main.write_text(source + "\n// save trigger\n")
            wait(lambda: starts() >= 2)
            count = starts()
            main.write_text(source + "\nINVALID JAVA SOURCE\n")
            wait(lambda: "编译失败，未触发重启" in text())
            time.sleep(3)
            assert starts() == count
            main.write_text(source)
            wait(lambda: starts() > count)
        finally:
            if process.poll() is None:
                if os.name == "nt":
                    subprocess.run(["taskkill", "/PID", str(process.pid), "/T", "/F"], check=False, capture_output=True)
                else:
                    process.terminate()
                    try: process.wait(timeout=15)
                    except subprocess.TimeoutExpired: os.killpg(process.pid, signal.SIGKILL)
            process.wait(timeout=15)


if __name__ == "__main__":
    with tempfile.TemporaryDirectory(prefix="zed-native-") as directory:
        root = Path(directory).resolve()
        check_install(root)
        check_python(root)
        check_java(root)
    print("PASS: native Java run/arguments/reload/failure recovery, Python environment and installer layout")
