/* Deliberate native-memory faults used only as disposable sanitizer controls. */
#include <stdlib.h>
#include <string.h>

/* Fixed mode frees exactly the allocation whose lifetime leaked in the positive control. */
static void allocation(void) {
    unsigned char *value = malloc(4096);
    if (!value) exit(2);
    memset(value, 1, 4096);
#ifdef FIXED
    free(value);
#endif
}

/* A successful sanitizer must reject the leaking build and accept the fixed build. */
int main(int argc, char **argv) {
    /* Use-after-free and boundary-write controls share the same allocation and cleanup. */
    if (argc == 2 && strcmp(argv[1], "uaf") == 0) {
        volatile unsigned char *value = malloc(16);
        if (!value) return 2;
#ifdef FIXED
        value[0] = 1;
        free((void *)value);
#else
        free((void *)value);
        value[0] = 1;
#endif
        return 0;
    }
    if (argc == 2 && strcmp(argv[1], "overflow") == 0) {
        volatile unsigned char *value = malloc(16);
        if (!value) return 2;
#ifdef FIXED
        value[15] = 1;
#else
        value[16] = 1;
#endif
        free((void *)value);
        return 0;
    }
    allocation();
    return 0;
}
