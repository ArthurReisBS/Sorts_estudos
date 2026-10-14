// Runner do lado Rust. O visualizador troca as marcas __VT_*__ e compila este
// arquivo com rustc, ligado ao rastreador em C (libvt.a). O seu sort de
// ../Rust/ entra inteiro dentro do módulo `sorts` via include!.
//
//   ./runner trace <n> <seed> <padrao> <timeout_s>
//   ./runner bench <seed> <padrao> <reps> <timeout_s> <n1> [n2 ...]

#![allow(warnings)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::time::Instant;

type Elem = __VT_ELEM__;

#[allow(warnings)]
mod sorts {
    include!("__VT_SORT_PATH__");

    pub fn __vt_entry(v: &mut Vec<super::Elem>) {
        let n = v.len();
        __VT_CALL__;
    }
}

unsafe extern "C" {
    fn vt_init(timeout_s: i32);
    fn vt_alloc(n: i64, elem_size: i64, is_signed: i32) -> *mut u8;
    fn vt_owns(p: *const u8) -> i32;
    fn vt_start();
    fn vt_stop();
    fn vt_dump();
    fn vt_generate(out: *mut i64, n: i64, seed: u32, pattern: i32);
    fn vt_print_line(tag: *const u8, v: *const i64, n: i64);
    fn vt_print_text(s: *const u8);
    fn vt_print_int(v: i64);
}

// O vetor rastreado mora numa região do rastreador, não do alocador. Se o sort
// receber &mut Vec e realocar ou trocar o vetor, o alocador não pode tentar
// liberar essa região.
struct VtAlloc;

unsafe impl GlobalAlloc for VtAlloc {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        System.alloc(l)
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        if vt_owns(p) == 0 {
            System.dealloc(p, l)
        }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, new: usize) -> *mut u8 {
        if vt_owns(p) == 0 {
            return System.realloc(p, l, new);
        }
        let np = System.alloc(Layout::from_size_align_unchecked(new, l.align()));
        if !np.is_null() {
            std::ptr::copy_nonoverlapping(p, np, l.size().min(new));
        }
        np
    }
}

#[global_allocator]
static ALLOC: VtAlloc = VtAlloc;

fn text(s: &str) {
    let mut b = s.as_bytes().to_vec();
    b.push(0);
    unsafe { vt_print_text(b.as_ptr()) }
}

fn line(tag: &str, v: &[i64]) {
    let mut t = tag.as_bytes().to_vec();
    t.push(0);
    unsafe { vt_print_line(t.as_ptr(), v.as_ptr(), v.len() as i64) }
}

fn generate(n: usize, seed: u32, pattern: i32) -> Vec<i64> {
    let mut v = vec![0i64; n];
    unsafe { vt_generate(v.as_mut_ptr(), n as i64, seed, pattern) };
    v
}

fn is_signed() -> bool {
    (0 as Elem).checked_sub(1 as Elem).is_some()
}

fn ok(init: &[i64], fin: &[i64]) -> bool {
    if init.len() != fin.len() || fin.windows(2).any(|w| w[0] > w[1]) {
        return false;
    }
    let (mut a, mut b) = (init.to_vec(), fin.to_vec());
    a.sort_unstable();
    b.sort_unstable();
    a == b
}

fn panic_msg(e: Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = e.downcast_ref::<&str>() {
        s.to_string()
    } else if let Some(s) = e.downcast_ref::<String>() {
        s.clone()
    } else {
        "panic".into()
    }
}

fn trace(n: usize, seed: u32, pattern: i32) {
    let init = generate(n, seed, pattern);
    line("INIT", &init);

    let size = std::mem::size_of::<Elem>();
    let p = unsafe { vt_alloc(n as i64, size as i64, is_signed() as i32) } as *mut Elem;
    for (i, x) in init.iter().enumerate() {
        unsafe { p.add(i).write(*x as Elem) };
    }
    let mut v = unsafe { Vec::from_raw_parts(p, n, n) };

    unsafe { vt_start() };
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| sorts::__vt_entry(&mut v)));
    unsafe {
        vt_stop();
        vt_dump();
    }

    let fin: Vec<i64> = v.iter().map(|&x| x as i64).collect();
    if v.as_ptr() == p as *const Elem {
        std::mem::forget(v);
    }
    if let Err(e) = r {
        text(&format!("PANIC {}\n", panic_msg(e).replace('\n', " ")));
    }
    line("FINAL", &fin);
    text(if ok(&init, &fin) { "SORTED 1\n" } else { "SORTED 0\n" });
    text("END\n");
}

fn bench(seed: u32, pattern: i32, reps: usize, sizes: &[usize]) {
    for &n in sizes {
        let init = generate(n, seed, pattern);
        let mut times = Vec::with_capacity(reps);
        let mut good = true;
        // aquecimento: uma rodada sem cronometrar (cache e frequência da CPU)
        let mut v: Vec<Elem> = init.iter().map(|&x| x as Elem).collect();
        if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| sorts::__vt_entry(&mut v))).is_err() {
            text("PANIC durante o aquecimento\n");
            text("END\n");
            return;
        }
        for _ in 0..reps {
            let mut v: Vec<Elem> = init.iter().map(|&x| x as Elem).collect();
            let t = Instant::now();
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| sorts::__vt_entry(&mut v)));
            let dt = t.elapsed().as_nanos() as i64;
            if let Err(e) = r {
                text(&format!("PANIC {}\n", panic_msg(e).replace('\n', " ")));
                text("END\n");
                return;
            }
            times.push(dt);
            let fin: Vec<i64> = v.iter().map(|&x| x as i64).collect();
            good = good && ok(&init, &fin);
        }
        times.sort_unstable();
        text("BENCH ");
        unsafe {
            vt_print_int(n as i64);
            vt_print_text(b" \0".as_ptr());
            vt_print_int(times[reps / 2]);
            vt_print_text(b" \0".as_ptr());
            vt_print_int(times[0]);
        }
        text(if good { " 1\n" } else { " 0\n" });
    }
    text("END\n");
}

fn main() {
    // o aviso padrão de panic vai para o stderr; o GUI mostra a mensagem
    let a: Vec<String> = std::env::args().collect();
    let num = |i: usize| a[i].parse::<i64>().unwrap_or(0);
    if a.len() >= 6 && a[1] == "trace" {
        unsafe { vt_init(num(5) as i32) };
        trace(num(2) as usize, num(3) as u32, num(4) as i32);
    } else if a.len() >= 7 && a[1] == "bench" {
        unsafe { vt_init(num(5) as i32) };
        let sizes: Vec<usize> = a[6..].iter().filter_map(|s| s.parse().ok()).collect();
        bench(num(2) as u32, num(3) as i32, num(4).max(1) as usize, &sizes);
    } else {
        eprintln!("uso: trace <n> <seed> <padrao> <timeout> | bench <seed> <padrao> <reps> <timeout> <n...>");
        std::process::exit(2);
    }
}
