use std::{
    alloc::{GlobalAlloc, System},
    cell::UnsafeCell,
    fs::File,
    io::Write,
    sync::atomic::{AtomicBool, Ordering},
};

pub enum Event {
    Alloc { addr: usize, size: usize },
    Free { addr: usize, size: usize },
}

pub struct TracingAlloc {
    inner: System,
    active: AtomicBool,
    file_access_sync: AtomicBool,
    output_file: UnsafeCell<Option<File>>,
}

unsafe impl Sync for TracingAlloc {}

impl TracingAlloc {
    pub const fn new() -> Self {
        Self {
            inner: System,
            active: AtomicBool::new(false),
            file_access_sync: AtomicBool::new(false),
            output_file: UnsafeCell::new(None),
        }
    }

    pub fn enable_tracing(&self) {
        self.active.store(true, Ordering::SeqCst);
    }

    pub fn disable_tracing(&self) {
        self.active.store(false, Ordering::SeqCst);
    }

    pub fn set_file(&self, file: File) -> Option<File> {
        // Wait until the file is no longer being changed.
        while self.file_access_sync.load(Ordering::SeqCst) {}
        self.file_access_sync.store(true, Ordering::SeqCst);

        let old_file = unsafe {
            let fd_ptr = self.output_file.get();
            fd_ptr.replace(Some(file))
        };

        self.file_access_sync.store(false, Ordering::SeqCst);
        old_file
    }

    pub fn clear_file(&self) -> Option<File> {
        // Wait until the file is no longer being changed.
        while self.file_access_sync.load(Ordering::SeqCst) {}
        self.file_access_sync.store(true, Ordering::SeqCst);

        let old_file = unsafe {
            let fd_ptr = self.output_file.get();
            fd_ptr.replace(None)
        };

        self.file_access_sync.store(false, Ordering::SeqCst);
        old_file
    }

    fn write_ev(&self, ev: Event) {
        // Wait until the file is no longer being changed.
        while self.file_access_sync.load(Ordering::SeqCst) {}
        self.file_access_sync.store(true, Ordering::SeqCst);

        unsafe {
            if let Some(file) = &*self.output_file.get() {
                let (symbol, size) = match &ev {
                    Event::Alloc { size, .. } => ('A', size),
                    Event::Free { size, .. } => ('F', size),
                };

                writeln!(&*file, "{} {}", symbol, size).unwrap();
            }
        }

        self.file_access_sync.store(false, Ordering::SeqCst);
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
