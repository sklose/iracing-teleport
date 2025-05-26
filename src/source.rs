use lz4::block::compress_to_buffer;
use std::net::UdpSocket;
use std::sync::mpsc::{self, Receiver};
use std::{
    io,
    time::{Duration, Instant},
};

use crate::protocol::Sender;
use crate::telemetry::{Telemetry, TelemetryError, TelemetryProvider};

// Timeout before considering the connection lost
const DISCONNECT_TIMEOUT: Duration = Duration::from_secs(10);

// Individual wait interval to maintain shutdown responsiveness
const WAIT_INTERVAL_MS: u32 = 200;

// Maximum size of the compression buffer (2MB should be plenty)
const MAX_COMPRESSED_SIZE: usize = 4 * 1024 * 1024;

fn try_connect_telemetry(shutdown: &Receiver<()>) -> io::Result<Option<Telemetry>> {
    let result = match Telemetry::open() {
        Ok(telemetry) => {
            println!("Connected to racing session");
            println!("Memory region size: {} bytes", telemetry.size());
            Ok(Some(telemetry))
        }
        Err(TelemetryError::Unavailable) => Ok(None),
        Err(TelemetryError::Other(e)) => Err(io::Error::other(e.to_string())),
    }?;

    if result.is_none() {
        // Wait for either a shutdown signal or timeout
        match shutdown.recv_timeout(Duration::from_secs(10)) {
            Ok(_) => return Ok(None),                   // Shutdown requested
            Err(mpsc::RecvTimeoutError::Timeout) => (), // Continue trying
            Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(None), // Shutdown
        }
    }
    Ok(result)
}

pub fn run(bind: &str, target: &str, unicast: bool, shutdown: Receiver<()>) -> io::Result<()> {
    let socket = UdpSocket::bind(bind)
        .map_err(|e| io::Error::new(e.kind(), format!("Failed to bind UDP socket: {}", e)))?;

    if unicast {
        socket.connect(target).map_err(|e| {
            io::Error::new(
                e.kind(),
                format!("Failed to connect to racing session: {}", e),
            )
        })?;
    }

    // Keep trying to open telemetry until successful or interrupted
    println!("Waiting for racing session to start...");
    let mut telemetry = loop {
        match try_connect_telemetry(&shutdown)? {
            Some(telemetry) => break telemetry,
            None => {
                // Check if we were asked to shut down
                if shutdown.try_recv().is_ok() {
                    return Ok(());
                }
            }
        }
    };

    let mut compression_buf = vec![0u8; MAX_COMPRESSED_SIZE];
    let mut sender = Sender::new(MAX_COMPRESSED_SIZE);
    let mut start_time = Instant::now();
    let mut updates = 0;
    let mut last_data_time = Instant::now();

    loop {
        // Check for shutdown signal
        if shutdown.try_recv().is_ok() {
            return Ok(());
        }

        if !telemetry.wait_for_data(WAIT_INTERVAL_MS) {
            // Check if we've been waiting too long
            if last_data_time.elapsed() >= DISCONNECT_TIMEOUT {
                println!("Lost connection, attempting to reconnect...");
                // Drop the current telemetry instance
                drop(telemetry);

                // Try to establish a new connection
                loop {
                    match try_connect_telemetry(&shutdown)? {
                        Some(new_telemetry) => {
                            telemetry = new_telemetry;
                            last_data_time = Instant::now();
                            println!("Successfully reconnected to racing session");
                            break;
                        }
                        None => {
                            if shutdown.try_recv().is_ok() {
                                return Ok(());
                            }
                        }
                    }
                }
                continue;
            }
            // No data yet but haven't timed out, try again
            continue;
        }

        // Got data, reset the timeout
        last_data_time = Instant::now();

        // Compress the memory content
        let len = match compress_to_buffer(telemetry.as_slice(), None, true, &mut compression_buf) {
            Ok(len) => len,
            Err(e) => {
                println!("LZ4 compression failed: {}. Skipping this update.", e);
                continue;
            }
        };

        // Send the compressed data in fragments
        let send_result = if !unicast {
            sender.send(&compression_buf[..len], |data| {
                socket.send_to(data, target).map(|_| ())
            })
        } else {
            sender.send(&compression_buf[..len], |data| {
                socket.send(data).map(|_| ())
            })
        };

        send_result?;
        updates += 1;

        if start_time.elapsed() >= Duration::from_secs(5) {
            let rate = updates as f64 / 5.0;
            println!("[source] {:.2} updates/sec", rate);
            updates = 0;
            start_time = Instant::now();
        }
    }
}
