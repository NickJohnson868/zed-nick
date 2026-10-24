"""XML reference and Java type navigation through the actual LSP."""
from pathlib import Path
import tempfile
from smoke import Client, ROOT, PROJECT_ROOT

with tempfile.TemporaryDirectory(prefix="mybatis-xml-navigation-") as directory:
    root = Path(directory)
    java = root / "Model.java"
    java.write_text("package demo; public record Model(String name) {}", encoding="utf-8")
    xml = root / "Mapper.xml"
    other = root / "Other.xml"
    other.write_text("<mapper namespace='other.N'><resultMap id='base' type='demo.Model'/><sql id='cols'>id</sql></mapper>", encoding="utf-8")
    source = """<mapper namespace='demo.M'>
<resultMap id='base' type='demo.Model'/>
<resultMap id='child' extends='base'/>
<sql id='cols'>name</sql>
<select id='read' resultMap='base, other.N.base' resultType='demo.Model' parameterType='demo.Model'>
<include refid='cols'/><include refid='other.N.cols'/>
</select>
<resultMap id='nested' type='demo.Model'><association property='x' javaType='demo.Model' resultMap='base'/><collection property='y' ofType='demo.Model'/></resultMap>
</mapper>"""
    # Disk deliberately differs: local references must resolve the unsaved buffer.
    xml.write_text("<mapper namespace='demo.M'/>", encoding="utf-8")
    client = Client(root)
    try:
        client.open(xml, source)
        for name, path, line in [
            ("base,", xml, 1), ("other.N.base", other, 0),
            ("cols'/>", xml, 3), ("other.N.cols", other, 0),
            ("demo.Model", java, 0),
        ]:
            result = client.definition(xml, source, name)
            assert len(result) == 1 and result[0]["uri"] == path.as_uri(), (name, result)
            assert result[0]["range"]["start"]["line"] == line, result
        for key in ["extends", "resultType", "parameterType", "javaType", "ofType"]:
            point = source.index(key + "='") + len(key) + 2
            prefix = source[:point]
            result = client.request("textDocument/definition", {"textDocument":{"uri":xml.as_uri()},"position":{"line":prefix.count("\n"),"character":len(prefix.rsplit("\n",1)[-1])}})
            assert len(result) == 1 and result[0]["uri"] == (xml if key == "extends" else java).as_uri(), (key,result)
        assert client.definition(xml,source,"property") == []
    finally:
        client.close()

root = PROJECT_ROOT
xml = root / "ai-manager-app/ai-manager-management/src/main/resources/mapper/ImagePromptLibraryMapper.xml"
source = xml.read_text(encoding="utf-8-sig")
client = Client(root)
try:
    client.open(xml,source)
    for marker, expected in [("resultMap=\"ImagePromptLibraryRecordMap", xml.as_uri()),("refid=\"snapshotColumns",xml.as_uri()),("ai.manager.management.model.ImagePromptLibraryRecord", "ImagePromptLibraryRecord.java")]:
        point = source.index(marker) + (marker.index('=') + 2 if '=' in marker else 0)
        prefix = source[:point]
        result = client.request("textDocument/definition", {"textDocument":{"uri":xml.as_uri()},"position":{"line":prefix.count("\n"),"character":len(prefix.rsplit("\n",1)[-1])}})
        assert len(result) == 1 and result[0]["uri"].endswith(expected), (marker,result)
finally:
    client.close()
print("XML reference LSP OK: local and cross-namespace resultMap/sql, comma lists, extends, unsaved buffer, Java class/record type attributes, repository references")
