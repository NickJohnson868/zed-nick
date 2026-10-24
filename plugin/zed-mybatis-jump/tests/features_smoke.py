"""Real stdio tests for XML completion, mapping and parameter diagnostics."""
from pathlib import Path
import json, tempfile, time, statistics
from smoke import Client, ROOT, PROJECT_ROOT

def point(source,offset):
    prefix=source[:offset]
    return {"line":prefix.count("\n"),"character":len(prefix.rsplit("\n",1)[-1].encode("utf-16-le"))//2}

def completion(client,xml,source,marker):
    offset=source.index(marker)+len(marker)
    return client.request("textDocument/completion",{"textDocument":{"uri":xml.as_uri()},"position":point(source,offset)})

def errors(client,xml,version):
    while True:
        result=client.wait_diagnostics(xml,version)
        if result:return result

with tempfile.TemporaryDirectory(prefix="mybatis-features-") as temporary:
    root=Path(temporary)
    (root/"Base.java").write_text("package demo; public class Base { private long id; }",encoding="utf-8")
    dto=root/"Query.java"
    dto.write_text("package demo; public class Query extends Base { private String storeName; public boolean isEnabled() {return true;} }",encoding="utf-8")
    (root/"Mapper.java").write_text('''package demo; import org.apache.ibatis.annotations.Param;
public interface Mapper {
 String explicit(@Param("storeId") Long id, @Param("query") Query query);
 String bean(Query query);
 String scalar(String value);
 String dynamic(@Param("x") String value);
 String included(@Param("storeId") Long id);
}''',encoding="utf-8")
    (root/"Other.xml").write_text("<mapper namespace='other.N'><sql id='remoteColumns'>relname</sql><resultMap id='remoteMap' type='demo.Query'/></mapper>",encoding="utf-8")
    xml=root/"Mapper.xml"
    source='''<mapper namespace="demo.Mapper">
 <resultMap id="QueryMap" type="demo.Query">
  <id column="id" property="id"/>
  <result column="store_name" property="storeNmae"/>
  <result column="enabled" property="enabled"/>
 </resultMap>
 <sql id="columns">relname</sql>
 <select id="explicit">SELECT #{storeId}, #{query.storeName}, #{storeNmae}, #{query.badField}</select>
 <select id="bean">SELECT #{storeName}, #{id}, #{enabled}, #{storeNmae}</select>
 <select id="scalar">SELECT #{anything}</select>
 <select id="dynamic">SELECT #{unknown} <if test="x">FROM missing</if></select>
 <select id="included">SELECT <include refid="columns"/> FROM pg_catalog.pg_class WHERE oid=#{storeNmae}</select>
</mapper>'''
    xml.write_text(source,encoding="utf-8")
    client=Client(root)
    try:
        client.open(xml,source)
        result=errors(client,xml,1)
        assert len(result)==5,result
        assert sum(d["code"]=="mybatis-property" for d in result)==1,result
        assert sum(d["code"]=="mybatis-parameter" for d in result)==4,result
        samples=[('property="storeNmae"','property="st"','property="st',['storeName']),
                 ('property="storeNmae"','property=""','property="',['id','storeName','enabled']),
                 ('type="demo.Query"','type="demo.Qu"','type="demo.Qu',['demo.Query']),
                 ('refid="columns"','refid="col"','refid="col',['columns']),
                 ('refid="columns"','refid="other.N.rem"','refid="other.N.rem',['other.N.remoteColumns'])]
        for version,(old,new,marker,expected) in enumerate(samples,2):
            current=source.replace(old,new)
            client.notify("textDocument/didChange",{"textDocument":{"uri":xml.as_uri(),"version":version},"contentChanges":[{"text":current}]})
            items=completion(client,xml,current,marker)
            assert set(expected)<=set(item['label'] for item in items),(marker,items)
        current=source.replace('<select id="scalar">','<select id="scalar" resultMap="Qu">')
        client.notify("textDocument/didChange",{"textDocument":{"uri":xml.as_uri(),"version":10},"contentChanges":[{"text":current}]})
        assert any(i['label']=='QueryMap' for i in completion(client,xml,current,'resultMap="Qu'))
        fixed=source.replace('property="storeNmae"','property="storeName"').replace('#{storeNmae}, #{query.badField}','#{storeId}, #{query.storeName}').replace('#{enabled}, #{storeNmae}','#{enabled}, #{storeName}').replace('oid=#{storeNmae}','oid=#{storeId}')
        client.notify("textDocument/didChange",{"textDocument":{"uri":xml.as_uri(),"version":20},"contentChanges":[{"text":fixed}]})
        assert client.wait_diagnostics(xml,20)==[]
        times=[]
        for _ in range(20):
            t=time.perf_counter();completion(client,xml,fixed,'property="storeName');times.append((time.perf_counter()-t)*1000)
        print('Offline mapping/parameters/completion OK; warm property completion median_ms:',round(statistics.median(times),2))
    finally:client.close()

    settings=json.loads((PROJECT_ROOT/'.zed/mybatis-jump.json').read_text(encoding='utf-8-sig'))
    selected=settings['connections'][settings['active_connection']]
    selected['application_yaml']=str(PROJECT_ROOT/selected['application_yaml'])
    (root/'.zed').mkdir()
    (root/'.zed/mybatis-jump.json').write_text(json.dumps(settings),encoding='utf-8')
    source='''<mapper namespace="demo.Mapper">
 <resultMap id="columnsMap" type="demo.Query"><result column="known" property="storeName"/><result column="wrong_column" property="id"/></resultMap>
 <sql id="projection">relname AS known, oid AS id</sql>
 <select id="a" resultMap="columnsMap">SELECT <include refid="projection"/> FROM pg_catalog.pg_class</select>
 <resultMap id="dynamicMap" type="demo.Query"><result column="unknown_dynamic" property="id"/></resultMap>
 <select id="b" resultMap="dynamicMap">SELECT <if test="x">relname</if> FROM pg_catalog.pg_class</select>
</mapper>'''
    client=Client(root)
    try:
        client.open(xml,source)
        result=errors(client,xml,1)
        assert len(result)==1 and result[0]['code']=='mybatis-result-column',result
        location=result[0]['range'];line=source.splitlines()[location['start']['line']]
        assert line[location['start']['character']:location['end']['character']]=='wrong_column',result
        fixed=source.replace('wrong_column','id')
        client.notify("textDocument/didChange",{"textDocument":{"uri":xml.as_uri(),"version":2},"contentChanges":[{"text":fixed}]})
        assert client.wait_diagnostics(xml,2)==[]
        print('PostgreSQL resultMap columns OK: static includes, SQL aliases, exact squiggle, live repair, dynamic skipped')
    finally:client.close()
