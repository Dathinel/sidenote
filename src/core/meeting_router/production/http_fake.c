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
int main(int argc,char **argv){
 char *slash=strrchr(argv[0],'\\');if(slash){*slash=0;_chdir(argv[0]);}
 if(argc>1&&!strcmp(argv[1],"status")){
 puts("{\"version\":4,\"model\":{\"preparing\":null},\"capture\":{\"state\":\"idle\",\"pending_jobs\":0},\"meeting\":{\"state\":\"idle\"}}");return 0;}
 for(int i=1;i<argc;i++)if(!strcmp(argv[i],"--help")){puts("--model --language --json");return 0;}
 if(argc>1&&!strcmp(argv[1],"service")){
 count("starts");FILE *p=fopen("service-pid","w");fprintf(p,"%lu",GetCurrentProcessId());fclose(p);
 if(mode()==2)return 42;
 if(mode()==4){Sleep(30000);return 0;}
 WSADATA w;WSAStartup(MAKEWORD(2,2),&w);SOCKET listener=socket(AF_INET,SOCK_STREAM,0);
 struct sockaddr_in addr={0};addr.sin_family=AF_INET;addr.sin_addr.s_addr=htonl(INADDR_LOOPBACK);
 if(bind(listener,(struct sockaddr*)&addr,sizeof(addr))||listen(listener,4))return 91;
 int len=sizeof(addr);getsockname(listener,(struct sockaddr*)&addr,&len);
 printf("{\"url\":\"http://127.0.0.1:%d\",\"token\":\"fixture\",\"apiVersion\":\"2\",\"pid\":%lu}\n",ntohs(addr.sin_port),mode()==5?GetCurrentProcessId()+100000:GetCurrentProcessId());fflush(stdout);
 for(;;){SOCKET s=accept(listener,0,0);if(s==INVALID_SOCKET)return 92;
 char buf[16384]={0};int used=0,received=0;char *end=0;
 while(used<(int)sizeof(buf)-1){int n=recv(s,buf+used,sizeof(buf)-1-used,0);if(n<=0)break;used+=n;buf[used]=0;end=strstr(buf,"\r\n\r\n");if(end)break;}
 char *size=strstr(buf,"content-length:");if(!size)size=strstr(buf,"Content-Length:");
 if(!end||!size||!strstr(buf,"Bearer fixture")||!strstr(buf,"/transcriptions?model=parakeet_unified_en&language=en"))return 93;
 received=used-(int)(end-buf)-4;int total=atoi(size+15);
 while(received<total){int n=recv(s,buf,sizeof(buf),0);if(n<=0)break;received+=n;}
 count("http-calls");if(mode()==3){Sleep(30000);return 0;}
 const char *body=mode()==6?"{\"transcript\":\"\"}":"{\"transcript\":\"public HTTP\"}";
 char reply[512];int n=sprintf(reply,"HTTP/1.1 %s\r\nContent-Length: %d\r\nConnection: close\r\n\r\n%s",mode()==1?"503 Unavailable":"200 OK",(int)strlen(body),body);
 send(s,reply,n,0);closesocket(s);
 }
 }
 count("command-calls");if(strcmp(argv[1],"transcribe"))return 94;
 puts("{\"transcript\":\"public fallback\"}");return 0;
}
