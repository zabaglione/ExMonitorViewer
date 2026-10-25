use serde::Serialize;
use std::env;
use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::path::Path;
use windows::core::{BOOL, PWSTR};
use windows::Win32::Foundation::{CloseHandle, HANDLE, HWND, LPARAM, RECT};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HMONITOR, MONITORINFO, MONITORINFOEXW,
};
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_FORMAT, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClassNameW, GetWindow, GetWindowDisplayAffinity, GetWindowLongPtrW,
    GetWindowRect, GetWindowThreadProcessId, IsWindowVisible, GWL_EXSTYLE, GW_OWNER, WS_EX_TOPMOST,
};

#[derive(Debug)]
struct Options {
    process_names: Vec<String>,
    include_hidden: bool,
}

#[derive(Clone, Copy, Debug, Serialize)]
struct Rect {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

#[derive(Debug, Serialize)]
struct MonitorRecord {
    device: String,
    rect: Rect,
}

#[derive(Debug, Serialize)]
struct Affinity {
    status: &'static str,
    value: Option<String>,
    label: Option<&'static str>,
    error_hresult: Option<String>,
}

#[derive(Debug, Serialize)]
struct WindowRecord {
    hwnd: String,
    owner_hwnd: Option<String>,
    z_order_ordinal: u32,
    pid: u32,
    visible: bool,
    process_basename: String,
    class_name: String,
    extended_style: String,
    topmost: bool,
    rect: Option<Rect>,
    affinity: Affinity,
    display_overlaps: Vec<String>,
}

#[derive(Serialize)]
struct Report {
    read_only: bool,
    note: &'static str,
    monitors: Vec<MonitorRecord>,
    windows: Vec<WindowRecord>,
}

fn parse_options() -> Result<Options, String> {
    let mut args = env::args().skip(1);
    let mut names = Vec::new();
    let mut include_hidden = false;
    while let Some(arg) = args.next() {
        if arg == "--process" {
            let value = args
                .next()
                .ok_or("--process requires an executable basename")?;
            let base = Path::new(&value)
                .file_name()
                .and_then(|s| s.to_str())
                .ok_or("--process requires a valid executable basename")?
                .to_ascii_lowercase();
            if !base.ends_with(".exe") || base.contains(['/', '\\']) {
                return Err(
                    "--process requires an executable basename such as WebexHost.exe".into(),
                );
            }
            names.push(base);
        } else if arg == "--include-hidden" {
            include_hidden = true;
        } else if arg == "--help" || arg == "-h" {
            return Err(
                "usage: capture-diagnostics [--include-hidden] [--process executable.exe]..."
                    .into(),
            );
        } else {
            return Err(format!("unknown option: {}", ascii(&arg)));
        }
    }
    if names.is_empty() {
        names = vec![
            "webex.exe".into(),
            "webexhost.exe".into(),
            "cisco.exe".into(),
            "ciscocollabhost.exe".into(),
            "atmgr.exe".into(),
            "ptone.exe".into(),
        ];
    }
    names.sort();
    names.dedup();
    Ok(Options {
        process_names: names,
        include_hidden,
    })
}

fn ascii(value: &str) -> String {
    value
        .chars()
        .map(|ch| if ch.is_ascii() { ch } else { '?' })
        .collect()
}

fn rect_from(rect: RECT) -> Rect {
    Rect {
        left: rect.left,
        top: rect.top,
        right: rect.right,
        bottom: rect.bottom,
    }
}

fn intersects(a: Rect, b: Rect) -> bool {
    a.left < b.right && a.right > b.left && a.top < b.bottom && a.bottom > b.top
}

fn affinity_label(value: u32) -> &'static str {
    match value {
        0 => "NONE",
        1 => "MONITOR",
        0x11 => "EXCLUDEFROMCAPTURE",
        _ => "OTHER",
    }
}

fn get_process_basename(pid: u32) -> Result<String, String> {
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }
        .map_err(|e| format!("open:{:#010x}", e.code().0 as u32))?;
    let result = query_basename(process);
    let _ = unsafe { CloseHandle(process) };
    result
}

fn query_basename(process: HANDLE) -> Result<String, String> {
    let mut buffer = vec![0u16; 32768];
    let mut length = buffer.len() as u32;
    unsafe {
        QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_FORMAT(0),
            PWSTR(buffer.as_mut_ptr()),
            &mut length,
        )
    }
    .map_err(|e| format!("image:{:#010x}", e.code().0 as u32))?;
    let path = OsString::from_wide(&buffer[..length as usize]);
    let basename = Path::new(&path)
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    Ok(ascii(&basename).to_ascii_lowercase())
}

unsafe extern "system" fn collect_monitor(
    monitor: HMONITOR,
    _dc: windows::Win32::Graphics::Gdi::HDC,
    _rect: *mut RECT,
    data: LPARAM,
) -> BOOL {
    let records = &mut *(data.0 as *mut Vec<MonitorRecord>);
    let mut info = MONITORINFOEXW::default();
    info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
    let ok = GetMonitorInfoW(monitor, &mut info.monitorInfo as *mut MONITORINFO);
    if ok.as_bool() {
        let end = info
            .szDevice
            .iter()
            .position(|c| *c == 0)
            .unwrap_or(info.szDevice.len());
        let device = String::from_utf16_lossy(&info.szDevice[..end]);
        records.push(MonitorRecord {
            device: ascii(&device),
            rect: rect_from(info.monitorInfo.rcMonitor),
        });
    }
    BOOL(1)
}

struct WindowContext<'a> {
    names: &'a [String],
    monitors: &'a [MonitorRecord],
    include_hidden: bool,
    records: Vec<WindowRecord>,
}

unsafe extern "system" fn collect_window(hwnd: HWND, data: LPARAM) -> BOOL {
    let context = &mut *(data.0 as *mut WindowContext<'_>);
    let visible = IsWindowVisible(hwnd).as_bool();
    if !visible && !context.include_hidden {
        return BOOL(1);
    }
    let mut pid = 0u32;
    GetWindowThreadProcessId(hwnd, Some(&mut pid));
    if pid == 0 {
        return BOOL(1);
    }
    let basename = match get_process_basename(pid) {
        Ok(name) => name,
        Err(_) => return BOOL(1),
    };
    if !context.names.iter().any(|name| name == &basename) {
        return BOOL(1);
    }

    let mut class_buffer = [0u16; 512];
    let class_len = GetClassNameW(hwnd, &mut class_buffer);
    let class_name = if class_len > 0 {
        ascii(&String::from_utf16_lossy(
            &class_buffer[..class_len as usize],
        ))
    } else {
        String::new()
    };
    let mut raw_rect = RECT::default();
    let rect = GetWindowRect(hwnd, &mut raw_rect)
        .ok()
        .map(|_| rect_from(raw_rect));

    let mut raw_affinity = 0u32;
    let affinity = match GetWindowDisplayAffinity(hwnd, &mut raw_affinity) {
        Ok(()) => Affinity {
            status: "known",
            value: Some(format!("0x{:08X}", raw_affinity)),
            label: Some(affinity_label(raw_affinity)),
            error_hresult: None,
        },
        Err(error) => Affinity {
            status: "unknown",
            value: None,
            label: None,
            error_hresult: Some(format!("0x{:08X}", error.code().0 as u32)),
        },
    };
    let display_overlaps = rect
        .map(|window_rect| {
            context
                .monitors
                .iter()
                .filter(|monitor| intersects(window_rect, monitor.rect))
                .map(|monitor| monitor.device.clone())
                .collect()
        })
        .unwrap_or_default();
    let z_order_ordinal = context.records.len() as u32;
    context.records.push(WindowRecord {
        hwnd: format!("0x{:X}", hwnd.0 as usize),
        owner_hwnd: GetWindow(hwnd, GW_OWNER)
            .ok()
            .filter(|owner| !owner.0.is_null())
            .map(|owner| format!("0x{:X}", owner.0 as usize)),
        z_order_ordinal,
        pid,
        visible,
        process_basename: basename,
        class_name,
        extended_style: format!("0x{:08X}", GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32),
        topmost: GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOPMOST.0 != 0,
        rect,
        affinity,
        display_overlaps,
    });
    BOOL(1)
}

fn run(options: Options) -> Result<Report, String> {
    let mut monitors = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(
            None,
            None,
            Some(collect_monitor),
            LPARAM(&mut monitors as *mut _ as isize),
        );
    }
    let mut context = WindowContext {
        names: &options.process_names,
        monitors: &monitors,
        include_hidden: options.include_hidden,
        records: Vec::new(),
    };
    unsafe {
        EnumWindows(
            Some(collect_window),
            LPARAM(&mut context as *mut _ as isize),
        )
    }
    .map_err(|e| format!("EnumWindows failed: {:#010x}", e.code().0 as u32))?;
    context
        .records
        .sort_by(|a, b| (a.pid, &a.hwnd).cmp(&(b.pid, &b.hwnd)));
    let windows = std::mem::take(&mut context.records);
    drop(context);
    Ok(Report {
        read_only: true,
        note: "Window titles and meeting content were not queried.",
        monitors,
        windows,
    })
}

fn main() {
    let options = match parse_options() {
        Ok(options) => options,
        Err(message) => {
            eprintln!("{}", ascii(&message));
            std::process::exit(2);
        }
    };
    match run(options)
        .and_then(|report| serde_json::to_string_pretty(&report).map_err(|e| e.to_string()))
    {
        Ok(json) => println!("{}", ascii(&json)),
        Err(error) => {
            eprintln!("diagnostic error: {}", ascii(&error));
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn affinity_values_are_labeled_without_guessing_unknown_values() {
        assert_eq!(affinity_label(0), "NONE");
        assert_eq!(affinity_label(1), "MONITOR");
        assert_eq!(affinity_label(0x11), "EXCLUDEFROMCAPTURE");
        assert_eq!(affinity_label(2), "OTHER");
    }

    #[test]
    fn rectangle_overlap_uses_half_open_edges() {
        let a = Rect {
            left: 0,
            top: 0,
            right: 100,
            bottom: 100,
        };
        let touching = Rect {
            left: 100,
            top: 0,
            right: 200,
            bottom: 100,
        };
        let overlapping = Rect {
            left: 99,
            top: 99,
            right: 101,
            bottom: 101,
        };
        assert!(!intersects(a, touching));
        assert!(intersects(a, overlapping));
    }

    #[test]
    fn output_sanitization_is_ascii_only() {
        assert_eq!(ascii("Webex参加者"), "Webex???");
    }
}
