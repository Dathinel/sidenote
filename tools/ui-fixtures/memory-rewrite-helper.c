#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <windows.h>

int main(int argc, char **argv) {
    const char *marker = getenv("SIDENOTE_MEMORY_TEST_MARKER");
    const char *mode = getenv("SIDENOTE_MEMORY_TEST_MODE");
    char dir[MAX_PATH];
    char input[16000];
    if (!marker || !mode || !GetCurrentDirectoryA(sizeof dir, dir)) return 8;
    size_t count = fread(input, 1, sizeof input - 1, stdin);
    input[count] = 0;
    if (!strstr(input, "Use Friday afternoon.") || !strstr(input, "1800000000001-1")) return 9;
    int read_only = 0, shell_off = 0;
    for (int i = 1; i < argc; i++) {
        if (!strcmp(argv[i], "read-only")) read_only = 1;
        if (!strcmp(argv[i], "features.shell_tool=false")) shell_off = 1;
    }
    if (!read_only || !shell_off) return 10;
    FILE *out = fopen(marker, "w");
    if (!out) return 11;
    fprintf(out, "%lu\n%s\n", (unsigned long)GetCurrentProcessId(), dir);
    fclose(out);
    if (!strcmp(mode, "hold")) Sleep(120000);
    puts("{\"type\":\"item.completed\",\"item\":{\"type\":\"agent_message\",\"text\":\"{\\\"text\\\":\\\"The demo is on Friday afternoon.\\\"}\"}}");
    return 0;
}
