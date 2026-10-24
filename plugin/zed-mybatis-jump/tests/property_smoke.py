from pathlib import Path
import tempfile
from smoke import Client, ROOT, PROJECT_ROOT

with tempfile.TemporaryDirectory(prefix="mybatis-properties-") as temporary:
    root=Path(temporary)
    java=root/"Model.java"
    java.write_text("package demo; public class Model { private String name; void m() { String local; } }",encoding="utf-8")
    child=root/"Child.java"
    child.write_text("package demo; public record Child(String title) {}",encoding="utf-8")
    xml=root/"Mapper.xml"
    source="<mapper namespace='demo.M'><resultMap id='m' type='demo.Model'><result property='name'/><association property='child' javaType='demo.Child'><result property='title'/></association><result property='local'/></resultMap><select id='find' resultMap='m'/></mapper>"
    xml.write_text(source,encoding="utf-8")
    client=Client(root,link_support=True)
    try:
        client.open(xml,source)
        for marker,expected in [("name",java),("title",child),("demo.Model",java)]:
            point=source.index("property='"+marker+"'")+len("property='") if marker in ("name","title") else source.index(marker)
            link=client.request("textDocument/definition",{"textDocument":{"uri":xml.as_uri()},"position":{"line":0,"character":point}})
            assert len(link)==1 and link[0]["targetUri"]==expected.as_uri(),link
            assert link[0]["originSelectionRange"]["end"]["character"]-link[0]["originSelectionRange"]["start"]["character"]==len(marker),link
        assert client.definition(xml,source,"local") == []
    finally: client.close()
root=PROJECT_ROOT
xml=root/"ai-manager-app/ai-manager-management/src/main/resources/mapper/ImagePromptLibraryMapper.xml"
source=xml.read_text(encoding="utf-8-sig")
client=Client(root,link_support=True)
try:
    client.open(xml,source)
    # First commonBody is in a currently unresolved old type; choose the actual existing model.
    point=source.index('property="commonBody"',source.index('type="ai.manager.management.model.ImagePromptLibraryRecord"'))+len('property="')
    prefix=source[:point]
    result=client.request("textDocument/definition",{"textDocument":{"uri":xml.as_uri()},"position":{"line":prefix.count("\n"),"character":len(prefix.rsplit("\n",1)[-1])}})
    assert len(result)==1 and result[0]["targetUri"].endswith("ImagePromptLibraryRecord.java"),result
    assert result[0]["targetSelectionRange"]["start"]["line"]==23,result
finally: client.close()
print("Property + LocationLink OK: declared Java fields, record components, nested explicit owner, local-variable exclusion, precise clickable origin, real commonBody field")
