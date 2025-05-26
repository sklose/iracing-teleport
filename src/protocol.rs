use std::io;

// Maximum UDP payload size (leaving some headroom for IP/UDP headers)
pub const MAX_DATAGRAM_SIZE: usize = 9_000;

// Maximum payload size per datagram (header + data)
pub const MAX_PAYLOAD_SIZE: usize = MAX_DATAGRAM_SIZE - std::mem::size_of::<DatagramHeader>();

#[repr(C, packed)]
struct DatagramHeader {
    sequence: u32,     // Monotonically increasing sequence number
    fragment: u16,     // Fragment index within this sequence
    fragments: u16,    // Total number of fragments in this sequence
    payload_size: u32, // Size of the compressed payload across all fragments
}

pub struct Sender {
    sequence: u32,
    buffer: Vec<u8>,
}

impl Sender {
    pub fn new(max_payload_size: usize) -> Self {
        Self {
            sequence: 0,
            buffer: vec![0; max_payload_size + MAX_DATAGRAM_SIZE],
        }
    }

    pub fn send<F>(&mut self, data: &[u8], compressed_size: usize, mut send_fn: F) -> io::Result<()>
    where
        F: FnMut(&[u8]) -> io::Result<()>,
    {
        let fragments = compressed_size.div_ceil(MAX_PAYLOAD_SIZE);
        if fragments > u16::MAX as usize {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Compressed data too large",
            ));
        }

        // Prepare header
        let mut header = DatagramHeader {
            sequence: self.sequence,
            fragments: fragments as u16,
            fragment: 0,
            payload_size: compressed_size as u32,
        };

        let header_size = std::mem::size_of::<DatagramHeader>();

        // Send each fragment
        let mut offset = 0;
        for i in 0..fragments {
            // Update fragment number
            header.fragment = i as u16;

            // Copy header to buffer
            let header_bytes = unsafe {
                std::slice::from_raw_parts(&header as *const _ as *const u8, header_size)
            };
            self.buffer[..header_size].copy_from_slice(header_bytes);

            // Calculate fragment size
            let remaining = compressed_size - offset;
            let fragment_size = remaining.min(MAX_PAYLOAD_SIZE);

            // Copy fragment data
            let start = offset;
            let end = start + fragment_size;
            self.buffer[header_size..header_size + fragment_size]
                .copy_from_slice(&data[start..end]);

            // Send datagram
            send_fn(&self.buffer[..header_size + fragment_size])?;
            offset += fragment_size;
        }

        // Increment sequence number
        self.sequence = self.sequence.wrapping_add(1);
        Ok(())
    }
}

pub struct Receiver {
    buffer: Vec<u8>,
    fragments: Vec<bool>,
    current_sequence: Option<u32>,
    total_fragments: u16,
    received_fragments: u16,
    payload_size: u32,
}

impl Receiver {
    pub fn new(max_payload_size: usize) -> Self {
        Self {
            buffer: Vec::with_capacity(max_payload_size),
            fragments: Vec::new(),
            current_sequence: None,
            total_fragments: 0,
            received_fragments: 0,
            payload_size: 0,
        }
    }

    pub fn process_datagram(&mut self, data: &[u8]) -> Option<&[u8]> {
        // Ensure we have enough data for the header
        let header_size = std::mem::size_of::<DatagramHeader>();
        if data.len() < header_size {
            return None;
        }

        // Parse header
        let header = unsafe { &*(data.as_ptr() as *const DatagramHeader) };

        // Check if this is a new sequence
        if let Some(current) = self.current_sequence {
            if header.sequence != current {
                // Discard old sequence and start new one
                self.start_new_sequence(header);
            }
        } else {
            // First sequence
            self.start_new_sequence(header);
        }

        // Validate fragment
        if header.fragment >= header.fragments || header.fragments == 0 {
            return None;
        }

        // Check if we already received this fragment
        if self.fragments[header.fragment as usize] {
            return None;
        }

        // Copy fragment data
        let fragment_size = data.len() - header_size;
        let buffer_offset = header.fragment as usize * MAX_PAYLOAD_SIZE;

        if buffer_offset + fragment_size > self.buffer.len() {
            return None;
        }

        self.buffer[buffer_offset..buffer_offset + fragment_size]
            .copy_from_slice(&data[header_size..]);

        // Mark fragment as received
        self.fragments[header.fragment as usize] = true;
        self.received_fragments += 1;

        // Check if we have all fragments
        if self.received_fragments == self.total_fragments {
            let result = &self.buffer[..self.payload_size as usize];
            self.current_sequence = None;
            Some(result)
        } else {
            None
        }
    }

    fn start_new_sequence(&mut self, header: &DatagramHeader) {
        self.current_sequence = Some(header.sequence);
        self.total_fragments = header.fragments;
        self.received_fragments = 0;
        self.payload_size = header.payload_size;

        // Reset fragment tracking
        self.fragments.clear();
        self.fragments.resize(header.fragments as usize, false);

        // Ensure buffer has enough capacity and is properly sized
        self.buffer.clear();
        self.buffer.resize(header.payload_size as usize, 0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Helper function to create test data
    fn create_test_data(size: usize) -> Vec<u8> {
        (0..size).map(|i| (i % 256) as u8).collect()
    }

    #[test]
    fn test_single_fragment_send_receive() {
        let data = create_test_data(1000);
        let mut sent_datagrams = Vec::new();
        let mut sender = Sender::new(MAX_DATAGRAM_SIZE);

        // Send the data
        sender
            .send(&data, data.len(), |datagram| {
                sent_datagrams.push(datagram.to_vec());
                Ok(())
            })
            .unwrap();

        // Verify we got exactly one datagram
        assert_eq!(sent_datagrams.len(), 1);

        // Receive the data
        let mut receiver = Receiver::new(MAX_DATAGRAM_SIZE);
        let received = receiver.process_datagram(&sent_datagrams[0]).unwrap();

        // Verify the received data matches the original
        assert_eq!(received, data);
    }

    #[test]
    fn test_multi_fragment_send_receive() {
        let data = create_test_data(MAX_PAYLOAD_SIZE * 3 + 1000); // Will require 4 fragments
        let mut sent_datagrams = Vec::new();
        let mut sender = Sender::new(data.len());

        // Send the data
        sender
            .send(&data, data.len(), |datagram| {
                sent_datagrams.push(datagram.to_vec());
                Ok(())
            })
            .unwrap();

        // Verify we got 4 datagrams
        assert_eq!(sent_datagrams.len(), 4);

        // Receive the data in order
        let mut receiver = Receiver::new(data.len());
        for datagram in sent_datagrams.iter().take(3) {
            assert!(receiver.process_datagram(datagram).is_none()); // Partial data
        }
        let received = receiver.process_datagram(&sent_datagrams[3]).unwrap(); // Complete data

        // Verify the received data matches the original
        assert_eq!(received, data);
    }

    #[test]
    fn test_out_of_order_fragments() {
        let data = create_test_data(MAX_PAYLOAD_SIZE * 2 + 1000); // Will require 3 fragments
        let mut sent_datagrams = Vec::new();
        let mut sender = Sender::new(data.len());

        // Send the data
        sender
            .send(&data, data.len(), |datagram| {
                sent_datagrams.push(datagram.to_vec());
                Ok(())
            })
            .unwrap();

        // Receive the data out of order
        let mut receiver = Receiver::new(data.len());
        assert!(receiver.process_datagram(&sent_datagrams[2]).is_none()); // Last fragment
        assert!(receiver.process_datagram(&sent_datagrams[0]).is_none()); // First fragment
        let received = receiver.process_datagram(&sent_datagrams[1]).unwrap(); // Middle fragment completes

        // Verify the received data matches the original
        assert_eq!(received, data);
    }

    #[test]
    fn test_sequence_numbers() {
        let data = create_test_data(1000);
        let mut sender = Sender::new(MAX_DATAGRAM_SIZE);
        let mut last_sequence: Option<u32> = None;

        // Send the data multiple times
        for _ in 0..3 {
            let mut current_sequence = None;
            sender
                .send(&data, data.len(), |datagram| {
                    // Extract sequence number from header
                    let header = unsafe { &*(datagram.as_ptr() as *const DatagramHeader) };
                    current_sequence = Some(header.sequence);
                    Ok(())
                })
                .unwrap();

            if let Some(last) = last_sequence {
                assert_eq!(current_sequence.unwrap(), last.wrapping_add(1));
            }
            last_sequence = current_sequence;
        }
    }

    #[test]
    fn test_duplicate_fragment_handling() {
        let data = create_test_data(MAX_PAYLOAD_SIZE * 2); // Will require 2 fragments
        let mut sent_datagrams = Vec::new();
        let mut sender = Sender::new(data.len());

        // Send the data
        sender
            .send(&data, data.len(), |datagram| {
                sent_datagrams.push(datagram.to_vec());
                Ok(())
            })
            .unwrap();

        // Receive with duplicate fragments
        let mut receiver = Receiver::new(data.len());
        assert!(receiver.process_datagram(&sent_datagrams[0]).is_none()); // First fragment
        assert!(receiver.process_datagram(&sent_datagrams[0]).is_none()); // Duplicate first fragment
        let received = receiver.process_datagram(&sent_datagrams[1]).unwrap(); // Second fragment completes

        // Verify the received data matches the original
        assert_eq!(received, data);
    }

    #[test]
    fn test_invalid_fragment_number() {
        let data = create_test_data(1000);
        let mut sent_datagrams = Vec::new();
        let mut sender = Sender::new(data.len());

        // Send the data
        sender
            .send(&data, data.len(), |datagram| {
                sent_datagrams.push(datagram.to_vec());
                Ok(())
            })
            .unwrap();

        // Corrupt the fragment number in the header
        let mut corrupted = sent_datagrams[0].clone();
        let header = unsafe { &mut *(corrupted.as_mut_ptr() as *mut DatagramHeader) };
        header.fragment = 99; // Invalid fragment number

        // Attempt to receive corrupted datagram
        let mut receiver = Receiver::new(data.len());
        assert!(receiver.process_datagram(&corrupted).is_none());
    }

    #[test]
    fn test_oversized_data() {
        // Create data that would require u16::MAX + 1 fragments
        // Instead of actually allocating that much memory, we'll create a small buffer
        // and calculate the required size
        let required_size = MAX_PAYLOAD_SIZE * (u16::MAX as usize + 1);
        let test_data = create_test_data(1024); // Small test buffer
        let mut sender = Sender::new(1024);

        // Attempt to send oversized data by passing the large size
        let result = sender.send(&test_data, required_size, |_| Ok(()));
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::InvalidData);
    }
}
