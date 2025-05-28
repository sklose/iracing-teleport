use lz4::block::decompress_to_buffer;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::sync::mpsc::Receiver;
use std::{
    io,
    time::{Duration, Instant},
};

use crate::protocol::{MAX_DATAGRAM_SIZE, Receiver as ProtocolReceiver};
use crate::stats::StatisticsPrinter;
use crate::telemetry::{MAX_TELEMETRY_SIZE, Telemetry, TelemetryProvider};

const TELEMETRY_TIMEOUT: Duration = Duration::from_secs(10);

fn create_telemetry() -> io::Result<Telemetry> {
    let telemetry = Telemetry::create(MAX_TELEMETRY_SIZE)
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

    let mut rcv_buf = [0u8; MAX_DATAGRAM_SIZE];
    let mut protocol_receiver = ProtocolReceiver::new(MAX_TELEMETRY_SIZE);
    let mut telemetry: Option<Telemetry> = None;
    let mut last_update = Instant::now();
    let mut stats = StatisticsPrinter::new("target");
    let mut sequence_start_time: Option<Instant> = None;

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
                stats.add_protocol_bytes(amt);

                // Process the received datagram
                let (data, sequence_changed) = protocol_receiver.process_datagram(&rcv_buf[..amt]);

                if sequence_changed {
                    sequence_start_time = Some(Instant::now());
                }

                if let Some(data) = data {
                    // Create telemetry if it doesn't exist
                    if telemetry.is_none() {
                        telemetry = Some(create_telemetry()?);
                    }

                    // Process the complete payload
                    let telemetry = telemetry.as_mut().unwrap();
                    if !try_decompress_data(data, telemetry.as_slice_mut()) {
                        // Reset accumulated bytes since we failed to process this message
                        continue;
                    }

                    // Track both data and protocol bytes for the complete message
                    stats.add_bytes(data.len());

                    telemetry.signal_data_ready().map_err(|e| {
                        io::Error::other(format!("Failed to signal data ready: {}", e))
                    })?;

                    // Calculate total latency (source processing + target processing)
                    if let Some(start_time) = sequence_start_time.take() {
                        let source_time = protocol_receiver.last_source_time_us();
                        let target_time = start_time.elapsed().as_micros() as u64;
                        stats.add_latency(source_time + target_time);
                    }

                    last_update = Instant::now();
                    stats.add_update();

                    if stats.should_print() {
                        stats.print_and_reset();
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
