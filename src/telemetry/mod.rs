use std::fmt;

#[derive(Debug)]
pub enum TelemetryError {
    Unavailable,
    #[allow(dead_code)]
    Other(Box<dyn std::error::Error + Send + Sync>),
}

impl fmt::Display for TelemetryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TelemetryError::Unavailable => write!(f, "Telemetry not available"),
            TelemetryError::Other(e) => write!(f, "Telemetry error: {}", e),
        }
    }
}

impl std::error::Error for TelemetryError {}

/// Trait defining the interface for telemetry access
pub trait TelemetryProvider {
    /// Opens an existing telemetry mapping for reading (source mode)
    fn open() -> Result<Self, TelemetryError>
    where
        Self: Sized;

    /// Creates a new telemetry mapping for writing (target mode)
    fn create(size: usize) -> Result<Self, TelemetryError>
    where
        Self: Sized;

    /// Waits for the data valid event with a timeout
    fn wait_for_data(&self, timeout_ms: u32) -> bool;

    /// Signals that new data is available
    fn signal_data_ready(&self) -> Result<(), TelemetryError>;

    /// Gets a slice of the mapped memory
    fn as_slice(&self) -> &[u8];

    /// Gets a mutable slice of the mapped memory
    fn as_slice_mut(&mut self) -> &mut [u8];

    /// Returns the size of the mapped memory
    fn size(&self) -> usize;
}

#[cfg(windows)]
pub use windows::WindowsTelemetry as Telemetry;

#[cfg(not(windows))]
pub use mock::MockTelemetry as Telemetry;

#[cfg(windows)]
pub mod windows;

#[cfg(not(windows))]
pub mod mock;
