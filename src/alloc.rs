use once_cell::sync::Lazy;
use std::sync::RwLock;
use std::{
    alloc::{GlobalAlloc, System},
    fs::File,
    io::Write,
    sync::atomic::{AtomicBool, Ordering},
    time::Instant,
};

pub enum Event {
    Alloc { addr: usize, size: usize },
    Free { addr: usize, size: usize },
    Start,
    End,
}

struct TraceData {
    output_file: Option<File>,
    timestamp: Option<Instant>,
}

static OUTPUT_FILE: Lazy<RwLock<TraceData>> = Lazy::new(|| {
    RwLock::new(TraceData {
        output_file: None,
        timestamp: None,
    })
});

pub struct TracingAlloc {
    inner: System,
    active: AtomicBool,
}

unsafe impl Sync for TracingAlloc {}

impl TracingAlloc {
    pub const fn new() -> Self {
        Self {
            inner: System,
            active: AtomicBool::new(false),
        }
    }

    pub fn enable_tracing(&self) {
        let mut lock = OUTPUT_FILE.write().unwrap();
        lock.timestamp = Some(Instant::now());
        std::mem::drop(lock); // Must drop the lock before writing the start event.

        self.write_ev(Event::Start);
        self.active.store(true, Ordering::SeqCst);
    }

    pub fn disable_tracing(&self) {
        self.write_ev(Event::End);
        self.active.store(false, Ordering::SeqCst);
    }

    pub fn set_file(&self, file: File) -> Option<File> {
        let mut lock = OUTPUT_FILE.write().unwrap();
        lock.output_file.replace(file)
    }

    pub fn clear_file(&self) -> Option<File> {
        let mut lock = OUTPUT_FILE.write().unwrap();
        lock.output_file.take()
    }

    fn write_ev(&self, ev: Event) {
        // Read is fine here as File is Sync.
        let lock = OUTPUT_FILE.read().unwrap();

        if let (Some(file), Some(ts)) = (lock.output_file.as_ref(), lock.timestamp) {
            let elapsed = ts.elapsed();
            let (symbol, size) = match &ev {
                Event::Alloc { size, .. } => ('A', *size),
                Event::Free { size, .. } => ('F', *size),
                Event::Start => ('S', 0),
                Event::End => ('E', 0),
            };

            // Just eat the error so we don't get a panic during allocation.
            let _ = writeln!(&*file, "{} {} {}", symbol, elapsed.as_nanos(), size);
        }
    }
}

unsafe impl GlobalAlloc for TracingAlloc {
    unsafe fn alloc(&self, layout: std::alloc::Layout) -> *mut u8 {
        let res = self.inner.alloc(layout);

        if self.active.load(Ordering::SeqCst) {
            self.write_ev(Event::Alloc {
                addr: res as _,
                size: layout.size(),
            });
        }

        res
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: std::alloc::Layout) {
        if self.active.load(Ordering::SeqCst) {
            self.write_ev(Event::Free {
                addr: ptr as _,
                size: layout.size(),
            });
        }

        self.inner.dealloc(ptr, layout)
    }
}
