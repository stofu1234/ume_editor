//! Memory breakdown for the PoC (Windows only).
//!
//! `POC_MEMSTAT=<seconds>` writes a report to `memstat-<pid>.txt` after that
//! many seconds. The report splits the working set and committed memory by
//! region type (private / mapped file / image) and lists the largest modules,
//! mapped files, and private allocations, next to the Rust heap size counted
//! by a wrapping global allocator.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

pub struct CountingAlloc;

static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(layout) };
        if !p.is_null() {
            let live = LIVE.fetch_add(layout.size(), Ordering::Relaxed) + layout.size();
            PEAK.fetch_max(live, Ordering::Relaxed);
        }
        p
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) };
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let p = unsafe { System.realloc(ptr, layout, new_size) };
        if !p.is_null() {
            let live = LIVE.fetch_add(new_size, Ordering::Relaxed) + new_size;
            LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
            PEAK.fetch_max(live - layout.size(), Ordering::Relaxed);
        }
        p
    }
}

pub fn start_from_env() {
    let Some(secs) = std::env::var("POC_MEMSTAT")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
    else {
        return;
    };
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(secs));
        let path = format!("memstat-{}.txt", std::process::id());
        let _ = std::fs::write(&path, report());
    });
}

const MB: f64 = 1024.0 * 1024.0;

fn mb(bytes: usize) -> String {
    format!("{:8.1} MB", bytes as f64 / MB)
}

#[cfg(not(windows))]
pub fn report() -> String {
    format!(
        "rust heap live {} peak {}\n",
        mb(LIVE.load(Ordering::Relaxed)),
        mb(PEAK.load(Ordering::Relaxed))
    )
}

#[cfg(windows)]
pub fn report() -> String {
    use std::collections::HashMap;
    use std::fmt::Write as _;
    use windows_sys::Win32::System::Memory::{
        GetProcessHeaps, HEAP_SUMMARY, HeapSummary, MEM_COMMIT, MEM_IMAGE, MEM_MAPPED, MEM_PRIVATE,
        MEMORY_BASIC_INFORMATION, VirtualQuery,
    };
    use windows_sys::Win32::System::ProcessStatus::{
        GetMappedFileNameW, K32GetProcessMemoryInfo, K32QueryWorkingSet, PROCESS_MEMORY_COUNTERS_EX,
    };
    use windows_sys::Win32::System::Threading::GetCurrentProcess;

    const PAGE: usize = 4096;
    let process = unsafe { GetCurrentProcess() };
    let mut out = String::new();

    let mut pmc: PROCESS_MEMORY_COUNTERS_EX = unsafe { std::mem::zeroed() };
    pmc.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32;
    unsafe {
        K32GetProcessMemoryInfo(process, (&raw mut pmc).cast(), pmc.cb);
    }
    let _ = writeln!(out, "working set      {}", mb(pmc.WorkingSetSize));
    let _ = writeln!(out, "private usage    {}", mb(pmc.PrivateUsage));
    let _ = writeln!(
        out,
        "rust heap live   {}  (peak {})",
        mb(LIVE.load(Ordering::Relaxed)),
        mb(PEAK.load(Ordering::Relaxed))
    );

    // Win32 heaps (the Rust System allocator uses the process heap).
    let mut heaps = vec![std::ptr::null_mut(); 256];
    let n = unsafe { GetProcessHeaps(heaps.len() as u32, heaps.as_mut_ptr()) } as usize;
    let mut heap_commit = 0;
    let mut heap_alloc = 0;
    for &h in &heaps[..n.min(heaps.len())] {
        let mut s = HEAP_SUMMARY {
            cb: std::mem::size_of::<HEAP_SUMMARY>() as u32,
            ..Default::default()
        };
        if unsafe { HeapSummary(h, 0, &mut s) } != 0 {
            heap_commit += s.cbCommitted;
            heap_alloc += s.cbAllocated;
        }
    }
    let _ = writeln!(
        out,
        "win32 heaps      {}  committed, {} allocated, {n} heaps",
        mb(heap_commit),
        mb(heap_alloc)
    );

    // Walk the address space.
    struct Region {
        base: usize,
        end: usize,
        alloc_base: usize,
        kind: u32,
        committed: bool,
    }
    let mut regions = Vec::new();
    let mut addr = 0usize;
    loop {
        let mut mbi: MEMORY_BASIC_INFORMATION = unsafe { std::mem::zeroed() };
        let got = unsafe {
            VirtualQuery(
                addr as *const _,
                &mut mbi,
                std::mem::size_of::<MEMORY_BASIC_INFORMATION>(),
            )
        };
        if got == 0 {
            break;
        }
        let base = mbi.BaseAddress as usize;
        let end = base + mbi.RegionSize;
        regions.push(Region {
            base,
            end,
            alloc_base: mbi.AllocationBase as usize,
            kind: mbi.Type,
            committed: mbi.State == MEM_COMMIT,
        });
        if end <= addr {
            break;
        }
        addr = end;
    }

    // Working set pages.
    let mut ws_pages: Vec<(usize, bool)> = Vec::new();
    let mut cap = 1 << 18;
    loop {
        let mut buf = vec![0usize; cap + 1];
        let ok = unsafe {
            K32QueryWorkingSet(
                process,
                buf.as_mut_ptr().cast(),
                (buf.len() * std::mem::size_of::<usize>()) as u32,
            )
        };
        let count = buf[0];
        if ok != 0 {
            ws_pages = buf[1..=count.min(cap)]
                .iter()
                .map(|&f| (f & !0xfff, f & 0x100 != 0))
                .collect();
            break;
        }
        if count <= cap || cap > 1 << 24 {
            break;
        }
        cap = count + 4096;
    }
    ws_pages.sort_unstable();

    let mapped_name = |base: usize| -> String {
        let mut name = [0u16; 512];
        let len = unsafe {
            GetMappedFileNameW(
                process,
                base as *const _,
                name.as_mut_ptr(),
                name.len() as u32,
            )
        } as usize;
        let s = String::from_utf16_lossy(&name[..len]);
        match s.rsplit('\\').next() {
            Some(file) if !file.is_empty() => file.to_string(),
            _ => "(anonymous)".to_string(),
        }
    };

    #[derive(Default)]
    struct Sum {
        commit: usize,
        ws: usize,
        ws_shared: usize,
    }
    let mut by_kind: HashMap<&str, Sum> = HashMap::new();
    let mut by_name: HashMap<(&str, String), Sum> = HashMap::new();
    let mut by_alloc: HashMap<usize, Sum> = HashMap::new();
    let mut names: HashMap<usize, String> = HashMap::new();
    let mut pi = 0;
    for r in &regions {
        while pi < ws_pages.len() && ws_pages[pi].0 < r.base {
            pi += 1;
        }
        let mut ws = 0;
        let mut ws_shared = 0;
        while pi < ws_pages.len() && ws_pages[pi].0 < r.end {
            ws += PAGE;
            if ws_pages[pi].1 {
                ws_shared += PAGE;
            }
            pi += 1;
        }
        let kind = match r.kind {
            MEM_PRIVATE => "private",
            MEM_MAPPED => "mapped",
            MEM_IMAGE => "image",
            _ => continue,
        };
        let commit = if r.committed { r.end - r.base } else { 0 };
        let add = |s: &mut Sum| {
            s.commit += commit;
            s.ws += ws;
            s.ws_shared += ws_shared;
        };
        add(by_kind.entry(kind).or_default());
        if kind == "private" {
            add(by_alloc.entry(r.alloc_base).or_default());
        } else {
            let name = names
                .entry(r.alloc_base)
                .or_insert_with(|| mapped_name(r.alloc_base))
                .clone();
            add(by_name.entry((kind, name)).or_default());
        }
    }

    let _ = writeln!(
        out,
        "\n{:<44} {:>11} {:>11} {:>11}",
        "", "committed", "ws", "ws shared"
    );
    for kind in ["private", "mapped", "image"] {
        if let Some(s) = by_kind.get(kind) {
            let _ = writeln!(
                out,
                "{kind:<44} {} {} {}",
                mb(s.commit),
                mb(s.ws),
                mb(s.ws_shared)
            );
        }
    }

    for kind in ["image", "mapped"] {
        let mut rows: Vec<_> = by_name.iter().filter(|((k, _), _)| *k == kind).collect();
        rows.sort_by_key(|(_, s)| std::cmp::Reverse(s.ws));
        let _ = writeln!(out, "\ntop {kind} by working set:");
        for ((_, name), s) in rows.iter().take(25) {
            let _ = writeln!(
                out,
                "  {name:<42} {} {} {}",
                mb(s.commit),
                mb(s.ws),
                mb(s.ws_shared)
            );
        }
    }

    let mut hist: HashMap<usize, (usize, usize)> = HashMap::new();
    for s in by_alloc.values() {
        let e = hist.entry(s.commit.div_ceil(1 << 20)).or_default();
        e.0 += 1;
        e.1 += s.ws;
    }
    let mut hist: Vec<_> = hist.into_iter().collect();
    hist.sort_by_key(|(_, (_, ws))| std::cmp::Reverse(*ws));
    let _ = writeln!(
        out,
        "\nprivate allocations by committed size (rounded up to MB):"
    );
    for (size, (n, ws)) in hist.iter().take(12) {
        let _ = writeln!(out, "  {size:>4} MB x {n:<5} ws {}", mb(*ws));
    }

    let mut rows: Vec<_> = by_alloc.iter().collect();
    rows.sort_by_key(|(_, s)| std::cmp::Reverse(s.commit));
    let _ = writeln!(out, "\ntop private allocations by committed size:");
    for (base, s) in rows.iter().take(8) {
        let _ = writeln!(
            out,
            "  {:#018x}{:26} {} {} {}",
            base,
            "",
            mb(s.commit),
            mb(s.ws),
            mb(s.ws_shared)
        );
    }
    out
}
