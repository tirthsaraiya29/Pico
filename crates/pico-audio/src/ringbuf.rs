//! Lock-free Single-Producer Single-Consumer (SPSC) bounded audio ring buffer.
//! Real-time audio callbacks perform zero mutex locking and zero heap allocation.

use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

/// Shared lock-free ring buffer state with cache-line-padded atomic cursors.
struct RingBufferInner<T: Copy + Default> {
    buffer: Box<[UnsafeCell<T>]>,
    mask: usize,
    capacity: usize,
    head: CachePadded<AtomicUsize>,
    tail: CachePadded<AtomicUsize>,
    clear_requested: AtomicBool,
}

#[repr(align(64))]
struct CachePadded<T>(T);

unsafe impl<T: Copy + Default + Send> Send for RingBufferInner<T> {}
unsafe impl<T: Copy + Default + Send> Sync for RingBufferInner<T> {}

/// Producer handle used by capture/synthesis threads (single producer at a time cleanly or guarded outside RT thread).
pub struct SpscProducer<T: Copy + Default> {
    inner: Arc<RingBufferInner<T>>,
}

/// Consumer handle held exclusively by the real-time audio callback or processing loop (zero locks, zero allocs).
pub struct SpscConsumer<T: Copy + Default> {
    inner: Arc<RingBufferInner<T>>,
}

impl<T: Copy + Default> RingBufferInner<T> {
    fn new(min_capacity: usize) -> Self {
        let cap = min_capacity.max(16).next_power_of_two();
        let mut storage = Vec::with_capacity(cap);
        for _ in 0..cap {
            storage.push(UnsafeCell::new(T::default()));
        }
        Self {
            buffer: storage.into_boxed_slice(),
            mask: cap - 1,
            capacity: cap,
            head: CachePadded(AtomicUsize::new(0)),
            tail: CachePadded(AtomicUsize::new(0)),
            clear_requested: AtomicBool::new(false),
        }
    }

    #[inline]
    fn available_read(&self) -> usize {
        if self.clear_requested.load(Ordering::Acquire) {
            return 0;
        }
        let h = self.head.0.load(Ordering::Acquire);
        let t = self.tail.0.load(Ordering::Acquire);
        h.wrapping_sub(t)
    }

    #[inline]
    fn available_write(&self) -> usize {
        let h = self.head.0.load(Ordering::Acquire);
        let t = self.tail.0.load(Ordering::Acquire);
        self.capacity.saturating_sub(h.wrapping_sub(t))
    }
}

/// Creates a bounded lock-free SPSC ring buffer pair with power-of-two capacity >= `min_capacity`.
pub fn spsc_ring_buffer<T: Copy + Default>(min_capacity: usize) -> (SpscProducer<T>, SpscConsumer<T>) {
    let inner = Arc::new(RingBufferInner::new(min_capacity));
    (
        SpscProducer {
            inner: Arc::clone(&inner),
        },
        SpscConsumer { inner },
    )
}

impl<T: Copy + Default> SpscProducer<T> {
    /// Pushes as many items from `slice` as fit without blocking; returns count written.
    pub fn push_slice(&self, slice: &[T]) -> usize {
        let h = self.inner.head.0.load(Ordering::Relaxed);
        let t = self.inner.tail.0.load(Ordering::Acquire);
        let used = h.wrapping_sub(t);
        let free = self.inner.capacity.saturating_sub(used);
        let to_write = free.min(slice.len());

        for (i, &item) in slice[..to_write].iter().enumerate() {
            let slot = (h.wrapping_add(i)) & self.inner.mask;
            unsafe {
                *self.inner.buffer[slot].get() = item;
            }
        }

        self.inner
            .head
            .0
            .store(h.wrapping_add(to_write), Ordering::Release);
        to_write
    }

    /// Signals consumer to discard all queued samples in O(1) without touching consumer `tail` from producer thread.
    pub fn request_clear(&self) {
        self.inner.clear_requested.store(true, Ordering::Release);
    }

    pub fn is_empty(&self) -> bool {
        self.inner.available_read() == 0
    }

    pub fn len(&self) -> usize {
        self.inner.available_read()
    }

    pub fn available_write(&self) -> usize {
        self.inner.available_write()
    }
}

impl<T: Copy + Default> SpscConsumer<T> {
    /// Pops up to `out.len()` elements into caller slice without locking or allocation.
    pub fn pop_slice(&self, out: &mut [T]) -> usize {
        if self.inner.clear_requested.swap(false, Ordering::AcqRel) {
            let h = self.inner.head.0.load(Ordering::Acquire);
            self.inner.tail.0.store(h, Ordering::Release);
            return 0;
        }

        let t = self.inner.tail.0.load(Ordering::Relaxed);
        let h = self.inner.head.0.load(Ordering::Acquire);
        let avail = h.wrapping_sub(t);
        let to_read = avail.min(out.len());

        for (i, dst) in out[..to_read].iter_mut().enumerate() {
            let slot = (t.wrapping_add(i)) & self.inner.mask;
            unsafe {
                *dst = *self.inner.buffer[slot].get();
            }
        }

        self.inner
            .tail
            .0
            .store(t.wrapping_add(to_read), Ordering::Release);
        to_read
    }

    /// Fast O(1) clear check when consumer runs its own loop or callback.
    #[inline]
    pub fn pop_one(&self) -> Option<T> {
        if self.inner.clear_requested.swap(false, Ordering::AcqRel) {
            let h = self.inner.head.0.load(Ordering::Acquire);
            self.inner.tail.0.store(h, Ordering::Release);
            return None;
        }
        let t = self.inner.tail.0.load(Ordering::Relaxed);
        let h = self.inner.head.0.load(Ordering::Acquire);
        if h == t {
            return None;
        }
        let slot = t & self.inner.mask;
        let val = unsafe { *self.inner.buffer[slot].get() };
        self.inner.tail.0.store(t.wrapping_add(1), Ordering::Release);
        Some(val)
    }

    pub fn len(&self) -> usize {
        self.inner.available_read()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_spsc_push_pop_and_clear() {
        let (prod, cons) = spsc_ring_buffer::<f32>(1024);
        assert!(cons.is_empty());

        let data: Vec<f32> = (0..500).map(|i| i as f32).collect();
        let written = prod.push_slice(&data);
        assert_eq!(written, 500);
        assert_eq!(cons.len(), 500);

        let mut out = vec![0.0f32; 200];
        assert_eq!(cons.pop_slice(&mut out), 200);
        assert_eq!(out[0], 0.0);
        assert_eq!(out[199], 199.0);
        assert_eq!(cons.len(), 300);

        // Request lock-free instant clear
        prod.request_clear();
        assert_eq!(cons.pop_slice(&mut out), 0);
        assert!(cons.is_empty());
    }

    #[test]
    fn test_spsc_concurrent_stress() {
        let (prod, cons) = spsc_ring_buffer::<i16>(4096);
        let total = 200_000usize;

        let p_handle = std::thread::spawn(move || {
            let mut sent = 0usize;
            while sent < total {
                let n = prod.push_slice(&vec![(sent % 1000) as i16; (total - sent).min(256)]);
                if n > 0 {
                    sent += n;
                } else {
                    std::thread::yield_now();
                }
            }
        });

        let c_handle = std::thread::spawn(move || {
            let mut recvd = 0usize;
            let mut buf = [0i16; 128];
            while recvd < total {
                let n = cons.pop_slice(&mut buf);
                if n > 0 {
                    recvd += n;
                } else {
                    std::thread::yield_now();
                }
            }
            recvd
        });

        p_handle.join().unwrap();
        assert_eq!(c_handle.join().unwrap(), total);
    }
}
