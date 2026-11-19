/* Synthetic process-boundary fixture, used only in temporary test directories. */
#include <winsock2.h>
#include <windows.h>
#include <stdio.h>
#include <string.h>
#include <stdlib.h>
#include <direct.h>
static int count(const char *path) {
 int n=0;FILE *f=fopen(path,"r");if(f){fscanf(f,"%d",&n);fclose(f);}
 f=fopen(path,"w");fprintf(f,"%d",n+1);fclose(f);return n;
}
static int mode(void){int n=0;FILE *f=fopen("mode","r");if(f){fscanf(f,"%d",&n);fclose(f);}return n;}
static int read_reply(const char *name,char *out,int size){FILE *f=fopen(name,"rb");int n=0;if(f){n=(int)fread(out,1,size-1,f);fclose(f);}out[n]=0;return n;}
static void emit_reply(const char *name){char value[8192];read_reply(name,value,sizeof(value));fputs(value,stdout);}
static DWORD WINAPI lease(void *unused){char b;DWORD n;(void)unused;while(ReadFile(GetStdHandle(STD_INPUT_HANDLE),&b,1,&n,0)&&n){}ExitProcess(0);return 0;}
int main(int argc,char **argv){
 char *slash=strrchr(argv[0],'\\');if(slash){*slash=0;_chdir(argv[0]);}
 if(argc>1&&!strcmp(argv[1],"status")){
 FILE *active=fopen("owner-active","r");if(active){fclose(active);puts("{\"version\":4,\"model\":{\"preparing\":null},\"capture\":{\"state\":\"recording\",\"pending_jobs\":1},\"meeting\":{\"state\":\"idle\"}}");return 0;}
 emit_reply("status-reply");return 0;}
 for(int i=1;i<argc;i++)if(!strcmp(argv[i],"--help")){emit_reply("help-reply");return 0;}
 if(argc>1&&!strcmp(argv[1],"service")){
 count("starts");FILE *p=fopen("service-pid","w");fprintf(p,"%lu",GetCurrentProcessId());fclose(p);
 if(mode()==2)return 42;
 if(mode()==4){Sleep(30000);return 0;}
 if(mode()==10){FILE *a=fopen("owner-active","w");fclose(a);Sleep(30000);return 0;}
 WSADATA w;WSAStartup(MAKEWORD(2,2),&w);SOCKET listener=socket(AF_INET,SOCK_STREAM,0);
 struct sockaddr_in addr={0};addr.sin_family=AF_INET;addr.sin_addr.s_addr=htonl(INADDR_LOOPBACK);
 if(bind(listener,(struct sockaddr*)&addr,sizeof(addr))||listen(listener,4))return 91;
 int len=sizeof(addr);getsockname(listener,(struct sockaddr*)&addr,&len);
 char version[32],kind[32];read_reply("api-version",version,sizeof(version));read_reply("ready-type",kind,sizeof(kind));
 printf("{\"type\":\"%s\",\"url\":\"http://127.0.0.1:%d\",\"token\":\"fixture\",\"apiVersion\":\"%s\",\"pid\":%lu}\n",kind,ntohs(addr.sin_port),version,mode()==5?GetCurrentProcessId()+100000:GetCurrentProcessId());fflush(stdout);
 HANDLE host=CreateThread(0,0,lease,0,0,0);if(!host)return 95;CloseHandle(host);
 for(;;){SOCKET s=accept(listener,0,0);if(s==INVALID_SOCKET)return 92;
 char buf[16384]={0};int used=0,received=0;char *end=0;
 while(used<(int)sizeof(buf)-1){int n=recv(s,buf+used,sizeof(buf)-1-used,0);if(n<=0)break;used+=n;buf[used]=0;end=strstr(buf,"\r\n\r\n");if(end)break;}
 char *size=strstr(buf,"content-length:");if(!size)size=strstr(buf,"Content-Length:");
 if(!end||!size||!strstr(buf,"Bearer fixture")||!strstr(buf,"/transcriptions?model=parakeet_unified_en&language=en"))return 93;
 received=used-(int)(end-buf)-4;int total=atoi(size+15);
 while(received<total){int n=recv(s,buf,sizeof(buf),0);if(n<=0)break;received+=n;}
 int request=count("http-calls");if(mode()==3||mode()==9){Sleep(30000);return 0;}
 if(mode()==11){FILE *a=fopen("owner-active","w");fclose(a);Sleep(30000);return 0;}
 int fail=mode()==1||(mode()==12&&request>0);
 char body[8192];int body_size=read_reply(fail?"http-error":"http-reply",body,sizeof(body));
 char reply[16384];int n=sprintf(reply,"HTTP/1.1 %s\r\nContent-Length: %d\r\nConnection: close\r\n\r\n%s",fail?"409 Conflict":"200 OK",body_size,body);
 send(s,reply,n,0);closesocket(s);
 }
 }
 count("command-calls");if(mode()==9)Sleep(30000);if(strcmp(argv[1],"transcribe"))return 94;
 emit_reply("top-reply");return 0;
}
