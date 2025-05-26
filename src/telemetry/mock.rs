use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use super::{TelemetryError, TelemetryProvider};

#[derive(Default)]
struct State {
    ready: bool,
    last_signal: Option<Instant>,
    waiters: u32,
}

pub struct MockTelemetry {
    buffer: Vec<u8>,
    state: Arc<(Mutex<State>, Condvar)>,
}

impl TelemetryProvider for MockTelemetry {
    fn open() -> Result<Self, TelemetryError> {
        // Simulate no telemetry available initially
        Err(TelemetryError::Unavailable)
    }

    fn create(size: usize) -> Result<Self, TelemetryError> {
        Ok(Self {
            buffer: vec![0; size],
            state: Arc::new((Mutex::new(State::default()), Condvar::new())),
        })
    }

    fn wait_for_data(&self, timeout_ms: u32) -> bool {
        let (lock, cvar) = &*self.state;
        let start = Instant::now();
        let timeout = Duration::from_millis(timeout_ms as u64);

        println!("Entering wait_for_data with timeout {}ms", timeout_ms);

        // Get the lock and wait for the condition
        let mut state = lock.lock().unwrap();
        println!("Got lock, ready={}, waiters={}", state.ready, state.waiters);

        // If we were recently signaled (within half the timeout), return true immediately
        if let Some(last_signal) = state.last_signal {
            if start.duration_since(last_signal) < timeout / 2 {
                println!("Using recent signal");
                state.last_signal = None;
                state.ready = false; // Reset ready state when using recent signal
                return true;
            }
        }

        // Return immediately if already signaled
        if state.ready {
            println!("Already signaled");
            state.ready = false; // Reset ready state when using it
            return true;
        }

        // Register as a waiter
        state.waiters += 1;
        println!("Registered as waiter {}", state.waiters);

        // Wait for signal with timeout
        println!("Starting wait with timeout");
        let result = cvar.wait_timeout(state, timeout).unwrap();
        let mut state = result.0;
        let timed_out = result.1.timed_out();
        println!("Wait finished, timed_out={}", timed_out);

        // Unregister as a waiter
        state.waiters -= 1;
        println!("Unregistered waiter, {} remaining", state.waiters);

        // Reset ready state if this is the last waiter
        if state.waiters == 0 {
            println!("Last waiter, resetting state");
            state.ready = false;
            state.last_signal = None;
        }

        !timed_out
    }

    fn signal_data_ready(&self) -> Result<(), TelemetryError> {
        let (lock, cvar) = &*self.state;
        let mut state = lock.lock().unwrap();
        println!("Signaling data ready");
        state.ready = true;
        state.last_signal = Some(Instant::now());
        cvar.notify_all();
        Ok(())
    }

    fn as_slice(&self) -> &[u8] {
        &self.buffer
    }

    fn as_slice_mut(&mut self) -> &mut [u8] {
        &mut self.buffer
    }

    fn size(&self) -> usize {
        self.buffer.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    const SHORT_WAIT: u32 = 20; // Short wait for quick operations
    const LONG_WAIT: u32 = 40; // Longer wait for operations that need more time
    const TIMING_MARGIN: u32 = 10; // Additional margin for slower debug builds

    #[test]
    fn test_mock_telemetry() {
        println!("\nStarting basic telemetry test");

        // Test that open returns Unavailable
        assert!(matches!(
            MockTelemetry::open(),
            Err(TelemetryError::Unavailable)
        ));

        // Test create and basic operations
        let mut telemetry = MockTelemetry::create(1024).unwrap();
        assert_eq!(telemetry.size(), 1024);

        // Test writing and reading data
        telemetry.as_slice_mut()[0] = 42;
        assert_eq!(telemetry.as_slice()[0], 42);

        println!("\nTesting single signal/wait cycle");

        // Test data ready signaling
        let telemetry = Arc::new(telemetry);
        let telemetry_clone = telemetry.clone();

        // Spawn a thread that will signal data ready after a delay
        thread::spawn(move || {
            println!("Signal thread sleeping");
            thread::sleep(Duration::from_millis(SHORT_WAIT as u64 / 2));
            println!("Signal thread woke up");
            telemetry_clone.signal_data_ready().unwrap();
        });

        // Wait for data and verify we didn't time out
        assert!(telemetry.wait_for_data(SHORT_WAIT + TIMING_MARGIN));

        println!("\nTesting timeout behavior");

        // Test timeout when no data is signaled
        let telemetry = MockTelemetry::create(1024).unwrap();
        assert!(!telemetry.wait_for_data(SHORT_WAIT));

        println!("\nTesting multiple signal/wait cycles");

        // Test multiple wait and signal cycles
        let telemetry = Arc::new(telemetry);
        let telemetry_clone = telemetry.clone();

        thread::spawn(move || {
            for i in 0..3 {
                thread::sleep(Duration::from_millis(SHORT_WAIT as u64 / 4));
                println!("Signaling data ready (iteration {})", i);
                telemetry_clone.signal_data_ready().unwrap();
            }
        });

        // Each wait should succeed
        for i in 0..3 {
            println!("Waiting for data (iteration {})", i);
            assert!(
                telemetry.wait_for_data(SHORT_WAIT + TIMING_MARGIN),
                "wait_for_data failed on iteration {}",
                i
            );
            // Add a small delay between waits to ensure signals don't get coalesced
            thread::sleep(Duration::from_millis(5));
        }

        // The next wait should time out since no more signals are coming
        println!("Testing final timeout");
        assert!(!telemetry.wait_for_data(SHORT_WAIT));
        println!("Basic telemetry test completed");
    }

    #[test]
    fn test_rapid_signals() {
        println!("\nStarting rapid signals test");
        let telemetry = Arc::new(MockTelemetry::create(1024).unwrap());
        let telemetry_clone = telemetry.clone();

        // Spawn a thread that rapidly signals
        thread::spawn(move || {
            for i in 0..3 {
                telemetry_clone.signal_data_ready().unwrap();
                println!("Rapid signal {}", i);
                thread::sleep(Duration::from_millis(5));
            }
        });

        // Should be able to catch at least a few signals
        let mut signals_caught = 0;
        for i in 0..5 {
            if telemetry.wait_for_data(SHORT_WAIT) {
                signals_caught += 1;
                println!("Caught signal {} of {}", signals_caught, i);
            }
        }

        assert!(signals_caught > 0, "Should catch at least one signal");
        println!("Rapid signals test completed");
    }

    #[test]
    fn test_concurrent_waiters() {
        println!("\nStarting concurrent waiters test");
        let telemetry = Arc::new(MockTelemetry::create(1024).unwrap());
        let signal_telemetry = telemetry.clone();

        // Spawn multiple waiting threads
        let mut handles = vec![];
        for i in 0..3 {
            let waiter_telemetry = telemetry.clone();
            handles.push(thread::spawn(move || {
                println!("Waiter {} starting", i);
                let result = waiter_telemetry.wait_for_data(LONG_WAIT + TIMING_MARGIN);
                println!("Waiter {} finished with result {}", i, result);
                result
            }));
        }

        // Signal after a short delay
        thread::sleep(Duration::from_millis(SHORT_WAIT as u64 / 2));
        println!("Sending signal to all waiters");
        signal_telemetry.signal_data_ready().unwrap();

        // All waiters should succeed
        for (i, handle) in handles.into_iter().enumerate() {
            assert!(handle.join().unwrap(), "Waiter {} failed", i);
        }
        println!("Concurrent waiters test completed");
    }
}
