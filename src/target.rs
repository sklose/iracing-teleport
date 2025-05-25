use lz4::block::decompress_to_buffer;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::{io, time::Instant};

use crate::telemetry::Telemetry;

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

    // Create telemetry mapping
    let mapping_size = 32 * 1024 * 1024; // 32 MB
    let mut telemetry = Telemetry::create(mapping_size)
        .map_err(|e| io::Error::other(format!("Failed to create telemetry: {}", e)))?;

    println!("Memory-mapped file and data-valid event created.");

    let mut start_time = Instant::now();
    let mut updates = 0;

    while running.load(Ordering::SeqCst) {
        let (amt, _) = socket.recv_from(&mut rcv_buf)?;
        decompress_to_buffer(&rcv_buf[0..amt], None, telemetry.as_slice_mut())
            .expect("LZ4 decompression failed");

        telemetry
            .signal_data_ready()
            .map_err(|e| io::Error::other(format!("Failed to signal data ready: {}", e)))?;

        updates += 1;

        if start_time.elapsed() >= std::time::Duration::from_secs(30) {
            let rate = updates as f64 / 30.0;
            println!("[target] {:.2} updates/sec", rate);
            updates = 0;
            start_time = Instant::now();
        }
    }

    Ok(())
}
