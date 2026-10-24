from pathlib import Path
import tempfile,time,statistics
from smoke import Client,PROJECT_ROOT

with tempfile.TemporaryDirectory(prefix='mybatis-parameter-jump-') as directory:
 root=Path(directory);xml=root/'Mapper.xml';java=root/'Mapper.java';dto=root/'Dto.java';other=root/'Other.xml'
 java.write_text('package demo; import org.apache.ibatis.annotations.Param; public interface Mapper { String find(@Param("organizationId") Long id, @Param("query") Dto query); String bean(Dto dto); }',encoding='utf-8')
 dto.write_text('package demo; public class Dto { private String name; }',encoding='utf-8')
 other.write_text('<mapper namespace="other.M"><sql id="vars"><bind name="remote" value="1"/></sql></mapper>',encoding='utf-8')
 source='''<mapper namespace="demo.Mapper">
 <sql id="vars"><bind name="bound" value="1"/></sql>
 <select id="find"><include refid="vars"/><include refid="other.M.vars"/>
 SELECT #{bound}, #{remote}, #{organizationId}, #{query.name}
 <foreach collection="items" item="entry" index="idx">#{entry}, #{idx}</foreach>
 </select>
 <select id="bean">SELECT #{name}</select>
</mapper>'''
 client=Client(root,link_support=True)
 try:
  client.open(xml,source)
  for placeholder,expected in [('bound',xml),('remote',other),('organizationId',java),('query.name',dto),('entry',xml),('idx',xml),('name',dto)]:
   result=client.definition(xml,source,'#{'+placeholder)
   # definition() points at #; request explicitly inside the variable.
   p=source.index('#{'+placeholder)+2;pre=source[:p]
   result=client.request('textDocument/definition',{'textDocument':{'uri':xml.as_uri()},'position':{'line':pre.count('\n'),'character':len(pre.rsplit('\n',1)[-1])}})
   assert len(result)==1 and result[0]['targetUri']==expected.as_uri(),(placeholder,result)
   assert result[0]['originSelectionRange']['end']['character']-result[0]['originSelectionRange']['start']['character']==len(placeholder),result
 finally:client.close()

xml=PROJECT_ROOT/'ai-manager-app/ai-manager-o2o-operations/src/main/resources/mapper/O2oOperationTaskMapper.xml';source=xml.read_text(encoding='utf-8-sig');client=Client(PROJECT_ROOT,link_support=True)
try:
 client.open(xml,source)
 for name in ['sourceProductCodes','operationUpdateStoreCode','operationTargetProduct','organizationId','status']:
  p=source.index('#{'+name)+2;pre=source[:p];params={'textDocument':{'uri':xml.as_uri()},'position':{'line':pre.count('\n'),'character':len(pre.rsplit('\n',1)[-1])}};times=[]
  for _ in range(10):
   t=time.perf_counter();r=client.request('textDocument/definition',params);times.append((time.perf_counter()-t)*1000);assert r,(name,r)
  if name in ['sourceProductCodes','operationUpdateStoreCode','operationTargetProduct','status']:assert r[0]['targetUri']==xml.as_uri(),r
  print(name,'warm_median_ms',round(statistics.median(times[1:]),2))
finally:client.close()
print('Parameter navigation OK: included bind, cross-namespace bind, @Param, DTO field, foreach item/index, real screenshot cases, origin ranges')
