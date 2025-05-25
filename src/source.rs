use lz4::block::compress_to_buffer;
use std::net::UdpSocket;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::{
    io, thread,
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
        Err(TelemetryError::Unavailable) => {
            thread::sleep(Duration::from_secs(1));
            Ok(None)
        }
        Err(TelemetryError::Other(e)) => Err(io::Error::other(e.to_string())),
    }
}

pub fn run(bind: &str, target: &str, unicast: bool, running: Arc<AtomicBool>) -> io::Result<()> {
    let socket = UdpSocket::bind(bind).expect("Failed to bind UDP socket");
    if unicast {
        socket
            .connect(target)
            .expect("Failed to connect to racing session");
    }

    // Keep trying to open telemetry until successful or interrupted
    println!("Waiting for racing session to start...");
    let mut telemetry = loop {
        if !running.load(Ordering::SeqCst) {
            return Ok(());
        }

        if let Some(telemetry) = connect_telemetry()? {
            break telemetry;
        }
    };

    let mut buf = [0u8; 64 * 1024];
    let mut start_time = Instant::now();
    let mut updates = 0;

    while running.load(Ordering::SeqCst) {
        if !telemetry.wait_for_data(200) {
            println!("Lost connection, attempting to reconnect...");
            // Drop the current telemetry instance
            drop(telemetry);

            // Try to establish a new connection
            loop {
                if !running.load(Ordering::SeqCst) {
                    return Ok(());
                }

                if let Some(new_telemetry) = connect_telemetry()? {
                    telemetry = new_telemetry;
                    println!("Successfully reconnected to racing session");
                    break;
                }
            }
            continue;
        }

        // Compress the memory content
        let len = compress_to_buffer(telemetry.as_slice(), None, true, &mut buf)
            .expect("LZ4 compression failed");

        if !unicast {
            socket.send_to(&buf[..len], target).unwrap();
        } else {
            socket.send(&buf[..len]).unwrap();
        }

        updates += 1;

        if start_time.elapsed() >= Duration::from_secs(30) {
            let rate = updates as f64 / 30.0;
            println!("[source] {:.2} updates/sec", rate);
            updates = 0;
            start_time = Instant::now();
        }
    }

    Ok(())
}
