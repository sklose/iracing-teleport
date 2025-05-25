use std::fmt;
use windows::{
    Win32::Foundation::*, Win32::System::Memory::*, Win32::System::Threading::*, core::*,
};

#[derive(Debug)]
pub enum TelemetryError {
    Unavailable,
    Other(windows::core::Error),
}

impl fmt::Display for TelemetryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TelemetryError::Unavailable => write!(f, "Telemetry not available"),
            TelemetryError::Other(e) => write!(f, "Windows error: {}", e),
        }
    }
}

impl std::error::Error for TelemetryError {}

impl From<windows::core::Error> for TelemetryError {
    fn from(err: windows::core::Error) -> Self {
        TelemetryError::Other(err)
    }
}

pub struct Telemetry {
    h_map: HANDLE,
    h_event: HANDLE,
    view: *mut u8,
    size: usize,
}

impl Telemetry {
    /// Opens an existing telemetry mapping for reading (source mode)
    pub fn open() -> std::result::Result<Self, TelemetryError> {
        unsafe {
            // Try to open the event
            let h_event = match OpenEventW(
                SYNCHRONIZATION_SYNCHRONIZE,
                false,
                w!("Local\\IRSDKDataValidEvent"),
            ) {
                Ok(handle) => handle,
                Err(_) => return Err(TelemetryError::Unavailable),
            };

            // Try to open the memory mapped file
            let h_map =
                match OpenFileMappingW(FILE_MAP_READ.0, false, w!("Local\\IRSDKMemMapFileName")) {
                    Ok(handle) => handle,
                    Err(_) => {
                        CloseHandle(h_event).ok();
                        return Err(TelemetryError::Unavailable);
                    }
                };

            let h_view = MapViewOfFile(h_map, FILE_MAP_READ, 0, 0, 0);
            let view = h_view.Value as *mut u8;

            if view.is_null() {
                CloseHandle(h_map)?;
                CloseHandle(h_event)?;
                return Err(windows::core::Error::from_win32().into());
            }

            // Get the size of the mapped region
            let mut mem_info = MEMORY_BASIC_INFORMATION::default();
            if VirtualQuery(
                Some(view as *const _),
                &mut mem_info,
                std::mem::size_of::<MEMORY_BASIC_INFORMATION>(),
            ) == 0
            {
                UnmapViewOfFile(MEMORY_MAPPED_VIEW_ADDRESS {
                    Value: view as *mut _,
                })?;
                CloseHandle(h_map)?;
                CloseHandle(h_event)?;
                return Err(windows::core::Error::from_win32().into());
            }

            Ok(Self {
                h_map,
                h_event,
                view,
                size: mem_info.RegionSize,
            })
        }
    }

    /// Creates a new telemetry mapping for writing (target mode)
    pub fn create(size: usize) -> std::result::Result<Self, TelemetryError> {
        unsafe {
            let h_map = CreateFileMappingW(
                INVALID_HANDLE_VALUE,
                None,
                PAGE_READWRITE,
                0,
                size as u32,
                w!("Local\\IRSDKMemMapFileName"),
            )?;

            if h_map.is_invalid() {
                return Err(windows::core::Error::from_win32().into());
            }

            let view = MapViewOfFile(h_map, FILE_MAP_WRITE, 0, 0, size).Value as *mut u8;
            if view.is_null() {
                CloseHandle(h_map)?;
                return Err(windows::core::Error::from_win32().into());
            }

            let h_event = CreateEventW(
                None,
                false, // auto reset
                false, // initial state: not signaled
                w!("Local\\IRSDKDataValidEvent"),
            )?;

            if h_event.is_invalid() {
                UnmapViewOfFile(MEMORY_MAPPED_VIEW_ADDRESS {
                    Value: view as *mut _,
                })?;
                CloseHandle(h_map)?;
                return Err(windows::core::Error::from_win32().into());
            }

            Ok(Self {
                h_map,
                h_event,
                view,
                size,
            })
        }
    }

    /// Waits for the data valid event with a timeout
    pub fn wait_for_data(&self, timeout_ms: u32) -> bool {
        unsafe { WaitForSingleObject(self.h_event, timeout_ms) == WAIT_EVENT(0) }
    }

    /// Signals that new data is available
    pub fn signal_data_ready(&self) -> windows::core::Result<()> {
        unsafe {
            SetEvent(self.h_event)?;
            Ok(())
        }
    }

    /// Gets a slice of the mapped memory
    pub fn as_slice(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.view, self.size) }
    }

    /// Gets a mutable slice of the mapped memory
    pub fn as_slice_mut(&mut self) -> &mut [u8] {
        unsafe { std::slice::from_raw_parts_mut(self.view, self.size) }
    }

    /// Returns the size of the mapped memory
    pub fn size(&self) -> usize {
        self.size
    }
}

impl Drop for Telemetry {
    fn drop(&mut self) {
        unsafe {
            let _ = UnmapViewOfFile(MEMORY_MAPPED_VIEW_ADDRESS {
                Value: self.view as *mut _,
            });
            let _ = CloseHandle(self.h_map);
            let _ = CloseHandle(self.h_event);
        }
    }
}
