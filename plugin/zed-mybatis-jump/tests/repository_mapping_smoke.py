import sys
sys.path.insert(0,'tests')
from smoke import Client,ROOT, PROJECT_ROOT
root=PROJECT_ROOT;xml=root/'ai-manager-app/ai-manager-management/src/main/resources/mapper/ImagePromptLibraryMapper.xml';s=xml.read_text(encoding='utf-8-sig');c=Client(root)
try:
 c.open(xml,s)
 baseline=c.wait_diagnostics(xml,1)
 # Config change can publish an initial empty list; force a later version to wait for a completed run.
 c.notify('textDocument/didChange',{'textDocument':{'uri':xml.as_uri(),'version':2},'contentChanges':[{'text':s}]})
 baseline=c.wait_diagnostics(xml,2)
 print('Repository baseline codes:',[d['code'] for d in baseline])
 changed=s.replace('column="style_body"','column="style_body1"')
 c.notify('textDocument/didChange',{'textDocument':{'uri':xml.as_uri(),'version':3},'contentChanges':[{'text':changed}]})
 result=c.wait_diagnostics(xml,3)
 errors=[d for d in result if d['code']=='mybatis-result-column' and 'style_body1' in d['message']]
 assert len(errors)==1,result
 print('Real ImagePromptLibraryMapper style_body1 exact diagnostic: OK')
finally:c.close()
