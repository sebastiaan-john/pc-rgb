//! Minimal Windows HID access (SetupAPI + hid.dll), no external crates.

use std::io;
use std::ptr::{null, null_mut};
use windows_sys::core::GUID;
use windows_sys::Win32::Devices::DeviceAndDriverInstallation::*;
use windows_sys::Win32::Devices::HumanInterfaceDevice::*;
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Storage::FileSystem::*;

pub struct HidInfo {
    pub path: Vec<u16>, // NUL-terminated
    pub usage: u16,
}

pub struct HidDevice {
    handle: HANDLE,
    feature_len: usize,
    buf: Vec<u8>,
}

unsafe impl Send for HidDevice {}

impl Drop for HidDevice {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.handle) };
    }
}

fn open_path(path: &[u16], access: u32) -> io::Result<HANDLE> {
    let h = unsafe {
        CreateFileW(
            path.as_ptr(),
            access,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            null(),
            OPEN_EXISTING,
            0,
            null_mut(),
        )
    };
    if h == INVALID_HANDLE_VALUE {
        Err(io::Error::last_os_error())
    } else {
        Ok(h)
    }
}

fn caps(h: HANDLE) -> Option<HIDP_CAPS> {
    unsafe {
        let mut pp: PHIDP_PREPARSED_DATA = 0;
        if HidD_GetPreparsedData(h, &mut pp) == 0 {
            return None;
        }
        let mut caps: HIDP_CAPS = std::mem::zeroed();
        let ok = HidP_GetCaps(pp, &mut caps) == HIDP_STATUS_SUCCESS;
        HidD_FreePreparsedData(pp);
        ok.then_some(caps)
    }
}

/// Parse "vid_XXXX&pid_YYYY" out of a device interface path.
fn ids_from_path(path: &str) -> Option<(u16, u16)> {
    let p = path.to_ascii_lowercase();
    let v = p.find("vid_")?;
    let d = p.find("pid_")?;
    let vid = u16::from_str_radix(p.get(v + 4..v + 8)?, 16).ok()?;
    let pid = u16::from_str_radix(p.get(d + 4..d + 8)?, 16).ok()?;
    Some((vid, pid))
}

/// Enumerate present HID interfaces for a VID/PID.
pub fn enumerate(vid: u16, pid: u16) -> Vec<HidInfo> {
    let mut out = Vec::new();
    unsafe {
        let mut guid: GUID = std::mem::zeroed();
        HidD_GetHidGuid(&mut guid);
        let set = SetupDiGetClassDevsW(&guid, null(), null_mut(), DIGCF_PRESENT | DIGCF_DEVICEINTERFACE);
        if set == INVALID_HANDLE_VALUE as HDEVINFO {
            return out;
        }
        let mut idx = 0;
        loop {
            let mut ifd: SP_DEVICE_INTERFACE_DATA = std::mem::zeroed();
            ifd.cbSize = std::mem::size_of::<SP_DEVICE_INTERFACE_DATA>() as u32;
            if SetupDiEnumDeviceInterfaces(set, null(), &guid, idx, &mut ifd) == 0 {
                break;
            }
            idx += 1;

            let mut need = 0u32;
            SetupDiGetDeviceInterfaceDetailW(set, &ifd, null_mut(), 0, &mut need, null_mut());
            if need == 0 {
                continue;
            }
            // u64 backing store for alignment.
            let mut buf = vec![0u64; (need as usize + 7) / 8];
            let detail = buf.as_mut_ptr() as *mut SP_DEVICE_INTERFACE_DETAIL_DATA_W;
            (*detail).cbSize = std::mem::size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>() as u32;
            if SetupDiGetDeviceInterfaceDetailW(set, &ifd, detail, need, null_mut(), null_mut()) == 0 {
                continue;
            }
            let p = (*detail).DevicePath.as_ptr();
            let mut len = 0;
            while *p.add(len) != 0 {
                len += 1;
            }
            let path16 = std::slice::from_raw_parts(p, len);
            let path_str = String::from_utf16_lossy(path16);
            let Some((v, d)) = ids_from_path(&path_str) else { continue };
            if v != vid || d != pid {
                continue;
            }
            let mut path: Vec<u16> = path16.to_vec();
            path.push(0);
            // Access 0 is enough to read caps, even for exclusive devices.
            let Ok(h) = open_path(&path, 0) else { continue };
            let c = caps(h);
            CloseHandle(h);
            if let Some(c) = c {
                out.push(HidInfo { path, usage: c.Usage });
            }
        }
        SetupDiDestroyDeviceInfoList(set);
    }
    out
}

impl HidDevice {
    pub fn open(info: &HidInfo) -> io::Result<Self> {
        let handle = open_path(&info.path, GENERIC_READ | GENERIC_WRITE)?;
        let feature_len = caps(handle).map(|c| c.FeatureReportByteLength as usize).unwrap_or(0);
        if feature_len == 0 {
            unsafe { CloseHandle(handle) };
            return Err(io::Error::new(io::ErrorKind::Unsupported, "no feature reports"));
        }
        Ok(HidDevice { handle, feature_len, buf: vec![0; feature_len] })
    }

    /// Send a feature report; `data[0]` is the report id. Zero-padded to the report length.
    pub fn set_feature(&mut self, data: &[u8]) -> io::Result<()> {
        let n = data.len().min(self.feature_len);
        self.buf[..n].copy_from_slice(&data[..n]);
        self.buf[n..].fill(0);
        let ok = unsafe { HidD_SetFeature(self.handle, self.buf.as_ptr() as _, self.feature_len as u32) };
        if ok == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    pub fn get_feature(&mut self, report_id: u8) -> io::Result<&[u8]> {
        self.buf.fill(0);
        self.buf[0] = report_id;
        let ok = unsafe { HidD_GetFeature(self.handle, self.buf.as_mut_ptr() as _, self.feature_len as u32) };
        if ok == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(&self.buf)
        }
    }
}
