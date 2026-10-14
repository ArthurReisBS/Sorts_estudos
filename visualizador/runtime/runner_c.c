// Runner do lado C. É compilado junto com um arquivo gerado (entry.c) que
// inclui o seu sort de ../C/ e define vt_entry(), vt_elem_size e vt_elem_signed.
//
//   ./runner trace <n> <seed> <padrao> <timeout_s>
//   ./runner bench <seed> <padrao> <reps> <timeout_s> <n1> [n2 ...]

#define _GNU_SOURCE
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

void vt_entry(void *v, long n);
extern const int vt_elem_size;
extern const int vt_elem_signed;

void vt_init(int timeout_s);
void *vt_alloc(long n, long elem_size, int is_signed);
void vt_start(void);
void vt_stop(void);
void vt_dump(void);
void vt_generate(int64_t *out, long n, uint32_t seed, int pattern);
void vt_print_line(const char *tag, const int64_t *v, long n);
void vt_print_text(const char *s);
void vt_print_int(int64_t v);

static void store(void *v, long i, int64_t x) {
    switch (vt_elem_size) {
    case 1: ((int8_t *)v)[i] = (int8_t)x; break;
    case 2: ((int16_t *)v)[i] = (int16_t)x; break;
    case 4: ((int32_t *)v)[i] = (int32_t)x; break;
    default: ((int64_t *)v)[i] = x; break;
    }
}

static int64_t load(const void *v, long i) {
    switch (vt_elem_size) {
    case 1: return vt_elem_signed ? ((const int8_t *)v)[i] : ((const uint8_t *)v)[i];
    case 2: return vt_elem_signed ? ((const int16_t *)v)[i] : ((const uint16_t *)v)[i];
    case 4: return vt_elem_signed ? ((const int32_t *)v)[i] : ((const uint32_t *)v)[i];
    default: return ((const int64_t *)v)[i];
    }
}

static int is_sorted(const int64_t *v, long n) {
    for (long i = 1; i < n; i++)
        if (v[i - 1] > v[i]) return 0;
    return 1;
}

// mesmos valores nos dois vetores (o sort não pode inventar nem perder elementos);
// os valores gerados vão de 1 a n, então dá para contar
static int same_multiset(const int64_t *a, const int64_t *b, long n) {
    long *ca = calloc(n + 1, sizeof(long)), *cb = calloc(n + 1, sizeof(long));
    int ok = 1;
    for (long i = 0; i < n && ok; i++) {
        if (b[i] < 1 || b[i] > n) ok = 0;
        else { ca[a[i]]++; cb[b[i]]++; }
    }
    for (long i = 0; i <= n && ok; i++) ok = ca[i] == cb[i];
    free(ca); free(cb);
    return ok;
}

static int trace(long n, uint32_t seed, int pattern) {
    int64_t *init = malloc(sizeof(int64_t) * n), *fin = malloc(sizeof(int64_t) * n);
    vt_generate(init, n, seed, pattern);
    vt_print_line("INIT", init, n);

    void *v = vt_alloc(n, vt_elem_size, vt_elem_signed);
    for (long i = 0; i < n; i++) store(v, i, init[i]);

    vt_start();
    vt_entry(v, n);
    vt_stop();
    vt_dump();

    for (long i = 0; i < n; i++) fin[i] = load(v, i);
    vt_print_line("FINAL", fin, n);
    vt_print_text(is_sorted(fin, n) && same_multiset(init, fin, n) ? "SORTED 1\n" : "SORTED 0\n");
    vt_print_text("END\n");
    return 0;
}

static int cmp_i64(const void *a, const void *b) {
    int64_t x = *(const int64_t *)a, y = *(const int64_t *)b;
    return (x > y) - (x < y);
}

static int bench(uint32_t seed, int pattern, int reps, int argc, char **argv) {
    for (int k = 0; k < argc; k++) {
        long n = atol(argv[k]);
        int64_t *init = malloc(sizeof(int64_t) * n), *fin = malloc(sizeof(int64_t) * n);
        int64_t *times = malloc(sizeof(int64_t) * reps);
        void *v = malloc((size_t)n * vt_elem_size);
        vt_generate(init, n, seed, pattern);
        int ok = 1;
        // aquecimento: uma rodada sem cronometrar (cache e frequência da CPU)
        for (long i = 0; i < n; i++) store(v, i, init[i]);
        vt_entry(v, n);
        for (int r = 0; r < reps; r++) {
            for (long i = 0; i < n; i++) store(v, i, init[i]);
            struct timespec t0, t1;
            clock_gettime(CLOCK_MONOTONIC, &t0);
            vt_entry(v, n);
            clock_gettime(CLOCK_MONOTONIC, &t1);
            times[r] = (int64_t)(t1.tv_sec - t0.tv_sec) * 1000000000LL + (t1.tv_nsec - t0.tv_nsec);
            for (long i = 0; i < n; i++) fin[i] = load(v, i);
            ok = ok && is_sorted(fin, n) && same_multiset(init, fin, n);
        }
        qsort(times, reps, sizeof(int64_t), cmp_i64);
        vt_print_text("BENCH ");
        vt_print_int(n);
        vt_print_text(" ");
        vt_print_int(times[reps / 2]);
        vt_print_text(" ");
        vt_print_int(times[0]);
        vt_print_text(ok ? " 1\n" : " 0\n");
        free(init); free(fin); free(times); free(v);
    }
    vt_print_text("END\n");
    return 0;
}

int main(int argc, char **argv) {
    if (argc >= 6 && strcmp(argv[1], "trace") == 0) {
        vt_init(atoi(argv[5]));
        return trace(atol(argv[2]), (uint32_t)strtoul(argv[3], NULL, 10), atoi(argv[4]));
    }
    if (argc >= 7 && strcmp(argv[1], "bench") == 0) {
        vt_init(atoi(argv[5]));
        return bench((uint32_t)strtoul(argv[2], NULL, 10), atoi(argv[3]), atoi(argv[4]), argc - 6, argv + 6);
    }
    fprintf(stderr, "uso: trace <n> <seed> <padrao> <timeout> | bench <seed> <padrao> <reps> <timeout> <n...>\n");
    return 2;
}
