use super::{TelemetryError, TelemetryProvider};
use rand::{Rng, thread_rng};
use std::cell::UnsafeCell;
use std::time::{Duration, Instant};

// Import constants from protocol module for consistent sizing
const MAX_DATAGRAM_SIZE: usize = 65_000;
const HEADER_SIZE: usize = 12; // size of DatagramHeader (sequence: u32 + fragment: u16 + fragments: u16 + payload_size: u32)
const MAX_PAYLOAD_SIZE: usize = MAX_DATAGRAM_SIZE - HEADER_SIZE;

// 60Hz update rate
const UPDATE_INTERVAL: Duration = Duration::from_nanos(16_666_667); // 1/60th of a second

pub struct MockTelemetry {
    buffer: UnsafeCell<Vec<u8>>,
    last_update: UnsafeCell<Instant>,
}

// Safe to share between threads since we handle synchronization
unsafe impl Sync for MockTelemetry {}

impl MockTelemetry {
    fn generate_test_data(size: usize) -> Vec<u8> {
        let mut rng = thread_rng();
        let mut buffer = vec![0u8; size];
        rng.fill(&mut buffer[..]);
        buffer
    }

    // Helper to safely update last_update time and generate new data
    fn update_state(&mut self) {
        // Update the buffer with new random data
        let buffer = self.buffer.get_mut();
        let mut rng = thread_rng();
        rng.fill(buffer.as_mut_slice());

        // Update timestamp
        *self.last_update.get_mut() = Instant::now();
    }

    // Helper to check if it's time for an update
    fn is_update_due(&self) -> bool {
        unsafe { Instant::now().duration_since(*self.last_update.get()) >= UPDATE_INTERVAL }
    }
}

impl TelemetryProvider for MockTelemetry {
    fn open() -> Result<Self, TelemetryError> {
        // When opening as source, create random test data that spans multiple datagrams
        let min_size = MAX_PAYLOAD_SIZE + 1000; // Guarantees at least 2 fragments
        Ok(Self {
            buffer: UnsafeCell::new(Self::generate_test_data(min_size)),
            last_update: UnsafeCell::new(Instant::now() - UPDATE_INTERVAL), // Allow immediate first update
        })
    }

    fn create(size: usize) -> Result<Self, TelemetryError> {
        // Target just allocates empty buffer of requested size
        Ok(Self {
            buffer: UnsafeCell::new(vec![0; size]),
            last_update: UnsafeCell::new(Instant::now()),
        })
    }

    fn wait_for_data(&mut self, timeout_ms: u32) -> bool {
        let timeout = Duration::from_millis(timeout_ms as u64);
        let start = Instant::now();

        // Wait until either timeout or next update interval
        while start.elapsed() < timeout {
            if self.is_update_due() {
                self.update_state();
                return true;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        false
    }

    fn signal_data_ready(&mut self) -> Result<(), TelemetryError> {
        // In real telemetry, this would send UDP data
        // For mock, just check if we need to update
        if self.is_update_due() {
            self.update_state();
        }
        Ok(())
    }

    fn as_slice(&self) -> &[u8] {
        unsafe { (*self.buffer.get()).as_slice() }
    }

    fn as_slice_mut(&mut self) -> &mut [u8] {
        unsafe { (*self.buffer.get()).as_mut_slice() }
    }

    fn size(&self) -> usize {
        unsafe { (*self.buffer.get()).len() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    #[test]
    fn test_mock_telemetry() {
        // Create source with random test data
        let mut source = MockTelemetry::open().unwrap();
        let source_size = source.size();
        assert!(
            source_size > MAX_PAYLOAD_SIZE,
            "Source buffer size {} should be larger than MAX_PAYLOAD_SIZE {}",
            source_size,
            MAX_PAYLOAD_SIZE
        );

        // Create target with same size as source
        let mut target = MockTelemetry::create(source_size).unwrap();

        // Test writing and reading data
        source.as_slice_mut()[0] = 42;
        target.as_slice_mut().copy_from_slice(source.as_slice());
        assert_eq!(target.as_slice()[0], 42);

        // Test data ready signaling at 60Hz
        let start = Instant::now();
        let mut updates = 0;
        while start.elapsed() < Duration::from_millis(100) {
            if source.wait_for_data(20) {
                updates += 1;
                target.signal_data_ready().unwrap();
            }
        }
        // Should have seen roughly 6 updates (60Hz * 0.1s = 6)
        assert!(
            updates >= 5 && updates <= 7,
            "Expected ~6 updates, got {}",
            updates
        );
    }

    #[test]
    fn test_rapid_signals() {
        let mut source = MockTelemetry::open().unwrap();
        let mut target = MockTelemetry::create(source.size()).unwrap();

        // Should only get updates at 60Hz
        let start = Instant::now();
        let mut updates = 0;
        while start.elapsed() < Duration::from_millis(50) {
            if source.wait_for_data(10) {
                updates += 1;
                target.signal_data_ready().unwrap();
            }
        }
        // Should have seen roughly 3 updates (60Hz * 0.05s = 3)
        assert!(
            updates >= 2 && updates <= 4,
            "Expected ~3 updates, got {}",
            updates
        );
    }

    #[test]
    fn test_data_variation() {
        let mut source = MockTelemetry::open().unwrap();
        let mut last_data = source.as_slice().to_vec();
        let mut different = false;

        // Check that data changes between updates
        for _ in 0..10 {
            if source.wait_for_data(20) {
                if source.as_slice() != last_data {
                    different = true;
                    break;
                }
                last_data = source.as_slice().to_vec();
            }
            thread::sleep(Duration::from_millis(16)); // Just under 60Hz
        }
        assert!(different, "Data should change between updates");
    }
}
