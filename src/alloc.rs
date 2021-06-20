use std::{
    alloc::{GlobalAlloc, System},
    cell::{Cell, RefCell},
    fs::File,
    io::{BufWriter, Write},
    time::Instant,
};

pub enum Event {
    Alloc { addr: usize, size: usize },
    Free { addr: usize, size: usize },
    Start,
    End,
}

struct TraceData {
    output_file: Option<BufWriter<File>>,
    timestamp: Option<Instant>,
}

thread_local! {
    static OUTPUT_FILE: RefCell<TraceData> = RefCell::new(TraceData {
        output_file: None,
        timestamp: None,
    });

    static ACTIVE: Cell<bool> = Cell::new(false);
}

pub struct TracingAlloc;

unsafe impl Sync for TracingAlloc {}

impl TracingAlloc {
    pub fn enable_tracing(&self) {
        OUTPUT_FILE.with(|output_file| {
            let mut lock = output_file.borrow_mut();
            lock.timestamp = Some(Instant::now());
        });

        self.write_ev(Event::Start);
        ACTIVE.with(|active| active.set(true));
    }

    pub fn disable_tracing(&self) {
        self.write_ev(Event::End);
        ACTIVE.with(|active| active.set(false));
    }

    pub fn set_file(&self, file: BufWriter<File>) -> Option<BufWriter<File>> {
        OUTPUT_FILE.with(|output_file| {
            let mut lock = output_file.borrow_mut();
            lock.output_file.replace(file)
        })
    }

    pub fn clear_file(&self) -> Option<BufWriter<File>> {
        OUTPUT_FILE.with(|output_file| {
            let mut lock = output_file.borrow_mut();
            lock.output_file.take()
        })
    }

    fn write_ev(&self, ev: Event) {
        OUTPUT_FILE.with(|output_file| {
            let mut lock = output_file.borrow_mut();

            if let (Some(ts), Some(file)) = (lock.timestamp, lock.output_file.as_mut()) {
                let elapsed = ts.elapsed();
                let (symbol, size) = match &ev {
                    Event::Alloc { size, .. } => ('A', *size),
                    Event::Free { size, .. } => ('F', *size),
                    Event::Start => ('S', 0),
                    Event::End => ('E', 0),
                };

                // Just eat the error so we don't get a panic during allocation.
                let _ = writeln!(file, "{} {} {}", symbol, elapsed.as_nanos(), size);
            }
        });
    }
}

unsafe impl GlobalAlloc for TracingAlloc {
    unsafe fn alloc(&self, layout: std::alloc::Layout) -> *mut u8 {
        let res = System.alloc(layout);

        ACTIVE.with(|active| {
            if active.get() {
                self.write_ev(Event::Alloc {
                    addr: res as _,
                    size: layout.size(),
                });
            }
        });

        res
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: std::alloc::Layout) {
        ACTIVE.with(|active| {
            if active.get() {
                self.write_ev(Event::Free {
                    addr: ptr as _,
                    size: layout.size(),
                });
            }
        });

        System.dealloc(ptr, layout)
    }
}
