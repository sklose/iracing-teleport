use windows::{
    Win32::Foundation::*, Win32::System::Memory::*, Win32::System::Threading::*, core::*,
};

use super::{TelemetryError, TelemetryProvider};

pub struct WindowsTelemetry {
    h_map: HANDLE,
    h_event: HANDLE,
    view: *mut u8,
    size: usize,
}

impl TelemetryProvider for WindowsTelemetry {
    fn open() -> std::result::Result<Self, TelemetryError> {
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
                CloseHandle(h_map).ok();
                CloseHandle(h_event).ok();
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

    fn create(size: usize) -> std::result::Result<Self, TelemetryError> {
        unsafe {
            let h_map = CreateFileMappingW(
                INVALID_HANDLE_VALUE,
                None,
                PAGE_READWRITE,
                0,
                size as u32,
                w!("Local\\IRSDKMemMapFileName"),
            )
            .map_err(|e| TelemetryError::Other(Box::new(e)))?;

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
            )
            .map_err(|e| TelemetryError::Other(Box::new(e)))?;

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

    fn wait_for_data(&self, timeout_ms: u32) -> bool {
        unsafe { WaitForSingleObject(self.h_event, timeout_ms) == WAIT_EVENT(0) }
    }

    fn signal_data_ready(&self) -> std::result::Result<(), TelemetryError> {
        unsafe {
            SetEvent(self.h_event).map_err(|e| TelemetryError::Other(Box::new(e)))?;
            Ok(())
        }
    }

    fn as_slice(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.view, self.size) }
    }

    fn as_slice_mut(&mut self) -> &mut [u8] {
        unsafe { std::slice::from_raw_parts_mut(self.view, self.size) }
    }

    fn size(&self) -> usize {
        self.size
    }
}

impl Drop for WindowsTelemetry {
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

impl From<windows::core::Error> for TelemetryError {
    fn from(err: windows::core::Error) -> Self {
        TelemetryError::Other(Box::new(err))
    }
}
