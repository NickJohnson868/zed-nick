import ctypes as c
from ctypes import wintypes as w
import os, pathlib, subprocess, time, tempfile
if os.name != 'nt':
    print('SKIP: Windows ConPTY console test')
    raise SystemExit(0)

k = c.WinDLL('kernel32', use_last_error=True)
class COORD(c.Structure): _fields_=[('X',c.c_short),('Y',c.c_short)]
class SI(c.Structure):
    _fields_=[('cb',w.DWORD),('lpReserved',w.LPWSTR),('lpDesktop',w.LPWSTR),('lpTitle',w.LPWSTR),('dwX',w.DWORD),('dwY',w.DWORD),('dwXSize',w.DWORD),('dwYSize',w.DWORD),('dwXCountChars',w.DWORD),('dwYCountChars',w.DWORD),('dwFillAttribute',w.DWORD),('dwFlags',w.DWORD),('wShowWindow',w.WORD),('cbReserved2',w.WORD),('lpReserved2',c.POINTER(c.c_ubyte)),('hStdInput',w.HANDLE),('hStdOutput',w.HANDLE),('hStdError',w.HANDLE)]
class SIX(c.Structure): _fields_=[('StartupInfo',SI),('lpAttributeList',c.c_void_p)]
class PI(c.Structure): _fields_=[('hProcess',w.HANDLE),('hThread',w.HANDLE),('dwProcessId',w.DWORD),('dwThreadId',w.DWORD)]
k.CreatePipe.argtypes=[c.POINTER(w.HANDLE),c.POINTER(w.HANDLE),c.c_void_p,w.DWORD]
k.CreatePseudoConsole.argtypes=[COORD,w.HANDLE,w.HANDLE,w.DWORD,c.POINTER(w.HANDLE)]
k.InitializeProcThreadAttributeList.argtypes=[c.c_void_p,w.DWORD,w.DWORD,c.POINTER(c.c_size_t)]
k.UpdateProcThreadAttribute.argtypes=[c.c_void_p,w.DWORD,c.c_size_t,c.c_void_p,c.c_size_t,c.c_void_p,c.c_void_p]
k.CreateProcessW.argtypes=[w.LPCWSTR,w.LPWSTR,c.c_void_p,c.c_void_p,w.BOOL,w.DWORD,c.c_void_p,w.LPCWSTR,c.POINTER(SIX),c.POINTER(PI)]
k.PeekNamedPipe.argtypes=[w.HANDLE,c.c_void_p,w.DWORD,c.c_void_p,c.POINTER(w.DWORD),c.c_void_p]
k.ReadFile.argtypes=[w.HANDLE,c.c_void_p,w.DWORD,c.POINTER(w.DWORD),c.c_void_p]
k.WaitForSingleObject.argtypes=[w.HANDLE,w.DWORD]
k.CloseHandle.argtypes=[w.HANDLE]
k.ClosePseudoConsole.argtypes=[w.HANDLE]
k.DeleteProcThreadAttributeList.argtypes=[c.c_void_p]
temporary=tempfile.TemporaryDirectory(prefix='zed-console-')
root=pathlib.Path(temporary.name)
source=root/'EncodingProbe.java'
source.write_text('''import java.nio.charset.StandardCharsets;
public class EncodingProbe {
 public static void main(String[] args) throws Exception {
  if(args.length>0) {
   JavaRunner.initializeWindowsConsole();
  }
  System.out.write("ENCODING_MARKER=任务等待前置条件，门店同步完成\\n".getBytes(StandardCharsets.UTF_8));System.out.flush();
 }
}''',encoding='utf-8')
java=str(pathlib.Path(os.environ['JAVA_HOME'])/'bin/java.exe')
subprocess.run([str(pathlib.Path(java).with_name('javac.exe')),'-encoding','UTF-8','-d',str(root),str(source),str(pathlib.Path(__file__).resolve().parents[1]/'scripts/JavaRunner.java')],check=True)
def probe(fix):
    ri,wi,ro,wo,hpc=(w.HANDLE() for _ in range(5))
    assert k.CreatePipe(c.byref(ri),c.byref(wi),None,0)
    assert k.CreatePipe(c.byref(ro),c.byref(wo),None,0)
    assert k.CreatePseudoConsole(COORD(200,30),ri,wo,0,c.byref(hpc))==0
    size=c.c_size_t()
    k.InitializeProcThreadAttributeList(None,1,0,c.byref(size))
    attributes=c.create_string_buffer(size.value)
    assert k.InitializeProcThreadAttributeList(attributes,1,0,c.byref(size))
    assert k.UpdateProcThreadAttribute(attributes,0,0x20016,hpc,c.sizeof(w.HANDLE),None,None)
    si=SIX();si.StartupInfo.cb=c.sizeof(SIX);si.lpAttributeList=c.cast(attributes,c.c_void_p)
    process=PI()
    arguments=[java,'-Dstdout.encoding=UTF-8','-cp',str(root),'EncodingProbe']+(['fix'] if fix else [])
    command='cmd.exe /d /s /c "chcp 936 >nul & '+' '.join('"'+argument+'"' for argument in arguments)+' >CONOUT$ 2>&1"'
    assert k.CreateProcessW(os.environ['COMSPEC'],c.create_unicode_buffer(command),None,None,False,0x80000,None,str(root),c.byref(si),c.byref(process)),c.get_last_error()
    data=bytearray();deadline=time.monotonic()+30;exited=None
    try:
        while time.monotonic()<deadline:
            available=w.DWORD()
            if k.PeekNamedPipe(ro,None,0,None,c.byref(available),None) and available.value:
                buffer=c.create_string_buffer(available.value);read=w.DWORD()
                if k.ReadFile(ro,buffer,len(buffer),c.byref(read),None):data.extend(buffer.raw[:read.value])
            if k.WaitForSingleObject(process.hProcess,0)==0:
                if exited is None:exited=time.monotonic()
                if time.monotonic()-exited>0.4:break
            time.sleep(.02)
        else:raise RuntimeError('ConPTY probe timed out')
    finally:
        k.DeleteProcThreadAttributeList(attributes)
        k.ClosePseudoConsole(hpc)
        for handle in [ri,wi,ro,wo,process.hProcess,process.hThread]:k.CloseHandle(handle)
    return data.decode('utf-8',errors='replace')
before=probe(False);after=probe(True)
expected='ENCODING_MARKER=任务等待前置条件，门店同步完成'
assert expected not in before,repr(before)
assert expected in after,repr(after)
detached=subprocess.run([java,'-Dstdout.encoding=UTF-8','-cp',str(root),'EncodingProbe','fix'],creationflags=0x8000000,capture_output=True,text=True,encoding='utf-8',check=True)
assert expected in detached.stdout,repr(detached.stdout)
print('PASS: reproduced UTF-8 log corruption in ConPTY codepage 936; shared-console UTF-8 setup fixes it')

temporary.cleanup()
