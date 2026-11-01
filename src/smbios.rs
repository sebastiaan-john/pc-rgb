//! Read DIMM part numbers from the raw SMBIOS table (type 17), no WMI/COM needed.

use windows_sys::Win32::System::SystemInformation::GetSystemFirmwareTable;

pub fn dimm_part_numbers() -> Vec<String> {
    const RSMB: u32 = u32::from_be_bytes(*b"RSMB");
    let mut out = Vec::new();
    unsafe {
        let n = GetSystemFirmwareTable(RSMB, 0, std::ptr::null_mut(), 0);
        if n == 0 {
            return out;
        }
        let mut buf = vec![0u8; n as usize];
        if GetSystemFirmwareTable(RSMB, 0, buf.as_mut_ptr() as _, n) != n {
            return out;
        }
        // RawSMBIOSData: 4 header bytes, u32 length, then the table.
        let len = u32::from_le_bytes(buf[4..8].try_into().unwrap()) as usize;
        let table = &buf[8..(8 + len).min(buf.len())];
        let mut i = 0;
        while i + 4 <= table.len() {
            let (ty, flen) = (table[i], table[i + 1] as usize);
            if flen < 4 || i + flen > table.len() {
                break;
            }
            let formatted = &table[i..i + flen];
            // Strings follow the formatted area, ended by a double NUL.
            let mut j = i + flen;
            let strings_start = j;
            while j + 1 < table.len() && !(table[j] == 0 && table[j + 1] == 0) {
                j += 1;
            }
            let strings: Vec<&[u8]> = table[strings_start..j].split(|&b| b == 0).collect();
            if ty == 17 && flen > 0x1A {
                let idx = formatted[0x1A] as usize;
                if idx > 0 {
                    if let Some(s) = strings.get(idx - 1) {
                        let s = String::from_utf8_lossy(s).trim().to_string();
                        if !s.is_empty() {
                            out.push(s);
                        }
                    }
                }
            }
            if ty == 127 {
                break;
            }
            i = j + 2;
        }
    }
    out
}
