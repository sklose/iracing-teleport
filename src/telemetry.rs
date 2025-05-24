use windows::{
    Win32::Foundation::*, Win32::System::Memory::*, Win32::System::Threading::*, core::*,
};

pub struct Telemetry {
    h_map: HANDLE,
    h_event: HANDLE,
    view: *mut u8,
    size: usize,
}

impl Telemetry {
    /// Opens an existing telemetry mapping for reading (source mode)
    pub unsafe fn open() -> windows::core::Result<Self> {
        unsafe {
            let h_map = OpenFileMappingW(FILE_MAP_READ.0, false, w!("Local\\IRSDKMemMapFileName"))?;
            let h_view = MapViewOfFile(h_map, FILE_MAP_READ, 0, 0, 0);
            let view = h_view.Value as *mut u8;

            if view.is_null() {
                return Err(windows::core::Error::from_win32());
            }

            // Get the size of the mapped region
            let mut mem_info = MEMORY_BASIC_INFORMATION::default();
            if VirtualQuery(
                Some(view as *const _),
                &mut mem_info,
                std::mem::size_of::<MEMORY_BASIC_INFORMATION>(),
            ) == 0
            {
                return Err(windows::core::Error::from_win32());
            }

            let h_event = OpenEventW(
                SYNCHRONIZATION_ACCESS_RIGHTS(0x00100000), //SYNCHRONIZE
                false,
                w!("Local\\IRSDKDataValidEvent"),
            )?;

            if h_event.is_invalid() {
                return Err(windows::core::Error::from_win32());
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
    pub unsafe fn create(size: usize) -> windows::core::Result<Self> {
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
                return Err(windows::core::Error::from_win32());
            }

            let view = MapViewOfFile(h_map, FILE_MAP_WRITE, 0, 0, size).Value as *mut u8;
            if view.is_null() {
                CloseHandle(h_map)?;
                return Err(windows::core::Error::from_win32());
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
                return Err(windows::core::Error::from_win32());
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
    pub unsafe fn wait_for_data(&self, timeout_ms: u32) -> bool {
        unsafe { WaitForSingleObject(self.h_event, timeout_ms) == WAIT_EVENT(0) }
    }

    /// Signals that new data is available
    pub unsafe fn signal_data_ready(&self) -> windows::core::Result<()> {
        unsafe {
            SetEvent(self.h_event)?;
            Ok(())
        }
    }

    /// Gets a slice of the mapped memory
    pub unsafe fn as_slice(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.view, self.size) }
    }

    /// Gets a mutable slice of the mapped memory
    pub unsafe fn as_slice_mut(&mut self) -> &mut [u8] {
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
