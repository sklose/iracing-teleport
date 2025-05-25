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

pub fn run(bind: &str, target: &str, unicast: bool, running: Arc<AtomicBool>) -> io::Result<()> {
    let socket = UdpSocket::bind(bind).expect("Failed to bind UDP socket");
    if unicast {
        socket.connect(target).expect("Failed to connect to target");
    }

    // Keep trying to open telemetry until successful or interrupted
    println!("Waiting for target to start...");
    let telemetry = loop {
        if !running.load(Ordering::SeqCst) {
            return Ok(());
        }

        match Telemetry::open() {
            Ok(telemetry) => {
                println!("Connected to target");
                println!("Memory region size: {} bytes", telemetry.size());
                break telemetry;
            }
            Err(TelemetryError::Unavailable) => {
                thread::sleep(Duration::from_secs(1));
                continue;
            }
            Err(TelemetryError::Other(e)) => {
                return Err(io::Error::other(e.to_string()));
            }
        }
    };

    let mut buf = [0u8; 64 * 1024];
    let mut start_time = Instant::now();
    let mut updates = 0;

    while running.load(Ordering::SeqCst) {
        if !telemetry.wait_for_data(200) {
            continue; // Timeout or error
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
