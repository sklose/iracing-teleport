use lz4::block::compress_to_buffer;
use std::net::UdpSocket;
use std::sync::mpsc::{self, Receiver};
use std::{
    io,
    time::{Duration, Instant},
};

use crate::protocol::Sender;
use crate::stats::StatisticsPrinter;
use crate::telemetry::{MAX_TELEMETRY_SIZE, Telemetry, TelemetryError, TelemetryProvider};

// Timeout before considering the connection lost
const DISCONNECT_TIMEOUT: Duration = Duration::from_secs(10);

// Individual wait interval to maintain shutdown responsiveness
const WAIT_INTERVAL_MS: u32 = 200;

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

    let mut compression_buf = vec![0u8; MAX_TELEMETRY_SIZE];
    let mut sender = Sender::new();
    let mut stats = StatisticsPrinter::new("source");
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

        let data = telemetry.as_slice();

        // Compress the memory content
        let len = match compress_to_buffer(data, None, true, &mut compression_buf) {
            Ok(len) => len,
            Err(e) => {
                println!("LZ4 compression failed: {}. Skipping this update.", e);
                continue;
            }
        };

        stats.add_bytes(len);

        // Calculate processing time in microseconds
        let processing_time = last_data_time.elapsed().as_micros() as u64;

        // Send the compressed data in fragments
        let send_result = if !unicast {
            sender.send(&compression_buf[..len], processing_time, |data| {
                stats.add_protocol_bytes(data.len());
                socket.send_to(data, target).map(|_| ())
            })
        } else {
            sender.send(&compression_buf[..len], processing_time, |data| {
                stats.add_protocol_bytes(data.len());
                socket.send(data).map(|_| ())
            })
        };

        send_result?;

        stats.add_update();
        stats.add_latency(processing_time);

        if stats.should_print() {
            stats.print_and_reset();
        }
    }
}
