#!/usr/bin/env python3
import ctypes as C
import os

X = C.CDLL('libX11.so.6')
S = C.CDLL('libXext.so.6')
L = C.CDLL(None)
class Info(C.Structure):
    _fields_ = [('shmseg', C.c_ulong), ('shmid', C.c_int), ('shmaddr', C.c_void_p), ('readOnly', C.c_int)]
class Error(C.Structure):
    _fields_ = [('type', C.c_int), ('display', C.c_void_p), ('resourceid', C.c_ulong), ('serial', C.c_ulong), ('error_code', C.c_ubyte), ('request_code', C.c_ubyte), ('minor_code', C.c_ubyte)]
errors = []
@C.CFUNCTYPE(C.c_int, C.c_void_p, C.POINTER(Error))
def handler(display, error):
    e = error.contents
    errors.append([e.error_code,e.request_code,e.minor_code])
    return 0
X.XOpenDisplay.argtypes = [C.c_char_p]
X.XOpenDisplay.restype = C.c_void_p
X.XSetErrorHandler.argtypes = [C.c_void_p]
X.XSync.argtypes = [C.c_void_p,C.c_int]
X.XCloseDisplay.argtypes = [C.c_void_p]
S.XShmAttach.argtypes = [C.c_void_p,C.POINTER(Info)]
S.XShmDetach.argtypes = [C.c_void_p,C.POINTER(Info)]
L.shmat.restype = C.c_void_p
L.shmat.argtypes = [C.c_int,C.c_void_p,C.c_int]
L.shmdt.argtypes = [C.c_void_p]
d = X.XOpenDisplay(b':0')
assert d, 'Cannot open display'
X.XSetErrorHandler(handler)
ident=L.shmget(0,4096,0o1600)
assert ident >= 0, 'shmget failed'
addr=L.shmat(ident,None,0)
info=Info(0,ident,addr,False)
result=S.XShmAttach(d,C.byref(info))
X.XSync(d,False)
print({'uid':os.getuid(),'shmid':ident,'attach_return':result,'x11_errors':errors},flush=True)
if not errors:
    S.XShmDetach(d,C.byref(info))
    X.XSync(d,False)
X.XCloseDisplay(d)
L.shmdt(addr)
L.shmctl(ident,0,None)
