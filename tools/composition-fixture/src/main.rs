use std::env;
use std::mem::size_of;
use windows::core::{w, BOOL, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateSolidBrush, DeleteObject, DrawTextW, EndPaint, EnumDisplayMonitors, FillRect,
    GetMonitorInfoW, UpdateWindow, DT_CENTER, DT_SINGLELINE, DT_VCENTER, HDC, HMONITOR,
    MONITORINFO, MONITORINFOEXW,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW, GetWindowRect,
    PostQuitMessage, RegisterClassW, SetTimer, SetWindowDisplayAffinity, SetWindowPos, ShowWindow,
    TranslateMessage, HWND_NOTOPMOST, HWND_TOPMOST, SWP_NOACTIVATE, SWP_SHOWWINDOW,
    SW_SHOWNOACTIVATE, WDA_EXCLUDEFROMCAPTURE, WDA_NONE, WM_DESTROY, WM_PAINT, WM_TIMER, WNDCLASSW,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_POPUP,
};

const MARKER_PROPERTY: PCWSTR = w!("ExMonitorViewer.CompositionOverlay");
const LIFETIME_SECONDS: u32 = 30;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Protected,
    Unprotected,
    NotTopmost,
    OtherMonitor,
    Marked,
    LegacyClass,
}

#[derive(Debug)]
struct Options {
    mode: Mode,
    display: String,
    seconds: u32,
}

#[derive(Clone, Debug)]
struct Monitor {
    device: String,
    rect: RECT,
}

fn parse_options(args: impl IntoIterator<Item = String>) -> Result<Options, String> {
    let mut mode = Mode::Protected;
    let mut display = "\\\\.\\DISPLAY2".to_owned();
    let mut seconds = LIFETIME_SECONDS;
    let mut args = args.into_iter();
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--mode" => {
                mode = match args.next().as_deref() {
                    Some("protected") => Mode::Protected,
                    Some("unprotected") => Mode::Unprotected,
                    Some("not-topmost") => Mode::NotTopmost,
                    Some("other-monitor") => Mode::OtherMonitor,
                    Some("marked") => Mode::Marked,
                    Some("legacy-class") => Mode::LegacyClass,
                    _ => return Err("invalid mode".into()),
                };
            }
            "--display" => {
                display = args.next().ok_or("--display requires a device name")?;
                if !display.starts_with("\\\\.\\DISPLAY") || display.len() > 32 {
                    return Err("--display must name a Windows display device".into());
                }
            }
            "--seconds" => {
                seconds = args
                    .next()
                    .and_then(|value| value.parse::<u32>().ok())
                    .filter(|value| (5..=120).contains(value))
                    .ok_or("--seconds must be between 5 and 120")?;
            }
            "--help" | "-h" => {
                return Err("usage: composition-fixture [--mode protected|unprotected|not-topmost|other-monitor|marked|legacy-class] [--display \\\\.\\DISPLAY2] [--seconds 5..120]".into());
            }
            _ => return Err("unknown option".into()),
        }
    }
    Ok(Options {
        mode,
        display,
        seconds,
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
    info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
    if GetMonitorInfoW(monitor, &mut info.monitorInfo as *mut MONITORINFO).as_bool() {
        let end = info
            .szDevice
            .iter()
            .position(|value| *value == 0)
            .unwrap_or(info.szDevice.len());
        output.push(Monitor {
            device: String::from_utf16_lossy(&info.szDevice[..end]),
            rect: info.monitorInfo.rcMonitor,
        });
    }
    BOOL(1)
}

fn enumerate_monitors() -> Result<Vec<Monitor>, String> {
    let mut monitors = Vec::new();
    let succeeded = unsafe {
        EnumDisplayMonitors(
            None,
            None,
            Some(collect_monitor),
            LPARAM(&mut monitors as *mut _ as isize),
        )
        .as_bool()
    };
    if succeeded && !monitors.is_empty() {
        Ok(monitors)
    } else {
        Err("display enumeration failed".into())
    }
}

fn ascii(value: &str) -> String {
    value
        .chars()
        .map(|character| if character.is_ascii() { character } else { '?' })
        .collect()
}

unsafe extern "system" fn fixture_window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_PAINT => {
            let mut paint = windows::Win32::Graphics::Gdi::PAINTSTRUCT::default();
            let dc = BeginPaint(hwnd, &mut paint);
            let mut client = RECT::default();
            let _ = windows::Win32::UI::WindowsAndMessaging::GetClientRect(hwnd, &mut client);
            let dark = CreateSolidBrush(windows::Win32::Foundation::COLORREF(0x001b3d55));
            let bright = CreateSolidBrush(windows::Win32::Foundation::COLORREF(0x00d723ae));
            let _ = FillRect(dc, &client, dark);
            let stripe = RECT {
                left: 0,
                top: 0,
                right: client.right,
                bottom: (client.bottom / 3).max(1),
            };
            let _ = FillRect(dc, &stripe, bright);
            let mut text: Vec<u16> = "TEST FIXTURE - SYNTHETIC WINDOW".encode_utf16().collect();
            let _ = DrawTextW(
                dc,
                text.as_mut_slice(),
                &mut client,
                DT_CENTER | DT_VCENTER | DT_SINGLELINE,
            );
            let _ = DeleteObject(dark.into());
            let _ = DeleteObject(bright.into());
            let _ = EndPaint(hwnd, &paint);
            LRESULT(0)
        }
        WM_TIMER => {
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, message, wparam, lparam),
    }
}

fn run() -> Result<(), String> {
    let options = parse_options(env::args().skip(1))?;
    let mut monitors = enumerate_monitors()?;
    let selected_index = monitors
        .iter()
        .position(|monitor| monitor.device.eq_ignore_ascii_case(&options.display))
        .ok_or("selected display is not present")?;
    let monitor_index = if options.mode == Mode::OtherMonitor {
        (0..monitors.len())
            .find(|index| *index != selected_index)
            .ok_or("other-monitor mode requires at least two displays")?
    } else {
        selected_index
    };
    let monitor = monitors.swap_remove(monitor_index);
    let window_width = 480i32.min(monitor.rect.right - monitor.rect.left).max(100);
    let window_height = 180i32.min(monitor.rect.bottom - monitor.rect.top).max(60);
    let x = monitor.rect.left + 80;
    let y = monitor.rect.top + 80;
    let instance = unsafe { GetModuleHandleW(None) }
        .map_err(|error| format!("GetModuleHandleW:{:#010x}", error.code().0 as u32))?;
    let class_name = if options.mode == Mode::LegacyClass {
        w!("ExMonitorViewerWebexOverlay")
    } else {
        w!("CompositionFixtureWindow")
    };
    let class = WNDCLASSW {
        hInstance: instance.into(),
        lpszClassName: class_name,
        lpfnWndProc: Some(fixture_window_proc),
        ..Default::default()
    };
    unsafe {
        if RegisterClassW(&class) == 0 {
            return Err("RegisterClassW failed".into());
        }
    }
    let title = w!("TEST FIXTURE - synthetic composition source");
    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
            class_name,
            title,
            WS_POPUP,
            x,
            y,
            window_width,
            window_height,
            None,
            None,
            Some(instance.into()),
            None,
        )
    }
    .map_err(|error| format!("CreateWindowExW:{:#010x}", error.code().0 as u32))?;

    let affinity = if options.mode == Mode::Unprotected {
        WDA_NONE
    } else {
        WDA_EXCLUDEFROMCAPTURE
    };
    let marker_set = if options.mode == Mode::Marked {
        unsafe {
            windows::Win32::UI::WindowsAndMessaging::SetPropW(
                hwnd,
                MARKER_PROPERTY,
                Some(windows::Win32::Foundation::HANDLE(
                    std::ptr::dangling_mut::<core::ffi::c_void>(),
                )),
            )
        }
        .is_ok()
    } else {
        false
    };
    if options.mode == Mode::Marked && !marker_set {
        unsafe {
            let _ = DestroyWindow(hwnd);
        }
        return Err("marker property setup failed".into());
    }
    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        let _ = UpdateWindow(hwnd);
    }
    if let Err(error) = unsafe { SetWindowDisplayAffinity(hwnd, affinity) } {
        unsafe {
            let _ = DestroyWindow(hwnd);
        }
        return Err(format!(
            "SetWindowDisplayAffinity:{:#010x}",
            error.code().0 as u32
        ));
    }
    let z_order = if options.mode == Mode::NotTopmost {
        HWND_NOTOPMOST
    } else {
        HWND_TOPMOST
    };
    unsafe {
        if let Err(error) = SetWindowPos(
            hwnd,
            Some(z_order),
            x,
            y,
            window_width,
            window_height,
            SWP_NOACTIVATE | SWP_SHOWWINDOW,
        ) {
            let _ = DestroyWindow(hwnd);
            return Err(format!("SetWindowPos:{:#010x}", error.code().0 as u32));
        }
        let _ = UpdateWindow(hwnd);
    }
    let mut actual_affinity = 0u32;
    unsafe {
        windows::Win32::UI::WindowsAndMessaging::GetWindowDisplayAffinity(
            hwnd,
            &mut actual_affinity,
        )
    }
    .map_err(|error| format!("GetWindowDisplayAffinity:{:#010x}", error.code().0 as u32))?;
    if actual_affinity != affinity.0 {
        unsafe {
            let _ = DestroyWindow(hwnd);
        }
        return Err("fixture display affinity did not match the requested mode".into());
    }
    let mut actual_rect = RECT::default();
    unsafe { GetWindowRect(hwnd, &mut actual_rect) }
        .map_err(|error| format!("GetWindowRect:{:#010x}", error.code().0 as u32))?;
    let topmost = unsafe {
        windows::Win32::UI::WindowsAndMessaging::GetWindowLongPtrW(
            hwnd,
            windows::Win32::UI::WindowsAndMessaging::GWL_EXSTYLE,
        ) as u32
            & windows::Win32::UI::WindowsAndMessaging::WS_EX_TOPMOST.0
            != 0
    };
    println!(
        "mode={} display={} rect={},{},{},{} topmost={} affinity=0x{:02X} marker={} lifetime_seconds={}",
        mode_name(options.mode),
        ascii(&monitor.device),
        actual_rect.left,
        actual_rect.top,
        actual_rect.right,
        actual_rect.bottom,
        u8::from(topmost),
        actual_affinity,
        u8::from(marker_set),
        options.seconds
    );
    unsafe {
        if SetTimer(Some(hwnd), 1, options.seconds * 1000, None) == 0 {
            let _ = DestroyWindow(hwnd);
            return Err("SetTimer failed".into());
        }
        let mut message = windows::Win32::UI::WindowsAndMessaging::MSG::default();
        loop {
            let result = GetMessageW(&mut message, None, 0, 0).0;
            if result <= 0 {
                break;
            }
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
    Ok(())
}

fn mode_name(mode: Mode) -> &'static str {
    match mode {
        Mode::Protected => "protected",
        Mode::Unprotected => "unprotected",
        Mode::NotTopmost => "not_topmost",
        Mode::OtherMonitor => "other_monitor",
        Mode::Marked => "marked",
        Mode::LegacyClass => "legacy_class",
    }
}

fn main() {
    if let Err(error) = run() {
        eprintln!("ERROR: {}", ascii(&error));
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_protected_non_webex_fixture_and_negative_modes() {
        for (name, expected) in [
            ("protected", Mode::Protected),
            ("unprotected", Mode::Unprotected),
            ("not-topmost", Mode::NotTopmost),
            ("other-monitor", Mode::OtherMonitor),
            ("marked", Mode::Marked),
            ("legacy-class", Mode::LegacyClass),
        ] {
            let options = parse_options([
                "--mode".to_owned(),
                name.to_owned(),
                "--display".to_owned(),
                "\\\\.\\DISPLAY2".to_owned(),
            ])
            .unwrap();
            assert_eq!(options.mode, expected);
        }
    }

    #[test]
    fn fixture_lifetime_is_bounded() {
        assert!(parse_options(["--seconds".to_owned(), "4".to_owned()]).is_err());
        assert!(parse_options(["--seconds".to_owned(), "121".to_owned()]).is_err());
    }
}
