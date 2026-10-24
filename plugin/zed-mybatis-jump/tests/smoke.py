"""Exercise the real stdio LSP binary, including live edits and repository XML."""
import json
from pathlib import Path
import queue
import subprocess
import tempfile
import threading
import time
import os


ROOT = Path(__file__).resolve().parents[1]
PROJECT_ROOT = Path(os.environ.get("MYBATIS_PROJECT_ROOT", "C:/Users/62453/Documents/projects/ai-manager-server"))
EXE = Path(os.environ.get("MYBATIS_LSP_BINARY", ROOT / "bin/mybatis-jump-lsp.exe"))


class Client:
    def __init__(self, root, link_support=False):
        self.process = subprocess.Popen(
            [str(EXE)],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
        )
        self.replies = queue.Queue()
        self.sequence = 0
        self.notifications = []
        threading.Thread(target=self.read, daemon=True).start()
        result = self.request("initialize", {"rootUri": root.as_uri(), "capabilities":{"textDocument":{"definition":{"linkSupport":link_support}}}})
        assert result["capabilities"]["definitionProvider"] is True
        assert "." in result["capabilities"]["completionProvider"]["triggerCharacters"]
        self.notify("initialized", {})

    def read(self):
        try:
            while True:
                headers = {}
                while True:
                    line = self.process.stdout.readline()
                    if not line:
                        return
                    if line == b"\r\n":
                        break
                    key, value = line.decode().split(":", 1)
                    headers[key.lower()] = value.strip()
                size = int(headers["content-length"])
                self.replies.put(json.loads(self.process.stdout.read(size)))
        finally:
            self.replies.put({"error": "server exited"})

    def send(self, value):
        data = json.dumps({"jsonrpc": "2.0", **value}, ensure_ascii=False).encode()
        self.process.stdin.write(f"Content-Length: {len(data)}\r\n\r\n".encode() + data)
        self.process.stdin.flush()

    def notify(self, method, params):
        self.send({"method": method, "params": params})

    def request(self, method, params):
        self.sequence += 1
        self.send({"id": self.sequence, "method": method, "params": params})
        while True:
            result = self.replies.get(timeout=15)
            if "id" in result or "error" in result:
                break
            self.notifications.append(result)
        assert result.get("id") == self.sequence, result
        assert "error" not in result, result
        return result["result"]

    def definition(self, path, text, name):
        offset = text.index(name)
        prefix = text[:offset]
        point = {"line": prefix.count("\n"), "character": len(prefix.rsplit("\n", 1)[-1].encode("utf-16-le")) // 2}
        return self.request("textDocument/definition", {"textDocument": {"uri": path.as_uri()}, "position": point})

    def open(self, path, text):
        self.notify("textDocument/didOpen", {"textDocument": {"uri": path.as_uri(), "languageId": "java", "version": 1, "text": text}})

    def wait_diagnostics(self, path, version):
        deadline = time.monotonic() + 15
        def matches(message):
            return message.get("method") == "textDocument/publishDiagnostics" and message["params"]["uri"] == path.as_uri() and message["params"].get("version") == version
        while True:
            for index, message in enumerate(self.notifications):
                if matches(message):
                    self.notifications.pop(index)
                    return message["params"]["diagnostics"]
            message = self.replies.get(timeout=max(0.1, deadline-time.monotonic()))
            assert "error" not in message, message
            self.notifications.append(message)
            assert time.monotonic() < deadline, "diagnostics timeout"

    def close(self):
        try:
            self.request("shutdown", None)
            self.notify("exit", None)
            assert self.process.wait(timeout=5) == 0
        finally:
            if self.process.poll() is None:
                self.process.kill()
            self.process.stdin.close()
            self.process.stdout.close()


def fixture_check():
    with tempfile.TemporaryDirectory(prefix="mybatis-jump-") as temporary:
        root = Path(temporary)
        java = root / "StoreMapper.java"
        xml = root / "StoreMapper.xml"
        source = "package demo; public interface StoreMapper {\n String find();\n String next();\n}"
        java.write_text(source, encoding="utf-8")
        xml.write_text("<mapper namespace='demo.StoreMapper'>\n<select id='find'>SELECT 1</select></mapper>", encoding="utf-8")
        client = Client(root)
        try:
            client.open(java, source)
            results = client.definition(java, source, "find")
            assert len(results) == 1 and results[0]["uri"] == xml.as_uri()
            assert results[0]["range"]["start"]["line"] == 1
            xml_source = xml.read_text(encoding="utf-8")
            client.open(xml, xml_source)
            reverse = client.definition(xml, xml_source, "find")
            assert len(reverse) == 1 and reverse[0]["uri"] == java.as_uri()
            assert reverse[0]["range"]["start"]["line"] == 1
            namespace = client.definition(xml, xml_source, "demo.StoreMapper")
            assert len(namespace) == 1 and namespace[0]["range"]["start"]["line"] == 0
            assert client.definition(java, source, "String") == []
            assert client.definition(java, source, "next") == []
            xml.write_text("<mapper namespace='demo.StoreMapper'>\n\n<update id='next'>UPDATE x SET y=1</update></mapper>", encoding="utf-8")
            assert client.definition(java, source, "next")[0]["range"]["start"]["line"] == 2
            assert client.definition(java, source, "find") == []
            changed = source.replace("interface", "class")
            client.notify("textDocument/didChange", {"textDocument": {"uri": java.as_uri(), "version": 2}, "contentChanges": [{"text": changed}]})
            assert client.definition(java, changed, "next") == []
            # Generated targets must never add spurious definitions.
            (root / "target").mkdir()
            (root / "target/Mapper.xml").write_text("<mapper namespace='demo.StoreMapper'><select id='find'/></mapper>")
            client.open(java, source)
            assert client.definition(java, source, "find") == []
        finally:
            client.close()
    print("LSP fixture: namespace, ids, live XML edits, unsaved Java edits, non-Mapper fallback, target exclusion, shutdown OK")


def repository_check():
    root = PROJECT_ROOT
    relative = "ai-manager-infra/ai-manager-database/src/main/java/ai/manager/database/time/mapper/ServerClockMapper.java"
    java = root / relative
    source = java.read_text(encoding="utf-8-sig")
    client = Client(root)
    try:
        client.open(java, source)
        start = time.perf_counter()
        results = client.definition(java, source, "selectCurrentTime")
        assert len(results) == 1, results
        assert results[0]["uri"].endswith("/resources/mapper/ServerClockMapper.xml")
        assert results[0]["range"]["start"]["line"] == 4
        elapsed = time.perf_counter() - start
        print(f"Repository Mapper -> XML: OK, {elapsed:.3f}s")
        xml = root / "ai-manager-infra/ai-manager-database/src/main/resources/mapper/ServerClockMapper.xml"
        xml_source = xml.read_text(encoding="utf-8-sig")
        client.open(xml, xml_source)
        reverse = client.definition(xml, xml_source, "selectCurrentTime")
        assert len(reverse) == 1 and reverse[0]["uri"] == java.as_uri()
        assert reverse[0]["range"]["start"]["line"] == 17
        print("Repository XML id -> Mapper method: OK")
    finally:
        client.close()


if __name__ == "__main__":
    fixture_check()
    repository_check()
