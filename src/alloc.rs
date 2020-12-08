use std::{
    alloc::{GlobalAlloc, System},
    cell::UnsafeCell,
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

pub struct TracingAlloc {
    inner: System,
    active: AtomicBool,
    file_access_sync: AtomicBool,
    output_file: UnsafeCell<Option<File>>,
    timestamp: UnsafeCell<Option<Instant>>,
}

unsafe impl Sync for TracingAlloc {}

impl TracingAlloc {
    pub const fn new() -> Self {
        Self {
            inner: System,
            active: AtomicBool::new(false),
            file_access_sync: AtomicBool::new(false),
            output_file: UnsafeCell::new(None),
            timestamp: UnsafeCell::new(None),
        }
    }

    pub fn enable_tracing(&self) {
        // Wait until the file is no longer being changed.
        while self
            .file_access_sync
            .compare_and_swap(false, true, Ordering::Relaxed)
            != false
        {}
        std::sync::atomic::fence(Ordering::Acquire);

        unsafe {
            self.timestamp.get().write(Some(Instant::now()));
        }

        self.file_access_sync.store(false, Ordering::Release);
        self.write_ev(Event::Start);

        self.active.store(true, Ordering::SeqCst);
    }

    pub fn disable_tracing(&self) {
        self.write_ev(Event::End);
        self.active.store(false, Ordering::SeqCst);
    }

    pub fn set_file(&self, file: File) -> Option<File> {
        // Wait until the file is no longer being changed.
        while self
            .file_access_sync
            .compare_and_swap(false, true, Ordering::Relaxed)
            != false
        {}
        std::sync::atomic::fence(Ordering::Acquire);

        let old_file = unsafe {
            let fd_ptr = self.output_file.get();
            fd_ptr.replace(Some(file))
        };

        self.file_access_sync.store(false, Ordering::Release);
        old_file
    }

    pub fn clear_file(&self) -> Option<File> {
        // Wait until the file is no longer being changed.
        while self
            .file_access_sync
            .compare_and_swap(false, true, Ordering::Relaxed)
            != false
        {}
        std::sync::atomic::fence(Ordering::Acquire);

        let old_file = unsafe {
            let fd_ptr = self.output_file.get();
            fd_ptr.replace(None)
        };

        self.file_access_sync.store(false, Ordering::Release);
        old_file
    }

    fn write_ev(&self, ev: Event) {
        // Wait until the file is no longer being changed.
        while self
            .file_access_sync
            .compare_and_swap(false, true, Ordering::Relaxed)
            != false
        {}
        std::sync::atomic::fence(Ordering::Acquire);

        unsafe {
            if let (Some(file), Some(ts)) = (&*self.output_file.get(), *self.timestamp.get()) {
                let elapsed = ts.elapsed();
                let (symbol, size) = match &ev {
                    Event::Alloc { size, .. } => ('A', *size),
                    Event::Free { size, .. } => ('F', *size),
                    Event::Start => ('S', 0),
                    Event::End => ('E', 0),
                };

                writeln!(&*file, "{} {} {}", symbol, elapsed.as_nanos(), size).unwrap();
            }
        }

        self.file_access_sync.store(false, Ordering::Release);
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
