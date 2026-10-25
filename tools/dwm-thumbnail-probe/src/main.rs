use serde::Serialize;
use std::env;
use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use windows::core::{w, BOOL, PCWSTR, PWSTR};
use windows::Win32::Foundation::{
    CloseHandle, COLORREF, HANDLE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM,
};
use windows::Win32::Graphics::Dwm::{
    DwmRegisterThumbnail, DwmUnregisterThumbnail, DwmUpdateThumbnailProperties,
    DWM_THUMBNAIL_PROPERTIES, DWM_TNP_RECTDESTINATION, DWM_TNP_SOURCECLIENTAREAONLY,
    DWM_TNP_VISIBLE,
};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateSolidBrush, DeleteObject, EndPaint, EnumDisplayMonitors, FillRect, GetDC,
    GetMonitorInfoW, GetPixel, UpdateWindow, HDC, HMONITOR, MONITORINFO, MONITORINFOEXW,
    PAINTSTRUCT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_FORMAT, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::HiDpi::{
    SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, EnumWindows, GetMessageW,
    GetWindowDisplayAffinity, GetWindowRect, GetWindowThreadProcessId, IsWindow, IsWindowVisible,
    KillTimer, PostQuitMessage, RegisterClassW, SetTimer, SetWindowDisplayAffinity, ShowWindow,
    TranslateMessage, UnregisterClassW, WindowFromPoint, SW_SHOWNOACTIVATE, WDA_EXCLUDEFROMCAPTURE,
    WDA_NONE, WM_CLOSE, WM_DESTROY, WM_TIMER, WNDCLASSW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    WS_EX_TOPMOST, WS_POPUP,
};

const LIFETIME_MS: u32 = 15_000;
const WM_TIMER_LIFETIME: usize = 1;
const WM_TIMER_PIXEL_CHECK: usize = 2;
const SELF_NONE_RGB: [u8; 3] = [25, 190, 240];
const SELF_EXCLUDED_RGB: [u8; 3] = [230, 35, 170];
const DESTINATION_RGB: [u8; 3] = [18, 20, 25];
static DESTINATION_DESTROYED: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy, Debug, Default, Serialize)]
struct Rect {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

impl Rect {
    fn width(self) -> i32 {
        self.right - self.left
    }
    fn height(self) -> i32 {
        self.bottom - self.top
    }
    fn area(self) -> i64 {
        i64::from(self.width().max(0)) * i64::from(self.height().max(0))
    }
}

fn intersects(a: Rect, b: Rect) -> bool {
    a.left < b.right && a.right > b.left && a.top < b.bottom && a.bottom > b.top
}

fn from_rect(rect: RECT) -> Rect {
    Rect {
        left: rect.left,
        top: rect.top,
        right: rect.right,
        bottom: rect.bottom,
    }
}

#[derive(Clone, Debug)]
struct Monitor {
    device: String,
    rect: Rect,
}

#[derive(Clone, Debug)]
struct Source {
    hwnd: HWND,
    pid: u32,
    basename: String,
    rect: Rect,
    affinity: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Webex,
    SelfTest,
}

#[derive(Debug)]
struct Options {
    mode: Mode,
    source_display: String,
    source_hwnd: Option<HWND>,
}

#[derive(Serialize)]
struct ThumbnailStatus {
    source_kind: String,
    source_basename: String,
    source_pid: u32,
    source_rect: Rect,
    source_affinity: String,
    register_status: &'static str,
    update_status: &'static str,
    preview_visible_requested: bool,
    visible_pixels_confirmed: bool,
}

#[derive(Serialize)]
struct PixelCheck {
    normal_source_matches_color: bool,
    excluded_source_matches_color: bool,
    excluded_source_matches_destination_background: bool,
    excluded_source_matches_black: bool,
    classification: &'static str,
}

#[derive(Serialize)]
struct Report {
    mode: &'static str,
    read_only_foreign_windows: bool,
    source_monitor: String,
    destination_monitor: String,
    destination_affinity_set: bool,
    destination_affinity_verified: bool,
    lifetime_seconds: u32,
    thumbnail_status: Vec<ThumbnailStatus>,
    synthetic_pixel_check: Option<PixelCheck>,
    note: &'static str,
}

fn ascii(text: &str) -> String {
    text.chars()
        .map(|ch| if ch.is_ascii() { ch } else { '?' })
        .collect()
}

fn parse_options() -> Result<Options, String> {
    let mut mode = Mode::Webex;
    let mut source_display = "\\\\.\\DISPLAY2".to_string();
    let mut source_hwnd = None;
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--mode" => {
                mode = match args.next().as_deref() {
                    Some("webex") => Mode::Webex,
                    Some("self-test") => Mode::SelfTest,
                    _ => return Err("--mode must be webex or self-test".into()),
                };
            }
            "--display" => {
                source_display = args.next().ok_or("--display requires a display device name")?;
                if !source_display.starts_with("\\\\.\\DISPLAY") || source_display.len() > 32 {
                    return Err("--display must look like \\\\.\\DISPLAY2".into());
                }
            }
            "--source-hwnd" => {
                let value = args.next().ok_or("--source-hwnd requires a hexadecimal HWND")?;
                let parsed = usize::from_str_radix(value.trim_start_matches("0x"), 16)
                    .map_err(|_| "--source-hwnd must be hexadecimal")?;
                source_hwnd = Some(HWND(parsed as *mut core::ffi::c_void));
            }
            "--help" | "-h" => return Err("usage: dwm-thumbnail-probe [--mode webex|self-test] [--display \\\\.\\DISPLAY2] [--source-hwnd 0x1234]".into()),
            _ => return Err(format!("unknown option: {}", ascii(&arg))),
        }
    }
    if mode == Mode::SelfTest && source_hwnd.is_some() {
        return Err("--source-hwnd is only valid in webex mode".into());
    }
    Ok(Options {
        mode,
        source_display,
        source_hwnd,
    })
}

unsafe extern "system" fn collect_monitor(
    monitor: HMONITOR,
    _dc: HDC,
    _rect: *mut RECT,
    data: LPARAM,
) -> BOOL {
    let output = &mut *(data.0 as *mut Vec<Monitor>);
    let mut info = MONITORINFOEXW::default();
    info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
    if GetMonitorInfoW(monitor, &mut info.monitorInfo as *mut MONITORINFO).as_bool() {
        let end = info
            .szDevice
            .iter()
            .position(|c| *c == 0)
            .unwrap_or(info.szDevice.len());
        output.push(Monitor {
            device: ascii(&String::from_utf16_lossy(&info.szDevice[..end])),
            rect: from_rect(info.monitorInfo.rcMonitor),
        });
    }
    BOOL(1)
}

fn monitors() -> Result<Vec<Monitor>, String> {
    let mut output = Vec::new();
    let ok = unsafe {
        EnumDisplayMonitors(
            None,
            None,
            Some(collect_monitor),
            LPARAM(&mut output as *mut _ as isize),
        )
    };
    if ok.as_bool() {
        Ok(output)
    } else {
        Err("EnumDisplayMonitors failed".into())
    }
}

fn process_basename(pid: u32) -> Result<String, String> {
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }
        .map_err(|e| format!("OpenProcess:{:#010x}", e.code().0 as u32))?;
    let result = query_basename(handle);
    let _ = unsafe { CloseHandle(handle) };
    result
}

fn query_basename(handle: HANDLE) -> Result<String, String> {
    let mut buf = vec![0u16; 32768];
    let mut size = buf.len() as u32;
    unsafe {
        QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_FORMAT(0),
            PWSTR(buf.as_mut_ptr()),
            &mut size,
        )
    }
    .map_err(|e| format!("QueryFullProcessImageNameW:{:#010x}", e.code().0 as u32))?;
    let path = OsString::from_wide(&buf[..size as usize]);
    let base = Path::new(&path)
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    Ok(ascii(&base))
}

#[derive(Default)]
struct WindowScan {
    display: Rect,
    sources: Vec<Source>,
    selected_hwnd: Option<HWND>,
    invalid_explicit: bool,
}

unsafe extern "system" fn scan_window(hwnd: HWND, data: LPARAM) -> BOOL {
    let scan = &mut *(data.0 as *mut WindowScan);
    let explicit = scan.selected_hwnd == Some(hwnd);
    if !IsWindowVisible(hwnd).as_bool() {
        return BOOL(1);
    }
    let mut pid = 0u32;
    GetWindowThreadProcessId(hwnd, Some(&mut pid));
    if pid == 0 {
        return BOOL(1);
    }
    let basename = match process_basename(pid) {
        Ok(name) => name,
        Err(_) => {
            if explicit {
                scan.invalid_explicit = true;
            }
            return BOOL(1);
        }
    };
    if !["CiscoCollabHost.exe", "WebexHost.exe"]
        .iter()
        .any(|n| n.eq_ignore_ascii_case(&basename))
    {
        if explicit {
            scan.invalid_explicit = true;
        }
        return BOOL(1);
    }
    let mut raw_rect = RECT::default();
    if GetWindowRect(hwnd, &mut raw_rect).is_err() {
        if explicit {
            scan.invalid_explicit = true;
        }
        return BOOL(1);
    }
    let rect = from_rect(raw_rect);
    if rect.width() <= 50 || rect.height() <= 20 || !intersects(rect, scan.display) {
        if explicit {
            scan.invalid_explicit = true;
        }
        return BOOL(1);
    }
    let mut affinity = 0;
    if GetWindowDisplayAffinity(hwnd, &mut affinity).is_err()
        || affinity != WDA_EXCLUDEFROMCAPTURE.0
    {
        if explicit {
            scan.invalid_explicit = true;
        }
        return BOOL(1);
    }
    scan.sources.push(Source {
        hwnd,
        pid,
        basename,
        rect,
        affinity,
    });
    BOOL(1)
}

fn locate_source(display: &Monitor, explicit: Option<HWND>) -> Result<Source, String> {
    let mut scan = WindowScan {
        display: display.rect,
        selected_hwnd: explicit,
        ..Default::default()
    };
    unsafe { EnumWindows(Some(scan_window), LPARAM(&mut scan as *mut _ as isize)) }
        .map_err(|e| format!("EnumWindows:{:#010x}", e.code().0 as u32))?;
    if explicit.is_some() && scan.invalid_explicit {
        return Err(
            "explicit HWND failed process, visibility, size, monitor, or affinity validation"
                .into(),
        );
    }
    if let Some(hwnd) = explicit {
        return scan
            .sources
            .into_iter()
            .find(|s| s.hwnd == hwnd)
            .ok_or("explicit HWND not found".into());
    }
    scan.sources
        .into_iter()
        .max_by_key(|s| s.rect.area())
        .ok_or("no visible eligible Webex window on selected display".into())
}

fn colorref(rgb: [u8; 3]) -> COLORREF {
    COLORREF(u32::from(rgb[0]) | (u32::from(rgb[1]) << 8) | (u32::from(rgb[2]) << 16))
}

fn rgb_from_colorref(value: COLORREF) -> [u8; 3] {
    [value.0 as u8, (value.0 >> 8) as u8, (value.0 >> 16) as u8]
}

fn color_matches(actual: [u8; 3], expected: [u8; 3]) -> bool {
    actual
        .iter()
        .zip(expected)
        .all(|(a, e)| i16::from(*a).abs_diff(i16::from(e)) <= 28)
}

unsafe extern "system" fn probe_wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_TIMER if wparam.0 == WM_TIMER_LIFETIME => {
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }
        WM_CLOSE => {
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }
        WM_DESTROY => {
            DESTINATION_DESTROYED.store(true, Ordering::Release);
            PostQuitMessage(0);
            LRESULT(0)
        }
        0x000F => {
            let mut ps = PAINTSTRUCT::default();
            let dc = BeginPaint(hwnd, &mut ps);
            let kind = windows::Win32::UI::WindowsAndMessaging::GetWindowLongPtrW(
                hwnd,
                windows::Win32::UI::WindowsAndMessaging::GWLP_USERDATA,
            );
            let rgb = match kind {
                1 => SELF_NONE_RGB,
                2 => SELF_EXCLUDED_RGB,
                _ => DESTINATION_RGB,
            };
            let brush = CreateSolidBrush(colorref(rgb));
            let mut client = RECT::default();
            let _ = windows::Win32::UI::WindowsAndMessaging::GetClientRect(hwnd, &mut client);
            let _ = FillRect(dc, &client, brush);
            let _ = DeleteObject(brush.into());
            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

fn create_window(
    class_name: PCWSTR,
    title: PCWSTR,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
) -> Result<HWND, String> {
    let instance = unsafe { GetModuleHandleW(None) }
        .map_err(|e| format!("GetModuleHandleW:{:#010x}", e.code().0 as u32))?;
    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
            class_name,
            title,
            WS_POPUP,
            x,
            y,
            width,
            height,
            None,
            None,
            Some(instance.into()),
            None,
        )
    }
    .map_err(|e| format!("CreateWindowExW:{:#010x}", e.code().0 as u32))?;
    Ok(hwnd)
}

fn register_class(name: PCWSTR) -> Result<(), String> {
    let instance = unsafe { GetModuleHandleW(None) }
        .map_err(|e| format!("GetModuleHandleW:{:#010x}", e.code().0 as u32))?;
    let class = WNDCLASSW {
        lpfnWndProc: Some(probe_wndproc),
        hInstance: instance.into(),
        lpszClassName: name,
        ..Default::default()
    };
    if unsafe { RegisterClassW(&class) } == 0 {
        return Err("RegisterClassW failed".into());
    }
    Ok(())
}

fn set_userdata(hwnd: HWND, value: isize) {
    unsafe {
        windows::Win32::UI::WindowsAndMessaging::SetWindowLongPtrW(
            hwnd,
            windows::Win32::UI::WindowsAndMessaging::GWLP_USERDATA,
            value,
        );
    }
}

fn set_thumbnail(source: HWND, destination: HWND, rect: RECT) -> Result<isize, String> {
    let thumbnail = unsafe { DwmRegisterThumbnail(destination, source) }
        .map_err(|e| format!("DwmRegisterThumbnail:{:#010x}", e.code().0 as u32))?;
    let props = DWM_THUMBNAIL_PROPERTIES {
        dwFlags: DWM_TNP_RECTDESTINATION | DWM_TNP_VISIBLE | DWM_TNP_SOURCECLIENTAREAONLY,
        rcDestination: rect,
        fVisible: BOOL(1),
        fSourceClientAreaOnly: BOOL(1),
        opacity: 255,
        ..Default::default()
    };
    if let Err(error) = unsafe { DwmUpdateThumbnailProperties(thumbnail, &props) } {
        let _ = unsafe { DwmUnregisterThumbnail(thumbnail) };
        return Err(format!(
            "DwmUpdateThumbnailProperties:{:#010x}",
            error.code().0 as u32
        ));
    }
    Ok(thumbnail)
}

fn query_destination_affinity(hwnd: HWND) -> Result<u32, String> {
    let mut affinity = 0;
    unsafe { GetWindowDisplayAffinity(hwnd, &mut affinity) }
        .map_err(|e| format!("GetWindowDisplayAffinity:{:#010x}", e.code().0 as u32))?;
    Ok(affinity)
}

fn run() -> Result<Report, String> {
    let options = parse_options()?;
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    let all_monitors = monitors()?;
    let source_monitor = all_monitors
        .iter()
        .find(|m| m.device.eq_ignore_ascii_case(&options.source_display))
        .ok_or("selected source display not found")?
        .clone();
    let destination_monitor = all_monitors
        .iter()
        .find(|m| {
            m.device.eq_ignore_ascii_case("\\\\.\\DISPLAY1") && m.device != source_monitor.device
        })
        .or_else(|| {
            all_monitors
                .iter()
                .filter(|m| m.device != source_monitor.device)
                .max_by_key(|m| m.rect.area())
        })
        .ok_or("no distinct destination display available")?
        .clone();

    let mut source_records: Vec<(String, Source)> = Vec::new();
    let mut synthetic_sources = Vec::new();
    let (preview_x, preview_y, preview_width, preview_height) = if options.mode == Mode::SelfTest {
        let x = destination_monitor.rect.left + 24;
        let y = destination_monitor.rect.top + 24;
        register_class(w!("DwmThumbnailProbeSelfSource"))?;
        register_class(w!("DwmThumbnailProbePreview"))?;
        let normal = create_window(
            w!("DwmThumbnailProbeSelfSource"),
            w!(""),
            x,
            y + 280,
            320,
            180,
        )?;
        let excluded = create_window(
            w!("DwmThumbnailProbeSelfSource"),
            w!(""),
            x + 340,
            y + 280,
            320,
            180,
        )?;
        set_userdata(normal, 1);
        set_userdata(excluded, 2);
        unsafe {
            let _ = ShowWindow(normal, SW_SHOWNOACTIVATE);
            let _ = ShowWindow(excluded, SW_SHOWNOACTIVATE);
            let _ = UpdateWindow(normal);
            let _ = UpdateWindow(excluded);
        }
        unsafe {
            if let Err(error) = SetWindowDisplayAffinity(normal, WDA_NONE) {
                let _ = DestroyWindow(normal);
                let _ = DestroyWindow(excluded);
                return Err(format!("Set source NONE:{:#010x}", error.code().0 as u32));
            }
            if let Err(error) = SetWindowDisplayAffinity(excluded, WDA_EXCLUDEFROMCAPTURE) {
                let _ = DestroyWindow(normal);
                let _ = DestroyWindow(excluded);
                return Err(format!(
                    "Set source EXCLUDE:{:#010x}",
                    error.code().0 as u32
                ));
            }
        }
        let normal_read = query_destination_affinity(normal)?;
        let excluded_read = query_destination_affinity(excluded)?;
        if normal_read != WDA_NONE.0 || excluded_read != WDA_EXCLUDEFROMCAPTURE.0 {
            let _ = unsafe { DestroyWindow(normal) };
            let _ = unsafe { DestroyWindow(excluded) };
            return Err("synthetic source affinity did not verify".into());
        }
        synthetic_sources.push(("self_none".to_string(), normal));
        synthetic_sources.push(("self_excluded".to_string(), excluded));
        source_records.push((
            "self_none".to_string(),
            Source {
                hwnd: normal,
                pid: std::process::id(),
                basename: "dwm-thumbnail-probe.exe".into(),
                rect: Rect {
                    left: x,
                    top: y + 280,
                    right: x + 320,
                    bottom: y + 460,
                },
                affinity: normal_read,
            },
        ));
        source_records.push((
            "self_excluded".to_string(),
            Source {
                hwnd: excluded,
                pid: std::process::id(),
                basename: "dwm-thumbnail-probe.exe".into(),
                rect: Rect {
                    left: x + 340,
                    top: y + 280,
                    right: x + 660,
                    bottom: y + 460,
                },
                affinity: excluded_read,
            },
        ));
        (x, y, 720, 232)
    } else {
        let source = locate_source(&source_monitor, options.source_hwnd)?;
        let ratio = source.rect.width().max(1) as f64 / source.rect.height().max(1) as f64;
        let width = 960i32;
        let height = ((width as f64 / ratio).round() as i32).clamp(120, 640);
        let x = destination_monitor.rect.left + 24;
        let y = destination_monitor.rect.top + 24;
        register_class(w!("DwmThumbnailProbePreview"))?;
        source_records.push(("webex_excluded".into(), source));
        (x, y, width, height)
    };

    let (x, y, width, height) = (preview_x, preview_y, preview_width, preview_height);
    let destination =
        match create_window(w!("DwmThumbnailProbePreview"), w!(""), x, y, width, height) {
            Ok(hwnd) => hwnd,
            Err(error) => {
                for (_, hwnd) in synthetic_sources {
                    let _ = unsafe { DestroyWindow(hwnd) };
                }
                return Err(error);
            }
        };
    if options.mode == Mode::SelfTest {
        set_userdata(destination, 3);
    }
    unsafe {
        let _ = ShowWindow(destination, SW_SHOWNOACTIVATE);
        let _ = UpdateWindow(destination);
    }
    let destination_affinity_set = unsafe {
        SetWindowDisplayAffinity(
            destination,
            if options.mode == Mode::Webex {
                WDA_EXCLUDEFROMCAPTURE
            } else {
                WDA_NONE
            },
        )
    }
    .is_ok();
    let expected_affinity = if options.mode == Mode::Webex {
        WDA_EXCLUDEFROMCAPTURE.0
    } else {
        WDA_NONE.0
    };
    let affinity_verified = destination_affinity_set
        && query_destination_affinity(destination).ok() == Some(expected_affinity);
    if !affinity_verified {
        let _ = unsafe { DestroyWindow(destination) };
        for (_, hwnd) in synthetic_sources {
            let _ = unsafe { DestroyWindow(hwnd) };
        }
        return Err(
            "destination affinity set/readback did not verify; foreign thumbnail was not registered".into(),
        );
    }

    let mut thumbnail_ids = Vec::new();
    let mut statuses = Vec::new();
    let inner = RECT {
        left: 8,
        top: 8,
        right: width - 8,
        bottom: height - 8,
    };
    if options.mode == Mode::Webex {
        let (kind, source) = &source_records[0];
        match set_thumbnail(source.hwnd, destination, inner) {
            Ok(id) => {
                thumbnail_ids.push(id);
                statuses.push(ThumbnailStatus {
                    source_kind: kind.clone(),
                    source_basename: source.basename.clone(),
                    source_pid: source.pid,
                    source_rect: source.rect,
                    source_affinity: format!("0x{:08X}", source.affinity),
                    register_status: "success",
                    update_status: "success",
                    preview_visible_requested: true,
                    visible_pixels_confirmed: false,
                });
            }
            Err(error) => {
                let _ = unsafe { DestroyWindow(destination) };
                return Err(error);
            }
        }
    } else {
        for (index, (kind, source)) in source_records.iter().enumerate() {
            let left = if index == 0 { 8 } else { width / 2 };
            let right = if index == 0 { width / 2 - 4 } else { width - 8 };
            let rect = RECT {
                left,
                top: 8,
                right,
                bottom: height - 8,
            };
            match set_thumbnail(source.hwnd, destination, rect) {
                Ok(id) => {
                    thumbnail_ids.push(id);
                    statuses.push(ThumbnailStatus {
                        source_kind: kind.clone(),
                        source_basename: source.basename.clone(),
                        source_pid: source.pid,
                        source_rect: source.rect,
                        source_affinity: format!("0x{:08X}", source.affinity),
                        register_status: "success",
                        update_status: "success",
                        preview_visible_requested: true,
                        visible_pixels_confirmed: false,
                    });
                }
                Err(error) => {
                    for id in thumbnail_ids {
                        let _ = unsafe { DwmUnregisterThumbnail(id) };
                    }
                    let _ = unsafe { DestroyWindow(destination) };
                    for (_, hwnd) in synthetic_sources {
                        let _ = unsafe { DestroyWindow(hwnd) };
                    }
                    return Err(error);
                }
            }
        }
    }

    let sample_timer = if options.mode == Mode::SelfTest {
        unsafe { SetTimer(Some(destination), WM_TIMER_PIXEL_CHECK, 900, None) }
    } else {
        1
    };
    if sample_timer == 0 {
        for id in thumbnail_ids {
            let _ = unsafe { DwmUnregisterThumbnail(id) };
        }
        let _ = unsafe { DestroyWindow(destination) };
        for (_, hwnd) in synthetic_sources {
            let _ = unsafe { DestroyWindow(hwnd) };
        }
        return Err("pixel-check timer failed; thumbnail was not retained".into());
    }
    let timer = unsafe { SetTimer(Some(destination), WM_TIMER_LIFETIME, LIFETIME_MS, None) };
    if timer == 0 {
        if options.mode == Mode::SelfTest {
            let _ = unsafe { KillTimer(Some(destination), WM_TIMER_PIXEL_CHECK) };
        }
        for id in thumbnail_ids {
            let _ = unsafe { DwmUnregisterThumbnail(id) };
        }
        let _ = unsafe { DestroyWindow(destination) };
        for (_, hwnd) in synthetic_sources {
            let _ = unsafe { DestroyWindow(hwnd) };
        }
        return Err("SetTimer failed; thumbnail was not retained".into());
    }
    let mut pixel_check = None;
    let mut checked_pixels = false;
    loop {
        let mut message = windows::Win32::UI::WindowsAndMessaging::MSG::default();
        let got = unsafe { GetMessageW(&mut message, None, 0, 0) };
        if got.0 <= 0 {
            break;
        }
        unsafe {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
        if options.mode == Mode::SelfTest
            && !checked_pixels
            && !DESTINATION_DESTROYED.load(Ordering::Acquire)
            && message.message == WM_TIMER
            && message.wParam.0 == WM_TIMER_PIXEL_CHECK
        {
            checked_pixels = true;
            pixel_check = Some(check_synthetic_pixels(destination));
        }
    }

    for id in thumbnail_ids {
        let _ = unsafe { DwmUnregisterThumbnail(id) };
    }
    let _ = unsafe { KillTimer(Some(destination), WM_TIMER_LIFETIME) };
    if options.mode == Mode::SelfTest {
        let _ = unsafe { KillTimer(Some(destination), WM_TIMER_PIXEL_CHECK) };
    }
    let _ = unsafe { DestroyWindow(destination) };
    for (_, hwnd) in synthetic_sources {
        let _ = unsafe { DestroyWindow(hwnd) };
    }
    let _ = unsafe {
        UnregisterClassW(
            w!("DwmThumbnailProbeSelfSource"),
            GetModuleHandleW(None).ok().map(Into::into),
        )
    };
    let _ = unsafe {
        UnregisterClassW(
            w!("DwmThumbnailProbePreview"),
            GetModuleHandleW(None).ok().map(Into::into),
        )
    };

    Ok(Report {
        mode: if options.mode == Mode::Webex {
            "webex"
        } else {
            "self-test"
        },
        read_only_foreign_windows: true,
        source_monitor: if options.mode == Mode::SelfTest {
            destination_monitor.device.clone()
        } else {
            source_monitor.device
        },
        destination_monitor: destination_monitor.device,
        destination_affinity_set,
        destination_affinity_verified: affinity_verified,
        lifetime_seconds: LIFETIME_MS / 1000,
        thumbnail_status: statuses,
        synthetic_pixel_check: pixel_check,
        note: if options.mode == Mode::Webex {
            "DWM API success requests a thumbnail but does not confirm visible pixels. No Webex settings were changed."
        } else {
            "Pixel readback samples only synthetic probe windows created by this process; no screenshot was saved."
        },
    })
}

fn check_synthetic_pixels(destination: HWND) -> PixelCheck {
    let inconclusive = || PixelCheck {
        normal_source_matches_color: false,
        excluded_source_matches_color: false,
        excluded_source_matches_destination_background: false,
        excluded_source_matches_black: false,
        classification: "inconclusive",
    };
    if !unsafe { IsWindow(Some(destination)).as_bool() }
        || !unsafe { IsWindowVisible(destination).as_bool() }
    {
        return inconclusive();
    }
    let mut window_rect = RECT::default();
    if unsafe { GetWindowRect(destination, &mut window_rect) }.is_err() {
        return inconclusive();
    }
    let rect = from_rect(window_rect);
    let sample_points = [
        POINT {
            x: rect.left + rect.width() / 4,
            y: rect.top + rect.height() / 2,
        },
        POINT {
            x: rect.left + rect.width() * 3 / 4,
            y: rect.top + rect.height() / 2,
        },
    ];
    for point in sample_points {
        if unsafe { WindowFromPoint(point) } != destination {
            return inconclusive();
        }
        let mut owner_pid = 0;
        unsafe {
            GetWindowThreadProcessId(destination, Some(&mut owner_pid));
        }
        if owner_pid != std::process::id() {
            return inconclusive();
        }
    }
    let dc = unsafe { GetDC(None) };
    if dc.is_invalid() {
        return inconclusive();
    }
    let normal = unsafe { GetPixel(dc, sample_points[0].x, sample_points[0].y) };
    let excluded = unsafe { GetPixel(dc, sample_points[1].x, sample_points[1].y) };
    let _ = unsafe { windows::Win32::Graphics::Gdi::ReleaseDC(None, dc) };
    let normal_match = color_matches(rgb_from_colorref(normal), SELF_NONE_RGB);
    let excluded_match = color_matches(rgb_from_colorref(excluded), SELF_EXCLUDED_RGB);
    let excluded_blank = color_matches(rgb_from_colorref(excluded), DESTINATION_RGB);
    let excluded_black = color_matches(rgb_from_colorref(excluded), [0, 0, 0]);
    let classification = if normal_match && excluded_blank {
        "normal_visible_excluded_blank"
    } else if normal_match && excluded_black {
        "normal_visible_excluded_black"
    } else if normal_match && excluded_match {
        "both_visible"
    } else {
        "inconclusive"
    };
    PixelCheck {
        normal_source_matches_color: normal_match,
        excluded_source_matches_color: excluded_match,
        excluded_source_matches_destination_background: excluded_blank,
        excluded_source_matches_black: excluded_black,
        classification,
    }
}

fn main() {
    match run().and_then(|report| serde_json::to_string_pretty(&report).map_err(|e| e.to_string()))
    {
        Ok(json) => println!("{}", ascii(&json)),
        Err(error) => {
            #[derive(Serialize)]
            struct Failure {
                status: &'static str,
                error: String,
                foreign_thumbnail_registered: bool,
            }
            println!(
                "{}",
                serde_json::to_string(&Failure {
                    status: "error",
                    error: ascii(&error),
                    foreign_thumbnail_registered: false
                })
                .unwrap_or_else(|_| "{\"status\":\"error\"}".into())
            );
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn geometry_overlap_and_color_checks_are_bounded() {
        assert!(intersects(
            Rect {
                left: 0,
                top: 0,
                right: 100,
                bottom: 100
            },
            Rect {
                left: 99,
                top: 99,
                right: 200,
                bottom: 200
            }
        ));
        assert!(!intersects(
            Rect {
                left: 0,
                top: 0,
                right: 100,
                bottom: 100
            },
            Rect {
                left: 100,
                top: 0,
                right: 200,
                bottom: 100
            }
        ));
        assert!(color_matches(SELF_NONE_RGB, SELF_NONE_RGB));
        assert!(!color_matches(SELF_NONE_RGB, SELF_EXCLUDED_RGB));
        assert!(color_matches([0, 0, 0], [0, 0, 0]));
    }
}
