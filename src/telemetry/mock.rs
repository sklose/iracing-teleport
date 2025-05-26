use super::{TelemetryError, TelemetryProvider};
use crate::protocol::{MAX_PAYLOAD_SIZE};
use rand::{Rng, thread_rng};
use std::cell::UnsafeCell;

// Telemetry can be larger than a single datagram since the protocol handles fragmentation
const MOCK_TELEMETRY_SIZE: usize = MAX_PAYLOAD_SIZE * 4; // Example: 4 fragments worth of data

pub struct MockTelemetry {
    buffer: UnsafeCell<Vec<u8>>,
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

    fn update_data(&mut self) {
        // Update the buffer with new random data
        let buffer = self.buffer.get_mut();
        let mut rng = thread_rng();
        rng.fill(buffer.as_mut_slice());
    }
}

impl TelemetryProvider for MockTelemetry {
    fn open() -> Result<Self, TelemetryError> {
        // When opening as source, create random test data that spans multiple datagrams
        Ok(Self {
            buffer: UnsafeCell::new(Self::generate_test_data(MOCK_TELEMETRY_SIZE)),
        })
    }

    fn create(size: usize) -> Result<Self, TelemetryError> {
        // Target just allocates empty buffer of requested size
        Ok(Self {
            buffer: UnsafeCell::new(vec![0; size]),
        })
    }

    fn wait_for_data(&mut self, _timeout_ms: u32) -> bool {
        // For testing, always generate new data and return true
        self.update_data();
        true
    }

    fn signal_data_ready(&mut self) -> Result<(), TelemetryError> {
        // For testing, just generate new data
        self.update_data();
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

    #[test]
    fn test_mock_telemetry() {
        // Create source with random test data
        let mut source = MockTelemetry::open().unwrap();
        let source_size = source.size();
        assert!(
            source_size > MAX_PAYLOAD_SIZE,
            "Source buffer size {} should be larger than MAX_PAYLOAD_SIZE {} to test fragmentation",
            source_size,
            MAX_PAYLOAD_SIZE
        );

        // Create target with same size as source
        let mut target = MockTelemetry::create(source_size).unwrap();
        
        // Test writing and reading data
        source.as_slice_mut()[0] = 42;
        target.as_slice_mut().copy_from_slice(source.as_slice());
        assert_eq!(target.as_slice()[0], 42);

        // Test that we can get data updates
        assert!(source.wait_for_data(20));
        target.signal_data_ready().unwrap();
    }

    #[test]
    fn test_data_variation() {
        let mut source = MockTelemetry::open().unwrap();
        let initial_data = source.as_slice().to_vec();
        
        // Get new data
        assert!(source.wait_for_data(20));
        
        // Verify data changed
        assert_ne!(source.as_slice(), &initial_data[..], "Data should change between updates");
    }
}
