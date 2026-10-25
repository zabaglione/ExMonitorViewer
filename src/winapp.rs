use std::ffi::c_void;
use std::io::Write;
use std::mem::size_of;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicIsize, AtomicU32, Ordering};
use std::time::{Duration, Instant};
use windows::core::{w, Error, HRESULT, PCSTR, PCWSTR};
use windows::Win32::Foundation::{
    CloseHandle, HANDLE, HMODULE, HWND, LPARAM, LRESULT, RECT, WAIT_FAILED, WAIT_OBJECT_0, WPARAM,
};
use windows::Win32::Graphics::Direct3D::Fxc::D3DCompile;
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE_UNKNOWN, D3D_FEATURE_LEVEL_11_0, D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST,
};
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::*;
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTOPRIMARY,
};
use windows::Win32::System::Console::{AttachConsole, ATTACH_PARENT_PROCESS};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::SetProcessDpiAwarenessContext;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows_core::Interface;

static APP_HWND: AtomicIsize = AtomicIsize::new(0);
static MONITOR_MENU_HANDLE: AtomicIsize = AtomicIsize::new(0);
static RESIZE_WIDTH: AtomicU32 = AtomicU32::new(960);
static RESIZE_HEIGHT: AtomicU32 = AtomicU32::new(540);
static CURRENT_MONITOR: AtomicU32 = AtomicU32::new(0);
static FPS_LIMIT: AtomicU32 = AtomicU32::new(60);
static TOPMOST: AtomicBool = AtomicBool::new(false);
static FULLSCREEN: AtomicBool = AtomicBool::new(false);
static WINDOW_LEFT: AtomicI32 = AtomicI32::new(100);
static WINDOW_TOP: AtomicI32 = AtomicI32::new(100);
static WINDOW_WIDTH: AtomicI32 = AtomicI32::new(1000);
static WINDOW_HEIGHT: AtomicI32 = AtomicI32::new(600);
static WINDOW_MINIMIZED: AtomicBool = AtomicBool::new(false);
static DISPLAY_CHANGED: AtomicBool = AtomicBool::new(false);
static QUIT: AtomicBool = AtomicBool::new(false);

const MONITOR_MENU_BASE: usize = 1000;
const FPS_MENU_30: usize = 9001;
const FPS_MENU_60: usize = 9002;
const FPS_MENU_MAX: usize = 9003;
const TOPMOST_MENU: usize = 9004;
const DXGI_STATUS_OCCLUDED_CODE: HRESULT = HRESULT(0x087A0001u32 as i32);

fn log(message: &str) {
    let sanitized = message
        .chars()
        .map(|c| if c.is_ascii() { c } else { '?' })
        .collect::<String>();
    println!("{sanitized}");
    if let Ok(executable) = std::env::current_exe() {
        if let Some(directory) = executable.parent() {
            if let Ok(mut file) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(directory.join("exmonitor-viewer.log"))
            {
                let _ = writeln!(file, "{sanitized}");
            }
        }
    }
}

struct Options {
    list_monitors: bool,
    monitor: Option<usize>,
    fps: Option<u32>,
    size: Option<(u32, u32)>,
    benchmark_seconds: Option<u64>,
    report_path: Option<String>,
}

fn parse_options(args: &[String], monitor_count: usize) -> windows::core::Result<Options> {
    let mut options = Options {
        list_monitors: false,
        monitor: None,
        fps: None,
        size: None,
        benchmark_seconds: None,
        report_path: None,
    };
    let mut values = args.iter().skip(1);
    while let Some(argument) = values.next() {
        match argument.as_str() {
            "--list-monitors" => options.list_monitors = true,
            "--monitor" => {
                let number = values
                    .next()
                    .and_then(|value| value.parse::<usize>().ok())
                    .filter(|number| (1..=monitor_count).contains(number))
                    .ok_or_else(invalid_arguments)?;
                options.monitor = Some(number - 1);
            }
            "--fps" => {
                let value = values
                    .next()
                    .and_then(|value| value.parse::<u32>().ok())
                    .filter(|fps| *fps == 0 || *fps == 30 || *fps == 60)
                    .ok_or_else(invalid_arguments)?;
                options.fps = Some(value);
            }
            "--size" => {
                let (width, height) = values
                    .next()
                    .and_then(|value| value.split_once('x'))
                    .and_then(|(width, height)| {
                        Some((width.parse::<u32>().ok()?, height.parse::<u32>().ok()?))
                    })
                    .filter(|(width, height)| {
                        (320..=7680).contains(width) && (240..=4320).contains(height)
                    })
                    .ok_or_else(invalid_arguments)?;
                options.size = Some((width, height));
            }
            "--benchmark" => {
                let seconds = values
                    .next()
                    .and_then(|value| value.parse::<u64>().ok())
                    .filter(|seconds| (1..=3600).contains(seconds))
                    .ok_or_else(invalid_arguments)?;
                options.benchmark_seconds = Some(seconds);
            }
            "--report" => {
                let path = values
                    .next()
                    .filter(|value| !value.trim().is_empty())
                    .ok_or_else(invalid_arguments)?;
                options.report_path = Some(path.clone());
            }
            _ => return Err(invalid_arguments()),
        }
    }
    Ok(options)
}

fn invalid_arguments() -> windows::core::Error {
    Error::from_hresult(HRESULT(0x80070057u32 as i32))
}

pub fn show_error(message: &str) {
    let safe = ascii(message);
    log(&safe);
    unsafe {
        let caption = wide("ExMonitorViewer");
        let body = wide(&safe);
        let _ = MessageBoxW(
            None,
            PCWSTR(body.as_ptr()),
            PCWSTR(caption.as_ptr()),
            MB_ICONERROR | MB_OK,
        );
    }
}

pub fn run() -> windows::core::Result<()> {
    let args = std::env::args().collect::<Vec<_>>();
    if args
        .iter()
        .any(|arg| arg == "--help" || arg == "-h" || arg == "--list-monitors")
    {
        unsafe {
            let _ = AttachConsole(ATTACH_PARENT_PROCESS);
        }
    }
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        println!("ExMonitorViewer - GPU extended-display window viewer");
        println!("Usage: exmonitor-viewer [--list-monitors] [--monitor N] [--fps 30|60|0] [--size WIDTHxHEIGHT] [--benchmark SECONDS] [--report PATH]");
        println!("FPS 0 means unlimited. Monitor numbers are 1-based and match --list-monitors.");
        return Ok(());
    }
    unsafe {
        let _ = SetProcessDpiAwarenessContext(
            windows::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
        );
        let instance = GetModuleHandleW(None)?;
        let displays = enumerate_displays()?;
        if displays.is_empty() {
            return Err(Error::from_win32());
        }

        let options = parse_options(&args, displays.len())?;

        if options.list_monitors {
            for (index, display) in displays.iter().enumerate() {
                println!(
                    "{}: {} ({}x{})",
                    index + 1,
                    display.name,
                    display.width,
                    display.height
                );
            }
            return Ok(());
        }

        let initial = options
            .monitor
            .unwrap_or_else(|| displays.iter().position(|d| !d.primary).unwrap_or(0));
        let current = initial.min(displays.len() - 1);
        CURRENT_MONITOR.store(current as u32, Ordering::Relaxed);
        let fps = options.fps.unwrap_or(60);
        FPS_LIMIT.store(fps, Ordering::Relaxed);
        let benchmark_seconds = options.benchmark_seconds;
        let report_path = options.report_path;
        let (client_width, client_height) = options.size.unwrap_or((960, 540));
        RESIZE_WIDTH.store(client_width, Ordering::Relaxed);
        RESIZE_HEIGHT.store(client_height, Ordering::Relaxed);

        let class = w!("ExMonitorViewerWindow");
        let app_icon = LoadIconW(Some(instance.into()), PCWSTR(101 as *const u16))?;
        let wc = WNDCLASSW {
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            hIcon: app_icon,
            hInstance: instance.into(),
            lpszClassName: class,
            lpfnWndProc: Some(window_proc),
            style: CS_HREDRAW | CS_VREDRAW,
            ..Default::default()
        };
        RegisterClassW(&wc);

        let monitor_menu = CreatePopupMenu()?;
        MONITOR_MENU_HANDLE.store(monitor_menu.0 as isize, Ordering::Relaxed);
        for (index, display) in displays.iter().enumerate() {
            let label = format!(
                "{}: {} ({}x{}){}",
                index + 1,
                display.name,
                display.width,
                display.height,
                if display.primary { " [Primary]" } else { "" }
            );
            let label = wide(&label);
            AppendMenuW(
                monitor_menu,
                MF_STRING,
                MONITOR_MENU_BASE + index,
                PCWSTR(label.as_ptr()),
            )?;
        }
        let fps_menu = CreatePopupMenu()?;
        AppendMenuW(fps_menu, MF_STRING, FPS_MENU_30, w!("30 FPS"))?;
        AppendMenuW(fps_menu, MF_STRING, FPS_MENU_60, w!("60 FPS"))?;
        AppendMenuW(fps_menu, MF_STRING, FPS_MENU_MAX, w!("Unlimited"))?;
        let menu = CreateMenu()?;
        AppendMenuW(menu, MF_POPUP, monitor_menu.0 as usize, w!("Monitor"))?;
        AppendMenuW(menu, MF_POPUP, fps_menu.0 as usize, w!("FPS"))?;
        AppendMenuW(menu, MF_STRING, TOPMOST_MENU, w!("Always on top"))?;

        let mut window_rect = RECT {
            left: 0,
            top: 0,
            right: client_width as i32,
            bottom: client_height as i32,
        };
        AdjustWindowRectEx(&mut window_rect, WS_OVERLAPPEDWINDOW, true, WS_EX_APPWINDOW)?;
        let hwnd = CreateWindowExW(
            WS_EX_APPWINDOW,
            class,
            w!("ExMonitorViewer"),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            displays
                .iter()
                .find(|display| display.primary)
                .map(|display| display.left + 80)
                .unwrap_or(80),
            displays
                .iter()
                .find(|display| display.primary)
                .map(|display| display.top + 80)
                .unwrap_or(80),
            window_rect.right - window_rect.left,
            window_rect.bottom - window_rect.top,
            None,
            Some(menu),
            Some(instance.into()),
            None,
        )?;
        APP_HWND.store(hwnd.0 as isize, Ordering::Relaxed);
        let _ = SendMessageW(
            hwnd,
            WM_SETICON,
            Some(WPARAM(1)),
            Some(LPARAM(app_icon.0 as isize)),
        );
        let _ = SendMessageW(
            hwnd,
            WM_SETICON,
            Some(WPARAM(0)),
            Some(LPARAM(app_icon.0 as isize)),
        );

        let mut renderer = Renderer::new(&displays[current], hwnd)?;
        log("GPU capture renderer initialized");
        let mut message = MSG::default();
        let initial_limit = FPS_LIMIT.load(Ordering::Relaxed);
        let mut next_frame = Instant::now() + frame_period(initial_limit);
        let mut stats_at = Instant::now();
        let mut captured = 0u32;
        let mut presented = 0u32;
        let mut total_captured = 0u64;
        let mut total_presented = 0u64;
        let started = Instant::now();
        let mut present_ready = false;
        let mut occluded = false;

        'main_loop: while !QUIT.load(Ordering::Relaxed) {
            if !pump_messages(&mut message) {
                break;
            }
            if benchmark_seconds
                .is_some_and(|seconds| started.elapsed() >= Duration::from_secs(seconds))
            {
                break;
            }

            let monitor_index = CURRENT_MONITOR.load(Ordering::Relaxed) as usize;
            let user_selected_monitor = monitor_index != renderer.monitor_index;
            let display_changed = DISPLAY_CHANGED.swap(false, Ordering::Relaxed);
            if user_selected_monitor || display_changed {
                renderer.release_held_frame().ok();
                let old_name = renderer.monitor_name.clone();
                renderer.context.ClearState();
                renderer.context.Flush();
                drop(renderer);
                let refreshed = enumerate_displays()?;
                refresh_monitor_menu(&refreshed, hwnd)?;
                let next_index = if user_selected_monitor {
                    monitor_index.min(refreshed.len().saturating_sub(1))
                } else {
                    refreshed
                        .iter()
                        .position(|display| display.name == old_name)
                        .unwrap_or(monitor_index.min(refreshed.len().saturating_sub(1)))
                };
                let next = refreshed.get(next_index).ok_or_else(Error::from_win32)?;
                CURRENT_MONITOR.store(next_index as u32, Ordering::Relaxed);
                renderer = Renderer::new(next, hwnd)?;
                present_ready = false;
                next_frame = Instant::now() + frame_period(FPS_LIMIT.load(Ordering::Relaxed));
                occluded = false;
                log("Capture monitor changed");
            }

            if WINDOW_MINIMIZED.load(Ordering::Relaxed) {
                renderer.release_held_frame().ok();
                wait_for_messages(50);
                continue;
            }

            if occluded {
                renderer.release_held_frame().ok();
                wait_for_messages(100);
                let status = renderer.swapchain.Present(0, DXGI_PRESENT_TEST).0;
                if status == DXGI_STATUS_OCCLUDED_CODE.0 {
                    continue;
                }
                if status < 0 {
                    if is_device_lost(HRESULT(status)) {
                        DISPLAY_CHANGED.store(true, Ordering::Relaxed);
                        continue;
                    }
                    return Err(Error::from_hresult(HRESULT(status)));
                }
                occluded = false;
                present_ready = true;
                next_frame = Instant::now();
                continue;
            }

            let requested_width = RESIZE_WIDTH.load(Ordering::Relaxed);
            let requested_height = RESIZE_HEIGHT.load(Ordering::Relaxed);
            if requested_width > 0
                && requested_height > 0
                && (requested_width != renderer.window_width
                    || requested_height != renderer.window_height)
            {
                if let Err(error) = renderer.resize(requested_width, requested_height) {
                    if is_device_lost(error.code()) {
                        DISPLAY_CHANGED.store(true, Ordering::Relaxed);
                        continue;
                    }
                    return Err(error);
                }
            }

            let limit = FPS_LIMIT.load(Ordering::Relaxed);
            let acquisition_deadline = if limit > 0 {
                next_frame
                    .checked_sub(Duration::from_millis(2))
                    .unwrap_or_else(Instant::now)
            } else {
                Instant::now() + Duration::from_millis(8)
            };
            loop {
                match renderer.acquire_latest_frame() {
                    Ok(true) => {
                        captured += 1;
                        total_captured += 1;
                        break;
                    }
                    Ok(false) => {}
                    Err(error) if is_device_lost(error.code()) => {
                        log(&format!(
                            "Capture reset {:#010X}; recreating renderer",
                            error.code().0 as u32
                        ));
                        renderer.context.ClearState();
                        renderer.release_held_frame().ok();
                        renderer.context.Flush();
                        let old_name = renderer.monitor_name.clone();
                        drop(renderer);
                        let refreshed = enumerate_displays()?;
                        refresh_monitor_menu(&refreshed, hwnd)?;
                        let next_index = refreshed
                            .iter()
                            .position(|display| display.name == old_name)
                            .unwrap_or(monitor_index.min(refreshed.len().saturating_sub(1)));
                        let next = refreshed.get(next_index).ok_or_else(Error::from_win32)?;
                        CURRENT_MONITOR.store(next_index as u32, Ordering::Relaxed);
                        renderer = Renderer::new(next, hwnd)?;
                        present_ready = false;
                        next_frame =
                            Instant::now() + frame_period(FPS_LIMIT.load(Ordering::Relaxed));
                        occluded = false;
                        continue 'main_loop;
                    }
                    Err(error) => return Err(error),
                }
                if Instant::now() >= acquisition_deadline {
                    break;
                }
                if !pump_messages(&mut message) {
                    break 'main_loop;
                }
                if WINDOW_MINIMIZED.load(Ordering::Relaxed)
                    || DISPLAY_CHANGED.load(Ordering::Relaxed)
                    || CURRENT_MONITOR.load(Ordering::Relaxed) as usize != renderer.monitor_index
                    || RESIZE_WIDTH.load(Ordering::Relaxed) != renderer.window_width
                    || RESIZE_HEIGHT.load(Ordering::Relaxed) != renderer.window_height
                {
                    continue 'main_loop;
                }
            }

            while !present_ready || (limit > 0 && Instant::now() < next_frame) {
                let remaining = if limit > 0 {
                    duration_until(next_frame, Instant::now())
                } else {
                    Duration::from_millis(8)
                };
                let timeout = duration_millis(remaining.min(Duration::from_millis(8)));
                let wait = if present_ready {
                    MsgWaitForMultipleObjectsEx(None, timeout, QS_ALLINPUT, MWMO_INPUTAVAILABLE)
                } else {
                    MsgWaitForMultipleObjectsEx(
                        Some(&[renderer.frame_latency_event]),
                        timeout,
                        QS_ALLINPUT,
                        MWMO_INPUTAVAILABLE,
                    )
                };
                if wait == WAIT_FAILED {
                    return Err(Error::from_win32());
                }
                if !present_ready && wait == WAIT_OBJECT_0 {
                    present_ready = true;
                }
                if !pump_messages(&mut message) {
                    break 'main_loop;
                }
                if WINDOW_MINIMIZED.load(Ordering::Relaxed)
                    || DISPLAY_CHANGED.load(Ordering::Relaxed)
                    || CURRENT_MONITOR.load(Ordering::Relaxed) as usize != renderer.monitor_index
                    || RESIZE_WIDTH.load(Ordering::Relaxed) != renderer.window_width
                    || RESIZE_HEIGHT.load(Ordering::Relaxed) != renderer.window_height
                {
                    continue 'main_loop;
                }
            }
            match renderer.render() {
                Ok(true) => {
                    presented += 1;
                    total_presented += 1;
                }
                Ok(false) => {
                    occluded = true;
                    renderer.release_held_frame().ok();
                    continue;
                }
                Err(error) if is_device_lost(error.code()) => {
                    DISPLAY_CHANGED.store(true, Ordering::Relaxed);
                    occluded = false;
                    continue;
                }
                Err(error) => return Err(error),
            }
            present_ready = false;
            if limit > 0 {
                let step = Duration::from_secs_f64(1.0 / f64::from(limit));
                next_frame += step;
                if next_frame < Instant::now() {
                    let behind = Instant::now().duration_since(next_frame);
                    let missed = (behind.as_nanos() / step.as_nanos()) as u32 + 1;
                    next_frame += step * missed;
                }
            }
            if stats_at.elapsed() >= Duration::from_millis(500) {
                let elapsed = stats_at.elapsed().as_secs_f64();
                let last_capture_fps = captured as f64 / elapsed;
                let last_submit_fps = presented as f64 / elapsed;
                captured = 0;
                presented = 0;
                stats_at = Instant::now();
                let title = format!("ExMonitorViewer | Capture {last_capture_fps:.0} FPS | Present submit {last_submit_fps:.0} FPS | {}x{} | {}",
                    renderer.source_width, renderer.source_height, renderer.monitor_name);
                let title = wide(&title);
                SetWindowTextW(hwnd, PCWSTR(title.as_ptr()))?;
            }
        }
        renderer.release_held_frame().ok();
        if let Some(path) =
            report_path.or_else(|| benchmark_seconds.map(|_| "benchmark-report.txt".to_owned()))
        {
            let elapsed = started.elapsed().as_secs_f64().max(0.001);
            let report = format!(
                "monitor={}\ncapture_resolution={}x{}\nclient_resolution={}x{}\nbenchmark_seconds={:.3}\ncapture_fps={:.3}\npresent_submit_fps={:.3}\ncaptured_frames={}\npresent_submits={}\n",
                ascii(&renderer.monitor_name), renderer.source_width, renderer.source_height,
                renderer.window_width, renderer.window_height,
                elapsed, total_captured as f64 / elapsed, total_presented as f64 / elapsed,
                total_captured, total_presented
            );
            std::fs::write(path, report)?;
        }
    }
    Ok(())
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

fn ascii(value: &str) -> String {
    value
        .chars()
        .map(|character| if character.is_ascii() { character } else { '?' })
        .collect()
}

fn is_device_lost(code: HRESULT) -> bool {
    code == DXGI_ERROR_ACCESS_LOST
        || code == DXGI_ERROR_DEVICE_REMOVED
        || code == DXGI_ERROR_DEVICE_RESET
}

fn duration_until(deadline: Instant, now: Instant) -> Duration {
    deadline.saturating_duration_since(now)
}

fn frame_period(limit: u32) -> Duration {
    if limit == 0 {
        Duration::ZERO
    } else {
        Duration::from_secs_f64(1.0 / f64::from(limit))
    }
}

fn duration_millis(duration: Duration) -> u32 {
    duration.as_millis().clamp(1, 1000) as u32
}

unsafe fn wait_for_messages(timeout_ms: u32) {
    let _ = MsgWaitForMultipleObjectsEx(None, timeout_ms, QS_ALLINPUT, MWMO_INPUTAVAILABLE);
}

unsafe fn pump_messages(message: &mut MSG) -> bool {
    while PeekMessageW(message, None, 0, 0, PM_REMOVE).into() {
        if message.message == WM_QUIT {
            QUIT.store(true, Ordering::Relaxed);
            return false;
        }
        let _ = TranslateMessage(message);
        DispatchMessageW(message);
    }
    !QUIT.load(Ordering::Relaxed)
}

impl Drop for Renderer {
    fn drop(&mut self) {
        unsafe {
            if self.frame_held {
                self.duplication.ReleaseFrame().ok();
                self.frame_held = false;
            }
            self.context.ClearState();
            self.context.Flush();
            let _ = CloseHandle(self.frame_latency_event);
        }
    }
}

#[derive(Clone)]
struct Display {
    name: String,
    width: u32,
    height: u32,
    left: i32,
    top: i32,
    rotation: DXGI_MODE_ROTATION,
    primary: bool,
    output: IDXGIOutput,
    adapter: IDXGIAdapter1,
    factory: IDXGIFactory1,
}

unsafe fn enumerate_displays() -> windows::core::Result<Vec<Display>> {
    let factory: IDXGIFactory1 = CreateDXGIFactory1()?;
    let mut displays = Vec::new();
    let mut adapter_index = 0;
    loop {
        let adapter = match factory.EnumAdapters1(adapter_index) {
            Ok(value) => value,
            Err(_) => break,
        };
        let mut output_index = 0;
        loop {
            let output = match adapter.EnumOutputs(output_index) {
                Ok(value) => value,
                Err(_) => break,
            };
            let description = output.GetDesc()?;
            let name = String::from_utf16_lossy(
                &description
                    .DeviceName
                    .iter()
                    .take_while(|value| **value != 0)
                    .copied()
                    .collect::<Vec<_>>(),
            );
            let width = description
                .DesktopCoordinates
                .right
                .saturating_sub(description.DesktopCoordinates.left) as u32;
            let height = description
                .DesktopCoordinates
                .bottom
                .saturating_sub(description.DesktopCoordinates.top) as u32;
            if width > 0 && height > 0 {
                let mut info = MONITORINFO {
                    cbSize: size_of::<MONITORINFO>() as u32,
                    ..Default::default()
                };
                let monitor = description.Monitor;
                let primary =
                    GetMonitorInfoW(monitor, &mut info).as_bool() && info.dwFlags & 1 != 0;
                displays.push(Display {
                    name,
                    width,
                    height,
                    left: description.DesktopCoordinates.left,
                    top: description.DesktopCoordinates.top,
                    rotation: description.Rotation,
                    primary,
                    output,
                    adapter: adapter.clone(),
                    factory: factory.clone(),
                });
            }
            output_index += 1;
        }
        adapter_index += 1;
    }
    displays.sort_by_key(|display| display_number(&display.name));
    Ok(displays)
}

unsafe fn refresh_monitor_menu(displays: &[Display], hwnd: HWND) -> windows::core::Result<()> {
    let menu = HMENU(MONITOR_MENU_HANDLE.load(Ordering::Relaxed) as *mut c_void);
    let count = GetMenuItemCount(Some(menu)).max(0);
    for index in (0..count).rev() {
        DeleteMenu(menu, index as u32, MF_BYPOSITION)?;
    }
    for (index, display) in displays.iter().enumerate() {
        let label = format!(
            "{}: {} ({}x{}){}",
            index + 1,
            display.name,
            display.width,
            display.height,
            if display.primary { " [Primary]" } else { "" }
        );
        let label = wide(&label);
        AppendMenuW(
            menu,
            MF_STRING,
            MONITOR_MENU_BASE + index,
            PCWSTR(label.as_ptr()),
        )?;
    }
    DrawMenuBar(hwnd)
}

fn display_number(name: &str) -> u32 {
    name.rsplit_once("DISPLAY")
        .and_then(|(_, number)| number.parse::<u32>().ok())
        .unwrap_or(u32::MAX)
}

struct Renderer {
    monitor_index: usize,
    monitor_name: String,
    source_width: u32,
    source_height: u32,
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    duplication: IDXGIOutputDuplication,
    capture_texture: ID3D11Texture2D,
    capture_view: ID3D11ShaderResourceView,
    swapchain: IDXGISwapChain1,
    frame_latency_event: HANDLE,
    target_view: Option<ID3D11RenderTargetView>,
    vertex_shader: ID3D11VertexShader,
    pixel_shader: ID3D11PixelShader,
    sampler: ID3D11SamplerState,
    window_width: u32,
    window_height: u32,
    has_frame: bool,
    frame_held: bool,
    rotated: bool,
}

impl Renderer {
    unsafe fn new(display: &Display, hwnd: HWND) -> windows::core::Result<Self> {
        let adapter: IDXGIAdapter = display.adapter.cast()?;
        let mut device = None;
        let mut context = None;
        let mut feature_level = D3D_FEATURE_LEVEL_11_0;
        D3D11CreateDevice(
            Some(&adapter),
            D3D_DRIVER_TYPE_UNKNOWN,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            Some(&[D3D_FEATURE_LEVEL_11_0]),
            D3D11_SDK_VERSION,
            Some(&mut device),
            Some(&mut feature_level),
            Some(&mut context),
        )?;
        let device = device.ok_or_else(Error::from_win32)?;
        let context = context.ok_or_else(Error::from_win32)?;
        let dxgi_device: IDXGIDevice1 = device.cast()?;
        dxgi_device.SetMaximumFrameLatency(1)?;
        let output1: IDXGIOutput1 = display.output.cast()?;
        let duplication = display
            .output
            .cast::<IDXGIOutput5>()
            .and_then(|output5| output5.DuplicateOutput1(&device, 0, &[DXGI_FORMAT_B8G8R8A8_UNORM]))
            .or_else(|_| output1.DuplicateOutput(&device))?;
        let capture_desc = duplication.GetDesc();
        let source_width = capture_desc.ModeDesc.Width;
        let source_height = capture_desc.ModeDesc.Height;
        let texture_desc = D3D11_TEXTURE2D_DESC {
            Width: source_width,
            Height: source_height,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
            CPUAccessFlags: 0,
            MiscFlags: 0,
        };
        let mut capture_texture = None;
        device.CreateTexture2D(&texture_desc, None, Some(&mut capture_texture))?;
        let capture_texture = capture_texture.ok_or_else(Error::from_win32)?;
        let mut capture_view = None;
        device.CreateShaderResourceView(&capture_texture, None, Some(&mut capture_view))?;
        let capture_view = capture_view.ok_or_else(Error::from_win32)?;

        let factory: IDXGIFactory2 = display.factory.cast()?;
        let swap_desc = DXGI_SWAP_CHAIN_DESC1 {
            Width: 960,
            Height: 540,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            Stereo: false.into(),
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
            BufferCount: 2,
            Scaling: DXGI_SCALING_STRETCH,
            SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
            AlphaMode: DXGI_ALPHA_MODE_IGNORE,
            Flags: DXGI_SWAP_CHAIN_FLAG_FRAME_LATENCY_WAITABLE_OBJECT.0 as u32,
        };
        let swapchain = factory.CreateSwapChainForHwnd(&device, hwnd, &swap_desc, None, None)?;
        let swapchain2: IDXGISwapChain2 = swapchain.cast()?;
        swapchain2.SetMaximumFrameLatency(1)?;
        let frame_latency_event = swapchain2.GetFrameLatencyWaitableObject();
        if frame_latency_event.0.is_null() {
            return Err(Error::from_win32());
        }
        let target_view = make_target_view(&device, &swapchain)?;
        let rotation = match display.rotation {
            DXGI_MODE_ROTATION_ROTATE90 => 1,
            DXGI_MODE_ROTATION_ROTATE180 => 2,
            DXGI_MODE_ROTATION_ROTATE270 => 3,
            _ => 0,
        };
        let pixel_source = PIXEL_SHADER.replace("ROTATION_INDEX", &rotation.to_string());
        let (vertex_shader, pixel_shader) = make_shaders(&device, &pixel_source)?;
        let sampler_desc = D3D11_SAMPLER_DESC {
            Filter: D3D11_FILTER_MIN_MAG_MIP_LINEAR,
            AddressU: D3D11_TEXTURE_ADDRESS_CLAMP,
            AddressV: D3D11_TEXTURE_ADDRESS_CLAMP,
            AddressW: D3D11_TEXTURE_ADDRESS_CLAMP,
            MipLODBias: 0.0,
            MaxAnisotropy: 1,
            ComparisonFunc: D3D11_COMPARISON_NEVER,
            BorderColor: [0.0; 4],
            MinLOD: 0.0,
            MaxLOD: f32::MAX,
        };
        let mut sampler = None;
        device.CreateSamplerState(&sampler_desc, Some(&mut sampler))?;
        let sampler = sampler.ok_or_else(Error::from_win32)?;
        Ok(Self {
            monitor_index: CURRENT_MONITOR.load(Ordering::Relaxed) as usize,
            monitor_name: display.name.clone(),
            source_width,
            source_height,
            device,
            context,
            duplication,
            capture_texture,
            capture_view,
            swapchain,
            frame_latency_event,
            target_view: Some(target_view),
            vertex_shader,
            pixel_shader,
            sampler,
            window_width: 960,
            window_height: 540,
            has_frame: false,
            frame_held: false,
            rotated: rotation == 1 || rotation == 3,
        })
    }

    unsafe fn acquire_latest_frame(&mut self) -> windows::core::Result<bool> {
        if self.frame_held {
            self.duplication.ReleaseFrame()?;
            self.frame_held = false;
        }
        let mut frame_info = DXGI_OUTDUPL_FRAME_INFO::default();
        let mut resource = None;
        match self
            .duplication
            .AcquireNextFrame(2, &mut frame_info, &mut resource)
        {
            Ok(()) => {
                let frame_result = (|| -> windows::core::Result<()> {
                    let resource = resource.ok_or_else(Error::from_win32)?;
                    let texture: ID3D11Texture2D = resource.cast()?;
                    self.context.CopyResource(&self.capture_texture, &texture);
                    Ok(())
                })();
                if let Err(error) = frame_result {
                    self.duplication.ReleaseFrame().ok();
                    return Err(error);
                }
                self.frame_held = true;
                self.has_frame = true;
                Ok(frame_info.LastPresentTime != 0)
            }
            Err(error) if error.code() == DXGI_ERROR_WAIT_TIMEOUT => Ok(false),
            Err(error) => Err(error),
        }
    }

    unsafe fn render(&self) -> windows::core::Result<bool> {
        let fitted = crate::math::fit_rect(
            if self.rotated {
                self.source_height as f32
            } else {
                self.source_width as f32
            },
            if self.rotated {
                self.source_width as f32
            } else {
                self.source_height as f32
            },
            self.window_width as f32,
            self.window_height as f32,
        );
        let viewport = D3D11_VIEWPORT {
            TopLeftX: fitted.x,
            TopLeftY: fitted.y,
            Width: fitted.width,
            Height: fitted.height,
            MinDepth: 0.0,
            MaxDepth: 1.0,
        };
        let target_view = self.target_view.as_ref().ok_or_else(Error::from_win32)?;
        self.context
            .OMSetRenderTargets(Some(&[Some(target_view.clone())]), None);
        self.context.RSSetViewports(Some(&[viewport]));
        self.context
            .ClearRenderTargetView(target_view, &[0.0, 0.0, 0.0, 1.0]);
        if self.has_frame {
            self.context
                .IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            self.context.VSSetShader(&self.vertex_shader, None);
            self.context.PSSetShader(&self.pixel_shader, None);
            self.context
                .PSSetShaderResources(0, Some(&[Some(self.capture_view.clone())]));
            self.context
                .PSSetSamplers(0, Some(&[Some(self.sampler.clone())]));
            self.context.Draw(3, 0);
            self.context.PSSetShaderResources(0, Some(&[None]));
        }
        let result = self.swapchain.Present(0, DXGI_PRESENT(0));
        match result.0 {
            0 => Ok(true),
            status if status == DXGI_STATUS_OCCLUDED_CODE.0 => Ok(false),
            _ => result.ok().map(|()| true),
        }
    }

    unsafe fn resize(&mut self, width: u32, height: u32) -> windows::core::Result<()> {
        self.context
            .OMSetRenderTargets(None, None::<&ID3D11DepthStencilView>);
        self.target_view = None;
        self.swapchain.ResizeBuffers(
            0,
            width,
            height,
            DXGI_FORMAT_UNKNOWN,
            DXGI_SWAP_CHAIN_FLAG_FRAME_LATENCY_WAITABLE_OBJECT,
        )?;
        self.target_view = Some(make_target_view(&self.device, &self.swapchain)?);
        self.window_width = width;
        self.window_height = height;
        Ok(())
    }

    unsafe fn release_held_frame(&mut self) -> windows::core::Result<()> {
        if self.frame_held {
            self.duplication.ReleaseFrame()?;
            self.frame_held = false;
        }
        Ok(())
    }
}

unsafe fn make_target_view(
    device: &ID3D11Device,
    swapchain: &IDXGISwapChain1,
) -> windows::core::Result<ID3D11RenderTargetView> {
    let buffer: ID3D11Texture2D = swapchain.GetBuffer(0)?;
    let mut target = None;
    device.CreateRenderTargetView(&buffer, None, Some(&mut target))?;
    target.ok_or_else(Error::from_win32)
}

unsafe fn compile_shader(
    source: &str,
    entry: PCSTR,
    target: PCSTR,
) -> windows::core::Result<windows::Win32::Graphics::Direct3D::ID3DBlob> {
    let source = source.as_bytes();
    let mut bytecode = None;
    D3DCompile(
        source.as_ptr() as *const c_void,
        source.len(),
        PCSTR::null(),
        None,
        None,
        entry,
        target,
        0,
        0,
        &mut bytecode,
        None,
    )?;
    bytecode.ok_or_else(Error::from_win32)
}

unsafe fn make_shaders(
    device: &ID3D11Device,
    pixel_source: &str,
) -> windows::core::Result<(ID3D11VertexShader, ID3D11PixelShader)> {
    let vertex = compile_shader(
        VERTEX_SHADER,
        PCSTR(c"vs_main".as_ptr() as *const u8),
        PCSTR(c"vs_4_0".as_ptr() as *const u8),
    )?;
    let pixel = compile_shader(
        pixel_source,
        PCSTR(c"ps_main".as_ptr() as *const u8),
        PCSTR(c"ps_4_0".as_ptr() as *const u8),
    )?;
    let mut vertex_shader = None;
    device.CreateVertexShader(
        std::slice::from_raw_parts(
            vertex.GetBufferPointer() as *const u8,
            vertex.GetBufferSize(),
        ),
        None,
        Some(&mut vertex_shader),
    )?;
    let mut pixel_shader = None;
    device.CreatePixelShader(
        std::slice::from_raw_parts(pixel.GetBufferPointer() as *const u8, pixel.GetBufferSize()),
        None,
        Some(&mut pixel_shader),
    )?;
    Ok((
        vertex_shader.ok_or_else(Error::from_win32)?,
        pixel_shader.ok_or_else(Error::from_win32)?,
    ))
}

const VERTEX_SHADER: &str = r#"
struct Out { float4 position : SV_POSITION; float2 uv : TEXCOORD0; };
Out vs_main(uint id : SV_VertexID) {
    float2 p = float2((id << 1) & 2, id & 2);
    Out o;
    o.position = float4(p.x * 2.0 - 1.0, 1.0 - p.y * 2.0, 0.0, 1.0);
    o.uv = p;
    return o;
}
"#;

const PIXEL_SHADER: &str = r#"
Texture2D screen_image : register(t0);
SamplerState screen_sampler : register(s0);
#define ROTATION ROTATION_INDEX
float4 ps_main(float4 position : SV_POSITION, float2 uv : TEXCOORD0) : SV_TARGET {
    float2 sample_uv = uv;
#if ROTATION == 1
    sample_uv = float2(uv.y, 1.0 - uv.x);
#elif ROTATION == 2
    sample_uv = float2(1.0 - uv.x, 1.0 - uv.y);
#elif ROTATION == 3
    sample_uv = float2(1.0 - uv.y, uv.x);
#endif
    return screen_image.Sample(screen_sampler, sample_uv);
}
"#;

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_SIZE => {
            let minimized = wparam.0 == SIZE_MINIMIZED as usize;
            WINDOW_MINIMIZED.store(minimized, Ordering::Relaxed);
            if !minimized {
                RESIZE_WIDTH.store((lparam.0 as u32 & 0xffff).max(1), Ordering::Relaxed);
                RESIZE_HEIGHT.store(((lparam.0 as u32 >> 16) & 0xffff).max(1), Ordering::Relaxed);
            }
            LRESULT(0)
        }
        WM_COMMAND => {
            let id = wparam.0 & 0xffff;
            match id {
                MONITOR_MENU_BASE..=2000 => {
                    CURRENT_MONITOR.store((id - MONITOR_MENU_BASE) as u32, Ordering::Relaxed)
                }
                FPS_MENU_30 => FPS_LIMIT.store(30, Ordering::Relaxed),
                FPS_MENU_60 => FPS_LIMIT.store(60, Ordering::Relaxed),
                FPS_MENU_MAX => FPS_LIMIT.store(0, Ordering::Relaxed),
                TOPMOST_MENU => {
                    let enabled = !TOPMOST.load(Ordering::Relaxed);
                    TOPMOST.store(enabled, Ordering::Relaxed);
                    let _ = SetWindowPos(
                        hwnd,
                        Some(if enabled {
                            HWND_TOPMOST
                        } else {
                            HWND_NOTOPMOST
                        }),
                        0,
                        0,
                        0,
                        0,
                        SWP_NOMOVE | SWP_NOSIZE,
                    );
                }
                _ => {}
            }
            LRESULT(0)
        }
        WM_KEYDOWN => {
            let key = wparam.0 as u32;
            match key {
                0x7a => toggle_fullscreen(hwnd),
                0x31..=0x39 => CURRENT_MONITOR.store(key - 0x31, Ordering::Relaxed),
                _ => {}
            }
            LRESULT(0)
        }
        WM_DISPLAYCHANGE => {
            DISPLAY_CHANGED.store(true, Ordering::Relaxed);
            LRESULT(0)
        }
        WM_DESTROY => {
            QUIT.store(true, Ordering::Relaxed);
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, message, wparam, lparam),
    }
}

unsafe fn toggle_fullscreen(hwnd: HWND) {
    let enabled = !FULLSCREEN.load(Ordering::Relaxed);
    FULLSCREEN.store(enabled, Ordering::Relaxed);
    if enabled {
        let mut current = RECT::default();
        if GetWindowRect(hwnd, &mut current).is_ok() {
            WINDOW_LEFT.store(current.left, Ordering::Relaxed);
            WINDOW_TOP.store(current.top, Ordering::Relaxed);
            WINDOW_WIDTH.store(current.right - current.left, Ordering::Relaxed);
            WINDOW_HEIGHT.store(current.bottom - current.top, Ordering::Relaxed);
        }
        let monitor = MonitorFromPoint(
            windows::Win32::Foundation::POINT { x: 0, y: 0 },
            MONITOR_DEFAULTTOPRIMARY,
        );
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if GetMonitorInfoW(monitor, &mut info).as_bool() {
            SetWindowLongPtrW(hwnd, GWL_STYLE, (WS_POPUP | WS_VISIBLE).0 as isize);
            let _ = SetWindowPos(
                hwnd,
                Some(if TOPMOST.load(Ordering::Relaxed) {
                    HWND_TOPMOST
                } else {
                    HWND_TOP
                }),
                info.rcMonitor.left,
                info.rcMonitor.top,
                info.rcMonitor.right - info.rcMonitor.left,
                info.rcMonitor.bottom - info.rcMonitor.top,
                SWP_FRAMECHANGED | SWP_SHOWWINDOW,
            );
        }
    } else {
        SetWindowLongPtrW(
            hwnd,
            GWL_STYLE,
            (WS_OVERLAPPEDWINDOW | WS_VISIBLE).0 as isize,
        );
        let _ = SetWindowPos(
            hwnd,
            Some(if TOPMOST.load(Ordering::Relaxed) {
                HWND_TOPMOST
            } else {
                HWND_NOTOPMOST
            }),
            WINDOW_LEFT.load(Ordering::Relaxed),
            WINDOW_TOP.load(Ordering::Relaxed),
            WINDOW_WIDTH.load(Ordering::Relaxed),
            WINDOW_HEIGHT.load(Ordering::Relaxed),
            SWP_FRAMECHANGED | SWP_SHOWWINDOW,
        );
    }
}
