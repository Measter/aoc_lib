use std::{
    alloc::{handle_alloc_error, GlobalAlloc, Layout, System},
    marker::PhantomData,
    ptr::NonNull,
    slice::{self, Iter},
};

use super::Event;

const fn _is_copy<T: Copy>() {}
const _IS_COPY: () = _is_copy::<Event>();

// Effectively a simple purpose-specific reimplementation of a Vec so we can
// store the buffer in memory instead of a file.

const INITIAL_SIZE: usize = 1024;

struct EventPtr {
    ptr: NonNull<Event>,
    _marker: PhantomData<Event>,
}

pub struct Buffer {
    capacity: usize,
    length: usize,
    buf: EventPtr,
}

impl Buffer {
    pub fn new() -> Self {
        let layout = Layout::array::<Event>(INITIAL_SIZE).expect("Overflowed layout calculation");

        // SAFETY: The layout is of non-zero size.
        let Some(ptr) = NonNull::new(unsafe { System.alloc(layout).cast::<Event>() }) else {
            handle_alloc_error(layout)
        };

        Self {
            capacity: INITIAL_SIZE,
            length: 0,
            buf: EventPtr {
                ptr,
                _marker: PhantomData,
            },
        }
    }

    pub fn clear(&mut self) {
        self.length = 0;
    }

    pub fn push(&mut self, event: Event) {
        if self.length == self.capacity {
            self.grow();
        }

        // SAFETY: We just made sure the allocation is large enough.
        unsafe {
            self.buf.ptr.as_ptr().add(self.length).write(event);
            self.length += 1;
        }
    }

    fn grow(&mut self) {
        let layout = Layout::array::<Event>(self.capacity).expect("Overflowed layout calculation");
        let new_capacity = self
            .capacity
            .checked_mul(2)
            .expect("Buffer grow overflowed usize");

        let new_layout =
            Layout::array::<Event>(new_capacity).expect("Overflowed layout calculation");

        // SAFETY: We know we didn't overflow the size, and that will not overflow an isize.
        // Layout uses the current capacity of the allocation.
        let Some(ptr) = NonNull::new(unsafe {
            System
                .realloc(
                    self.buf.ptr.as_ptr().cast::<u8>(),
                    layout,
                    new_layout.size(),
                )
                .cast::<Event>()
        }) else {
            handle_alloc_error(layout)
        };

        self.buf.ptr = ptr;
        self.capacity = new_capacity;
    }

    pub fn iter(&self) -> Iter<'_, Event> {
        // SAFETY: The pointer is never null, values below self.length are always inialized.
        unsafe { slice::from_raw_parts(self.buf.ptr.as_ptr(), self.length).iter() }
    }
}

impl Drop for Buffer {
    fn drop(&mut self) {
        let layout = Layout::array::<Event>(self.capacity).expect("Overflowed layout calculation");

        // SAFETY: There is always an allocation, and our store type is Copy, and therefore doesn't need to be dropped.
        unsafe {
            System.dealloc(self.buf.ptr.as_ptr().cast::<u8>(), layout);
        }
    }
}
