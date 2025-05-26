use lz4::block::decompress_to_buffer;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::sync::mpsc::Receiver;
use std::{
    io,
    time::{Duration, Instant},
};

use crate::protocol::Receiver as ProtocolReceiver;
use crate::telemetry::{Telemetry, TelemetryProvider};

const TELEMETRY_TIMEOUT: Duration = Duration::from_secs(10);
const MAPPING_SIZE: usize = 2 * 1024 * 1024; // 2 MB

fn create_telemetry() -> io::Result<Telemetry> {
    let telemetry = Telemetry::create(MAPPING_SIZE)
        .map_err(|e| io::Error::other(format!("Failed to create telemetry: {}", e)))?;
    println!("Memory-mapped file and data-valid event created.");
    Ok(telemetry)
}

fn setup_multicast(socket: &UdpSocket, bind: &str, group: &str) -> io::Result<()> {
    let group_ip: Ipv4Addr = group
        .parse()
        .map_err(|e| io::Error::other(format!("Invalid multicast group IP: {}", e)))?;

    let local_ip = match bind.parse::<SocketAddr>() {
        Ok(addr) => match addr.ip() {
            IpAddr::V4(ipv4) => ipv4,
            _ => return Err(io::Error::other("Only IPv4 is supported for multicast")),
        },
        Err(_) => Ipv4Addr::UNSPECIFIED,
    };

    socket
        .join_multicast_v4(&group_ip, &local_ip)
        .map_err(|e| io::Error::new(e.kind(), format!("Failed to join multicast group: {}", e)))?;

    println!("Joined multicast group: {}", group_ip);
    Ok(())
}

fn try_decompress_data(compressed: &[u8], target: &mut [u8]) -> bool {
    match decompress_to_buffer(compressed, None, target) {
        Ok(_) => true,
        Err(e) => {
            eprintln!("LZ4 decompression failed: {}. Skipping this update.", e);
            false
        }
    }
}

pub fn run(bind: &str, unicast: bool, group: String, shutdown: Receiver<()>) -> io::Result<()> {
    let socket = UdpSocket::bind(bind)
        .map_err(|e| io::Error::new(e.kind(), format!("Failed to bind to {}: {}", bind, e)))?;
    println!("Target bound to {}", bind);

    if !unicast {
        setup_multicast(&socket, bind, &group)?;
    }

    let mut rcv_buf = [0u8; 65_536];
    let mut protocol_receiver = ProtocolReceiver::new(MAPPING_SIZE);
    let mut telemetry: Option<Telemetry> = None;
    let mut last_update = Instant::now();
    let mut start_time = Instant::now();
    let mut updates = 0;

    // Set a short timeout on UDP receive to check for telemetry timeout
    socket
        .set_read_timeout(Some(Duration::from_secs(1)))
        .map_err(|e| io::Error::new(e.kind(), format!("Failed to set socket timeout: {}", e)))?;

    loop {
        // Check for shutdown signal
        if shutdown.try_recv().is_ok() {
            return Ok(());
        }

        match socket.recv_from(&mut rcv_buf) {
            Ok((amt, _)) => {
                // Process the received datagram
                if let Some(data) = protocol_receiver.process_datagram(&rcv_buf[..amt]) {
                    // Create telemetry if it doesn't exist
                    if telemetry.is_none() {
                        telemetry = Some(create_telemetry()?);
                    }

                    // Process the complete payload
                    let telemetry = telemetry.as_mut().unwrap();
                    if !try_decompress_data(data, telemetry.as_slice_mut()) {
                        continue;
                    }

                    telemetry.signal_data_ready().map_err(|e| {
                        io::Error::other(format!("Failed to signal data ready: {}", e))
                    })?;

                    last_update = Instant::now();
                    updates += 1;

                    if start_time.elapsed() >= Duration::from_secs(30) {
                        let rate = updates as f64 / 30.0;
                        println!("[target] {:.2} updates/sec", rate);
                        updates = 0;
                        start_time = Instant::now();
                    }
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
            Err(e) => {
                return Err(io::Error::new(
                    e.kind(),
                    format!("UDP receive error: {}", e),
                ));
            }
        }
    }
}
