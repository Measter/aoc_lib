use std::{
    alloc::{GlobalAlloc, System},
    cell::{Cell, RefCell},
    time::{Duration, Instant},
};

use self::buffer::Buffer;

mod buffer;

#[derive(Copy, Clone)]
pub enum EventKind {
    Alloc { size: usize },
    Free { size: usize },
    Start,
    End,
}

#[derive(Clone, Copy)]
pub struct Event {
    pub time: Duration,
    pub kind: EventKind,
}

struct TraceData {
    buffer: Buffer,
    start_time: Instant,
}

thread_local! {
    static TRACE_BUFFER: RefCell<TraceData> = RefCell::new(TraceData {
        buffer: Buffer::new(),
        start_time: Instant::now(),
    });

    static ACTIVE: Cell<bool> = Cell::new(false);
}

pub struct TracingAlloc;

unsafe impl Sync for TracingAlloc {}

impl TracingAlloc {
    pub fn enable_tracing(&self) {
        TRACE_BUFFER.with_borrow_mut(|buffer| {
            buffer.start_time = Instant::now();
        });

        self.write_ev(EventKind::Start);
        ACTIVE.with(|active| active.set(true));
    }

    pub fn disable_tracing(&self) {
        self.write_ev(EventKind::End);
        ACTIVE.with(|active| active.set(false));
    }

    pub fn iter_with(&self, f: impl FnMut(&Event)) {
        TRACE_BUFFER.with_borrow(|buffer| {
            buffer.buffer.iter().for_each(f);
        })
    }

    pub fn clear_buffer(&self) {
        TRACE_BUFFER.with_borrow_mut(|buffer| {
            buffer.buffer.clear();
        })
    }

    fn write_ev(&self, kind: EventKind) {
        TRACE_BUFFER.with(|output_file| {
            let mut lock = output_file.borrow_mut();
            let time = lock.start_time.elapsed();
            lock.buffer.push(Event { time, kind })
        });
    }
}

unsafe impl GlobalAlloc for TracingAlloc {
    unsafe fn alloc(&self, layout: std::alloc::Layout) -> *mut u8 {
        let res = System.alloc(layout);

        if ACTIVE.get() {
            self.write_ev(EventKind::Alloc {
                size: layout.size(),
            });
        }

        res
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: std::alloc::Layout) {
        if ACTIVE.get() {
            self.write_ev(EventKind::Free {
                size: layout.size(),
            });
        }

        System.dealloc(ptr, layout)
    }
}
