use core::mem::MaybeUninit;

/// Trait for accessing the worker memory buffer, which can be used for parsing HTTP requests or constructing responses.
pub trait HttpMemoryBuffer {
    /// Get a mutable reference to the worker memory buffer, which is a slice of uninitialized bytes.
    fn get_buffer(&mut self) -> &mut [MaybeUninit<u8>];
}

/// Placeholder for future worker-specific memory management, such as arenas or buffers
#[cfg(feature = "tokio_impl")]
pub struct HttpWorkerMemory<const SIZE: usize> {
    // Placeholder for future worker-specific memory management, such as arenas or buffers
    buffer: core::pin::Pin<Box<[MaybeUninit<u8>; SIZE]>>,
}

#[cfg(feature = "tokio_impl")]
impl<const SIZE: usize> HttpWorkerMemory<SIZE> {
    /// Create a new HttpWorkerMemory instance with an uninitialized buffer
    pub fn new() -> Self {
        Self {
            buffer: Box::pin([MaybeUninit::<u8>::uninit(); SIZE]),
        }
    }
}

#[cfg(feature = "tokio_impl")]
impl<const SIZE: usize> Default for HttpWorkerMemory<SIZE> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "tokio_impl")]
impl<const SIZE: usize> HttpMemoryBuffer for HttpWorkerMemory<SIZE> {
    fn get_buffer(&mut self) -> &mut [MaybeUninit<u8>] {
        self.buffer.as_mut_slice()
    }
}

/// Placeholder for future worker-specific memory management, such as arenas or buffers
#[cfg(feature = "embassy_impl")]
pub struct HttpWorkerMemory<'buf, const SIZE: usize> {
    buffer: &'buf mut [MaybeUninit<u8>; SIZE],
}
#[cfg(feature = "embassy_impl")]
impl<'buf, const SIZE: usize> HttpWorkerMemory<'buf, SIZE> {
    /// Create a new HttpWorkerMemory instance with an uninitialized buffer
    pub const fn new(buffer: &'buf mut [MaybeUninit<u8>; SIZE]) -> Self {
        Self { buffer }
    }
}
#[cfg(feature = "embassy_impl")]
impl<'buf, const SIZE: usize> HttpMemoryBuffer for HttpWorkerMemory<'buf, SIZE> {
    fn get_buffer(&mut self) -> &mut [MaybeUninit<u8>] {
        self.buffer.as_mut_slice()
    }
}
