"""Windows accounting includes descendants that exit between samples."""
import ctypes
from ctypes import wintypes
import subprocess
import sys
import psutil


class Accounting(ctypes.Structure):
    _fields_ = [(name, ctypes.c_longlong) for name in (
        "user", "kernel", "period_user", "period_kernel")] + [
        (name, wintypes.DWORD) for name in (
            "faults", "processes", "active", "terminated")]


class CpuJob:
    def __init__(self):
        self.api = ctypes.WinDLL("kernel32", use_last_error=True)
        self.api.CreateJobObjectW.argtypes = [ctypes.c_void_p, wintypes.LPCWSTR]
        self.api.CreateJobObjectW.restype = wintypes.HANDLE
        self.api.AssignProcessToJobObject.argtypes = [wintypes.HANDLE, wintypes.HANDLE]
        self.api.QueryInformationJobObject.argtypes = [
            wintypes.HANDLE, ctypes.c_int, ctypes.c_void_p, wintypes.DWORD, ctypes.c_void_p]
        self.api.TerminateJobObject.argtypes = [wintypes.HANDLE, wintypes.UINT]
        self.api.CloseHandle.argtypes = [wintypes.HANDLE]
        self.handle = self.api.CreateJobObjectW(None, None)
        if not self.handle:
            raise ctypes.WinError(ctypes.get_last_error())

    def start(self, command, **kwargs):
        proc = subprocess.Popen(command, creationflags=(
            subprocess.CREATE_NO_WINDOW | 0x00000004), **kwargs)  # CREATE_SUSPENDED
        if not self.api.AssignProcessToJobObject(self.handle, int(proc._handle)):
            proc.kill()
            proc.wait()
            raise ctypes.WinError(ctypes.get_last_error())
        psutil.Process(proc.pid).resume()
        return proc

    def sample(self):
        value = Accounting()
        if not self.api.QueryInformationJobObject(
                self.handle, 1, ctypes.byref(value), ctypes.sizeof(value), None):
            raise ctypes.WinError(ctypes.get_last_error())
        return (value.user + value.kernel) / 10_000_000, value.processes, value.active

    def root_cpu(self, proc):
        values = [ctypes.c_ulonglong() for _ in range(4)]
        self.api.GetProcessTimes.argtypes = [wintypes.HANDLE] + [ctypes.c_void_p] * 4
        if not self.api.GetProcessTimes(int(proc._handle), *(ctypes.byref(v) for v in values)):
            raise ctypes.WinError(ctypes.get_last_error())
        return (values[2].value + values[3].value) / 10_000_000

    def close(self):
        try:
            if not self.api.TerminateJobObject(self.handle, 0):
                raise ctypes.WinError(ctypes.get_last_error())
        finally:
            self.api.CloseHandle(self.handle)


if __name__ == "__main__":
    job = CpuJob()
    try:
        child_code = "import time\nend=time.process_time()+0.25\nwhile time.process_time()<end: pass"
        parent_code = (
            "import subprocess,sys; subprocess.run([sys.executable,'-c'," +
            repr(child_code) + "],check=True,creationflags=0x08000000)")
        proc = job.start([sys.executable, "-c", parent_code])
        proc.wait(timeout=30)
        cpu, processes, active = job.sample()
        root_cpu = job.root_cpu(proc)
        assert proc.returncode == 0 and processes >= 2
        assert cpu - root_cpu >= 0.25, "exited child CPU was not counted"
        print(f"job accounting self-test: processes={processes}, active={active}, cpu_seconds={cpu:.6f}, root_cpu={root_cpu:.6f}, child_cpu={cpu-root_cpu:.6f}; PASS")
    finally:
        job.close()
