/* Benchmark-only job owner for the documented SDK child contract. */
#include <windows.h>
#include <stdio.h>
#include <wchar.h>
int wmain(void) {
    HANDLE job=CreateJobObjectW(0,0);JOBOBJECT_EXTENDED_LIMIT_INFORMATION limit={0};
    limit.BasicLimitInformation.LimitFlags=JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    if(!job||!SetInformationJobObject(job,JobObjectExtendedLimitInformation,&limit,sizeof(limit)))return 120;
    wchar_t command[32768];swprintf(command,32768,L"\"%ls\" service --embedded",HEX_COMMAND);
    STARTUPINFOW si={0};PROCESS_INFORMATION pi={0};si.cb=sizeof(si);si.dwFlags=STARTF_USESTDHANDLES;
    si.hStdInput=GetStdHandle(STD_INPUT_HANDLE);si.hStdOutput=GetStdHandle(STD_OUTPUT_HANDLE);si.hStdError=GetStdHandle(STD_ERROR_HANDLE);
    if(!CreateProcessW(HEX_COMMAND,command,0,0,TRUE,CREATE_SUSPENDED|CREATE_NO_WINDOW,0,0,&si,&pi))return 121;
    if(!AssignProcessToJobObject(job,pi.hProcess)){TerminateProcess(pi.hProcess,1);WaitForSingleObject(pi.hProcess,INFINITE);return 122;}
    if(ResumeThread(pi.hThread)==(DWORD)-1){TerminateJobObject(job,1);WaitForSingleObject(pi.hProcess,INFINITE);return 123;}
    WaitForSingleObject(pi.hProcess,INFINITE);DWORD code=1;GetExitCodeProcess(pi.hProcess,&code);
    CloseHandle(pi.hThread);CloseHandle(pi.hProcess);CloseHandle(job);return code;
}
