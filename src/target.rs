use lz4::block::decompress_to_buffer;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::{
    io,
    time::{Duration, Instant},
};

use crate::telemetry::Telemetry;

const TELEMETRY_TIMEOUT: Duration = Duration::from_secs(10);
const MAPPING_SIZE: usize = 32 * 1024 * 1024; // 32 MB

fn create_telemetry() -> io::Result<Telemetry> {
    let telemetry = Telemetry::create(MAPPING_SIZE)
        .map_err(|e| io::Error::other(format!("Failed to create telemetry: {}", e)))?;
    println!("Memory-mapped file and data-valid event created.");
    Ok(telemetry)
}

pub fn run(bind: &str, unicast: bool, group: String, running: Arc<AtomicBool>) -> io::Result<()> {
    let socket = UdpSocket::bind(bind)?;
    println!("Target bound to {}", bind);

    if !unicast {
        let group_ip: Ipv4Addr = group.parse().expect("Invalid multicast group IP");

        let local_ip = match bind.parse::<SocketAddr>() {
            Ok(addr) => match addr.ip() {
                IpAddr::V4(ipv4) => ipv4,
                _ => panic!("Only IPv4 is supported for multicast"),
            },
            Err(_) => Ipv4Addr::UNSPECIFIED,
        };

        socket
            .join_multicast_v4(&group_ip, &local_ip)
            .expect("Failed to join multicast group");

        println!("Joined multicast group: {}", group_ip);
    }

    let mut rcv_buf = [0u8; 64 * 1024];
    let mut telemetry: Option<Telemetry> = None;
    let mut last_update = Instant::now();
    let mut start_time = Instant::now();
    let mut updates = 0;

    // Set a short timeout on UDP receive to check for telemetry timeout
    socket.set_read_timeout(Some(Duration::from_secs(1)))?;

    while running.load(Ordering::SeqCst) {
        match socket.recv_from(&mut rcv_buf) {
            Ok((amt, _)) => {
                // Create telemetry if it doesn't exist
                if telemetry.is_none() {
                    telemetry = Some(create_telemetry()?);
                }

                // Process the received data
                let telemetry = telemetry.as_mut().unwrap();
                decompress_to_buffer(&rcv_buf[0..amt], None, telemetry.as_slice_mut())
                    .expect("LZ4 decompression failed");

                telemetry
                    .signal_data_ready()
                    .map_err(|e| io::Error::other(format!("Failed to signal data ready: {}", e)))?;

                last_update = Instant::now();
                updates += 1;

                if start_time.elapsed() >= Duration::from_secs(30) {
                    let rate = updates as f64 / 30.0;
                    println!("[target] {:.2} updates/sec", rate);
                    updates = 0;
                    start_time = Instant::now();
                }
            }
            Err(e)
                if e.kind() == io::ErrorKind::WouldBlock || e.kind() == io::ErrorKind::TimedOut =>
            {
                // Check if we should close telemetry due to timeout
                if telemetry.is_some() && last_update.elapsed() >= TELEMETRY_TIMEOUT {
                    println!(
                        "No updates received for {} seconds, closing telemetry",
                        TELEMETRY_TIMEOUT.as_secs()
                    );
                    telemetry = None;
                }
            }
            Err(e) => return Err(e),
        }
    }

    Ok(())
}
