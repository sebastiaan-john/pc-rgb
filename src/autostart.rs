//! "Start with Windows" via a logon scheduled task with highest privileges, so the app
//! starts elevated (needed for RAM/SMBus) without a UAC prompt.

use std::os::windows::process::CommandExt;
use std::process::{Command, Stdio};

const TASK: &str = "pc-rgb";
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

fn schtasks(args: &[&str]) -> bool {
    Command::new("schtasks")
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .status()
        .is_ok_and(|s| s.success())
}

pub fn enabled() -> bool {
    schtasks(&["/Query", "/TN", TASK])
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

pub fn set(on: bool) -> bool {
    if !on {
        return schtasks(&["/Delete", "/TN", TASK, "/F"]);
    }
    let Ok(exe) = std::env::current_exe() else { return false };
    let user = format!(
        "{}\\{}",
        std::env::var("USERDOMAIN").unwrap_or_default(),
        std::env::var("USERNAME").unwrap_or_default()
    );
    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo><Description>pc-rgb lighting</Description></RegistrationInfo>
  <Triggers><LogonTrigger><Enabled>true</Enabled><UserId>{user}</UserId></LogonTrigger></Triggers>
  <Principals><Principal id="Author"><UserId>{user}</UserId><LogonType>InteractiveToken</LogonType><RunLevel>HighestAvailable</RunLevel></Principal></Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>
    <Priority>4</Priority>
  </Settings>
  <Actions Context="Author"><Exec><Command>"{exe}"</Command></Exec></Actions>
</Task>"#,
        user = xml_escape(&user),
        exe = xml_escape(&exe.to_string_lossy()),
    );
    // schtasks wants UTF-16 LE with BOM.
    let mut bytes = vec![0xFF, 0xFE];
    bytes.extend(xml.encode_utf16().flat_map(u16::to_le_bytes));
    let path = std::env::temp_dir().join("pc-rgb-task.xml");
    if std::fs::write(&path, bytes).is_err() {
        return false;
    }
    let ok = schtasks(&["/Create", "/TN", TASK, "/XML", &path.to_string_lossy(), "/F"]);
    let _ = std::fs::remove_file(path);
    ok
}
