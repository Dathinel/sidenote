/* Public synthetic CLI fixture. All files are beside this temporary executable. */
#include <stdio.h>
#include <string.h>
#include <stdlib.h>
#include <direct.h>
#include <fcntl.h>
#include <io.h>
static void emit(const char *name, FILE *out) {
    FILE *f = fopen(name, "rb"); int c;
    if (f) { while ((c = fgetc(f)) != EOF) fputc(c, out); fclose(f); }
}
int main(int argc, char **argv) {
    _setmode(_fileno(stdout), _O_BINARY);
    _setmode(_fileno(stderr), _O_BINARY);
    char *slash = strrchr(argv[0], '\\');
    if (slash) { *slash = 0; if (_chdir(argv[0])) return 90; }
    if (argc > 1 && !strcmp(argv[1], "status")) {
        puts("{\"version\":4,\"model\":{\"preparing\":null},\"capture\":{\"state\":\"idle\",\"pending_jobs\":0},\"meeting\":{\"state\":\"idle\"}}"); return 0;
    }
    for (int i=1; i<argc; i++) if (!strcmp(argv[i], "--help")) { puts("--model --language"); return 0; }
    int count=0, code=0; FILE *f=fopen("calls", "r");
    if (f) { fscanf(f, "%d", &count); fclose(f); }
    f=fopen("calls", "w"); fprintf(f, "%d", count+1); fclose(f);
    if (count) { puts("public English"); return 0; }
    emit("reply", stdout); emit("stderr", stderr);
    f=fopen("exit", "r"); if (f) { fscanf(f, "%d", &code); fclose(f); }
    return code;
}
