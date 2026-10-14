// Rastreador de acessos ao vetor, usado pelos runners de C e de Rust.
//
// Como funciona: o vetor mora numa região própria de memória (mmap), com uma
// página de guarda antes e outra depois. Durante o sort a região inteira fica
// sem permissão (PROT_NONE). Cada leitura ou escrita no vetor gera um SIGSEGV;
// o handler anota o endereço, libera a região e liga o Trap Flag (x86_64),
// para que a CPU execute só aquela instrução e pare de novo com SIGTRAP. No
// SIGTRAP a região é comparada com uma cópia (shadow) para descobrir o que foi
// escrito, e volta a ficar protegida.
//
// Assim o código do sort não precisa de nenhuma alteração: ele mexe no vetor
// normalmente e cada acesso vira um evento. Acessos nas páginas de guarda viram
// eventos com índice fora do vetor (negativo ou >= n) em vez de corromper algo.

#define _GNU_SOURCE
#include <signal.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <ucontext.h>
#include <unistd.h>

#define VT_MAX_EVENTS 4000000

typedef struct {
    int64_t idx;
    int64_t val;
    uint8_t kind; // 'R' ou 'W'
} vt_event;

static unsigned char *reg;      // região inteira (guarda + dados + guarda)
static size_t reg_len;
static unsigned char *base;     // início do vetor
static unsigned char *shadow;   // cópia da região para detectar escritas
static size_t esz;              // tamanho de um elemento em bytes
static int esigned;
static long n_el;

static vt_event *evs;
static size_t nev;
static volatile sig_atomic_t tracing, truncated;

static unsigned char *pend_addr;
static int pend_write, pend_on;

static struct sigaction old_segv, old_trap, old_bus;
static int handlers_on;

// ---------- saída (só write(2), para poder ser usada dentro de handler) ----------

static char obuf[1 << 16];
static size_t olen;

static void out_flush(void) {
    size_t off = 0;
    while (off < olen) {
        ssize_t w = write(1, obuf + off, olen - off);
        if (w <= 0) break;
        off += (size_t)w;
    }
    olen = 0;
}

static void out_str(const char *s) {
    while (*s) {
        if (olen == sizeof obuf) out_flush();
        obuf[olen++] = *s++;
    }
}

static void out_int(int64_t v) {
    char tmp[24];
    int i = 0;
    uint64_t u = v < 0 ? (uint64_t)(-(v + 1)) + 1 : (uint64_t)v;
    do { tmp[i++] = (char)('0' + u % 10); u /= 10; } while (u);
    if (v < 0) tmp[i++] = '-';
    while (i) {
        if (olen == sizeof obuf) out_flush();
        obuf[olen++] = tmp[--i];
    }
}

// ---------- leitura de elementos de qualquer tamanho ----------

static int64_t load_el(const unsigned char *p) {
    switch (esz) {
    case 1: return esigned ? (int64_t)*(const int8_t *)p : (int64_t)*(const uint8_t *)p;
    case 2: { int16_t s; uint16_t u; memcpy(&s, p, 2); memcpy(&u, p, 2); return esigned ? s : u; }
    case 4: { int32_t s; uint32_t u; memcpy(&s, p, 4); memcpy(&u, p, 4); return esigned ? s : u; }
    default: { int64_t s; memcpy(&s, p, 8); return s; }
    }
}

static int64_t index_of(const unsigned char *p) {
    long off = (long)(p - base);
    long e = (long)esz;
    return off >= 0 ? off / e : -((-off + e - 1) / e);
}

static void push(uint8_t kind, int64_t idx, int64_t val) {
    if (nev >= VT_MAX_EVENTS) { truncated = 1; return; }
    evs[nev].kind = kind;
    evs[nev].idx = idx;
    evs[nev].val = val;
    nev++;
}

// ---------- handlers ----------

void vt_dump(void);
static void restore_handlers(void);

static void die_with_dump(const char *why, int code) {
    tracing = 0;
    if (reg) mprotect(reg, reg_len, PROT_READ | PROT_WRITE);
    vt_dump();
    out_str("CRASH ");
    out_str(why);
    out_str("\n");
    out_flush();
    _exit(code);
}

static void on_segv(int sig, siginfo_t *si, void *ctx) {
    ucontext_t *uc = (ucontext_t *)ctx;
    unsigned char *a = (unsigned char *)si->si_addr;
    if (!tracing || a < reg || a >= reg + reg_len) {
        die_with_dump(sig == SIGBUS ? "SIGBUS (acesso inválido à memória)"
                                    : "SIGSEGV (acesso inválido à memória)", 4);
    }
    pend_addr = a;
    pend_write = (uc->uc_mcontext.gregs[REG_ERR] & 2) != 0;
    pend_on = 1;
    mprotect(reg, reg_len, PROT_READ | PROT_WRITE);
    uc->uc_mcontext.gregs[REG_EFL] |= 0x100; // Trap Flag: para depois de 1 instrução
}

static void on_trap(int sig, siginfo_t *si, void *ctx) {
    (void)sig; (void)si;
    ucontext_t *uc = (ucontext_t *)ctx;
    uc->uc_mcontext.gregs[REG_EFL] &= ~(greg_t)0x100;
    if (!pend_on) return;
    pend_on = 0;

    if (!pend_write) push('R', index_of(pend_addr), load_el(pend_addr));

    // descobre tudo o que mudou na região desde o último passo
    int wrote = 0;
    for (size_t off = 0; off < reg_len; off += 64) {
        size_t len = reg_len - off < 64 ? reg_len - off : 64;
        if (memcmp(reg + off, shadow + off, len) == 0) continue;
        for (size_t b = off; b < off + len; b++) {
            if (reg[b] == shadow[b]) continue;
            // alinha no começo do elemento que contém esse byte
            unsigned char *p = reg + b;
            long rel = (long)(p - base);
            long e = (long)esz;
            long start = rel >= 0 ? rel - rel % e : rel - ((rel % e) + e) % e;
            unsigned char *ep = base + start;
            if (ep < reg) ep = reg;
            if (ep + esz > reg + reg_len) ep = reg + reg_len - esz;
            push('W', index_of(ep), load_el(ep));
            memcpy(shadow + (ep - reg), ep, esz);
            wrote = 1;
            b = (size_t)(ep - reg) + esz - 1;
        }
    }
    if (pend_write && !wrote) {
        // escreveu o mesmo valor que já estava lá: ainda conta como escrita
        unsigned char *p = base + index_of(pend_addr) * (long)esz;
        if (p >= reg && p + esz <= reg + reg_len) push('W', index_of(p), load_el(p));
    }

    if (truncated) {
        tracing = 0; // estourou o limite: o sort continua, mas sem rastrear
        restore_handlers();
        return;
    }
    mprotect(reg, reg_len, PROT_NONE);
}

static void on_alarm(int sig) {
    (void)sig;
    die_with_dump("TIMEOUT", 3);
}

static void on_fatal(int sig) {
    const char *why = sig == SIGFPE  ? "SIGFPE (divisão por zero?)"
                    : sig == SIGABRT ? "SIGABRT (abort)"
                    : sig == SIGILL  ? "SIGILL"
                                     : "sinal fatal";
    die_with_dump(why, 4);
}

static void install_handlers(void) {
    struct sigaction sa;
    memset(&sa, 0, sizeof sa);
    sa.sa_sigaction = on_segv;
    sa.sa_flags = SA_SIGINFO | SA_ONSTACK | SA_NODEFER;
    sigemptyset(&sa.sa_mask);
    sigaction(SIGSEGV, &sa, &old_segv);
    sigaction(SIGBUS, &sa, &old_bus);
    sa.sa_sigaction = on_trap;
    sigaction(SIGTRAP, &sa, &old_trap);
    handlers_on = 1;
}

static void restore_handlers(void) {
    if (!handlers_on) return;
    sigaction(SIGSEGV, &old_segv, NULL);
    sigaction(SIGBUS, &old_bus, NULL);
    sigaction(SIGTRAP, &old_trap, NULL);
    handlers_on = 0;
}

// ---------- API usada pelos runners ----------

// Prepara pilha alternativa (recursão infinita também vira relatório) e os
// handlers de timeout/crash. Vale para os dois modos, trace e bench.
void vt_init(int timeout_s) {
    static char *alt;
    if (!alt) {
        size_t sz = 1 << 16;
        alt = malloc(sz);
        stack_t st = {.ss_sp = alt, .ss_size = sz, .ss_flags = 0};
        sigaltstack(&st, NULL);
    }
    struct sigaction sa;
    memset(&sa, 0, sizeof sa);
    sa.sa_flags = SA_ONSTACK;
    sigemptyset(&sa.sa_mask);
    sa.sa_handler = on_alarm;
    sigaction(SIGALRM, &sa, NULL);
    sa.sa_handler = on_fatal;
    sigaction(SIGFPE, &sa, NULL);
    sigaction(SIGABRT, &sa, NULL);
    sigaction(SIGILL, &sa, NULL);
    // SIGSEGV fora do rastreamento também gera relatório
    sa.sa_flags = SA_SIGINFO | SA_ONSTACK | SA_NODEFER;
    sa.sa_sigaction = on_segv;
    sigaction(SIGSEGV, &sa, NULL);
    sigaction(SIGBUS, &sa, NULL);
    if (timeout_s > 0) alarm((unsigned)timeout_s);
}

void *vt_alloc(long n, long elem_size, int is_signed) {
    long page = sysconf(_SC_PAGESIZE);
    esz = (size_t)elem_size;
    esigned = is_signed;
    n_el = n;
    long data = n * elem_size;
    long data_pages = (data + page - 1) / page;
    if (data_pages < 1) data_pages = 1;
    reg_len = (size_t)((data_pages + 2) * page);
    reg = mmap(NULL, reg_len, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    if (reg == MAP_FAILED) _exit(5);
    shadow = malloc(reg_len);
    evs = malloc(sizeof(vt_event) * VT_MAX_EVENTS);
    if (!shadow || !evs) _exit(5);
    base = reg + page;
    return base;
}

int vt_owns(const void *p) {
    const unsigned char *c = (const unsigned char *)p;
    return reg && c >= reg && c < reg + reg_len;
}

void vt_start(void) {
    memcpy(shadow, reg, reg_len);
    nev = 0;
    truncated = 0;
    install_handlers();
    tracing = 1;
    mprotect(reg, reg_len, PROT_NONE);
}

void vt_stop(void) {
    tracing = 0;
    if (reg) mprotect(reg, reg_len, PROT_READ | PROT_WRITE);
    restore_handlers();
    vt_init(0); // handlers de crash de volta
}

void vt_dump(void) {
    for (size_t i = 0; i < nev; i++) {
        out_str(evs[i].kind == 'R' ? "R " : "W ");
        out_int(evs[i].idx);
        out_str(" ");
        out_int(evs[i].val);
        out_str("\n");
    }
    if (truncated) out_str("TRUNC\n");
    nev = 0;
    out_flush();
}

// ---------- utilitários compartilhados (mesma sequência nas duas linguagens) ----------

static uint32_t rng;

static uint32_t next_rand(void) {
    rng ^= rng << 13;
    rng ^= rng >> 17;
    rng ^= rng << 5;
    return rng;
}

// padrões: 0 aleatório, 1 crescente, 2 decrescente, 3 quase ordenado, 4 poucos valores
void vt_generate(int64_t *out, long n, uint32_t seed, int pattern) {
    rng = seed ? seed : 0x9E3779B9u;
    for (long i = 0; i < n; i++) out[i] = i + 1;
    if (pattern == 0) {
        for (long i = n - 1; i > 0; i--) {
            long j = (long)(next_rand() % (uint32_t)(i + 1));
            int64_t t = out[i]; out[i] = out[j]; out[j] = t;
        }
    } else if (pattern == 2) {
        for (long i = 0; i < n; i++) out[i] = n - i;
    } else if (pattern == 3) {
        long swaps = n / 10 > 0 ? n / 10 : 1;
        for (long k = 0; k < swaps; k++) {
            long i = (long)(next_rand() % (uint32_t)n);
            long j = (long)(next_rand() % (uint32_t)n);
            int64_t t = out[i]; out[i] = out[j]; out[j] = t;
        }
    } else if (pattern == 4) {
        for (long i = 0; i < n; i++) out[i] = 1 + (int64_t)(next_rand() % 5u) * (n / 5 > 0 ? n / 5 : 1);
    }
}

void vt_print_line(const char *tag, const int64_t *v, long n) {
    out_str(tag);
    for (long i = 0; i < n; i++) { out_str(" "); out_int(v[i]); }
    out_str("\n");
    out_flush();
}

void vt_print_text(const char *s) {
    out_str(s);
    out_flush();
}

void vt_print_int(int64_t v) {
    out_int(v);
    out_flush();
}
