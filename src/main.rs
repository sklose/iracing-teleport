use clap::{Parser, Subcommand};
use lz4::block::{compress_to_buffer, decompress_to_buffer};
use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};
use windows::{
    Win32::Foundation::*, Win32::System::Memory::*, Win32::System::Threading::*, core::*,
};

/// UDP LZ4 Source/Target application with unicast and multicast support
#[derive(Parser)]
#[command(name = "UDP LZ4 Source/Target", version, about)]
struct Cli {
    #[command(subcommand)]
    mode: Mode,
}

#[derive(Subcommand)]
enum Mode {
    /// Run as the source (sends compressed data at 60Hz)
    Source {
        /// Local bind address (e.g., 127.0.0.1:8080)
        #[arg(long, default_value = "127.0.0.1:0")]
        bind: String,

        /// Target address to send data to (e.g., 127.0.0.1:8081)
        #[arg(long, default_value = "127.0.0.1:8081")]
        target: String,

        /// Enable multicast mode
        #[arg(long)]
        multicast: bool,
    },

    /// Run as the target (receives compressed data)
    Target {
        /// Address to bind to for receiving (e.g., 127.0.0.1:8081)
        #[arg(long, default_value = "127.0.0.1:8081")]
        bind: String,

        /// Multicast group to join (required if --multicast is set)
        #[arg(long)]
        group: Option<String>,

        /// Enable multicast mode
        #[arg(long)]
        multicast: bool,
    },
}

fn run_source(
    bind: &str,
    target: &str,
    multicast: bool,
    running: Arc<AtomicBool>,
) -> windows::core::Result<()> {
    unsafe {
        let socket = UdpSocket::bind(bind).expect("Failed to bind UDP socket");
        if !multicast {
            socket.connect(target).expect("Failed to connect to target");
        }

        // Open memory-mapped file
        let h_map = OpenFileMappingW(FILE_MAP_READ.0, false, w!("Local\\IRSDKMemMapFileName"))?;
        let h_view = MapViewOfFile(h_map, FILE_MAP_READ, 0, 0, 0);
        let view = h_view.Value as *const u8;
        if view.is_null() {
            panic!("Failed to map view of file");
        }

        // Use VirtualQuery to determine the size of the region
        let mut mem_info = MEMORY_BASIC_INFORMATION::default();
        let result = VirtualQuery(
            Some(view as *const _),
            &mut mem_info,
            std::mem::size_of::<MEMORY_BASIC_INFORMATION>(),
        );
        if result == 0 {
            panic!("VirtualQuery failed");
        }

        let size = mem_info.RegionSize;
        println!("Memory region size: {} bytes", size);

        // Connect to event
        let h_event = OpenEventW(
            SYNCHRONIZATION_ACCESS_RIGHTS(0x00100000), //SYNCHRONIZE,
            false,
            w!("Local\\IRSDKDataValidEvent"),
        )?;
        if h_event.is_invalid() {
            panic!("Failed to create event: {:?}", GetLastError());
        }

        let data_slice = std::slice::from_raw_parts(view, size);
        let mut buf = [0u8; 64 * 1024];

        let mut start_time = std::time::Instant::now();
        let mut updates = 0;

        while running.load(Ordering::SeqCst) {
            if WaitForSingleObject(h_event, 200) != windows::Win32::Foundation::WAIT_EVENT(0) {
                continue; // Timeout or error
            }

            // Compress the memory content
            let len = compress_to_buffer(data_slice, None, true, &mut buf)
                .expect("LZ4 compression failed");

            if multicast {
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

        UnmapViewOfFile(h_view)?;
        CloseHandle(h_map)?;
        CloseHandle(h_event)?;
        Ok(())
    }
}

fn run_target(
    bind: &str,
    multicast: bool,
    group: Option<String>,
    running: Arc<AtomicBool>,
) -> io::Result<()> {
    let socket = UdpSocket::bind(bind)?;
    println!("Target bound to {}", bind);

    if multicast {
        let group_ip: Ipv4Addr = group
            .expect("Multicast group required with --multicast")
            .parse()
            .expect("Invalid multicast group IP");

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

    unsafe {
        // Allocate a backing file mapping for shared memory
        let mapping_size = 32 * 1024 * 1024; // 32 MB (adjust as needed)
        let h_map = CreateFileMappingW(
            INVALID_HANDLE_VALUE,
            None,
            PAGE_READWRITE,
            0,
            mapping_size,
            w!("Local\\IRSDKMemMapFileName"),
        )?;

        if h_map.is_invalid() {
            panic!("Failed to create file mapping: {:?}", GetLastError());
        }

        let view = MapViewOfFile(h_map, FILE_MAP_WRITE, 0, 0, mapping_size as usize);
        if view.Value.is_null() {
            panic!("Failed to map view of file: {:?}", GetLastError());
        }

        let data_slice =
            std::slice::from_raw_parts_mut(view.Value as *mut u8, mapping_size as usize);

        // Create auto-reset event
        let h_event = CreateEventW(
            None,
            false, // auto reset
            false, // initial state: not signaled
            w!("Local\\IRSDKDataValidEvent"),
        )?;
        if h_event.is_invalid() {
            panic!("Failed to create event: {:?}", GetLastError());
        }

        println!("Memory-mapped file and data-valid event created.");

        let mut start_time = std::time::Instant::now();
        let mut updates = 0;

        while running.load(Ordering::SeqCst) {
            let (amt, _) = socket.recv_from(&mut rcv_buf)?;
            decompress_to_buffer(&rcv_buf[0..amt], None, data_slice)
                .expect("LZ4 decompression failed");

            SetEvent(h_event)?;

            updates += 1;

            if start_time.elapsed() >= Duration::from_secs(30) {
                let rate = updates as f64 / 30.0;
                println!("[target] {:.2} updates/sec", rate);
                updates = 0;
                start_time = Instant::now();
            }
        }

        // Unreachable, but good practice:
        UnmapViewOfFile(view)?;
        CloseHandle(h_map)?;
        CloseHandle(h_event)?;
        Ok(())
    }
}

fn main() -> io::Result<()> {
    let cli = Cli::parse();

    let running: Arc<AtomicBool> = Arc::new(AtomicBool::new(true));
    let r = running.clone();
    ctrlc::set_handler(move || {
        println!("Received Ctrl+C, shutting down...");
        r.store(false, Ordering::SeqCst);
    })
    .expect("Error setting Ctrl+C handler");

    match cli.mode {
        Mode::Source {
            bind,
            target,
            multicast,
        } => run_source(&bind, &target, multicast, running).map_err(|e| {
            eprintln!("Error in source: {}", e);
            io::Error::other("Source error")
        }),

        Mode::Target {
            bind,
            group,
            multicast,
        } => run_target(&bind, multicast, group, running),
    }
}
