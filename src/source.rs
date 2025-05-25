use lz4::block::compress_to_buffer;
use std::net::UdpSocket;
use std::sync::mpsc::{self, Receiver};
use std::{
    io,
    time::{Duration, Instant},
};

use crate::telemetry::{Telemetry, TelemetryError};

fn connect_telemetry() -> io::Result<Option<Telemetry>> {
    match Telemetry::open() {
        Ok(telemetry) => {
            println!("Connected to racing session");
            println!("Memory region size: {} bytes", telemetry.size());
            Ok(Some(telemetry))
        }
        Err(TelemetryError::Unavailable) => Ok(None),
        Err(TelemetryError::Other(e)) => Err(io::Error::other(e.to_string())),
    }
}

fn try_connect_telemetry(shutdown: &Receiver<()>) -> io::Result<Option<Telemetry>> {
    let result = connect_telemetry()?;
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

    let mut buf = [0u8; 64 * 1024];
    let mut start_time = Instant::now();
    let mut updates = 0;

    loop {
        // Check for shutdown signal
        if shutdown.try_recv().is_ok() {
            return Ok(());
        }

        if !telemetry.wait_for_data(200) {
            println!("Lost connection, attempting to reconnect...");
            // Drop the current telemetry instance
            drop(telemetry);

            // Try to establish a new connection
            loop {
                match try_connect_telemetry(&shutdown)? {
                    Some(new_telemetry) => {
                        telemetry = new_telemetry;
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

        // Compress the memory content
        let len = compress_to_buffer(telemetry.as_slice(), None, true, &mut buf)
            .map_err(|e| io::Error::other(format!("LZ4 compression failed: {}", e)))?;

        if !unicast {
            socket
                .send_to(&buf[..len], target)
                .map_err(|e| io::Error::new(e.kind(), format!("Failed to send data: {}", e)))?;
        } else {
            socket
                .send(&buf[..len])
                .map_err(|e| io::Error::new(e.kind(), format!("Failed to send data: {}", e)))?;
        }

        updates += 1;

        if start_time.elapsed() >= Duration::from_secs(30) {
            let rate = updates as f64 / 30.0;
            println!("[source] {:.2} updates/sec", rate);
            updates = 0;
            start_time = Instant::now();
        }
    }
}
