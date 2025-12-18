//! Buffer pooling for reducing allocation overhead
//!
//! This module provides a buffer pool for reusing memory allocations
//! across multiple HTTP requests, reducing GC pressure and improving performance.

use std::sync::Arc;
use tokio::sync::Mutex;

/// A pool of reusable buffers for HTTP response bodies
pub struct BufferPool {
    buffers: Arc<Mutex<Vec<Vec<u8>>>>,
    buffer_size: usize,
    max_pool_size: usize,
}

impl BufferPool {
    /// Create a new buffer pool
    ///
    /// # Arguments
    /// * `buffer_size` - Initial size of each buffer (e.g., 512KB for typical market data)
    /// * `max_pool_size` - Maximum number of buffers to keep in the pool
    pub fn new(buffer_size: usize, max_pool_size: usize) -> Self {
        Self {
            buffers: Arc::new(Mutex::new(Vec::with_capacity(max_pool_size))),
            buffer_size,
            max_pool_size,
        }
    }

    /// Get a buffer from the pool, or create a new one if pool is empty
    pub async fn get(&self) -> Vec<u8> {
        let mut buffers = self.buffers.lock().await;

        match buffers.pop() {
            Some(mut buffer) => {
                buffer.clear();
                buffer
            },
            None => {
                // Pool is empty, create a new buffer
                Vec::with_capacity(self.buffer_size)
            },
        }
    }

    /// Return a buffer to the pool
    pub async fn return_buffer(&self, mut buffer: Vec<u8>) {
        let mut buffers = self.buffers.lock().await;

        // Only return to pool if we're under the size limit
        if buffers.len() < self.max_pool_size {
            buffer.clear();
            // Shrink if buffer grew too large
            if buffer.capacity() > self.buffer_size * 2 {
                buffer.shrink_to(self.buffer_size);
            }
            buffers.push(buffer);
        }
        // Otherwise, let the buffer be dropped
    }

    /// Get the current number of buffers in the pool
    pub async fn size(&self) -> usize {
        let buffers = self.buffers.lock().await;
        buffers.len()
    }

    /// Pre-allocate buffers in the pool
    pub async fn prewarm(&self, count: usize) {
        let mut buffers = self.buffers.lock().await;
        for _ in 0..count.min(self.max_pool_size) {
            buffers.push(Vec::with_capacity(self.buffer_size));
        }
    }
}

impl Default for BufferPool {
    fn default() -> Self {
        // Default: 512KB buffers, pool of 10
        Self::new(512 * 1024, 10)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_buffer_pool_get_and_return() {
        let pool = BufferPool::new(1024, 5);

        let buffer = pool.get().await;
        assert_eq!(buffer.capacity(), 1024);

        pool.return_buffer(buffer).await;
        assert_eq!(pool.size().await, 1);
    }

    #[tokio::test]
    async fn test_buffer_pool_prewarm() {
        let pool = BufferPool::new(1024, 5);
        pool.prewarm(3).await;
        assert_eq!(pool.size().await, 3);
    }

    #[tokio::test]
    async fn test_buffer_pool_max_size() {
        let pool = BufferPool::new(1024, 2);

        let buf1 = pool.get().await;
        let buf2 = pool.get().await;
        let buf3 = pool.get().await;

        pool.return_buffer(buf1).await;
        pool.return_buffer(buf2).await;
        pool.return_buffer(buf3).await; // This should be dropped, not added to pool

        assert_eq!(pool.size().await, 2); // Max size is 2
    }
}
