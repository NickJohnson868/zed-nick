"""Real PostgreSQL Parse/Describe checks; never executes Mapper SQL."""
import json
import subprocess
from smoke import Client, EXE, ROOT, PROJECT_ROOT


def quoted(name):
    return '"' + name.replace('"', '""') + '"'


root = PROJECT_ROOT
summary = json.loads(subprocess.check_output([str(EXE), "--check-db", str(root)], text=True))
assert summary["readonly"] == "on"
sample = summary["sample"]
table = quoted(sample["schema"]) + "." + quoted(sample["table"])
column = sample["columns"][0]
qualified = quoted(column)
uri = root / "temp/mybatis-postgres-protocol-check.xml"
client = Client(root)
try:
    source = f'''<mapper namespace="test.Validation">
  <select id="valid">SELECT s.{qualified} FROM {table} s</select>
  <select id="cte">WITH q AS (SELECT {qualified} FROM {table}) SELECT {qualified} FROM q</select>
  <select id="badColumn">SELECT s.zed_missing_field_92741 FROM {table} s</select>
  <select id="badTable">SELECT * FROM public.zed_missing_table_92741</select>
  <select id="syntax">SELECT * FRM {table}</select>
  <select id="dynamic">SELECT missing <if test="x">FROM missing</if></select>
  <select id="cdata"><![CDATA[SELECT zed_missing_field_92741 FROM {table}]]></select>
</mapper>'''
    client.open(uri, source)
    semantic = client.request("textDocument/semanticTokens/full", {"textDocument": {"uri": uri.as_uri()}})["data"]
    assert semantic and len(semantic) % 5 == 0
    assert {token[3] for token in [semantic[i:i+5] for i in range(0,len(semantic),5)]} >= {0,1}

    # Initial config refresh may first publish an empty list; wait for actual errors.
    diagnostics = []
    while not diagnostics:
        diagnostics = client.wait_diagnostics(uri, 1)
    assert len(diagnostics) == 4, diagnostics
    assert sorted(d["code"] for d in diagnostics) == ["42601", "42703", "42703", "42P01"]
    assert all(d["severity"] == 1 for d in diagnostics)
    bad = next(d for d in diagnostics if d["range"]["start"]["line"] == 3)
    location = bad["range"]
    line = source.splitlines()[3]
    fragment = line[location["start"]["character"]:location["end"]["character"]]
    assert "zed_missing_field_92741" in fragment, fragment
    # Repair SQL in an unsaved buffer: the old red lines must disappear.
    fixed = f'<mapper><select id="valid">SELECT s.{qualified} FROM {table} s</select></mapper>'
    client.notify("textDocument/didChange", {"textDocument": {"uri": uri.as_uri(), "version": 2}, "contentChanges": [{"text": fixed}]})
    assert client.wait_diagnostics(uri, 2) == []
    completion = f'<mapper><select id="x">SELECT s. FROM {table} s</select></mapper>'
    client.notify("textDocument/didChange", {"textDocument": {"uri": uri.as_uri(), "version": 3}, "contentChanges": [{"text": completion}]})
    point = completion.index("s.") + 2
    items = client.request("textDocument/completion", {"textDocument": {"uri": uri.as_uri()}, "position": {"line": 0, "character": point}})
    assert any(item["label"] == column for item in items), items
    # Registering completions for XML must not add any Java suggestions.
    java = root / "temp/MyBatisCheck.java"
    client.open(java, "class MyBatisCheck {}")
    assert client.request("textDocument/completion", {"textDocument": {"uri": java.as_uri()}, "position": {"line": 0, "character": 8}}) == []
finally:
    client.close()

print(f"PostgreSQL live checks OK: {summary['tables']} tables/views, {summary['columns']} columns; missing fields/tables, syntax, CTE, CDATA, exact red-line ranges, live repairs, field completion, Java isolation")
