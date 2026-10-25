use crate::cursor::{decode_shape, DecodedShape};
use crate::settings::{self, Settings, WindowSettings};
use crate::update::{self, CheckKind, ReleaseInfo, UpdateCheck};
use crate::win32_cursor;
use std::collections::HashMap;
use std::ffi::c_void;
use std::io::Write;
use std::mem::size_of;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicIsize, AtomicU32, Ordering};
use std::time::{Duration, Instant};
use windows::core::{w, Error, BOOL, HRESULT, PCSTR, PCWSTR};
use windows::Win32::Foundation::{
    CloseHandle, HANDLE, HMODULE, HWND, LPARAM, LRESULT, RECT, WAIT_FAILED, WAIT_OBJECT_0, WPARAM,
};
use windows::Win32::Graphics::Direct3D::Fxc::D3DCompile;
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE_UNKNOWN, D3D_FEATURE_LEVEL_11_0, D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST,
};
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::DirectComposition::{
    DCompositionCreateDevice, IDCompositionDevice, IDCompositionTarget, IDCompositionVisual,
};
use windows::Win32::Graphics::Dwm::{
    DwmGetWindowAttribute, DwmRegisterThumbnail, DwmUnregisterThumbnail,
    DwmUpdateThumbnailProperties, DWMWA_CLOAKED, DWM_THUMBNAIL_PROPERTIES, DWM_TNP_RECTDESTINATION,
    DWM_TNP_RECTSOURCE, DWM_TNP_SOURCECLIENTAREAONLY, DWM_TNP_VISIBLE,
};
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::*;
use windows::Win32::Graphics::Gdi::{
    ClientToScreen, GetMonitorInfoW, MonitorFromPoint, MonitorFromWindow, UpdateWindow,
    MONITORINFO, MONITOR_DEFAULTTONEAREST, MONITOR_DEFAULTTOPRIMARY,
};
use windows::Win32::System::Console::{AttachConsole, ATTACH_PARENT_PROCESS};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{GetCurrentProcessId, GetCurrentThreadId};
use windows::Win32::UI::HiDpi::SetProcessDpiAwarenessContext;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows_core::Interface;

static APP_HWND: AtomicIsize = AtomicIsize::new(0);
static MONITOR_MENU_HANDLE: AtomicIsize = AtomicIsize::new(0);
static SCALE_MENU_HANDLE: AtomicIsize = AtomicIsize::new(0);
static RESIZE_WIDTH: AtomicU32 = AtomicU32::new(960);
static RESIZE_HEIGHT: AtomicU32 = AtomicU32::new(540);
static SOURCE_WIDTH: AtomicU32 = AtomicU32::new(0);
static SOURCE_HEIGHT: AtomicU32 = AtomicU32::new(0);
static CURRENT_MONITOR: AtomicU32 = AtomicU32::new(0);
static FPS_LIMIT: AtomicU32 = AtomicU32::new(60);
static TOPMOST: AtomicBool = AtomicBool::new(false);
static FULLSCREEN: AtomicBool = AtomicBool::new(false);
static WINDOW_LEFT: AtomicI32 = AtomicI32::new(100);
static WINDOW_TOP: AtomicI32 = AtomicI32::new(100);
static WINDOW_WIDTH: AtomicI32 = AtomicI32::new(1000);
static WINDOW_HEIGHT: AtomicI32 = AtomicI32::new(600);
static WINDOW_MINIMIZED: AtomicBool = AtomicBool::new(false);
static WINDOW_MAXIMIZED: AtomicBool = AtomicBool::new(false);
static SETTINGS_TRACKING: AtomicBool = AtomicBool::new(false);
static SETTINGS_DIRTY: AtomicBool = AtomicBool::new(false);
static ACTIVE_SCALE: AtomicU32 = AtomicU32::new(0);
static LAST_SIZE_MODE: AtomicU32 = AtomicU32::new(0);
static ACTUAL_SCALE: AtomicU32 = AtomicU32::new(0);
static WINDOW_CLOSE_LOGGED: AtomicBool = AtomicBool::new(false);
static DISPLAY_CHANGED: AtomicBool = AtomicBool::new(false);
static QUIT: AtomicBool = AtomicBool::new(false);
static WEBEX_CONTROLS: AtomicBool = AtomicBool::new(false);
static CHECK_UPDATES_ON_STARTUP: AtomicBool = AtomicBool::new(true);
static MANUAL_UPDATE_REQUESTED: AtomicBool = AtomicBool::new(false);
static WEBEX_OVERLAYS_SUSPENDED: AtomicBool = AtomicBool::new(false);
static WEBEX_STATE_EPOCH: AtomicU32 = AtomicU32::new(0);
static WEBEX_MENU_ENTER_COUNT: AtomicU32 = AtomicU32::new(0);
static WEBEX_MENU_EXIT_COUNT: AtomicU32 = AtomicU32::new(0);
static WEBEX_MENU_TOGGLE_COUNT: AtomicU32 = AtomicU32::new(0);
static WEBEX_MENU_GUI_FLAGS: AtomicU32 = AtomicU32::new(0);
static WEBEX_MENU_RECHECK_ATTEMPTS: AtomicU32 = AtomicU32::new(0);
static IN_SIZE_MOVE: AtomicBool = AtomicBool::new(false);
static WEBEX_CURSOR_BENCHMARK: AtomicBool = AtomicBool::new(false);
static WEBEX_CURSOR_BENCHMARK_STARTED: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();

const MONITOR_MENU_BASE: usize = 1000;
const FPS_MENU_30: usize = 9001;
const FPS_MENU_60: usize = 9002;
const FPS_MENU_MAX: usize = 9003;
const TOPMOST_MENU: usize = 9004;
const WEBEX_CONTROLS_MENU: usize = 9005;
const CHECK_UPDATES_MENU: usize = 9006;
const CHECK_UPDATES_STARTUP_MENU: usize = 9007;
const WM_APP_WEBEX_MENU_RECHECK: u32 = WM_APP + 0x42;
const FIT_WINDOW_MENU: usize = 9010;
const SCALE_MENU_BASE: usize = 9020;
const SCALE_MENU_END: usize = SCALE_MENU_BASE + 6;
const SCALE_PRESETS: [u32; 7] = [25, 50, 75, 100, 125, 150, 200];
const DXGI_STATUS_OCCLUDED_CODE: HRESULT = HRESULT(0x087A0001u32 as i32);
const SYNTHETIC_CURSOR_WIDTH: u32 = 16;
const SYNTHETIC_CURSOR_HEIGHT: u32 = 24;

fn synthetic_cursor_pixels() -> Vec<u8> {
    let mut pixels = vec![0u8; (SYNTHETIC_CURSOR_WIDTH * SYNTHETIC_CURSOR_HEIGHT * 4) as usize];
    let set_pixel = |pixels: &mut [u8], x: u32, y: u32, color: [u8; 4]| {
        let offset = ((y * SYNTHETIC_CURSOR_WIDTH + x) * 4) as usize;
        pixels[offset..offset + 4].copy_from_slice(&color);
    };
    for y in 0..SYNTHETIC_CURSOR_HEIGHT {
        let right = if y < 17 { y / 2 } else { 6 + (y - 17).min(2) };
        let left = if y < 17 { 0 } else { 5 };
        for x in left..=right.max(left) {
            if x < SYNTHETIC_CURSOR_WIDTH {
                set_pixel(&mut pixels, x, y, [0, 0, 0, 255]);
            }
        }
    }
    for y in 2..15 {
        let right = y / 2;
        for x in 1..right {
            set_pixel(&mut pixels, x, y, [245, 248, 255, 255]);
        }
    }
    for y in 18..22 {
        for x in 6..8 {
            set_pixel(&mut pixels, x, y, [245, 248, 255, 255]);
        }
    }
    // A soft gray edge keeps the fixed test cursor visible on both light and dark scenes.
    for y in 0..SYNTHETIC_CURSOR_HEIGHT {
        let right = if y < 17 { y / 2 } else { 6 + (y - 17).min(2) };
        let left = if y < 17 { 0 } else { 5 };
        if left > 0 && left < SYNTHETIC_CURSOR_WIDTH {
            set_pixel(&mut pixels, left - 1, y, [64, 64, 64, 128]);
        }
        if right + 1 < SYNTHETIC_CURSOR_WIDTH {
            set_pixel(&mut pixels, right + 1, y, [64, 64, 64, 128]);
        }
    }
    pixels
}

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
    benchmark_cursor: bool,
    benchmark_webex_cursor: bool,
    report_path: Option<String>,
    settings_file: Option<String>,
    webex_controls: Option<bool>,
    check_updates: bool,
}

fn parse_options(args: &[String], monitor_count: usize) -> windows::core::Result<Options> {
    let mut options = Options {
        list_monitors: false,
        monitor: None,
        fps: None,
        size: None,
        benchmark_seconds: None,
        benchmark_cursor: false,
        benchmark_webex_cursor: false,
        report_path: None,
        settings_file: None,
        webex_controls: None,
        check_updates: false,
    };
    let mut values = args.iter().skip(1);
    while let Some(argument) = values.next() {
        match argument.as_str() {
            "--list-monitors" => options.list_monitors = true,
            "--check-updates" => options.check_updates = true,
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
            "--benchmark-cursor" => options.benchmark_cursor = true,
            "--benchmark-composition-cursor" | "--benchmark-webex-cursor" => {
                options.benchmark_webex_cursor = true
            }
            "--report" => {
                let path = values
                    .next()
                    .filter(|value| !value.trim().is_empty())
                    .ok_or_else(invalid_arguments)?;
                options.report_path = Some(path.clone());
            }
            "--settings-file" => {
                let path = values
                    .next()
                    .filter(|value| !value.trim().is_empty())
                    .ok_or_else(invalid_arguments)?;
                options.settings_file = Some(path.clone());
            }
            "--dwm-composition" | "--webex-controls" => options.webex_controls = Some(true),
            "--no-dwm-composition" | "--no-webex-controls" => options.webex_controls = Some(false),
            _ => return Err(invalid_arguments()),
        }
    }
    if options.benchmark_cursor && options.benchmark_seconds.is_none() {
        return Err(invalid_arguments());
    }
    if options.benchmark_webex_cursor
        && (options.benchmark_seconds.is_none() || options.webex_controls != Some(true))
    {
        return Err(invalid_arguments());
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

#[derive(Default)]
struct UpdateUi {
    active: Option<UpdateCheck>,
    pending: Option<(CheckKind, Result<ReleaseInfo, String>)>,
}

impl UpdateUi {
    fn start(&mut self, kind: CheckKind) {
        if let Some(active) = self.active.as_mut() {
            if kind == CheckKind::Manual {
                active.kind = CheckKind::Manual;
            }
            return;
        }
        if let Some((pending_kind, _)) = self.pending.as_mut() {
            if kind == CheckKind::Manual {
                *pending_kind = CheckKind::Manual;
            }
            return;
        }
        match UpdateCheck::start(kind) {
            Some(check) => self.active = Some(check),
            None if kind == CheckKind::Manual => {
                self.pending = Some((kind, Err("update worker could not start".into())))
            }
            None => log("WARNING: update worker could not start"),
        }
    }

    fn poll(&mut self) {
        let Some(check) = self.active.as_ref() else {
            return;
        };
        match check.try_finish() {
            Ok(Some(result)) => {
                let kind = check.kind;
                self.active = None;
                if kind == CheckKind::Automatic
                    && result.as_ref().is_ok_and(|release| {
                        !update::is_newer(release.version, update::current_version())
                    })
                {
                    return;
                }
                if kind == CheckKind::Automatic && result.is_err() {
                    if let Err(error) = result {
                        log(&format!("WARNING: automatic update check failed: {error}"));
                    }
                    return;
                }
                self.pending = Some((kind, result));
            }
            Ok(None) => {}
            Err(error) => {
                let kind = check.kind;
                self.active = None;
                if kind == CheckKind::Manual {
                    self.pending = Some((kind, Err(error)));
                } else {
                    log(&format!("WARNING: automatic update check failed: {error}"));
                }
            }
        }
    }
}

fn present_update_if_ready(hwnd: HWND, update_ui: &mut UpdateUi) {
    if update_ui.pending.is_none()
        || unsafe { GetForegroundWindow() } != hwnd
        || WINDOW_MINIMIZED.load(Ordering::Relaxed)
        || IN_SIZE_MOVE.load(Ordering::Relaxed)
        || WEBEX_OVERLAYS_SUSPENDED.load(Ordering::Relaxed)
    {
        return;
    }
    let (kind, result) = update_ui.pending.take().expect("pending update checked");
    let current = update::current_version();
    match update::classify_result(
        kind,
        result,
        current,
        CHECK_UPDATES_ON_STARTUP.load(Ordering::Relaxed),
    ) {
        update::Notification::Discard | update::Notification::Silent => {}
        update::Notification::UpToDate(version) => {
            show_update_message(
                hwnd,
                &format!("You are using the latest version ({version})."),
                false,
            );
        }
        update::Notification::ManualError(error) => {
            show_update_message(
                hwnd,
                &format!("Could not check for updates: {error}"),
                false,
            );
        }
        update::Notification::AutomaticError(error) => {
            log(&format!("WARNING: automatic update check failed: {error}"));
        }
        update::Notification::UpdateAvailable(version) => {
            let message = format!(
                "version {version} available (current {current}). Open the download page in your browser?"
            );
            let target =
                update::release_page_after_confirmation(show_update_message(hwnd, &message, true));
            if let Some(target) = target {
                use windows::Win32::UI::Shell::ShellExecuteW;
                let url = wide(target);
                let result = unsafe {
                    ShellExecuteW(
                        Some(hwnd),
                        windows::core::w!("open"),
                        windows::core::PCWSTR(url.as_ptr()),
                        windows::core::PCWSTR::null(),
                        windows::core::PCWSTR::null(),
                        SW_SHOWNORMAL,
                    )
                };
                if result.0 as isize <= 32 {
                    show_update_message(
                        hwnd,
                        "Could not open the release page in your browser.",
                        false,
                    );
                }
            }
        }
    }
}

fn show_update_message(hwnd: HWND, message: &str, confirmation: bool) -> bool {
    unsafe {
        hide_owned_webex_overlays(hwnd);
        let body = wide(message);
        let caption = wide("ExMonitorViewer");
        let flags = if confirmation {
            MB_ICONINFORMATION | MB_YESNO | MB_DEFBUTTON2
        } else {
            MB_ICONINFORMATION | MB_OK
        };
        let response = MessageBoxW(
            Some(hwnd),
            PCWSTR(body.as_ptr()),
            PCWSTR(caption.as_ptr()),
            flags,
        );
        invalidate_webex_state();
        confirmation && response == IDYES
    }
}

pub fn run() -> windows::core::Result<()> {
    let args = std::env::args().collect::<Vec<_>>();
    if args.iter().any(|arg| {
        arg == "--help" || arg == "-h" || arg == "--list-monitors" || arg == "--check-updates"
    }) {
        unsafe {
            let _ = AttachConsole(ATTACH_PARENT_PROCESS);
        }
    }
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        println!("ExMonitorViewer - GPU extended-display window viewer");
        println!("Usage: exmonitor-viewer [--list-monitors|--check-updates] [--monitor N] [--fps 30|60|0] [--size WIDTHxHEIGHT] [--settings-file PATH] [--benchmark SECONDS] [--report PATH] [--dwm-composition|--no-dwm-composition]");
        println!("--check-updates checks the latest stable release and prints an ASCII JSON result without opening a browser or changing settings.");
        println!("FPS 0 means unlimited. Monitor numbers are 1-based and match --list-monitors.");
        println!("--benchmark-cursor requires --benchmark and renders a synthetic GPU cursor for measurement.");
        println!("--benchmark-composition-cursor requires --benchmark and --dwm-composition; it moves only the proxy cursor over an eligible window for measurement.");
        println!("--benchmark-cursor selects a synthetic cursor shape. Without it, the system cursor shape is used.");
        println!("--benchmark-webex-cursor and --webex-controls|--no-webex-controls remain supported as legacy aliases.");
        return Ok(());
    }
    if args.iter().any(|arg| arg == "--check-updates") {
        let current = update::current_version();
        match update::check_latest() {
            Ok(latest) => println!("{}", update::cli_result_json(current, Ok(latest))),
            Err(error) => {
                println!("{}", update::cli_result_json(current, Err(&error)));
                std::process::exit(1);
            }
        }
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
        WEBEX_CURSOR_BENCHMARK.store(options.benchmark_webex_cursor, Ordering::Relaxed);

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

        let settings_path = if let Some(path) = options.settings_file.as_ref() {
            Some(std::path::PathBuf::from(path))
        } else {
            settings::default_path()
        };
        let loaded_settings = if options.benchmark_seconds.is_none() {
            settings_path.as_deref().and_then(|path| {
                if !path.exists() {
                    return None;
                }
                match settings::load(path) {
                    Ok(value) => Some(value),
                    Err(error) => {
                        log(&format!("WARNING: {error}; using defaults"));
                        None
                    }
                }
            })
        } else {
            None
        };

        let monitor_identities = displays
            .iter()
            .map(|display| (display.name.clone(), display.primary))
            .collect::<Vec<_>>();
        let initial = settings::choose_monitor(
            options.monitor,
            loaded_settings
                .as_ref()
                .and_then(|settings| settings.monitor.as_deref()),
            &monitor_identities,
        );
        let current = initial.min(displays.len() - 1);
        CURRENT_MONITOR.store(current as u32, Ordering::Relaxed);
        let fps = options
            .fps
            .or_else(|| loaded_settings.as_ref().map(|s| s.fps))
            .unwrap_or(60);
        FPS_LIMIT.store(fps, Ordering::Relaxed);
        let webex_controls = options
            .webex_controls
            .or_else(|| {
                loaded_settings
                    .as_ref()
                    .map(|settings| settings.webex_controls)
            })
            .unwrap_or(false);
        WEBEX_CONTROLS.store(webex_controls, Ordering::Relaxed);
        let check_updates_on_startup = loaded_settings
            .as_ref()
            .map(|settings| settings.check_updates_on_startup)
            .unwrap_or(true);
        CHECK_UPDATES_ON_STARTUP.store(check_updates_on_startup, Ordering::Relaxed);
        let benchmark_seconds = options.benchmark_seconds;
        let benchmark_cursor = options.benchmark_cursor;
        let report_path = options.report_path;
        let (client_width, client_height) = options.size.unwrap_or_else(|| {
            loaded_settings
                .as_ref()
                .map(|s| (s.window.width, s.window.height))
                .unwrap_or((960, 540))
        });
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
        let _ = CheckMenuRadioItem(
            monitor_menu,
            MONITOR_MENU_BASE as u32,
            (MONITOR_MENU_BASE + displays.len() - 1) as u32,
            (MONITOR_MENU_BASE + current) as u32,
            MF_BYCOMMAND.0,
        );
        let fps_menu = CreatePopupMenu()?;
        AppendMenuW(fps_menu, MF_STRING, FPS_MENU_30, w!("30 FPS"))?;
        AppendMenuW(fps_menu, MF_STRING, FPS_MENU_60, w!("60 FPS"))?;
        AppendMenuW(fps_menu, MF_STRING, FPS_MENU_MAX, w!("Unlimited"))?;
        let scale_menu = CreatePopupMenu()?;
        SCALE_MENU_HANDLE.store(scale_menu.0 as isize, Ordering::Relaxed);
        for (index, percent) in SCALE_PRESETS.iter().enumerate() {
            let label = wide(&format!("{percent}%"));
            AppendMenuW(
                scale_menu,
                MF_STRING,
                SCALE_MENU_BASE + index,
                PCWSTR(label.as_ptr()),
            )?;
        }
        let menu = CreateMenu()?;
        AppendMenuW(menu, MF_POPUP, monitor_menu.0 as usize, w!("Monitor"))?;
        AppendMenuW(menu, MF_POPUP, fps_menu.0 as usize, w!("FPS"))?;
        AppendMenuW(
            menu,
            MF_STRING,
            FIT_WINDOW_MENU,
            w!("Fit window to image\tF"),
        )?;
        AppendMenuW(menu, MF_POPUP, scale_menu.0 as usize, w!("Scale"))?;
        AppendMenuW(menu, MF_STRING, TOPMOST_MENU, w!("Always on top"))?;
        AppendMenuW(
            menu,
            MF_STRING,
            WEBEX_CONTROLS_MENU,
            PCWSTR(webex_controls_menu_label(webex_controls).as_ptr()),
        )?;
        if webex_controls {
            CheckMenuItem(
                menu,
                WEBEX_CONTROLS_MENU as u32,
                (MF_BYCOMMAND | MF_CHECKED).0,
            );
        }
        let help_menu = CreatePopupMenu()?;
        AppendMenuW(
            help_menu,
            MF_STRING,
            CHECK_UPDATES_MENU,
            w!("Check for updates..."),
        )?;
        AppendMenuW(
            help_menu,
            MF_STRING,
            CHECK_UPDATES_STARTUP_MENU,
            w!("Check on startup"),
        )?;
        if check_updates_on_startup {
            CheckMenuItem(
                help_menu,
                CHECK_UPDATES_STARTUP_MENU as u32,
                (MF_BYCOMMAND | MF_CHECKED).0,
            );
        }
        AppendMenuW(menu, MF_POPUP, help_menu.0 as usize, w!("Help"))?;
        let _ = CheckMenuRadioItem(
            fps_menu,
            FPS_MENU_30 as u32,
            FPS_MENU_MAX as u32,
            match fps {
                30 => FPS_MENU_30 as u32,
                60 => FPS_MENU_60 as u32,
                _ => FPS_MENU_MAX as u32,
            },
            MF_BYCOMMAND.0,
        );
        if loaded_settings
            .as_ref()
            .is_some_and(|settings| settings.topmost)
        {
            TOPMOST.store(true, Ordering::Relaxed);
            CheckMenuItem(menu, TOPMOST_MENU as u32, (MF_BYCOMMAND | MF_CHECKED).0);
        }

        let mut window_rect = RECT {
            left: 0,
            top: 0,
            right: client_width as i32,
            bottom: client_height as i32,
        };
        AdjustWindowRectEx(&mut window_rect, WS_OVERLAPPEDWINDOW, true, WS_EX_APPWINDOW)?;
        let default_display = displays
            .iter()
            .find(|display| display.primary)
            .unwrap_or(&displays[0]);
        let window_display = loaded_settings
            .as_ref()
            .and_then(|settings| {
                displays
                    .iter()
                    .max_by_key(|display| window_workarea_overlap(settings.window, display))
                    .filter(|display| window_workarea_overlap(settings.window, display) > 0)
            })
            .unwrap_or(default_display);
        let restored = loaded_settings.as_ref().map(|settings| {
            settings::clamp_rect(
                settings.window.left,
                settings.window.top,
                settings.window.width,
                settings.window.height,
                (
                    window_display.work_left,
                    window_display.work_top,
                    window_display.work_right,
                    window_display.work_bottom,
                ),
            )
        });
        let initial_left = restored
            .map(|window| window.left)
            .unwrap_or(default_display.left + 80);
        let initial_top = restored
            .map(|window| window.top)
            .unwrap_or(default_display.top + 80);
        let initial_width = if options.size.is_some() {
            window_rect.right - window_rect.left
        } else {
            restored
                .map(|window| window.width as i32)
                .unwrap_or(window_rect.right - window_rect.left)
        };
        let initial_height = if options.size.is_some() {
            window_rect.bottom - window_rect.top
        } else {
            restored
                .map(|window| window.height as i32)
                .unwrap_or(window_rect.bottom - window_rect.top)
        };
        let hwnd = CreateWindowExW(
            WS_EX_APPWINDOW,
            class,
            w!("ExMonitorViewer"),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            initial_left,
            initial_top,
            initial_width,
            initial_height,
            None,
            Some(menu),
            Some(instance.into()),
            None,
        )?;
        APP_HWND.store(hwnd.0 as isize, Ordering::Relaxed);
        cache_window_rect(hwnd);
        if TOPMOST.load(Ordering::Relaxed) {
            let _ = SetWindowPos(
                hwnd,
                Some(HWND_TOPMOST),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE,
            );
        }
        if options.size.is_none()
            && loaded_settings
                .as_ref()
                .is_some_and(|settings| settings.window.maximized)
        {
            let _ = ShowWindow(hwnd, SW_SHOWMAXIMIZED);
        }
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

        let mut renderer_startup_timing = TimingStats::default();
        let renderer_started = Instant::now();
        let mut renderer = Renderer::new(&displays[current], hwnd, benchmark_cursor)?;
        renderer_startup_timing.record(renderer_started);
        let (source_width, source_height) = renderer.logical_source_size();
        SOURCE_WIDTH.store(source_width, Ordering::Relaxed);
        SOURCE_HEIGHT.store(source_height, Ordering::Relaxed);
        update_scale_selection(
            RESIZE_WIDTH.load(Ordering::Relaxed),
            RESIZE_HEIGHT.load(Ordering::Relaxed),
        );
        SETTINGS_TRACKING.store(true, Ordering::Relaxed);
        log_window_state(hwnd, "startup");
        log("GPU capture renderer initialized");
        let mut message = MSG::default();
        let initial_limit = FPS_LIMIT.load(Ordering::Relaxed);
        let mut next_frame = Instant::now() + frame_period(initial_limit);
        let mut stats_at = Instant::now();
        let mut captured = 0u32;
        let mut presented = 0u32;
        let mut total_captured = 0u64;
        let mut total_presented = 0u64;
        let mut total_pointer_updates = 0u64;
        let mut total_shape_uploads = 0u64;
        let mut total_synthetic_position_updates = 0u64;
        let mut total_cursor_visible_presents = 0u64;
        let mut total_fixture_shape_uploads = u64::from(benchmark_cursor);
        let mut total_fixture_visible_presents = 0u64;
        let started = Instant::now();
        let mut present_ready = false;
        let mut occluded = false;
        let mut webex_overlays = WebexOverlayManager::default();
        let mut webex_cursor_proxy: Option<WebexCursorProxy> = None;
        let mut proxy_errors = 0u64;
        let mut proxy_error: Option<String> = None;
        let mut proxy_retry_after = Instant::now();
        let mut webex_sync_timing = TimingStats::default();
        let mut renderer_timing = TimingStats::default();
        let mut proxy_create_timing = TimingStats::default();
        let mut webex_overlay_sync_at = Instant::now();
        let mut observed_webex_state_epoch = WEBEX_STATE_EPOCH.load(Ordering::Relaxed);
        let mut update_ui = UpdateUi::default();
        if benchmark_seconds.is_none() && check_updates_on_startup {
            update_ui.start(CheckKind::Automatic);
        }

        'main_loop: while !QUIT.load(Ordering::Relaxed) {
            if !pump_messages(&mut message) {
                break;
            }
            update_ui.poll();
            if MANUAL_UPDATE_REQUESTED.swap(false, Ordering::AcqRel) {
                update_ui.start(CheckKind::Manual);
            }
            present_update_if_ready(hwnd, &mut update_ui);
            let state_epoch = WEBEX_STATE_EPOCH.load(Ordering::Acquire);
            if state_epoch != observed_webex_state_epoch {
                observed_webex_state_epoch = state_epoch;
                webex_cursor_proxy.take();
                webex_overlays.clear();
                webex_overlays.last_view_rect = None;
                webex_overlays.geometry_changed_at = None;
                webex_overlay_sync_at = Instant::now()
                    .checked_sub(Duration::from_millis(150))
                    .unwrap_or_else(Instant::now);
                present_ready = true;
                next_frame = Instant::now();
            }
            if benchmark_seconds
                .is_some_and(|seconds| started.elapsed() >= Duration::from_secs(seconds))
            {
                break;
            }

            if webex_overlay_sync_at.elapsed() >= Duration::from_millis(150) {
                let sync_started = Instant::now();
                webex_overlays.sync(hwnd, &renderer);
                webex_sync_timing.record(sync_started);
                webex_overlay_sync_at = Instant::now();
            }

            let monitor_index = CURRENT_MONITOR.load(Ordering::Relaxed) as usize;
            let user_selected_monitor = monitor_index != renderer.monitor_index;
            let display_changed = DISPLAY_CHANGED.swap(false, Ordering::Relaxed);
            if user_selected_monitor || display_changed {
                webex_cursor_proxy.take();
                webex_overlays.clear();
                webex_overlays.last_view_rect = None;
                webex_overlays.geometry_changed_at = None;
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
                let renderer_started = Instant::now();
                renderer = Renderer::new(next, hwnd, benchmark_cursor)?;
                renderer_startup_timing.record(renderer_started);
                let (source_width, source_height) = renderer.logical_source_size();
                SOURCE_WIDTH.store(source_width, Ordering::Relaxed);
                SOURCE_HEIGHT.store(source_height, Ordering::Relaxed);
                LAST_SIZE_MODE.store(0, Ordering::Relaxed);
                ACTUAL_SCALE.store(0, Ordering::Relaxed);
                update_scale_selection(
                    RESIZE_WIDTH.load(Ordering::Relaxed),
                    RESIZE_HEIGHT.load(Ordering::Relaxed),
                );
                total_fixture_shape_uploads += u64::from(benchmark_cursor);
                present_ready = false;
                next_frame = Instant::now() + frame_period(FPS_LIMIT.load(Ordering::Relaxed));
                occluded = false;
                log("Capture monitor changed");
            }

            if WINDOW_MINIMIZED.load(Ordering::Relaxed) {
                if let Some(proxy) = webex_cursor_proxy.as_ref() {
                    proxy.hide();
                }
                webex_overlays.clear();
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
                    Ok(update) => {
                        if update.desktop_updated {
                            captured += 1;
                            total_captured += 1;
                        }
                        if update.pointer_updated {
                            total_pointer_updates += 1;
                        }
                        if update.shape_uploaded {
                            total_shape_uploads += 1;
                        }
                        if update.desktop_updated {
                            break;
                        }
                    }
                    Err(error) if is_device_lost(error.code()) => {
                        log(&format!(
                            "Capture reset {:#010X}; recreating renderer",
                            error.code().0 as u32
                        ));
                        webex_cursor_proxy.take();
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
                        let renderer_started = Instant::now();
                        renderer = Renderer::new(next, hwnd, benchmark_cursor)?;
                        renderer_startup_timing.record(renderer_started);
                        let (source_width, source_height) = renderer.logical_source_size();
                        SOURCE_WIDTH.store(source_width, Ordering::Relaxed);
                        SOURCE_HEIGHT.store(source_height, Ordering::Relaxed);
                        LAST_SIZE_MODE.store(0, Ordering::Relaxed);
                        ACTUAL_SCALE.store(0, Ordering::Relaxed);
                        update_scale_selection(
                            RESIZE_WIDTH.load(Ordering::Relaxed),
                            RESIZE_HEIGHT.load(Ordering::Relaxed),
                        );
                        total_fixture_shape_uploads += u64::from(benchmark_cursor);
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
                    || WEBEX_STATE_EPOCH.load(Ordering::Acquire) != observed_webex_state_epoch
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
                    || WEBEX_STATE_EPOCH.load(Ordering::Acquire) != observed_webex_state_epoch
                {
                    continue 'main_loop;
                }
            }
            let render_started = Instant::now();
            let render_result = renderer.render();
            renderer_timing.record(render_started);
            if renderer.synthetic_position_updated {
                total_synthetic_position_updates += 1;
            }
            match render_result {
                Ok(true) => {
                    presented += 1;
                    total_presented += 1;
                    if !benchmark_cursor && renderer.cursor_intersects_display() {
                        total_cursor_visible_presents += 1;
                    }
                    if renderer.fixture_cursor_intersects_display() {
                        total_fixture_visible_presents += 1;
                    }
                    if WEBEX_CONTROLS.load(Ordering::Relaxed) && !webex_overlays.entries.is_empty()
                    {
                        if webex_cursor_proxy.is_none() && Instant::now() >= proxy_retry_after {
                            let create_started = Instant::now();
                            match WebexCursorProxy::new(hwnd, &renderer, &webex_overlays) {
                                Ok(proxy) => {
                                    proxy_create_timing.record(create_started);
                                    webex_cursor_proxy = Some(proxy);
                                    proxy_error = None;
                                }
                                Err(error) => {
                                    proxy_create_timing.record(create_started);
                                    let message = format!(
                                        "cursor proxy creation:{:#010x}",
                                        error.code().0 as u32
                                    );
                                    if proxy_error.as_deref() != Some(message.as_str()) {
                                        log(&format!("WARNING: DWM cursor proxy: {message}"));
                                        proxy_errors += 1;
                                    }
                                    proxy_error = Some(message);
                                    proxy_retry_after = Instant::now() + Duration::from_secs(3);
                                }
                            }
                        }
                        if let Some(proxy) = webex_cursor_proxy.as_mut() {
                            match proxy.draw(hwnd, &mut renderer, &webex_overlays) {
                                Ok(_) => proxy_error = None,
                                Err(error) => {
                                    let message = format!(
                                        "cursor proxy draw:{:#010x}",
                                        error.code().0 as u32
                                    );
                                    if proxy_error.as_deref() != Some(message.as_str()) {
                                        log(&format!("WARNING: DWM cursor proxy: {message}"));
                                        proxy_errors += 1;
                                    }
                                    proxy_error = Some(message);
                                    webex_cursor_proxy.take();
                                    proxy_retry_after = Instant::now() + Duration::from_secs(3);
                                }
                            }
                        }
                    } else if let Some(proxy) = webex_cursor_proxy.as_ref() {
                        proxy.hide();
                    }
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
                let sizing_label = match LAST_SIZE_MODE.load(Ordering::Relaxed) {
                    1 => format!(" | Fit {}%", ACTUAL_SCALE.load(Ordering::Relaxed)),
                    2 => format!(" | Scale {}%", ACTUAL_SCALE.load(Ordering::Relaxed)),
                    _ => String::new(),
                };
                let title = format!("ExMonitorViewer | Capture {last_capture_fps:.0} FPS | Present submit {last_submit_fps:.0} FPS | {}x{} | {}{}",
                    renderer.source_width, renderer.source_height, renderer.monitor_name, sizing_label);
                let title = wide(&title);
                SetWindowTextW(hwnd, PCWSTR(title.as_ptr()))?;
            }
        }
        let webex_overlay_count = webex_overlays.entries.len();
        let webex_overlay_max_count = webex_overlays.max_count;
        let webex_overlay_refreshes = webex_overlays.refreshes;
        let webex_overlay_errors = webex_overlays.errors;
        let proxy_submits = webex_cursor_proxy.as_ref().map_or(0, |proxy| proxy.submits);
        let proxy_visible_submits = webex_cursor_proxy
            .as_ref()
            .map_or(0, |proxy| proxy.visible_submits);
        let proxy_mode1_submits = webex_cursor_proxy
            .as_ref()
            .map_or(0, |proxy| proxy.mode1_submits);
        let proxy_mode2_submits = webex_cursor_proxy
            .as_ref()
            .map_or(0, |proxy| proxy.mode2_submits);
        let proxy_position_timing = webex_cursor_proxy
            .as_ref()
            .map_or(TimingStats::default(), |proxy| proxy.position_timing);
        let proxy_visibility_timing = webex_cursor_proxy
            .as_ref()
            .map_or(TimingStats::default(), |proxy| proxy.visibility_timing);
        let proxy_z_order_timing = webex_cursor_proxy
            .as_ref()
            .map_or(TimingStats::default(), |proxy| proxy.z_order_timing);
        let proxy_draw_timing = webex_cursor_proxy
            .as_ref()
            .map_or(TimingStats::default(), |proxy| proxy.draw_timing);
        let proxy_present_timing = webex_cursor_proxy
            .as_ref()
            .map_or(TimingStats::default(), |proxy| proxy.present_timing);
        let proxy_offset_timing = webex_cursor_proxy
            .as_ref()
            .map_or(TimingStats::default(), |proxy| proxy.offset_timing);
        let proxy_commit_timing = webex_cursor_proxy
            .as_ref()
            .map_or(TimingStats::default(), |proxy| proxy.commit_timing);
        let webex_controls_enabled = WEBEX_CONTROLS.load(Ordering::Relaxed);
        webex_overlays.clear();
        webex_cursor_proxy.take();
        renderer.release_held_frame().ok();
        if let Some(path) =
            report_path.or_else(|| benchmark_seconds.map(|_| "benchmark-report.txt".to_owned()))
        {
            let elapsed = started.elapsed().as_secs_f64().max(0.001);
            let report = format!(
                "monitor={}\ncapture_resolution={}x{}\nclient_resolution={}x{}\nbenchmark_seconds={:.3}\ncapture_fps={:.3}\npresent_submit_fps={:.3}\ncaptured_frames={}\npresent_submits={}\npointer_updates={}\ncursor_shape_uploads={}\ncursor_visible_present_submits={}\nfixture_shape_uploads={}\nfixture_position_updates={}\nfixture_visible_present_submits={}\ncursor_source={}\nfixture_path={}\nproxy_tint={}\nproxy_shape_source={}\nproxy_position_source={}\nwebex_controls_enabled={}\nwebex_state_epoch={}\nwebex_menu_enter_count={}\nwebex_menu_exit_count={}\nwebex_menu_toggle_count={}\nwebex_menu_gui_flags={:08X}\nwebex_overlays_suspended={}\nwebex_overlay_count_at_end={}\nwebex_overlay_max_count={}\nwebex_overlay_refreshes={}\nwebex_overlay_errors={}\nproxy_present_submits={}\nproxy_visible_submits={}\nproxy_mode1_submits={}\nproxy_mode2_submits={}\nproxy_errors={}\nwebex_sync_total_ms={:.3}\nwebex_sync_max_ms={:.3}\nwebex_sync_samples={}\nmain_render_total_ms={:.3}\nmain_render_max_ms={:.3}\nmain_render_samples={}\nproxy_create_total_ms={:.3}\nproxy_create_max_ms={:.3}\nproxy_create_samples={}\nproxy_position_total_ms={:.3}\nproxy_position_max_ms={:.3}\nproxy_position_samples={}\nproxy_visibility_total_ms={:.3}\nproxy_visibility_max_ms={:.3}\nproxy_visibility_samples={}\nproxy_z_order_total_ms={:.3}\nproxy_z_order_max_ms={:.3}\nproxy_z_order_samples={}\nproxy_draw_total_ms={:.3}\nproxy_draw_max_ms={:.3}\nproxy_draw_samples={}\nproxy_present_total_ms={:.3}\nproxy_present_max_ms={:.3}\nproxy_present_timing_samples={}\n",
                ascii(&renderer.monitor_name), renderer.source_width, renderer.source_height,
                renderer.window_width, renderer.window_height,
                elapsed, total_captured as f64 / elapsed, total_presented as f64 / elapsed,
                total_captured, total_presented, total_pointer_updates, total_shape_uploads,
                total_cursor_visible_presents, total_fixture_shape_uploads,
                total_synthetic_position_updates, total_fixture_visible_presents,
                if benchmark_cursor { "synthetic_benchmark" } else { "dxgi" },
                if options.benchmark_webex_cursor { "composition_window" } else { "default" },
                if options.benchmark_webex_cursor { "magenta" } else { "none" },
                if options.benchmark_webex_cursor && benchmark_cursor { "synthetic_fixture" } else { "system_hcursor" },
                if options.benchmark_webex_cursor { "diagnostic_panel_position" } else { "system_cursor_position" },
                webex_controls_enabled,
                WEBEX_STATE_EPOCH.load(Ordering::Relaxed),
                WEBEX_MENU_ENTER_COUNT.load(Ordering::Relaxed),
                WEBEX_MENU_EXIT_COUNT.load(Ordering::Relaxed),
                WEBEX_MENU_TOGGLE_COUNT.load(Ordering::Relaxed),
                WEBEX_MENU_GUI_FLAGS.load(Ordering::Relaxed),
                u32::from(WEBEX_OVERLAYS_SUSPENDED.load(Ordering::Relaxed)),
                webex_overlay_count,
                webex_overlay_max_count,
                webex_overlay_refreshes, webex_overlay_errors, proxy_submits,
                proxy_visible_submits, proxy_mode1_submits, proxy_mode2_submits, proxy_errors,
                webex_sync_timing.total_ms, webex_sync_timing.max_ms, webex_sync_timing.samples,
                renderer_timing.total_ms, renderer_timing.max_ms, renderer_timing.samples,
                proxy_create_timing.total_ms, proxy_create_timing.max_ms, proxy_create_timing.samples,
                proxy_position_timing.total_ms, proxy_position_timing.max_ms, proxy_position_timing.samples,
                proxy_visibility_timing.total_ms, proxy_visibility_timing.max_ms, proxy_visibility_timing.samples,
                proxy_z_order_timing.total_ms, proxy_z_order_timing.max_ms, proxy_z_order_timing.samples,
                proxy_draw_timing.total_ms, proxy_draw_timing.max_ms, proxy_draw_timing.samples,
                proxy_present_timing.total_ms, proxy_present_timing.max_ms, proxy_present_timing.samples
            );
            let report = format!(
                "{report}renderer_startup_total_ms={:.3}\nrenderer_startup_max_ms={:.3}\nrenderer_startup_samples={}\nproxy_visual_offset_total_ms={:.3}\nproxy_visual_offset_max_ms={:.3}\nproxy_visual_offset_samples={}\nproxy_commit_total_ms={:.3}\nproxy_commit_max_ms={:.3}\nproxy_commit_samples={}\n",
                renderer_startup_timing.total_ms,
                renderer_startup_timing.max_ms,
                renderer_startup_timing.samples,
                proxy_offset_timing.total_ms,
                proxy_offset_timing.max_ms,
                proxy_offset_timing.samples,
                proxy_commit_timing.total_ms,
                proxy_commit_timing.max_ms,
                proxy_commit_timing.samples
            );
            std::fs::write(path, report)?;
        }
        if benchmark_seconds.is_none() {
            if let Some(path) = settings_path.as_deref() {
                if loaded_settings.is_none()
                    && path.exists()
                    && !SETTINGS_DIRTY.load(Ordering::Relaxed)
                {
                    log("WARNING: settings were invalid; preserving the file because no settings changed");
                    return Ok(());
                }
                let selected_monitor = enumerate_displays()
                    .ok()
                    .and_then(|current_displays| {
                        current_displays
                            .get(CURRENT_MONITOR.load(Ordering::Relaxed) as usize)
                            .map(|d| d.name.clone())
                    })
                    .unwrap_or_else(|| renderer.monitor_name.clone());
                let saved = Settings {
                    schema_version: 1,
                    monitor: Some(selected_monitor),
                    fps: FPS_LIMIT.load(Ordering::Relaxed),
                    topmost: TOPMOST.load(Ordering::Relaxed),
                    webex_controls: WEBEX_CONTROLS.load(Ordering::Relaxed),
                    check_updates_on_startup: CHECK_UPDATES_ON_STARTUP.load(Ordering::Relaxed),
                    window: WindowSettings {
                        left: WINDOW_LEFT.load(Ordering::Relaxed),
                        top: WINDOW_TOP.load(Ordering::Relaxed),
                        width: WINDOW_WIDTH.load(Ordering::Relaxed).clamp(320, 8000) as u32,
                        height: WINDOW_HEIGHT.load(Ordering::Relaxed).clamp(240, 4500) as u32,
                        maximized: WINDOW_MAXIMIZED.load(Ordering::Relaxed),
                    },
                };
                if let Err(error) = settings::save(path, &saved) {
                    log(&format!("WARNING: {error}; settings were not saved"));
                }
            }
        }
    }
    Ok(())
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

fn invalidate_webex_state() {
    WEBEX_STATE_EPOCH.fetch_add(1, Ordering::AcqRel);
}

unsafe fn snapshot_webex_gui_flags() -> Option<u32> {
    let mut info = GUITHREADINFO {
        cbSize: size_of::<GUITHREADINFO>() as u32,
        ..Default::default()
    };
    if unsafe { GetGUIThreadInfo(GetCurrentThreadId(), &mut info) }.is_err() {
        WEBEX_MENU_GUI_FLAGS.store(u32::MAX, Ordering::Relaxed);
        return None;
    }
    WEBEX_MENU_GUI_FLAGS.store(info.flags.0, Ordering::Relaxed);
    Some(info.flags.0)
}

unsafe fn post_webex_menu_recheck(hwnd: HWND, reset_attempts: bool) {
    if reset_attempts {
        WEBEX_MENU_RECHECK_ATTEMPTS.store(0, Ordering::Relaxed);
    }
    let _ = unsafe { PostMessageW(Some(hwnd), WM_APP_WEBEX_MENU_RECHECK, WPARAM(0), LPARAM(0)) };
}

fn log_webex_ui_state(event: &str) {
    log(&format!(
        "DWM UI event={event} enabled={} suspended={} epoch={} menu_enter={} menu_exit={} menu_toggle={} gui_flags={:08X}",
        u32::from(WEBEX_CONTROLS.load(Ordering::Relaxed)),
        u32::from(WEBEX_OVERLAYS_SUSPENDED.load(Ordering::Relaxed)),
        WEBEX_STATE_EPOCH.load(Ordering::Relaxed),
        WEBEX_MENU_ENTER_COUNT.load(Ordering::Relaxed),
        WEBEX_MENU_EXIT_COUNT.load(Ordering::Relaxed),
        WEBEX_MENU_TOGGLE_COUNT.load(Ordering::Relaxed),
        WEBEX_MENU_GUI_FLAGS.load(Ordering::Relaxed),
    ));
}

fn webex_controls_menu_label(enabled: bool) -> Vec<u16> {
    wide(&format!(
        "DWM composition: {}",
        if enabled { "ON" } else { "OFF" }
    ))
}

unsafe fn hide_owned_webex_overlays(owner: HWND) {
    let _ = EnumThreadWindows(
        GetCurrentThreadId(),
        Some(hide_owned_webex_overlay_window),
        LPARAM(owner.0 as isize),
    );
}

unsafe extern "system" fn hide_owned_webex_overlay_window(hwnd: HWND, owner_param: LPARAM) -> BOOL {
    let owner = HWND(owner_param.0 as *mut c_void);
    if GetWindow(hwnd, GW_OWNER).is_ok_and(|candidate| candidate == owner) {
        let mut class_name = [0u16; 64];
        let length = GetClassNameW(hwnd, &mut class_name);
        if length > 0
            && is_composition_overlay_class(&String::from_utf16_lossy(
                &class_name[..length as usize],
            ))
        {
            let _ = ShowWindow(hwnd, SW_HIDE);
        }
    }
    BOOL(1)
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
    work_left: i32,
    work_top: i32,
    work_right: i32,
    work_bottom: i32,
    rotation: DXGI_MODE_ROTATION,
    primary: bool,
    output: IDXGIOutput,
    adapter: IDXGIAdapter1,
    factory: IDXGIFactory1,
}

#[derive(Clone, Copy)]
struct WebexWindow {
    hwnd: HWND,
    pid: u32,
    rect: RECT,
    z_order: usize,
}

struct WebexWindowScan {
    monitor: RECT,
    own_pid: u32,
    windows: Vec<WebexWindow>,
}

struct WebexOverlay {
    thumbnail: isize,
    window: HWND,
    source_rect: RECT,
    destination_rect: RECT,
    z_order: usize,
}

#[derive(Default)]
struct WebexOverlayManager {
    entries: HashMap<(isize, u32), WebexOverlay>,
    class_registered: bool,
    last_view_rect: Option<RECT>,
    geometry_changed_at: Option<Instant>,
    last_z_order_check: Option<Instant>,
    z_order_retry_after: Option<Instant>,
    last_error: Option<String>,
    refreshes: u64,
    max_count: u32,
    errors: u64,
}

const MAX_WEBEX_OVERLAYS: usize = 32;
const CURSOR_PROXY_CLIP_CAPACITY: usize = 32;

#[repr(C)]
#[derive(Clone, Copy)]
struct CursorProxyConstants {
    cursor_size: [f32; 2],
    output_size: [f32; 2],
    mode: u32,
    clip_count: u32,
    diagnostic_tint: u32,
    reserved: u32,
    padding: [f32; 2],
    alignment: [f32; 2],
    clips: [[f32; 4]; CURSOR_PROXY_CLIP_CAPACITY],
}

struct WebexCursorProxy {
    hwnd: HWND,
    swapchain: IDXGISwapChain1,
    composition: IDCompositionDevice,
    latency_device: IDXGIDevice1,
    _target: IDCompositionTarget,
    visual: IDCompositionVisual,
    target_view: Option<ID3D11RenderTargetView>,
    vertex_shader: ID3D11VertexShader,
    pixel_shader: ID3D11PixelShader,
    constants: ID3D11Buffer,
    size: (u32, u32),
    submits: u64,
    visible_submits: u64,
    mode1_submits: u64,
    mode2_submits: u64,
    last_z_check: Instant,
    position_timing: TimingStats,
    visibility_timing: TimingStats,
    z_order_timing: TimingStats,
    draw_timing: TimingStats,
    present_timing: TimingStats,
    commit_timing: TimingStats,
    offset_timing: TimingStats,
    host_rect: RECT,
    visual_offset: Option<(i32, i32)>,
}

#[derive(Clone, Copy, Debug, Default)]
struct TimingStats {
    total_ms: f64,
    max_ms: f64,
    samples: u64,
}

impl TimingStats {
    fn record(&mut self, started: Instant) {
        let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
        self.total_ms += elapsed_ms;
        self.max_ms = self.max_ms.max(elapsed_ms);
        self.samples += 1;
    }
}

impl WebexCursorProxy {
    unsafe fn new(
        owner: HWND,
        renderer: &Renderer,
        overlays: &WebexOverlayManager,
    ) -> windows::core::Result<Self> {
        let host_rect = webex_overlay_union_rect(overlays).ok_or_else(Error::from_win32)?;
        let host_width = host_rect.right - host_rect.left;
        let host_height = host_rect.bottom - host_rect.top;
        let hwnd = CreateWindowExW(
            WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_TRANSPARENT | WS_EX_NOREDIRECTIONBITMAP,
            w!("ExMonitorViewerCompositionOverlay"),
            w!(""),
            WS_POPUP,
            host_rect.left,
            host_rect.top,
            host_width,
            host_height,
            Some(owner),
            None,
            Some(GetModuleHandleW(None)?.into()),
            None,
        )?;
        if let Err(error) = mark_composition_overlay(hwnd) {
            let _ = DestroyWindow(hwnd);
            return Err(error);
        }
        unsafe {
            let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            let _ = UpdateWindow(hwnd);
        }
        let affinity = SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE).and_then(|_| {
            let mut value = 0u32;
            GetWindowDisplayAffinity(hwnd, &mut value)?;
            if value == 0x11 {
                Ok(())
            } else {
                Err(Error::from_win32())
            }
        });
        if let Err(error) = affinity {
            let _ = DestroyWindow(hwnd);
            return Err(error);
        }

        let result = (|| {
            let dxgi_device: IDXGIDevice = renderer.device.cast()?;
            let composition: IDCompositionDevice = DCompositionCreateDevice(&dxgi_device)?;
            let target = composition.CreateTargetForHwnd(hwnd, false)?;
            let visual = composition.CreateVisual()?;
            let factory: IDXGIFactory2 = renderer.swapchain.GetParent()?;
            let desc = DXGI_SWAP_CHAIN_DESC1 {
                Width: 16,
                Height: 16,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                Stereo: false.into(),
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
                BufferCount: 2,
                Scaling: DXGI_SCALING_STRETCH,
                SwapEffect: DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
                AlphaMode: DXGI_ALPHA_MODE_PREMULTIPLIED,
                Flags: 0,
            };
            let swapchain = factory.CreateSwapChainForComposition(&renderer.device, &desc, None)?;
            visual.SetContent(&swapchain)?;
            target.SetRoot(&visual)?;
            composition.Commit()?;
            let target_view = make_target_view(&renderer.device, &swapchain)?;
            let buffer = D3D11_BUFFER_DESC {
                ByteWidth: size_of::<CursorProxyConstants>() as u32,
                Usage: D3D11_USAGE_DEFAULT,
                BindFlags: D3D11_BIND_CONSTANT_BUFFER.0 as u32,
                CPUAccessFlags: 0,
                MiscFlags: 0,
                StructureByteStride: 0,
            };
            let mut constants = None;
            renderer
                .device
                .CreateBuffer(&buffer, None, Some(&mut constants))?;
            Ok::<_, Error>((
                composition,
                target,
                visual,
                swapchain,
                target_view,
                renderer.cursor_proxy_vertex_shader.clone(),
                renderer.cursor_proxy_pixel_shader.clone(),
                constants.ok_or_else(Error::from_win32)?,
            ))
        })();
        match result {
            Ok((
                composition,
                target,
                visual,
                swapchain,
                target_view,
                vertex_shader,
                pixel_shader,
                constants,
            )) => {
                let latency_device = match renderer.device.cast::<IDXGIDevice1>() {
                    Ok(device) => device,
                    Err(error) => {
                        let _ = DestroyWindow(hwnd);
                        return Err(error);
                    }
                };
                if let Err(error) = latency_device.SetMaximumFrameLatency(2) {
                    let _ = DestroyWindow(hwnd);
                    return Err(error);
                }
                Ok(Self {
                    hwnd,
                    swapchain,
                    composition,
                    latency_device,
                    _target: target,
                    visual,
                    target_view: Some(target_view),
                    vertex_shader,
                    pixel_shader,
                    constants,
                    size: (16, 16),
                    submits: 0,
                    visible_submits: 0,
                    mode1_submits: 0,
                    mode2_submits: 0,
                    last_z_check: Instant::now()
                        .checked_sub(Duration::from_secs(1))
                        .unwrap_or_else(Instant::now),
                    position_timing: TimingStats::default(),
                    visibility_timing: TimingStats::default(),
                    z_order_timing: TimingStats::default(),
                    draw_timing: TimingStats::default(),
                    present_timing: TimingStats::default(),
                    commit_timing: TimingStats::default(),
                    offset_timing: TimingStats::default(),
                    host_rect,
                    visual_offset: None,
                })
            }
            Err(error) => {
                let _ = DestroyWindow(hwnd);
                Err(error)
            }
        }
    }

    fn hide(&self) {
        unsafe {
            if IsWindowVisible(self.hwnd).as_bool() {
                let _ = ShowWindow(self.hwnd, SW_HIDE);
            }
        }
    }

    unsafe fn draw(
        &mut self,
        owner: HWND,
        renderer: &mut Renderer,
        overlays: &WebexOverlayManager,
    ) -> windows::core::Result<bool> {
        if !renderer.has_frame || overlays.entries.is_empty() {
            self.hide();
            return Ok(false);
        }
        let diagnostic_panel_position = WEBEX_CURSOR_BENCHMARK.load(Ordering::Relaxed);
        let synthetic_proxy_cursor = diagnostic_panel_position && renderer.synthetic_cursor;
        let (shape, cursor_position, cursor_view) = if synthetic_proxy_cursor {
            let Some(shape) = renderer.active_cursor_shape() else {
                self.hide();
                return Ok(false);
            };
            (
                shape,
                renderer.active_cursor_position(),
                renderer.active_cursor_view(),
            )
        } else {
            unsafe { renderer.update_proxy_cursor() };
            if !renderer.proxy_cursor_visible {
                self.hide();
                return Ok(false);
            }
            let Some(shape) = renderer.proxy_cursor_shape else {
                self.hide();
                return Ok(false);
            };
            let cursor_bounds = win32_cursor::cursor_bounds(
                renderer.proxy_cursor_screen_position,
                renderer.proxy_cursor_hotspot,
                shape,
            );
            let (logical_width, logical_height) = renderer.logical_source_size();
            let width_scale = logical_width as f64 / renderer.monitor_width.max(1) as f64;
            let height_scale = logical_height as f64 / renderer.monitor_height.max(1) as f64;
            let position = [
                ((cursor_bounds.left - renderer.monitor_left) as f64 * width_scale).round() as i32,
                ((cursor_bounds.top - renderer.monitor_top) as f64 * height_scale).round() as i32,
            ];
            (shape, position, renderer.proxy_cursor_view.clone())
        };
        let Some(cursor_view) = cursor_view else {
            self.hide();
            return Ok(false);
        };
        let fit = renderer.fit_display_rect();
        let (logical_width, logical_height) = renderer.logical_source_size();
        if logical_width == 0 || logical_height == 0 || fit.width <= 0.0 || fit.height <= 0.0 {
            self.hide();
            return Ok(false);
        }
        let mut origin = windows::Win32::Foundation::POINT { x: 0, y: 0 };
        if !unsafe { ClientToScreen(owner, &mut origin) }.as_bool() {
            self.hide();
            return Ok(false);
        }
        let scale_x = fit.width / logical_width as f32;
        let scale_y = fit.height / logical_height as f32;
        let scale = scale_x.min(scale_y);
        if !scale.is_finite() || scale <= 0.0 {
            self.hide();
            return Ok(false);
        }
        let mut cursor_position = cursor_position;
        if diagnostic_panel_position {
            if let Some(panel) = overlays.entries.values().next() {
                let panel_left = panel
                    .destination_rect
                    .left
                    .max(origin.x + fit.x.round() as i32);
                let panel_top = panel
                    .destination_rect
                    .top
                    .max(origin.y + fit.y.round() as i32);
                let panel_right = panel
                    .destination_rect
                    .right
                    .min(origin.x + (fit.x + fit.width).round() as i32);
                let panel_bottom = panel
                    .destination_rect
                    .bottom
                    .min(origin.y + (fit.y + fit.height).round() as i32);
                if panel_right > panel_left && panel_bottom > panel_top {
                    let started = WEBEX_CURSOR_BENCHMARK_STARTED.get_or_init(Instant::now);
                    let t = started.elapsed().as_secs_f32();
                    let px = panel_left
                        + (((t * 0.91).sin() * 0.5 + 0.5) * (panel_right - panel_left) as f32)
                            as i32;
                    let py = panel_top
                        + (((t * 0.73).cos() * 0.5 + 0.5) * (panel_bottom - panel_top) as f32)
                            as i32;
                    cursor_position = [
                        ((px - origin.x - fit.x.round() as i32) as f32 / scale).round() as i32,
                        ((py - origin.y - fit.y.round() as i32) as f32 / scale).round() as i32,
                    ];
                }
            }
        }
        let draw_width = (shape.width as f32 * scale).ceil().max(1.0) as i32;
        let draw_height = (shape.height as f32 * scale).ceil().max(1.0) as i32;
        let padding = 2;
        let window_width = draw_width + padding * 2;
        let window_height = draw_height + padding * 2;
        let left =
            origin.x + fit.x.round() as i32 + (cursor_position[0] as f32 * scale).round() as i32
                - padding;
        let top =
            origin.y + fit.y.round() as i32 + (cursor_position[1] as f32 * scale).round() as i32
                - padding;
        let mut constants = CursorProxyConstants {
            cursor_size: [shape.width as f32, shape.height as f32],
            output_size: [draw_width as f32, draw_height as f32],
            mode: shape.mode,
            clip_count: 0,
            diagnostic_tint: u32::from(diagnostic_panel_position),
            reserved: 0,
            padding: [padding as f32, padding as f32],
            alignment: [0.0; 2],
            clips: [[0.0; 4]; CURSOR_PROXY_CLIP_CAPACITY],
        };
        for entry in overlays.entries.values() {
            if let Some(clipped) = intersect_rect(
                entry.destination_rect,
                RECT {
                    left,
                    top,
                    right: left + window_width,
                    bottom: top + window_height,
                },
            ) {
                if constants.clip_count as usize == CURSOR_PROXY_CLIP_CAPACITY {
                    break;
                }
                let i = constants.clip_count as usize;
                constants.clips[i] = [
                    (clipped.left - left) as f32,
                    (clipped.top - top) as f32,
                    (clipped.right - left) as f32,
                    (clipped.bottom - top) as f32,
                ];
                constants.clip_count += 1;
            }
        }
        if constants.clip_count == 0 {
            self.hide();
            return Ok(false);
        }
        let host_rect = webex_overlay_union_rect(overlays).ok_or_else(Error::from_win32)?;
        if !same_rect(self.host_rect, host_rect) {
            let started = Instant::now();
            SetWindowPos(
                self.hwnd,
                None,
                host_rect.left,
                host_rect.top,
                host_rect.right - host_rect.left,
                host_rect.bottom - host_rect.top,
                SWP_NOACTIVATE | SWP_NOZORDER | SWP_NOOWNERZORDER,
            )?;
            self.position_timing.record(started);
            self.host_rect = host_rect;
            self.visual_offset = None;
            self.last_z_check = Instant::now()
                .checked_sub(Duration::from_secs(1))
                .unwrap_or_else(Instant::now);
        }
        let new_size = (window_width as u32, window_height as u32);
        if new_size != self.size {
            renderer
                .context
                .OMSetRenderTargets(None, None::<&ID3D11DepthStencilView>);
            self.target_view = None;
            self.swapchain.ResizeBuffers(
                0,
                new_size.0,
                new_size.1,
                DXGI_FORMAT_UNKNOWN,
                DXGI_SWAP_CHAIN_FLAG(0),
            )?;
            self.target_view = Some(make_target_view(&renderer.device, &self.swapchain)?);
            self.size = new_size;
        }
        let offset = (left - self.host_rect.left, top - self.host_rect.top);
        if self.visual_offset != Some(offset) {
            let offset_started = Instant::now();
            self.visual.SetOffsetX2(offset.0 as f32)?;
            self.visual.SetOffsetY2(offset.1 as f32)?;
            self.offset_timing.record(offset_started);
            let commit_started = Instant::now();
            self.composition.Commit()?;
            self.commit_timing.record(commit_started);
            self.visual_offset = Some(offset);
        }
        let draw_started = Instant::now();
        renderer.context.UpdateSubresource(
            &self.constants,
            0,
            None,
            &constants as *const CursorProxyConstants as *const c_void,
            0,
            0,
        );
        let target_view = self.target_view.as_ref().ok_or_else(Error::from_win32)?;
        renderer
            .context
            .OMSetRenderTargets(Some(&[Some(target_view.clone())]), None);
        renderer.context.RSSetViewports(Some(&[D3D11_VIEWPORT {
            TopLeftX: 0.0,
            TopLeftY: 0.0,
            Width: new_size.0 as f32,
            Height: new_size.1 as f32,
            MinDepth: 0.0,
            MaxDepth: 1.0,
        }]));
        renderer
            .context
            .ClearRenderTargetView(target_view, &[0.0; 4]);
        renderer
            .context
            .IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
        renderer.context.VSSetShader(&self.vertex_shader, None);
        renderer.context.PSSetShader(&self.pixel_shader, None);
        renderer
            .context
            .PSSetShaderResources(0, Some(&[Some(cursor_view)]));
        renderer
            .context
            .PSSetConstantBuffers(0, Some(&[Some(self.constants.clone())]));
        renderer.context.Draw(3, 0);
        renderer.context.PSSetShaderResources(0, Some(&[None]));
        renderer
            .context
            .OMSetRenderTargets(None, None::<&ID3D11DepthStencilView>);
        self.draw_timing.record(draw_started);
        let present_started = Instant::now();
        self.swapchain.Present(0, DXGI_PRESENT(0)).ok()?;
        self.present_timing.record(present_started);
        unsafe {
            if !IsWindowVisible(self.hwnd).as_bool() {
                let visibility_started = Instant::now();
                let _ = ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
                self.visibility_timing.record(visibility_started);
            }
            if self.last_z_check.elapsed() >= Duration::from_millis(150) {
                let z_order_started = Instant::now();
                ensure_proxy_above_panels(self.hwnd, overlays)?;
                self.z_order_timing.record(z_order_started);
                self.last_z_check = Instant::now();
            }
        }
        self.submits += 1;
        self.visible_submits += 1;
        if shape.mode == 1 {
            self.mode1_submits += 1;
        }
        if shape.mode == 2 {
            self.mode2_submits += 1;
        }
        Ok(true)
    }
}

impl Drop for WebexCursorProxy {
    fn drop(&mut self) {
        unsafe {
            let _ = self.composition.Commit();
            if IsWindow(Some(self.hwnd)).as_bool() {
                let _ = DestroyWindow(self.hwnd);
            }
            let _ = self.latency_device.SetMaximumFrameLatency(1);
        }
    }
}

unsafe extern "system" fn collect_z_order(hwnd: HWND, parameter: LPARAM) -> BOOL {
    let windows = &mut *(parameter.0 as *mut Vec<HWND>);
    windows.push(hwnd);
    BOOL(1)
}

fn z_order_snapshot() -> windows::core::Result<Vec<HWND>> {
    let mut windows = Vec::new();
    unsafe {
        EnumWindows(
            Some(collect_z_order),
            LPARAM(&mut windows as *mut _ as isize),
        )?
    };
    Ok(windows)
}

fn overlay_ranks_are_above_owner(panel_ranks: &[usize], owner_rank: usize) -> bool {
    panel_ranks.iter().all(|rank| *rank < owner_rank)
        && panel_ranks.windows(2).all(|pair| pair[0] < pair[1])
}

fn ensure_overlays_above_owner(owner: HWND, panels: &[HWND]) -> windows::core::Result<()> {
    if panels.is_empty() {
        return Ok(());
    }
    let mut owner_pid = 0u32;
    let owner_thread = unsafe { GetWindowThreadProcessId(owner, Some(&mut owner_pid)) };
    if owner_thread == 0 || owner_pid == 0 {
        return Err(Error::from_win32());
    }
    let owner_topmost = unsafe { GetWindowLongPtrW(owner, GWL_EXSTYLE) as u32 & WS_EX_TOPMOST.0 };
    for panel in panels {
        if unsafe { GetWindow(*panel, GW_OWNER) }.ok() != Some(owner) {
            return Err(Error::from_win32());
        }
        let mut panel_pid = 0u32;
        let panel_thread = unsafe { GetWindowThreadProcessId(*panel, Some(&mut panel_pid)) };
        let panel_topmost =
            unsafe { GetWindowLongPtrW(*panel, GWL_EXSTYLE) as u32 & WS_EX_TOPMOST.0 };
        if panel_thread != owner_thread || panel_pid != owner_pid || panel_topmost != owner_topmost
        {
            return Err(Error::from_win32());
        }
    }

    let mut snapshot = z_order_snapshot()?;
    let owner_rank = snapshot
        .iter()
        .position(|window| *window == owner)
        .ok_or_else(Error::from_win32)?;
    let mut panel_ranks = panels
        .iter()
        .map(|panel| {
            snapshot
                .iter()
                .position(|window| window == panel)
                .ok_or_else(Error::from_win32)
        })
        .collect::<windows::core::Result<Vec<_>>>()?;
    let already_correct = overlay_ranks_are_above_owner(&panel_ranks, owner_rank);
    if !already_correct {
        for panel in panels {
            unsafe {
                SetWindowPos(
                    *panel,
                    Some(owner),
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER,
                )?;
                SetWindowPos(
                    owner,
                    Some(*panel),
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER,
                )?;
            }
        }
        snapshot = z_order_snapshot()?;
        let final_owner_rank = snapshot
            .iter()
            .position(|window| *window == owner)
            .ok_or_else(Error::from_win32)?;
        panel_ranks = panels
            .iter()
            .map(|panel| {
                snapshot
                    .iter()
                    .position(|window| window == panel)
                    .ok_or_else(Error::from_win32)
            })
            .collect::<windows::core::Result<Vec<_>>>()?;
        if !overlay_ranks_are_above_owner(&panel_ranks, final_owner_rank) {
            return Err(Error::from_win32());
        }
    }
    Ok(())
}

fn ensure_proxy_above_panels(
    cursor: HWND,
    overlays: &WebexOverlayManager,
) -> windows::core::Result<()> {
    let panels = overlays
        .entries
        .values()
        .map(|entry| entry.window)
        .collect::<Vec<_>>();
    if panels.is_empty() {
        return Ok(());
    }
    let cursor_topmost =
        unsafe { GetWindowLongPtrW(cursor, GWL_EXSTYLE) as u32 & WS_EX_TOPMOST.0 != 0 };
    if panels.iter().any(|panel| unsafe {
        GetWindowLongPtrW(*panel, GWL_EXSTYLE) as u32 & WS_EX_TOPMOST.0 != 0
    } != cursor_topmost)
    {
        unsafe {
            let _ = ShowWindow(cursor, SW_HIDE);
        }
        return Err(Error::from_win32());
    }
    let mut snapshot = z_order_snapshot()?;
    let panel_above = panels
        .iter()
        .filter_map(|panel| snapshot.iter().position(|window| window == panel))
        .min()
        .ok_or_else(Error::from_win32)?;
    let cursor_rank = snapshot
        .iter()
        .position(|window| *window == cursor)
        .ok_or_else(Error::from_win32)?;
    if cursor_rank >= panel_above {
        let panel = snapshot[panel_above];
        unsafe {
            SetWindowPos(
                cursor,
                Some(panel),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER,
            )?;
            SetWindowPos(
                panel,
                Some(cursor),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER,
            )?;
        }
        snapshot = z_order_snapshot()?;
    }
    let cursor_rank = snapshot
        .iter()
        .position(|window| *window == cursor)
        .ok_or_else(Error::from_win32)?;
    let highest_panel_rank = panels
        .iter()
        .filter_map(|panel| snapshot.iter().position(|window| window == panel))
        .min()
        .ok_or_else(Error::from_win32)?;
    if cursor_rank < highest_panel_rank {
        Ok(())
    } else {
        unsafe {
            let _ = ShowWindow(cursor, SW_HIDE);
        }
        Err(Error::from_win32())
    }
}

unsafe extern "system" fn webex_overlay_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_NCHITTEST => LRESULT(HTTRANSPARENT as isize),
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        WM_ERASEBKGND => LRESULT(1),
        _ => DefWindowProcW(hwnd, message, wparam, lparam),
    }
}

#[derive(Clone, Copy)]
struct CompositionCandidate {
    visible: bool,
    iconic: bool,
    rect: RECT,
    topmost: bool,
    affinity: Option<u32>,
    cloaked: Option<u32>,
}

fn is_eligible_composition_source(candidate: CompositionCandidate, monitor: RECT) -> bool {
    candidate.visible
        && !candidate.iconic
        && candidate.rect.right - candidate.rect.left >= 50
        && candidate.rect.bottom - candidate.rect.top >= 24
        && intersect_rect(candidate.rect, monitor).is_some()
        && candidate.topmost
        && candidate.affinity == Some(0x11)
        && candidate.cloaked == Some(0)
}

fn is_legacy_composition_overlay_class(class_name: &str) -> bool {
    class_name.eq_ignore_ascii_case("ExMonitorViewerWebexOverlay")
}

fn is_composition_overlay_class(class_name: &str) -> bool {
    class_name.eq_ignore_ascii_case("ExMonitorViewerCompositionOverlay")
        || is_legacy_composition_overlay_class(class_name)
}

fn should_exclude_composition_window(
    own_pid: u32,
    candidate_pid: u32,
    has_marker: bool,
    class_name: &str,
) -> bool {
    candidate_pid == own_pid || has_marker || is_composition_overlay_class(class_name)
}

unsafe fn has_composition_overlay_marker(hwnd: HWND) -> bool {
    !GetPropW(hwnd, w!("ExMonitorViewer.CompositionOverlay"))
        .0
        .is_null()
}

unsafe fn has_legacy_composition_overlay_class(hwnd: HWND) -> bool {
    let mut class_name = [0u16; 128];
    let length = GetClassNameW(hwnd, &mut class_name);
    if length <= 0 {
        return true;
    }
    let class_name = String::from_utf16_lossy(&class_name[..length as usize]);
    is_composition_overlay_class(&class_name)
}

fn mark_composition_overlay(hwnd: HWND) -> windows::core::Result<()> {
    unsafe {
        SetPropW(
            hwnd,
            w!("ExMonitorViewer.CompositionOverlay"),
            Some(HANDLE(std::ptr::dangling_mut::<c_void>())),
        )
    }
}

unsafe extern "system" fn collect_composition_windows(hwnd: HWND, parameter: LPARAM) -> BOOL {
    let scan = &mut *(parameter.0 as *mut WebexWindowScan);
    let visible = IsWindowVisible(hwnd).as_bool();
    let iconic = IsIconic(hwnd).as_bool();
    if !visible || iconic {
        return BOOL(1);
    }
    let mut rect = RECT::default();
    if GetWindowRect(hwnd, &mut rect).is_err()
        || rect.right - rect.left < 50
        || rect.bottom - rect.top < 24
        || intersect_rect(rect, scan.monitor).is_none()
    {
        return BOOL(1);
    }
    let topmost = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOPMOST.0 != 0;
    if !topmost {
        return BOOL(1);
    }
    let mut affinity = 0u32;
    let affinity = GetWindowDisplayAffinity(hwnd, &mut affinity)
        .ok()
        .map(|_| affinity);
    let mut cloaked = 0u32;
    let cloaked = DwmGetWindowAttribute(
        hwnd,
        DWMWA_CLOAKED,
        &mut cloaked as *mut _ as *mut c_void,
        size_of::<u32>() as u32,
    )
    .ok()
    .map(|_| cloaked);
    if !is_eligible_composition_source(
        CompositionCandidate {
            visible,
            iconic,
            rect,
            topmost,
            affinity,
            cloaked,
        },
        scan.monitor,
    ) {
        return BOOL(1);
    }

    let mut pid = 0u32;
    GetWindowThreadProcessId(hwnd, Some(&mut pid));
    if pid == 0
        || should_exclude_composition_window(
            scan.own_pid,
            pid,
            has_composition_overlay_marker(hwnd),
            if has_legacy_composition_overlay_class(hwnd) {
                "ExMonitorViewerWebexOverlay"
            } else {
                ""
            },
        )
    {
        return BOOL(1);
    }
    scan.windows.push(WebexWindow {
        hwnd,
        pid,
        rect,
        z_order: scan.windows.len(),
    });
    BOOL(1)
}

fn intersect_rect(a: RECT, b: RECT) -> Option<RECT> {
    let rect = RECT {
        left: a.left.max(b.left),
        top: a.top.max(b.top),
        right: a.right.min(b.right),
        bottom: a.bottom.min(b.bottom),
    };
    (rect.left < rect.right && rect.top < rect.bottom).then_some(rect)
}

fn same_rect(a: RECT, b: RECT) -> bool {
    a.left == b.left && a.top == b.top && a.right == b.right && a.bottom == b.bottom
}

fn webex_overlay_union_rect(overlays: &WebexOverlayManager) -> Option<RECT> {
    union_rects(
        overlays
            .entries
            .values()
            .map(|entry| entry.destination_rect),
    )
}

fn union_rects(rects: impl IntoIterator<Item = RECT>) -> Option<RECT> {
    let mut rects = rects.into_iter();
    let first = rects.next()?;
    Some(rects.fold(first, |bounds, rect| RECT {
        left: bounds.left.min(rect.left),
        top: bounds.top.min(rect.top),
        right: bounds.right.max(rect.right),
        bottom: bounds.bottom.max(rect.bottom),
    }))
}

impl WebexOverlayManager {
    fn clear(&mut self) {
        for (_, entry) in self.entries.drain() {
            unsafe {
                let _ = DwmUnregisterThumbnail(entry.thumbnail);
                if IsWindow(Some(entry.window)).as_bool() {
                    let _ = DestroyWindow(entry.window);
                }
            }
        }
        self.last_z_order_check = None;
        self.z_order_retry_after = None;
    }

    fn report_error(&mut self, error: String) {
        if self.last_error.as_deref() != Some(error.as_str()) {
            log(&format!("WARNING: DWM composition: {error}"));
            self.errors += 1;
        }
        self.last_error = Some(error);
    }

    fn register_class(&mut self) -> windows::core::Result<()> {
        if self.class_registered {
            return Ok(());
        }
        unsafe {
            let instance = GetModuleHandleW(None)?;
            let class = WNDCLASSW {
                hInstance: instance.into(),
                lpszClassName: w!("ExMonitorViewerCompositionOverlay"),
                lpfnWndProc: Some(webex_overlay_proc),
                ..Default::default()
            };
            if RegisterClassW(&class) == 0 {
                return Err(Error::from_win32());
            }
        }
        self.class_registered = true;
        Ok(())
    }

    fn sync(&mut self, owner: HWND, renderer: &Renderer) {
        if !WEBEX_CONTROLS.load(Ordering::Relaxed)
            || WINDOW_MINIMIZED.load(Ordering::Relaxed)
            || WEBEX_OVERLAYS_SUSPENDED.load(Ordering::Relaxed)
            || IN_SIZE_MOVE.load(Ordering::Relaxed)
        {
            self.clear();
            self.last_view_rect = None;
            self.geometry_changed_at = None;
            return;
        }
        if self
            .z_order_retry_after
            .is_some_and(|retry_after| Instant::now() < retry_after)
        {
            for entry in self.entries.values() {
                unsafe {
                    let _ = ShowWindow(entry.window, SW_HIDE);
                }
            }
            return;
        }
        self.z_order_retry_after = None;
        let mut owner_cloaked = 0u32;
        if !unsafe { IsWindowVisible(owner).as_bool() }
            || unsafe { IsIconic(owner).as_bool() }
            || unsafe {
                DwmGetWindowAttribute(
                    owner,
                    DWMWA_CLOAKED,
                    &mut owner_cloaked as *mut _ as *mut c_void,
                    size_of::<u32>() as u32,
                )
            }
            .is_err()
            || owner_cloaked != 0
        {
            self.clear();
            return;
        }

        let mut client = RECT::default();
        if unsafe { GetClientRect(owner, &mut client) }.is_err() {
            self.clear();
            return;
        }
        let mut origin = windows::Win32::Foundation::POINT { x: 0, y: 0 };
        if !unsafe { ClientToScreen(owner, &mut origin) }.as_bool() {
            self.clear();
            return;
        }
        let view_rect = RECT {
            left: origin.x,
            top: origin.y,
            right: origin.x + client.right,
            bottom: origin.y + client.bottom,
        };
        if self
            .last_view_rect
            .is_none_or(|previous| !same_rect(previous, view_rect))
        {
            self.clear();
            self.last_view_rect = Some(view_rect);
            self.geometry_changed_at = Some(Instant::now());
            return;
        }
        if self
            .geometry_changed_at
            .is_some_and(|changed| changed.elapsed() < Duration::from_millis(120))
        {
            self.clear();
            return;
        }

        let monitor_rect = RECT {
            left: renderer.monitor_left,
            top: renderer.monitor_top,
            right: renderer.monitor_left + renderer.monitor_width as i32,
            bottom: renderer.monitor_top + renderer.monitor_height as i32,
        };
        let mut scan = WebexWindowScan {
            monitor: monitor_rect,
            own_pid: unsafe { GetCurrentProcessId() },
            windows: Vec::new(),
        };
        if let Err(error) = unsafe {
            EnumWindows(
                Some(collect_composition_windows),
                LPARAM(&mut scan as *mut _ as isize),
            )
        } {
            self.clear();
            self.report_error(format!("EnumWindows:{:#010x}", error.code().0 as u32));
            return;
        }

        let fit = renderer.fit_display_rect();
        let logical_width = renderer.logical_source_size().0 as f32;
        let logical_height = renderer.logical_source_size().1 as f32;
        let mut desired = HashMap::new();
        for source in scan.windows {
            if let Some((screen_rect, source_rect)) = map_webex_rect(
                source.rect,
                monitor_rect,
                (renderer.monitor_width, renderer.monitor_height),
                (logical_width, logical_height),
                fit,
                view_rect,
            ) {
                desired.insert(
                    (source.hwnd.0 as isize, source.pid),
                    (source, screen_rect, source_rect),
                );
            }
        }
        if desired.len() > MAX_WEBEX_OVERLAYS {
            let mut ranked = desired
                .into_iter()
                .collect::<Vec<((isize, u32), (WebexWindow, RECT, RECT))>>();
            ranked.sort_by_key(|(_, (source, _, _))| {
                -i64::from(source.rect.right - source.rect.left)
                    * i64::from(source.rect.bottom - source.rect.top)
            });
            desired = ranked.into_iter().take(MAX_WEBEX_OVERLAYS).collect();
        }
        let mut display_order = desired
            .iter()
            .map(|(key, (source, _, _))| (*key, source.z_order))
            .collect::<Vec<_>>();
        display_order.sort_by_key(|(_, rank)| *rank);

        let stale = self
            .entries
            .keys()
            .filter(|key| !desired.contains_key(key))
            .copied()
            .collect::<Vec<_>>();
        for key in stale {
            self.remove(key);
        }
        let mut error_this_sync = false;
        let mut z_order_dirty = false;
        let mut failed_updates = Vec::new();
        for (key, (source, screen_rect, source_rect)) in desired {
            if let Some(entry) = self.entries.get_mut(&key) {
                if entry.z_order != source.z_order {
                    entry.z_order = source.z_order;
                    z_order_dirty = true;
                }
                if !same_rect(entry.destination_rect, screen_rect)
                    || !same_rect(entry.source_rect, source_rect)
                {
                    let width = screen_rect.right - screen_rect.left;
                    let height = screen_rect.bottom - screen_rect.top;
                    let position = unsafe {
                        SetWindowPos(
                            entry.window,
                            None,
                            screen_rect.left,
                            screen_rect.top,
                            width,
                            height,
                            SWP_NOACTIVATE | SWP_NOZORDER | SWP_NOOWNERZORDER | SWP_SHOWWINDOW,
                        )
                    };
                    let props = thumbnail_properties(source_rect, width, height);
                    if let Err(error) = position.and_then(|_| unsafe {
                        DwmUpdateThumbnailProperties(entry.thumbnail, &props)
                    }) {
                        let message = format!(
                            "DwmUpdateThumbnailProperties:{:#010x}",
                            error.code().0 as u32
                        );
                        self.report_error(message);
                        error_this_sync = true;
                        failed_updates.push(key);
                    } else {
                        entry.destination_rect = screen_rect;
                        entry.source_rect = source_rect;
                        self.refreshes += 1;
                    }
                } else if !unsafe { IsWindowVisible(entry.window).as_bool() } {
                    z_order_dirty = true;
                }
                continue;
            }
            match self.create_overlay(owner, source, screen_rect, source_rect) {
                Ok(entry) => {
                    self.entries.insert(key, entry);
                    self.refreshes += 1;
                    z_order_dirty = true;
                }
                Err(error) => {
                    self.report_error(format!("overlay creation:{:#010x}", error.code().0 as u32));
                    error_this_sync = true;
                }
            }
        }
        for key in failed_updates {
            self.remove(key);
            z_order_dirty = true;
        }
        let z_order_check_due = self
            .last_z_order_check
            .is_none_or(|last| last.elapsed() >= Duration::from_millis(150));
        if z_order_dirty || z_order_check_due {
            let panel_windows = display_order
                .iter()
                .filter_map(|(key, _)| self.entries.get(key))
                .map(|entry| entry.window)
                .collect::<Vec<_>>();
            match ensure_overlays_above_owner(owner, &panel_windows) {
                Ok(()) => {
                    self.last_z_order_check = Some(Instant::now());
                    for panel in panel_windows {
                        if !unsafe { IsWindowVisible(panel).as_bool() } {
                            unsafe {
                                let _ = ShowWindow(panel, SW_SHOWNOACTIVATE);
                            }
                        }
                    }
                }
                Err(error) => {
                    for panel in panel_windows {
                        unsafe {
                            let _ = ShowWindow(panel, SW_HIDE);
                        }
                    }
                    self.z_order_retry_after = Some(Instant::now() + Duration::from_secs(1));
                    self.report_error(format!("overlay z-order:{:#010x}", error.code().0 as u32));
                    error_this_sync = true;
                }
            }
        }
        self.max_count = self.max_count.max(self.entries.len() as u32);
        if !error_this_sync {
            self.last_error = None;
        }
    }

    fn create_overlay(
        &mut self,
        owner: HWND,
        source: WebexWindow,
        destination_rect: RECT,
        source_rect: RECT,
    ) -> windows::core::Result<WebexOverlay> {
        self.register_class()?;
        let width = destination_rect.right - destination_rect.left;
        let height = destination_rect.bottom - destination_rect.top;
        let overlay = unsafe {
            let instance = GetModuleHandleW(None)?;
            CreateWindowExW(
                WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_TRANSPARENT,
                w!("ExMonitorViewerCompositionOverlay"),
                w!(""),
                WS_POPUP,
                destination_rect.left,
                destination_rect.top,
                width,
                height,
                Some(owner),
                None,
                Some(instance.into()),
                None,
            )?
        };
        if let Err(error) = mark_composition_overlay(overlay) {
            unsafe {
                let _ = DestroyWindow(overlay);
            }
            return Err(error);
        }
        unsafe {
            let _ = ShowWindow(overlay, SW_SHOWNOACTIVATE);
            let _ = UpdateWindow(overlay);
        }
        let configured = unsafe {
            SetWindowDisplayAffinity(overlay, WDA_EXCLUDEFROMCAPTURE).and_then(|_| {
                let mut affinity = 0u32;
                GetWindowDisplayAffinity(overlay, &mut affinity)?;
                if affinity == 0x11 {
                    Ok(())
                } else {
                    Err(Error::from_win32())
                }
            })
        };
        if let Err(error) = configured {
            unsafe {
                let _ = DestroyWindow(overlay);
            }
            return Err(error);
        }
        let thumbnail = match unsafe { DwmRegisterThumbnail(overlay, source.hwnd) } {
            Ok(value) => value,
            Err(error) => {
                unsafe {
                    let _ = DestroyWindow(overlay);
                }
                return Err(error);
            }
        };
        let props = thumbnail_properties(source_rect, width, height);
        if let Err(error) = unsafe { DwmUpdateThumbnailProperties(thumbnail, &props) } {
            unsafe {
                let _ = DwmUnregisterThumbnail(thumbnail);
                let _ = DestroyWindow(overlay);
            }
            return Err(error);
        }
        Ok(WebexOverlay {
            thumbnail,
            window: overlay,
            source_rect,
            destination_rect,
            z_order: source.z_order,
        })
    }

    fn remove(&mut self, key: (isize, u32)) {
        if let Some(entry) = self.entries.remove(&key) {
            unsafe {
                let _ = DwmUnregisterThumbnail(entry.thumbnail);
                if IsWindow(Some(entry.window)).as_bool() {
                    let _ = DestroyWindow(entry.window);
                }
            }
        }
    }
}

impl Drop for WebexOverlayManager {
    fn drop(&mut self) {
        self.clear();
    }
}

fn thumbnail_properties(source: RECT, width: i32, height: i32) -> DWM_THUMBNAIL_PROPERTIES {
    DWM_THUMBNAIL_PROPERTIES {
        dwFlags: DWM_TNP_RECTDESTINATION
            | DWM_TNP_RECTSOURCE
            | DWM_TNP_SOURCECLIENTAREAONLY
            | DWM_TNP_VISIBLE,
        rcDestination: RECT {
            left: 0,
            top: 0,
            right: width,
            bottom: height,
        },
        rcSource: source,
        opacity: 255,
        fVisible: true.into(),
        fSourceClientAreaOnly: false.into(),
    }
}

fn map_webex_rect(
    source: RECT,
    monitor: RECT,
    monitor_size: (u32, u32),
    logical_size: (f32, f32),
    fit: crate::math::FitRect,
    viewer_rect: RECT,
) -> Option<(RECT, RECT)> {
    let clipped = intersect_rect(source, monitor)?;
    let scale_x = logical_size.0 / monitor_size.0.max(1) as f32;
    let scale_y = logical_size.1 / monitor_size.1.max(1) as f32;
    let to_view_x = |screen_x: i32| {
        fit.x + (screen_x - monitor.left) as f32 * scale_x * fit.width / logical_size.0
    };
    let to_view_y = |screen_y: i32| {
        fit.y + (screen_y - monitor.top) as f32 * scale_y * fit.height / logical_size.1
    };
    let left = viewer_rect.left + to_view_x(clipped.left).floor() as i32;
    let top = viewer_rect.top + to_view_y(clipped.top).floor() as i32;
    let right = viewer_rect.left + to_view_x(clipped.right).ceil() as i32;
    let bottom = viewer_rect.top + to_view_y(clipped.bottom).ceil() as i32;
    if right - left < 2 || bottom - top < 2 {
        return None;
    }
    Some((
        RECT {
            left,
            top,
            right,
            bottom,
        },
        RECT {
            left: clipped.left - source.left,
            top: clipped.top - source.top,
            right: clipped.right - source.left,
            bottom: clipped.bottom - source.top,
        },
    ))
}

fn window_workarea_overlap(window: WindowSettings, display: &Display) -> i64 {
    let right = window.left.saturating_add(window.width as i32);
    let bottom = window.top.saturating_add(window.height as i32);
    let overlap_w = right
        .min(display.work_right)
        .saturating_sub(window.left.max(display.work_left))
        .max(0) as i64;
    let overlap_h = bottom
        .min(display.work_bottom)
        .saturating_sub(window.top.max(display.work_top))
        .max(0) as i64;
    overlap_w * overlap_h
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
                let work = info.rcWork;
                displays.push(Display {
                    name,
                    width,
                    height,
                    left: description.DesktopCoordinates.left,
                    top: description.DesktopCoordinates.top,
                    work_left: work.left,
                    work_top: work.top,
                    work_right: work.right,
                    work_bottom: work.bottom,
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
    if !displays.is_empty() {
        let current = (CURRENT_MONITOR.load(Ordering::Relaxed) as usize).min(displays.len() - 1);
        let _ = CheckMenuRadioItem(
            menu,
            MONITOR_MENU_BASE as u32,
            (MONITOR_MENU_BASE + displays.len() - 1) as u32,
            (MONITOR_MENU_BASE + current) as u32,
            MF_BYCOMMAND.0,
        );
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
    monitor_left: i32,
    monitor_top: i32,
    monitor_width: u32,
    monitor_height: u32,
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
    cursor_proxy_vertex_shader: ID3D11VertexShader,
    cursor_proxy_pixel_shader: ID3D11PixelShader,
    sampler: ID3D11SamplerState,
    cursor_texture: Option<ID3D11Texture2D>,
    cursor_view: Option<ID3D11ShaderResourceView>,
    cursor_buffer: ID3D11Buffer,
    cursor_shape: Option<DecodedShape>,
    cursor_position: [i32; 2],
    cursor_visible: bool,
    proxy_cursor_view: Option<ID3D11ShaderResourceView>,
    proxy_cursor_shape: Option<DecodedShape>,
    proxy_cursor_handle: isize,
    proxy_cursor_hotspot: [i32; 2],
    proxy_cursor_screen_position: [i32; 2],
    proxy_cursor_visible: bool,
    proxy_cursor_retry_after: Option<Instant>,
    synthetic_cursor: bool,
    synthetic_started: Option<Instant>,
    synthetic_position_updated: bool,
    fixture_texture: Option<ID3D11Texture2D>,
    fixture_view: Option<ID3D11ShaderResourceView>,
    fixture_shape: Option<DecodedShape>,
    fixture_position: [i32; 2],
    window_width: u32,
    window_height: u32,
    has_frame: bool,
    frame_held: bool,
    rotated: bool,
}

#[derive(Clone, Copy, Debug, Default)]
struct AcquireUpdate {
    desktop_updated: bool,
    pointer_updated: bool,
    shape_uploaded: bool,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct CursorConstants {
    source_size: [f32; 2],
    display_size: [f32; 2],
    position: [f32; 2],
    shape_size: [f32; 2],
    mode: u32,
    visible: u32,
    padding: [u32; 2],
}

impl Renderer {
    fn logical_source_size(&self) -> (u32, u32) {
        if self.rotated {
            (self.source_height, self.source_width)
        } else {
            (self.source_width, self.source_height)
        }
    }

    fn fit_display_rect(&self) -> crate::math::FitRect {
        let (width, height) = self.logical_source_size();
        crate::math::fit_rect(
            width as f32,
            height as f32,
            self.window_width as f32,
            self.window_height as f32,
        )
    }

    fn active_cursor_shape(&self) -> Option<DecodedShape> {
        if self.synthetic_cursor {
            self.fixture_shape
        } else {
            self.cursor_shape
        }
    }

    fn active_cursor_position(&self) -> [i32; 2] {
        if self.synthetic_cursor {
            self.fixture_position
        } else {
            self.cursor_position
        }
    }

    fn active_cursor_view(&self) -> Option<ID3D11ShaderResourceView> {
        if self.synthetic_cursor {
            self.fixture_view.clone()
        } else {
            self.cursor_view.clone()
        }
    }

    unsafe fn update_proxy_cursor(&mut self) {
        let state = match unsafe { win32_cursor::query_cursor_state() } {
            Ok(state) => state,
            Err(_) => {
                self.proxy_cursor_visible = false;
                return;
            }
        };
        self.proxy_cursor_screen_position = state.screen_position;
        if !state.visible || state.handle == 0 {
            self.proxy_cursor_visible = false;
            return;
        }
        let now = Instant::now();
        let handle_changed = state.handle != self.proxy_cursor_handle;
        let retry_ready = self
            .proxy_cursor_retry_after
            .is_none_or(|retry_after| now >= retry_after);
        if handle_changed || (self.proxy_cursor_shape.is_none() && retry_ready) {
            if handle_changed {
                self.proxy_cursor_shape = None;
                self.proxy_cursor_view = None;
            }
            if !retry_ready && !handle_changed {
                self.proxy_cursor_visible = false;
                return;
            }
            self.proxy_cursor_handle = state.handle;
            match unsafe { win32_cursor::decode_cursor(state.handle) } {
                Ok(bitmap) => {
                    if unsafe { self.upload_proxy_cursor_shape(bitmap.shape, &bitmap.pixels) }
                        .is_err()
                    {
                        self.proxy_cursor_retry_after = Some(now + Duration::from_millis(500));
                        self.proxy_cursor_visible = false;
                        return;
                    }
                    self.proxy_cursor_shape = Some(bitmap.shape);
                    self.proxy_cursor_hotspot = bitmap.hotspot;
                    self.proxy_cursor_retry_after = None;
                }
                Err(_) => {
                    self.proxy_cursor_retry_after = Some(now + Duration::from_millis(500));
                    self.proxy_cursor_visible = false;
                    return;
                }
            }
        }
        self.proxy_cursor_visible =
            self.proxy_cursor_shape.is_some() && self.proxy_cursor_view.is_some();
    }

    unsafe fn upload_proxy_cursor_shape(
        &mut self,
        shape: DecodedShape,
        pixels: &[u8],
    ) -> windows::core::Result<()> {
        let desc = D3D11_TEXTURE2D_DESC {
            Width: shape.width,
            Height: shape.height,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_R8G8B8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
            CPUAccessFlags: 0,
            MiscFlags: 0,
        };
        let data = D3D11_SUBRESOURCE_DATA {
            pSysMem: pixels.as_ptr() as *const c_void,
            SysMemPitch: shape.width * 4,
            SysMemSlicePitch: 0,
        };
        let mut texture = None;
        self.device
            .CreateTexture2D(&desc, Some(&data), Some(&mut texture))?;
        let texture = texture.ok_or_else(Error::from_win32)?;
        let mut view = None;
        self.device
            .CreateShaderResourceView(&texture, None, Some(&mut view))?;
        self.proxy_cursor_view = Some(view.ok_or_else(Error::from_win32)?);
        Ok(())
    }

    unsafe fn new(
        display: &Display,
        hwnd: HWND,
        synthetic_cursor: bool,
    ) -> windows::core::Result<Self> {
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
        let (cursor_proxy_vertex_shader, cursor_proxy_pixel_shader) =
            compile_cursor_proxy_shaders(&device)?;
        let cursor_buffer_desc = D3D11_BUFFER_DESC {
            ByteWidth: size_of::<CursorConstants>() as u32,
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: D3D11_BIND_CONSTANT_BUFFER.0 as u32,
            CPUAccessFlags: 0,
            MiscFlags: 0,
            StructureByteStride: 0,
        };
        let mut cursor_buffer = None;
        device.CreateBuffer(&cursor_buffer_desc, None, Some(&mut cursor_buffer))?;
        let cursor_buffer = cursor_buffer.ok_or_else(Error::from_win32)?;
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
        let mut renderer = Self {
            monitor_index: CURRENT_MONITOR.load(Ordering::Relaxed) as usize,
            monitor_name: display.name.clone(),
            monitor_left: display.left,
            monitor_top: display.top,
            monitor_width: display.width,
            monitor_height: display.height,
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
            cursor_proxy_vertex_shader,
            cursor_proxy_pixel_shader,
            sampler,
            cursor_texture: None,
            cursor_view: None,
            cursor_buffer,
            cursor_shape: None,
            cursor_position: [0, 0],
            cursor_visible: false,
            proxy_cursor_view: None,
            proxy_cursor_shape: None,
            proxy_cursor_handle: 0,
            proxy_cursor_hotspot: [0, 0],
            proxy_cursor_screen_position: [0, 0],
            proxy_cursor_visible: false,
            proxy_cursor_retry_after: None,
            synthetic_cursor,
            synthetic_started: synthetic_cursor.then(Instant::now),
            synthetic_position_updated: false,
            fixture_texture: None,
            fixture_view: None,
            fixture_shape: None,
            fixture_position: [0, 0],
            window_width: 960,
            window_height: 540,
            has_frame: false,
            frame_held: false,
            rotated: rotation == 1 || rotation == 3,
        };
        if synthetic_cursor {
            let pixels = synthetic_cursor_pixels();
            renderer.upload_cursor_shape(
                DecodedShape {
                    width: SYNTHETIC_CURSOR_WIDTH,
                    height: SYNTHETIC_CURSOR_HEIGHT,
                    mode: 0,
                },
                &pixels,
                true,
            )?;
        }
        Ok(renderer)
    }

    unsafe fn acquire_latest_frame(&mut self) -> windows::core::Result<AcquireUpdate> {
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
                let desktop_updated = frame_info.LastPresentTime != 0;
                let mut update = AcquireUpdate {
                    desktop_updated,
                    ..Default::default()
                };
                let frame_result = (|| -> windows::core::Result<()> {
                    if desktop_updated || !self.has_frame {
                        let resource = resource.ok_or_else(Error::from_win32)?;
                        let texture: ID3D11Texture2D = resource.cast()?;
                        self.context.CopyResource(&self.capture_texture, &texture);
                        self.has_frame = true;
                    }
                    if frame_info.LastMouseUpdateTime != 0 {
                        let position = [
                            frame_info.PointerPosition.Position.x,
                            frame_info.PointerPosition.Position.y,
                        ];
                        let visible = frame_info.PointerPosition.Visible.as_bool();
                        update.pointer_updated =
                            position != self.cursor_position || visible != self.cursor_visible;
                        self.cursor_position = position;
                        self.cursor_visible = visible;
                    }
                    if frame_info.PointerShapeBufferSize > 0
                        || (self.cursor_visible && self.cursor_shape.is_none())
                    {
                        let (info, buffer) =
                            self.fetch_cursor_shape(frame_info.PointerShapeBufferSize)?;
                        let (shape, pixels) =
                            decode_shape(info.Type, info.Width, info.Height, info.Pitch, &buffer)
                                .ok_or_else(Error::from_win32)?;
                        self.upload_cursor_shape(shape, &pixels, false)?;
                        update.shape_uploaded = true;
                        update.pointer_updated = true;
                    }
                    Ok(())
                })();
                if let Err(error) = frame_result {
                    self.duplication.ReleaseFrame().ok();
                    return Err(error);
                }
                self.frame_held = true;
                Ok(update)
            }
            Err(error) if error.code() == DXGI_ERROR_WAIT_TIMEOUT => Ok(AcquireUpdate::default()),
            Err(error) => Err(error),
        }
    }

    unsafe fn upload_cursor_shape(
        &mut self,
        shape: DecodedShape,
        pixels: &[u8],
        fixture: bool,
    ) -> windows::core::Result<()> {
        let cached_shape = if fixture {
            self.fixture_shape
        } else {
            self.cursor_shape
        };
        if cached_shape
            .is_some_and(|current| current.width == shape.width && current.height == shape.height)
        {
            let texture = if fixture {
                self.fixture_texture.as_ref()
            } else {
                self.cursor_texture.as_ref()
            };
            if let Some(texture) = texture {
                self.context.UpdateSubresource(
                    texture,
                    0,
                    None,
                    pixels.as_ptr() as *const c_void,
                    shape.width * 4,
                    0,
                );
                if fixture {
                    self.fixture_shape = Some(shape);
                } else {
                    self.cursor_shape = Some(shape);
                }
                return Ok(());
            }
        }
        let desc = D3D11_TEXTURE2D_DESC {
            Width: shape.width,
            Height: shape.height,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_R8G8B8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
            CPUAccessFlags: 0,
            MiscFlags: 0,
        };
        let data = D3D11_SUBRESOURCE_DATA {
            pSysMem: pixels.as_ptr() as *const c_void,
            SysMemPitch: shape.width * 4,
            SysMemSlicePitch: 0,
        };
        let mut texture = None;
        self.device
            .CreateTexture2D(&desc, Some(&data), Some(&mut texture))?;
        let texture = texture.ok_or_else(Error::from_win32)?;
        let mut view = None;
        self.device
            .CreateShaderResourceView(&texture, None, Some(&mut view))?;
        let view = view.ok_or_else(Error::from_win32)?;
        if fixture {
            self.fixture_texture = Some(texture);
            self.fixture_view = Some(view);
            self.fixture_shape = Some(shape);
        } else {
            self.cursor_texture = Some(texture);
            self.cursor_view = Some(view);
            self.cursor_shape = Some(shape);
        }
        Ok(())
    }

    unsafe fn fetch_cursor_shape(
        &self,
        initial_size: u32,
    ) -> windows::core::Result<(DXGI_OUTDUPL_POINTER_SHAPE_INFO, Vec<u8>)> {
        const MAX_CURSOR_SHAPE_BYTES: u32 = 16 * 1024 * 1024;
        if initial_size > MAX_CURSOR_SHAPE_BYTES {
            return Err(Error::from_win32());
        }
        let mut buffer = vec![0u8; initial_size as usize];
        loop {
            let mut required = 0u32;
            let mut info = DXGI_OUTDUPL_POINTER_SHAPE_INFO::default();
            let size = u32::try_from(buffer.len()).map_err(|_| Error::from_win32())?;
            let pointer = if buffer.is_empty() {
                std::ptr::null_mut()
            } else {
                buffer.as_mut_ptr() as *mut c_void
            };
            match self
                .duplication
                .GetFramePointerShape(size, pointer, &mut required, &mut info)
            {
                Ok(()) => {
                    if required as usize > buffer.len() {
                        return Err(Error::from_win32());
                    }
                    buffer.truncate(required as usize);
                    return Ok((info, buffer));
                }
                Err(error) if error.code() == DXGI_ERROR_MORE_DATA => {
                    if required == 0 || required > MAX_CURSOR_SHAPE_BYTES {
                        return Err(Error::from_win32());
                    }
                    buffer.resize(required as usize, 0);
                }
                Err(error) => return Err(error),
            }
        }
    }

    unsafe fn render(&mut self) -> windows::core::Result<bool> {
        let fitted = self.fit_display_rect();
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
            let display_width = if self.rotated {
                self.source_height
            } else {
                self.source_width
            };
            let display_height = if self.rotated {
                self.source_width
            } else {
                self.source_height
            };
            self.synthetic_position_updated = false;
            if self.synthetic_cursor {
                let shape = self.fixture_shape.unwrap_or_default();
                let elapsed = self
                    .synthetic_started
                    .map(|started| started.elapsed().as_secs_f32())
                    .unwrap_or_default();
                let available_x = display_width.saturating_sub(shape.width) as f32;
                let available_y = display_height.saturating_sub(shape.height) as f32;
                let new_position = [
                    ((elapsed * 0.91).sin() * 0.5 + 0.5) * available_x,
                    ((elapsed * 0.73).cos() * 0.5 + 0.5) * available_y,
                ];
                let new_position = [
                    new_position[0].round() as i32,
                    new_position[1].round() as i32,
                ];
                self.synthetic_position_updated = new_position != self.fixture_position;
                self.fixture_position = new_position;
            }
            let (shape, position, visible, cursor_view) = if self.synthetic_cursor {
                (
                    self.fixture_shape.unwrap_or_default(),
                    self.fixture_position,
                    true,
                    self.fixture_view.clone(),
                )
            } else {
                (
                    self.cursor_shape.unwrap_or_default(),
                    self.cursor_position,
                    self.cursor_visible,
                    self.cursor_view.clone(),
                )
            };
            let constants = CursorConstants {
                source_size: [self.source_width as f32, self.source_height as f32],
                display_size: [display_width as f32, display_height as f32],
                position: [position[0] as f32, position[1] as f32],
                shape_size: [shape.width as f32, shape.height as f32],
                mode: shape.mode,
                visible: u32::from(visible && cursor_view.is_some()),
                padding: [0; 2],
            };
            self.context.UpdateSubresource(
                &self.cursor_buffer,
                0,
                None,
                &constants as *const CursorConstants as *const c_void,
                0,
                0,
            );
            self.context
                .IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            self.context.VSSetShader(&self.vertex_shader, None);
            self.context.PSSetShader(&self.pixel_shader, None);
            self.context
                .PSSetShaderResources(0, Some(&[Some(self.capture_view.clone())]));
            self.context
                .PSSetShaderResources(1, Some(std::slice::from_ref(&cursor_view)));
            self.context
                .PSSetConstantBuffers(0, Some(&[Some(self.cursor_buffer.clone())]));
            self.context
                .PSSetSamplers(0, Some(&[Some(self.sampler.clone())]));
            self.context.Draw(3, 0);
            self.context.PSSetShaderResources(0, Some(&[None, None]));
        }
        let result = self.swapchain.Present(0, DXGI_PRESENT(0));
        match result.0 {
            0 => Ok(true),
            status if status == DXGI_STATUS_OCCLUDED_CODE.0 => Ok(false),
            _ => result.ok().map(|()| true),
        }
    }

    fn cursor_intersects_display(&self) -> bool {
        let Some(shape) = self.cursor_shape else {
            return false;
        };
        self.shape_intersects_display(shape, self.cursor_position, self.cursor_visible)
    }

    fn fixture_cursor_intersects_display(&self) -> bool {
        let Some(shape) = self.fixture_shape else {
            return false;
        };
        self.shape_intersects_display(shape, self.fixture_position, self.synthetic_cursor)
    }

    fn shape_intersects_display(
        &self,
        shape: DecodedShape,
        position: [i32; 2],
        visible: bool,
    ) -> bool {
        let display_width = if self.rotated {
            self.source_height as i64
        } else {
            self.source_width as i64
        };
        let display_height = if self.rotated {
            self.source_width as i64
        } else {
            self.source_height as i64
        };
        let left = i64::from(position[0]);
        let top = i64::from(position[1]);
        self.has_frame
            && visible
            && left < display_width
            && top < display_height
            && left + i64::from(shape.width) > 0
            && top + i64::from(shape.height) > 0
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

unsafe fn compile_cursor_proxy_shaders(
    device: &ID3D11Device,
) -> windows::core::Result<(ID3D11VertexShader, ID3D11PixelShader)> {
    make_shaders(device, CURSOR_PROXY_PIXEL_SHADER)
}

const CURSOR_PROXY_PIXEL_SHADER: &str = r#"
Texture2D cursor_image : register(t0);
cbuffer CursorProxyConstants : register(b0) {
    float2 cursor_size;
    float2 output_size;
    uint cursor_mode;
    uint clip_count;
    uint diagnostic_tint;
    uint reserved;
    float2 padding;
    float2 alignment;
    float4 clips[32];
};
bool in_bounds(int2 p) {
    return all(p >= 0) && all(p < int2(cursor_size));
}
bool invert_pixel(int2 p) {
    if (!in_bounds(p)) return false;
    float4 source = cursor_image.Load(int3(p, 0));
    if (cursor_mode == 1) return source.r >= 0.5 && source.a >= 0.5;
    return cursor_mode == 2 && source.a >= 0.5 && any(source.rgb > (0.5 / 255.0));
}
float4 fallback_pixel(int2 p) {
    if (!in_bounds(p)) return float4(0, 0, 0, 0);
    float4 source = cursor_image.Load(int3(p, 0));
    if (cursor_mode == 0) {
        float alpha = saturate(source.a);
        return float4(source.rgb * alpha, alpha);
    }
    if (cursor_mode == 1) {
        bool and_bit = source.r >= 0.5;
        bool xor_bit = source.a >= 0.5;
        if (!and_bit) return xor_bit ? float4(1, 1, 1, 1) : float4(0, 0, 0, 1);
        if (!xor_bit) return float4(0, 0, 0, 0);
    } else {
        if (source.a < 0.5) return float4(source.rgb, 1.0);
        if (all(source.rgb <= (0.5 / 255.0))) return float4(0, 0, 0, 0);
    }
    bool edge = false;
    [unroll] for (int y = -1; y <= 1; ++y) {
        [unroll] for (int x = -1; x <= 1; ++x) {
            if (x == 0 && y == 0) continue;
            if (invert_pixel(p + int2(x, y)) != invert_pixel(p)) edge = true;
        }
    }
    return edge ? float4(0, 0, 0, 1) : float4(1, 1, 1, 1);
}
float4 ps_main(float4 position : SV_POSITION, float2 uv : TEXCOORD0) : SV_TARGET {
    float2 pixel = position.xy;
    bool inside = false;
    [loop] for (uint i = 0; i < clip_count; ++i) {
        if (all(pixel >= clips[i].xy) && all(pixel < clips[i].zw)) { inside = true; break; }
    }
    if (!inside) return float4(0, 0, 0, 0);
    float2 shape_pixel = (pixel - padding) * cursor_size / output_size - 0.5;
    int2 base = int2(floor(shape_pixel));
    float2 weight = frac(shape_pixel);
    float4 top = lerp(fallback_pixel(base), fallback_pixel(base + int2(1, 0)), weight.x);
    float4 bottom = lerp(fallback_pixel(base + int2(0, 1)), fallback_pixel(base + int2(1, 1)), weight.x);
    float4 result = lerp(top, bottom, weight.y);
    if (diagnostic_tint != 0) return float4(result.a, 0.0, result.a, result.a);
    return result;
}
"#;

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
Texture2D cursor_image : register(t1);
SamplerState screen_sampler : register(s0);
cbuffer CursorConstants : register(b0) {
    float2 source_size;
    float2 display_size;
    float2 cursor_position;
    float2 cursor_size;
    uint cursor_mode;
    uint cursor_visible;
    uint2 padding;
};
#define ROTATION ROTATION_INDEX
int2 source_pixel(int2 q) {

#if ROTATION == 1
    return int2(q.y, (int)source_size.y - 1 - q.x);
#elif ROTATION == 2
    return int2((int)source_size.x - 1 - q.x, (int)source_size.y - 1 - q.y);
#elif ROTATION == 3
    return int2((int)source_size.x - 1 - q.y, q.x);
#else
    return q;
#endif
}
float4 compose_pixel(int2 q) {
    q = clamp(q, int2(0, 0), int2(display_size) - 1);
    float4 background = screen_image.Load(int3(source_pixel(q), 0));
    if (cursor_visible == 0 || cursor_mode > 2) return background;
    float2 cursor_uv = float2(q) - cursor_position;
    if (any(cursor_uv < 0.0) || any(cursor_uv >= cursor_size)) return background;
    float4 pointer = cursor_image.Load(int3(int2(cursor_uv), 0));
    if (cursor_mode == 0) {
        return float4(pointer.rgb * pointer.a + background.rgb * (1.0 - pointer.a), 1.0);
    }
    uint3 bg = (uint3)round(saturate(background.rgb) * 255.0);
    if (cursor_mode == 1) {
        uint3 and_mask = (uint3)round(pointer.rgb * 255.0);
        uint xor_mask = (uint)round(pointer.a * 255.0);
        uint3 result = (bg & and_mask) ^ xor_mask;
        return float4(result / 255.0, 1.0);
    }
    uint3 color = (uint3)round(saturate(pointer.rgb) * 255.0);
    uint3 result = pointer.a < 0.5 ? color : (bg ^ color);
    return float4(result / 255.0, 1.0);
}
float4 ps_main(float4 position : SV_POSITION, float2 uv : TEXCOORD0) : SV_TARGET {
    float2 sample_uv = uv;
#if ROTATION == 1
    sample_uv = float2(uv.y, 1.0 - uv.x);
#elif ROTATION == 2
    sample_uv = float2(1.0 - uv.x, 1.0 - uv.y);
#elif ROTATION == 3
    sample_uv = float2(1.0 - uv.y, uv.x);
#endif
    if (cursor_visible != 0) {
        float2 pixel = uv * display_size - 0.5;
        float2 cursor_end = cursor_position + cursor_size;
        if (all(pixel >= cursor_position - 1.0) && all(pixel <= cursor_end + 1.0)) {
        float2 base = floor(pixel);
        float2 weight = frac(pixel);
        int2 q = int2(base);
        float4 top = lerp(compose_pixel(q), compose_pixel(q + int2(1, 0)), weight.x);
        float4 bottom = lerp(compose_pixel(q + int2(0, 1)), compose_pixel(q + int2(1, 1)), weight.x);
        return lerp(top, bottom, weight.y);
        }
    }
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
            if !FULLSCREEN.load(Ordering::Relaxed) && !minimized {
                let maximized = wparam.0 == SIZE_MAXIMIZED as usize;
                if SETTINGS_TRACKING.load(Ordering::Relaxed)
                    && WINDOW_MAXIMIZED.load(Ordering::Relaxed) != maximized
                {
                    SETTINGS_DIRTY.store(true, Ordering::Relaxed);
                }
                WINDOW_MAXIMIZED.store(maximized, Ordering::Relaxed);
                if maximized {
                    LAST_SIZE_MODE.store(0, Ordering::Relaxed);
                    ACTUAL_SCALE.store(0, Ordering::Relaxed);
                }
            }
            if !minimized {
                let client_width = (lparam.0 as u32 & 0xffff).max(1);
                let client_height = ((lparam.0 as u32 >> 16) & 0xffff).max(1);
                RESIZE_WIDTH.store(client_width, Ordering::Relaxed);
                RESIZE_HEIGHT.store(client_height, Ordering::Relaxed);
                update_scale_selection(client_width, client_height);
                cache_window_rect(hwnd);
            }
            LRESULT(0)
        }
        WM_GETMINMAXINFO => {
            let info = lparam.0 as *mut MINMAXINFO;
            if !info.is_null() {
                let info = &mut *info;
                info.ptMinTrackSize.x = 320;
                info.ptMinTrackSize.y = 240;
            }
            LRESULT(0)
        }
        WM_MOVE => {
            cache_window_rect(hwnd);
            LRESULT(0)
        }
        WM_CLOSE => {
            if !WINDOW_CLOSE_LOGGED.swap(true, Ordering::Relaxed) {
                log_window_state(hwnd, "close");
            }
            cache_window_rect(hwnd);
            DefWindowProcW(hwnd, message, wparam, lparam)
        }
        WM_COMMAND => {
            let id = wparam.0 & 0xffff;
            match id {
                FIT_WINDOW_MENU => {
                    ACTIVE_SCALE.store(0, Ordering::Relaxed);
                    update_scale_menu_check();
                    fit_window_to_image(hwnd);
                }
                SCALE_MENU_BASE..=SCALE_MENU_END => {
                    let percent = SCALE_PRESETS[id - SCALE_MENU_BASE];
                    scale_window_to_image(hwnd, percent);
                    update_scale_selection_from_window(hwnd);
                }
                MONITOR_MENU_BASE..=2000 => {
                    CURRENT_MONITOR.store((id - MONITOR_MENU_BASE) as u32, Ordering::Relaxed);
                    invalidate_webex_state();
                    SETTINGS_DIRTY.store(true, Ordering::Relaxed);
                    let menu = HMENU(MONITOR_MENU_HANDLE.load(Ordering::Relaxed) as *mut c_void);
                    let count = GetMenuItemCount(Some(menu)).max(1) as usize;
                    let _ = CheckMenuRadioItem(
                        menu,
                        MONITOR_MENU_BASE as u32,
                        (MONITOR_MENU_BASE + count - 1) as u32,
                        id as u32,
                        MF_BYCOMMAND.0,
                    );
                }
                FPS_MENU_30 => FPS_LIMIT.store(30, Ordering::Relaxed),
                FPS_MENU_60 => FPS_LIMIT.store(60, Ordering::Relaxed),
                FPS_MENU_MAX => FPS_LIMIT.store(0, Ordering::Relaxed),
                TOPMOST_MENU => {
                    let enabled = !TOPMOST.load(Ordering::Relaxed);
                    TOPMOST.store(enabled, Ordering::Relaxed);
                    invalidate_webex_state();
                    SETTINGS_DIRTY.store(true, Ordering::Relaxed);
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
                    let menu = GetMenu(hwnd);
                    let _ = CheckMenuItem(
                        menu,
                        TOPMOST_MENU as u32,
                        (MF_BYCOMMAND | if enabled { MF_CHECKED } else { MF_UNCHECKED }).0,
                    );
                }
                WEBEX_CONTROLS_MENU => {
                    let enabled = !WEBEX_CONTROLS.load(Ordering::Relaxed);
                    WEBEX_CONTROLS.store(enabled, Ordering::Relaxed);
                    WEBEX_MENU_TOGGLE_COUNT.fetch_add(1, Ordering::Relaxed);
                    hide_owned_webex_overlays(hwnd);
                    invalidate_webex_state();
                    SETTINGS_DIRTY.store(true, Ordering::Relaxed);
                    let _ = EndMenu();
                    unsafe { post_webex_menu_recheck(hwnd, true) };
                    let menu = GetMenu(hwnd);
                    let label = webex_controls_menu_label(enabled);
                    let _ = ModifyMenuW(
                        menu,
                        WEBEX_CONTROLS_MENU as u32,
                        MF_BYCOMMAND | MF_STRING,
                        WEBEX_CONTROLS_MENU,
                        PCWSTR(label.as_ptr()),
                    );
                    let _ = CheckMenuItem(
                        menu,
                        WEBEX_CONTROLS_MENU as u32,
                        (MF_BYCOMMAND | if enabled { MF_CHECKED } else { MF_UNCHECKED }).0,
                    );
                    let _ = DrawMenuBar(hwnd);
                    log(if enabled {
                        "DWM composition enabled"
                    } else {
                        "DWM composition disabled"
                    });
                    let _ = unsafe { snapshot_webex_gui_flags() };
                    log_webex_ui_state("toggle");
                }
                CHECK_UPDATES_MENU => {
                    MANUAL_UPDATE_REQUESTED.store(true, Ordering::Release);
                }
                CHECK_UPDATES_STARTUP_MENU => {
                    let enabled = !CHECK_UPDATES_ON_STARTUP.load(Ordering::Relaxed);
                    CHECK_UPDATES_ON_STARTUP.store(enabled, Ordering::Relaxed);
                    SETTINGS_DIRTY.store(true, Ordering::Relaxed);
                    let _ = CheckMenuItem(
                        HMENU(GetMenu(hwnd).0),
                        CHECK_UPDATES_STARTUP_MENU as u32,
                        (MF_BYCOMMAND | if enabled { MF_CHECKED } else { MF_UNCHECKED }).0,
                    );
                }
                _ => {}
            }
            if matches!(id, FPS_MENU_30 | FPS_MENU_60 | FPS_MENU_MAX) {
                SETTINGS_DIRTY.store(true, Ordering::Relaxed);
                let menu = GetMenu(hwnd);
                let _ = CheckMenuRadioItem(
                    menu,
                    FPS_MENU_30 as u32,
                    FPS_MENU_MAX as u32,
                    id as u32,
                    MF_BYCOMMAND.0,
                );
            }
            LRESULT(0)
        }
        WM_KEYDOWN => {
            let key = wparam.0 as u32;
            match key {
                0x7a => toggle_fullscreen(hwnd),
                0x46 => {
                    ACTIVE_SCALE.store(0, Ordering::Relaxed);
                    update_scale_menu_check();
                    fit_window_to_image(hwnd);
                }
                0x31..=0x39 => {
                    CURRENT_MONITOR.store(key - 0x31, Ordering::Relaxed);
                    SETTINGS_DIRTY.store(true, Ordering::Relaxed);
                }
                _ => {}
            }
            LRESULT(0)
        }
        WM_ENTERSIZEMOVE => {
            IN_SIZE_MOVE.store(true, Ordering::Relaxed);
            WEBEX_OVERLAYS_SUSPENDED.store(true, Ordering::Relaxed);
            hide_owned_webex_overlays(hwnd);
            LAST_SIZE_MODE.store(0, Ordering::Relaxed);
            ACTUAL_SCALE.store(0, Ordering::Relaxed);
            DefWindowProcW(hwnd, message, wparam, lparam)
        }
        WM_EXITSIZEMOVE => {
            IN_SIZE_MOVE.store(false, Ordering::Relaxed);
            WEBEX_OVERLAYS_SUSPENDED.store(false, Ordering::Relaxed);
            invalidate_webex_state();
            DefWindowProcW(hwnd, message, wparam, lparam)
        }
        WM_ENTERMENULOOP => {
            WEBEX_MENU_ENTER_COUNT.fetch_add(1, Ordering::Relaxed);
            WEBEX_OVERLAYS_SUSPENDED.store(true, Ordering::Relaxed);
            hide_owned_webex_overlays(hwnd);
            let result = DefWindowProcW(hwnd, message, wparam, lparam);
            let _ = unsafe { snapshot_webex_gui_flags() };
            log_webex_ui_state("menu_enter");
            result
        }
        WM_EXITMENULOOP => {
            WEBEX_MENU_EXIT_COUNT.fetch_add(1, Ordering::Relaxed);
            WEBEX_OVERLAYS_SUSPENDED.store(false, Ordering::Relaxed);
            invalidate_webex_state();
            let result = DefWindowProcW(hwnd, message, wparam, lparam);
            let _ = unsafe { snapshot_webex_gui_flags() };
            log_webex_ui_state("menu_exit");
            result
        }
        WM_APP_WEBEX_MENU_RECHECK => {
            let Some(gui_flags) = (unsafe { snapshot_webex_gui_flags() }) else {
                log_webex_ui_state("menu_recheck_flags_unavailable");
                return LRESULT(0);
            };
            let in_menu_mode = gui_flags & GUI_INMENUMODE.0 != 0;
            if in_menu_mode {
                if WEBEX_MENU_RECHECK_ATTEMPTS.fetch_add(1, Ordering::Relaxed) == 0 {
                    let _ = EndMenu();
                    unsafe { post_webex_menu_recheck(hwnd, false) };
                }
            } else {
                WEBEX_MENU_RECHECK_ATTEMPTS.store(0, Ordering::Relaxed);
                if WEBEX_OVERLAYS_SUSPENDED.swap(false, Ordering::Relaxed) {
                    invalidate_webex_state();
                }
            }
            log_webex_ui_state("menu_recheck");
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

unsafe fn cache_window_rect(hwnd: HWND) {
    if FULLSCREEN.load(Ordering::Relaxed)
        || WINDOW_MINIMIZED.load(Ordering::Relaxed)
        || WINDOW_MAXIMIZED.load(Ordering::Relaxed)
        || IsIconic(hwnd).as_bool()
        || IsZoomed(hwnd).as_bool()
    {
        return;
    }
    let mut rect = RECT::default();
    if GetWindowRect(hwnd, &mut rect).is_ok() {
        let width = rect.right.saturating_sub(rect.left);
        let height = rect.bottom.saturating_sub(rect.top);
        if (320..=8000).contains(&width) && (240..=4500).contains(&height) {
            if SETTINGS_TRACKING.load(Ordering::Relaxed)
                && (WINDOW_LEFT.load(Ordering::Relaxed) != rect.left
                    || WINDOW_TOP.load(Ordering::Relaxed) != rect.top
                    || WINDOW_WIDTH.load(Ordering::Relaxed) != width
                    || WINDOW_HEIGHT.load(Ordering::Relaxed) != height)
            {
                SETTINGS_DIRTY.store(true, Ordering::Relaxed);
            }
            WINDOW_LEFT.store(rect.left, Ordering::Relaxed);
            WINDOW_TOP.store(rect.top, Ordering::Relaxed);
            WINDOW_WIDTH.store(width, Ordering::Relaxed);
            WINDOW_HEIGHT.store(height, Ordering::Relaxed);
        }
    }
}

unsafe fn log_window_state(hwnd: HWND, reason: &str) {
    let mut outer = RECT::default();
    let mut client = RECT::default();
    if GetWindowRect(hwnd, &mut outer).is_err() || GetClientRect(hwnd, &mut client).is_err() {
        return;
    }
    let outer_width = outer.right.saturating_sub(outer.left);
    let outer_height = outer.bottom.saturating_sub(outer.top);
    let client_width = client.right.saturating_sub(client.left);
    let client_height = client.bottom.saturating_sub(client.top);
    let ex_style = WINDOW_EX_STYLE(GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32);
    let topmost = u8::from(ex_style.contains(WS_EX_TOPMOST));
    let maximized = u8::from(IsZoomed(hwnd).as_bool());
    log(&format!(
        "Window state event={reason} outer={},{},{},{} client={}x{} topmost={topmost} maximized={maximized} monitor={} fps={}",
        outer.left,
        outer.top,
        outer_width,
        outer_height,
        client_width,
        client_height,
        CURRENT_MONITOR.load(Ordering::Relaxed) + 1,
        FPS_LIMIT.load(Ordering::Relaxed),
    ));
}

unsafe fn update_scale_menu_check() {
    let menu = HMENU(SCALE_MENU_HANDLE.load(Ordering::Relaxed) as *mut c_void);
    let selected = ACTIVE_SCALE.load(Ordering::Relaxed);
    for (index, percent) in SCALE_PRESETS.iter().enumerate() {
        let state = if *percent == selected {
            MF_CHECKED
        } else {
            MF_UNCHECKED
        };
        let _ = CheckMenuItem(
            menu,
            (SCALE_MENU_BASE + index) as u32,
            (MF_BYCOMMAND | state).0,
        );
    }
}

unsafe fn update_scale_selection(client_width: u32, client_height: u32) {
    let source_width = SOURCE_WIDTH.load(Ordering::Relaxed);
    let source_height = SOURCE_HEIGHT.load(Ordering::Relaxed);
    let selected = SCALE_PRESETS
        .iter()
        .copied()
        .find(|percent| {
            crate::math::scale_client_size(source_width, source_height, *percent)
                == (client_width, client_height)
        })
        .unwrap_or(0);
    ACTIVE_SCALE.store(selected, Ordering::Relaxed);
    update_scale_menu_check();
}

unsafe fn update_scale_selection_from_window(hwnd: HWND) {
    let mut client = RECT::default();
    if GetClientRect(hwnd, &mut client).is_ok() {
        update_scale_selection(
            client.right.saturating_sub(client.left).max(0) as u32,
            client.bottom.saturating_sub(client.top).max(0) as u32,
        );
    }
}

unsafe fn fit_window_to_image(hwnd: HWND) {
    restore_normal_window(hwnd);
    let mut client = RECT::default();
    if GetClientRect(hwnd, &mut client).is_err() {
        return;
    }
    let source_width = SOURCE_WIDTH.load(Ordering::Relaxed);
    let source_height = SOURCE_HEIGHT.load(Ordering::Relaxed);
    let (width, height) = crate::math::fit_client_size(
        source_width,
        source_height,
        client.right.saturating_sub(client.left).max(0) as u32,
        client.bottom.saturating_sub(client.top).max(0) as u32,
    );
    let actual = resize_client_to_source(hwnd, width, height);
    ACTUAL_SCALE.store(actual, Ordering::Relaxed);
    LAST_SIZE_MODE.store(1, Ordering::Relaxed);
    update_scale_selection_from_window(hwnd);
    let title = if actual > 0 {
        wide(&format!("ExMonitorViewer - Fit ({actual}%)"))
    } else {
        wide("ExMonitorViewer")
    };
    let _ = SetWindowTextW(hwnd, PCWSTR(title.as_ptr()));
}

unsafe fn scale_window_to_image(hwnd: HWND, percent: u32) -> u32 {
    restore_normal_window(hwnd);
    let source_width = SOURCE_WIDTH.load(Ordering::Relaxed);
    let source_height = SOURCE_HEIGHT.load(Ordering::Relaxed);
    let (width, height) = crate::math::scale_client_size(source_width, source_height, percent);
    let actual = resize_client_to_source(hwnd, width, height);
    ACTUAL_SCALE.store(actual, Ordering::Relaxed);
    LAST_SIZE_MODE.store(2, Ordering::Relaxed);
    let title = if actual > 0 {
        wide(&format!("ExMonitorViewer - {actual}%"))
    } else {
        wide("ExMonitorViewer")
    };
    let _ = SetWindowTextW(hwnd, PCWSTR(title.as_ptr()));
    actual
}

unsafe fn restore_normal_window(hwnd: HWND) {
    if FULLSCREEN.load(Ordering::Relaxed) {
        toggle_fullscreen(hwnd);
    }
    if WINDOW_MAXIMIZED.load(Ordering::Relaxed) {
        let _ = ShowWindow(hwnd, SW_RESTORE);
    }
}

unsafe fn resize_client_to_source(hwnd: HWND, width: u32, height: u32) -> u32 {
    if width == 0 || height == 0 {
        return 0;
    }
    let source_width = SOURCE_WIDTH.load(Ordering::Relaxed);
    let source_height = SOURCE_HEIGHT.load(Ordering::Relaxed);
    if source_width == 0 || source_height == 0 {
        return 0;
    }
    let mut window = RECT::default();
    let mut client = RECT::default();
    if GetWindowRect(hwnd, &mut window).is_err() || GetClientRect(hwnd, &mut client).is_err() {
        return 0;
    }
    let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
    let mut monitor_info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    let has_workarea = GetMonitorInfoW(monitor, &mut monitor_info).as_bool();
    let (work_width, work_height) = if has_workarea {
        (
            monitor_info
                .rcWork
                .right
                .saturating_sub(monitor_info.rcWork.left),
            monitor_info
                .rcWork
                .bottom
                .saturating_sub(monitor_info.rcWork.top),
        )
    } else {
        (i32::MAX / 2, i32::MAX / 2)
    };
    let work_width = work_width.min(8000);
    let work_height = work_height.min(4500);
    let requested_scale =
        (width as f64 / source_width as f64).min(height as f64 / source_height as f64);
    let mut frame_width = window
        .right
        .saturating_sub(window.left)
        .saturating_sub(client.right.saturating_sub(client.left));
    let mut frame_height = window
        .bottom
        .saturating_sub(window.top)
        .saturating_sub(client.bottom.saturating_sub(client.top));
    let mut final_client = (0u32, 0u32);
    for _ in 0..4 {
        let max_scale = ((work_width - frame_width).max(1) as f64 / source_width as f64)
            .min((work_height - frame_height).max(1) as f64 / source_height as f64);
        let min_scale = (((320 - frame_width).max(1) as f64 + 0.5) / source_width as f64)
            .max(((240 - frame_height).max(1) as f64 + 0.5) / source_height as f64);
        let scale = requested_scale.min(max_scale).max(min_scale.min(max_scale));
        let client_width = (source_width as f64 * scale).round().max(1.0) as u32;
        let client_height = (source_height as f64 * scale).round().max(1.0) as u32;
        let outer_width = client_width as i32 + frame_width;
        let outer_height = client_height as i32 + frame_height;
        let mut left = window.left;
        let mut top = window.top;
        if has_workarea {
            let max_left = monitor_info
                .rcWork
                .right
                .saturating_sub(outer_width)
                .max(monitor_info.rcWork.left);
            let max_top = monitor_info
                .rcWork
                .bottom
                .saturating_sub(outer_height)
                .max(monitor_info.rcWork.top);
            left = left.clamp(monitor_info.rcWork.left, max_left);
            top = top.clamp(monitor_info.rcWork.top, max_top);
        }
        if SetWindowPos(
            hwnd,
            None,
            left,
            top,
            outer_width,
            outer_height,
            SWP_NOZORDER | SWP_NOACTIVATE,
        )
        .is_err()
        {
            return 0;
        }
        if GetWindowRect(hwnd, &mut window).is_err() || GetClientRect(hwnd, &mut client).is_err() {
            return 0;
        }
        final_client = (
            client.right.saturating_sub(client.left).max(0) as u32,
            client.bottom.saturating_sub(client.top).max(0) as u32,
        );
        let actual_frame_width = window
            .right
            .saturating_sub(window.left)
            .saturating_sub(final_client.0 as i32);
        let actual_frame_height = window
            .bottom
            .saturating_sub(window.top)
            .saturating_sub(final_client.1 as i32);
        if final_client == (client_width, client_height)
            && actual_frame_width == frame_width
            && actual_frame_height == frame_height
        {
            break;
        }
        frame_width = actual_frame_width;
        frame_height = actual_frame_height;
    }
    let actual_scale = (final_client.0 as f64 / source_width as f64)
        .min(final_client.1 as f64 / source_height as f64);
    ((actual_scale * 100.0).round() as u32).max(1)
}

unsafe fn toggle_fullscreen(hwnd: HWND) {
    LAST_SIZE_MODE.store(0, Ordering::Relaxed);
    ACTUAL_SCALE.store(0, Ordering::Relaxed);
    let enabled = !FULLSCREEN.load(Ordering::Relaxed);
    if enabled {
        FULLSCREEN.store(true, Ordering::Relaxed);
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
        let restore_maximized = WINDOW_MAXIMIZED.load(Ordering::Relaxed);
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
        FULLSCREEN.store(false, Ordering::Relaxed);
        if restore_maximized {
            let _ = ShowWindow(hwnd, SW_MAXIMIZE);
        }
    }
}

#[cfg(test)]
mod cursor_shader_tests {
    use super::*;

    #[test]
    fn protected_composition_source_eligibility_is_process_name_independent() {
        let monitor = RECT {
            left: 0,
            top: 0,
            right: 1920,
            bottom: 1080,
        };
        let protected_fixture_from_a_separate_process = CompositionCandidate {
            visible: true,
            iconic: false,
            rect: RECT {
                left: 100,
                top: 100,
                right: 400,
                bottom: 180,
            },
            topmost: true,
            affinity: Some(0x11),
            cloaked: Some(0),
        };
        assert!(is_eligible_composition_source(
            protected_fixture_from_a_separate_process,
            monitor
        ));
        for candidate in [
            CompositionCandidate {
                visible: false,
                ..protected_fixture_from_a_separate_process
            },
            CompositionCandidate {
                iconic: true,
                ..protected_fixture_from_a_separate_process
            },
            CompositionCandidate {
                rect: RECT {
                    left: 10,
                    top: 10,
                    right: 59,
                    bottom: 40,
                },
                ..protected_fixture_from_a_separate_process
            },
            CompositionCandidate {
                rect: RECT {
                    left: 1920,
                    top: 100,
                    right: 2220,
                    bottom: 180,
                },
                ..protected_fixture_from_a_separate_process
            },
            CompositionCandidate {
                topmost: false,
                ..protected_fixture_from_a_separate_process
            },
            CompositionCandidate {
                affinity: Some(0),
                ..protected_fixture_from_a_separate_process
            },
            CompositionCandidate {
                affinity: None,
                ..protected_fixture_from_a_separate_process
            },
            CompositionCandidate {
                cloaked: Some(1),
                ..protected_fixture_from_a_separate_process
            },
            CompositionCandidate {
                cloaked: None,
                ..protected_fixture_from_a_separate_process
            },
        ] {
            assert!(!is_eligible_composition_source(candidate, monitor));
        }
    }

    #[test]
    fn composition_window_exclusion_uses_pid_marker_and_legacy_class() {
        assert!(should_exclude_composition_window(100, 100, false, ""));
        assert!(should_exclude_composition_window(
            100,
            200,
            true,
            "RenamedAppClass"
        ));
        assert!(should_exclude_composition_window(
            100,
            200,
            false,
            "ExMonitorViewerWebexOverlay"
        ));
        assert!(should_exclude_composition_window(
            100,
            200,
            false,
            "ExMonitorViewerCompositionOverlay"
        ));
        assert!(!should_exclude_composition_window(
            100,
            200,
            false,
            "ProtectedFixtureWindow"
        ));
    }

    #[test]
    fn overlay_z_order_requires_panels_above_owner_in_source_order() {
        assert!(overlay_ranks_are_above_owner(&[1, 2], 3));
        assert!(!overlay_ranks_are_above_owner(&[2], 1));
        assert!(!overlay_ranks_are_above_owner(&[2, 1], 3));
    }

    #[test]
    fn maps_webex_monitor_intersection_into_fit_viewport_and_source_window() {
        let monitor = RECT {
            left: 100,
            top: 200,
            right: 2020,
            bottom: 1280,
        };
        let source = RECT {
            left: 110,
            top: 210,
            right: 210,
            bottom: 310,
        };
        let viewer = RECT {
            left: 50,
            top: 60,
            right: 1010,
            bottom: 600,
        };
        let (destination, source_crop) = map_webex_rect(
            source,
            monitor,
            (1920, 1080),
            (1920.0, 1080.0),
            crate::math::FitRect {
                x: 0.0,
                y: 0.0,
                width: 960.0,
                height: 540.0,
            },
            viewer,
        )
        .unwrap();
        assert_eq!(destination.left, 55);
        assert_eq!(destination.top, 65);
        assert_eq!(destination.right, 105);
        assert_eq!(destination.bottom, 115);
        assert_eq!(source_crop.left, 0);
        assert_eq!(source_crop.top, 0);
        assert_eq!(source_crop.right, 100);
        assert_eq!(source_crop.bottom, 100);
    }

    #[test]
    fn clips_webex_source_to_selected_monitor_without_rescaling_to_full_panel() {
        let monitor = RECT {
            left: 0,
            top: 0,
            right: 1920,
            bottom: 1080,
        };
        let source = RECT {
            left: -20,
            top: 100,
            right: 40,
            bottom: 160,
        };
        let viewer = RECT {
            left: 10,
            top: 20,
            right: 970,
            bottom: 560,
        };
        let (destination, source_crop) = map_webex_rect(
            source,
            monitor,
            (1920, 1080),
            (1920.0, 1080.0),
            crate::math::FitRect {
                x: 0.0,
                y: 0.0,
                width: 960.0,
                height: 540.0,
            },
            viewer,
        )
        .unwrap();
        assert_eq!((destination.left, destination.right), (10, 30));
        assert_eq!((source_crop.left, source_crop.right), (20, 60));
    }

    #[test]
    fn cursor_host_bounds_are_union_of_visible_panel_rectangles() {
        let bounds = union_rects([
            RECT {
                left: 100,
                top: 200,
                right: 300,
                bottom: 240,
            },
            RECT {
                left: 500,
                top: 150,
                right: 560,
                bottom: 400,
            },
        ])
        .unwrap();
        assert_eq!(
            (bounds.left, bounds.top, bounds.right, bounds.bottom),
            (100, 150, 560, 400)
        );
        assert!(union_rects([]).is_none());
    }

    #[test]
    fn proxy_color_is_premultiplied_and_clipped_to_panel_union() {
        let cursor = [255, 0, 0, 255, 0, 0, 0, 0];
        let (width, _, pixels) = unsafe {
            render_proxy_test(
                DecodedShape {
                    width: 2,
                    height: 1,
                    mode: 0,
                },
                &cursor,
                (4, 1),
            )
        };
        let blended = pixels[2 * width as usize + 3];
        assert!(
            (188..=194).contains(&blended[0]),
            "unexpected pixel: {blended:?}"
        );
        assert_eq!(blended[1], 0);
        assert!(
            (188..=194).contains(&blended[3]),
            "unexpected pixel: {blended:?}"
        );
        assert_eq!(pixels[2 * width as usize + 1][3], 0);
    }

    #[test]
    fn proxy_invert_fallback_is_visible_and_keeps_shape_boundary() {
        let cursor = vec![255; 3 * 3 * 4];
        let (width, _, pixels) = unsafe {
            render_proxy_test(
                DecodedShape {
                    width: 3,
                    height: 3,
                    mode: 1,
                },
                &cursor,
                (3, 3),
            )
        };
        assert_eq!(pixels[3 * width as usize + 3], [255, 255, 255, 255]);
        assert_eq!(pixels[2 * width as usize + 2], [0, 0, 0, 255]);
        assert_eq!(pixels[3 * width as usize + 1][3], 0);
    }

    #[test]
    fn proxy_monochrome_keeps_black_white_and_transparent_mask_pixels() {
        let cursor = [
            0, 0, 0, 0, 0, 0, 0, 255, 255, 255, 255, 0, 255, 255, 255, 255,
        ];
        let (width, _, pixels) = unsafe {
            render_proxy_test(
                DecodedShape {
                    width: 2,
                    height: 2,
                    mode: 1,
                },
                &cursor,
                (2, 2),
            )
        };
        assert_eq!(pixels[2 * width as usize + 2], [0, 0, 0, 255]);
        assert_eq!(pixels[2 * width as usize + 3], [255, 255, 255, 255]);
        assert_eq!(pixels[3 * width as usize + 2][3], 0);
    }

    #[test]
    fn proxy_masked_color_preserves_noninvert_color_pixels() {
        let cursor = [80, 140, 220, 0].repeat(4);
        let (width, _, pixels) = unsafe {
            render_proxy_test(
                DecodedShape {
                    width: 2,
                    height: 2,
                    mode: 2,
                },
                &cursor,
                (2, 2),
            )
        };
        assert_eq!(pixels[2 * width as usize + 2], [80, 140, 220, 255]);
    }

    #[test]
    fn proxy_masked_xor_zero_pixels_remain_transparent() {
        let cursor = [0, 0, 0, 255].repeat(4);
        let (_, _, pixels) = unsafe {
            render_proxy_test(
                DecodedShape {
                    width: 2,
                    height: 2,
                    mode: 2,
                },
                &cursor,
                (2, 2),
            )
        };
        assert_eq!(pixels[2 * 6 + 2][3], 0);
    }

    #[test]
    fn proxy_masked_low_nonzero_xor_is_not_treated_as_transparent() {
        let cursor = [1, 0, 0, 255].repeat(4);
        let (width, _, pixels) = unsafe {
            render_proxy_test(
                DecodedShape {
                    width: 2,
                    height: 2,
                    mode: 2,
                },
                &cursor,
                (2, 2),
            )
        };
        assert_eq!(pixels[2 * width as usize + 2], [0, 0, 0, 255]);
    }

    struct ShaderTestDevice {
        device: ID3D11Device,
        context: ID3D11DeviceContext,
        vertex_shader: ID3D11VertexShader,
        pixel_shader: ID3D11PixelShader,
        sampler: ID3D11SamplerState,
    }

    unsafe fn make_test_texture(
        device: &ID3D11Device,
        width: u32,
        height: u32,
        bytes: &[u8],
        bind_flags: u32,
        usage: D3D11_USAGE,
        cpu_access: u32,
    ) -> ID3D11Texture2D {
        let desc = D3D11_TEXTURE2D_DESC {
            Width: width,
            Height: height,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_R8G8B8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: usage,
            BindFlags: bind_flags,
            CPUAccessFlags: cpu_access,
            MiscFlags: 0,
        };
        let data = if bytes.is_empty() {
            None
        } else {
            Some(D3D11_SUBRESOURCE_DATA {
                pSysMem: bytes.as_ptr() as *const c_void,
                SysMemPitch: width * 4,
                SysMemSlicePitch: 0,
            })
        };
        let mut texture = None;
        device
            .CreateTexture2D(
                &desc,
                data.as_ref().map(|initial| initial as *const _),
                Some(&mut texture),
            )
            .unwrap();
        texture.unwrap()
    }

    unsafe fn make_test_device(rotation: u32) -> ShaderTestDevice {
        let mut device = None;
        let mut context = None;
        let mut feature_level = D3D_FEATURE_LEVEL_11_0;
        D3D11CreateDevice(
            None,
            windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_WARP,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            Some(&[D3D_FEATURE_LEVEL_11_0]),
            D3D11_SDK_VERSION,
            Some(&mut device),
            Some(&mut feature_level),
            Some(&mut context),
        )
        .unwrap();
        let device = device.unwrap();
        let context = context.unwrap();
        let pixel_source = PIXEL_SHADER.replace("ROTATION_INDEX", &rotation.to_string());
        let (vertex_shader, pixel_shader) = make_shaders(&device, &pixel_source).unwrap();
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
        device
            .CreateSamplerState(&sampler_desc, Some(&mut sampler))
            .unwrap();
        ShaderTestDevice {
            device,
            context,
            vertex_shader,
            pixel_shader,
            sampler: sampler.unwrap(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    unsafe fn render_cursor_test(
        source_size: (u32, u32),
        target_size: (u32, u32),
        source: &[u8],
        cursor_size: (u32, u32),
        cursor: &[u8],
        position: [i32; 2],
        mode: u32,
        rotation: u32,
    ) -> Vec<[u8; 4]> {
        let test = make_test_device(rotation);
        let source_texture = make_test_texture(
            &test.device,
            source_size.0,
            source_size.1,
            source,
            D3D11_BIND_SHADER_RESOURCE.0 as u32,
            D3D11_USAGE_IMMUTABLE,
            0,
        );
        let cursor_texture = make_test_texture(
            &test.device,
            cursor_size.0,
            cursor_size.1,
            cursor,
            D3D11_BIND_SHADER_RESOURCE.0 as u32,
            D3D11_USAGE_IMMUTABLE,
            0,
        );
        let target_texture = make_test_texture(
            &test.device,
            target_size.0,
            target_size.1,
            &[],
            D3D11_BIND_RENDER_TARGET.0 as u32,
            D3D11_USAGE_DEFAULT,
            0,
        );
        let mut source_view = None;
        test.device
            .CreateShaderResourceView(&source_texture, None, Some(&mut source_view))
            .unwrap();
        let mut cursor_view = None;
        test.device
            .CreateShaderResourceView(&cursor_texture, None, Some(&mut cursor_view))
            .unwrap();
        let mut target_view = None;
        test.device
            .CreateRenderTargetView(&target_texture, None, Some(&mut target_view))
            .unwrap();
        let display_size = if rotation == 1 || rotation == 3 {
            [source_size.1 as f32, source_size.0 as f32]
        } else {
            [source_size.0 as f32, source_size.1 as f32]
        };
        let constants = CursorConstants {
            source_size: [source_size.0 as f32, source_size.1 as f32],
            display_size,
            position: [position[0] as f32, position[1] as f32],
            shape_size: [cursor_size.0 as f32, cursor_size.1 as f32],
            mode,
            visible: 1,
            padding: [0; 2],
        };
        let cb_desc = D3D11_BUFFER_DESC {
            ByteWidth: size_of::<CursorConstants>() as u32,
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: D3D11_BIND_CONSTANT_BUFFER.0 as u32,
            CPUAccessFlags: 0,
            MiscFlags: 0,
            StructureByteStride: 0,
        };
        let cb_data = D3D11_SUBRESOURCE_DATA {
            pSysMem: &constants as *const CursorConstants as *const c_void,
            SysMemPitch: 0,
            SysMemSlicePitch: 0,
        };
        let mut constant_buffer = None;
        test.device
            .CreateBuffer(&cb_desc, Some(&cb_data), Some(&mut constant_buffer))
            .unwrap();
        let target = target_view.unwrap();
        test.context.OMSetRenderTargets(Some(&[Some(target)]), None);
        test.context.RSSetViewports(Some(&[D3D11_VIEWPORT {
            TopLeftX: 0.0,
            TopLeftY: 0.0,
            Width: target_size.0 as f32,
            Height: target_size.1 as f32,
            MinDepth: 0.0,
            MaxDepth: 1.0,
        }]));
        test.context
            .IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
        test.context.VSSetShader(&test.vertex_shader, None);
        test.context.PSSetShader(&test.pixel_shader, None);
        test.context
            .PSSetShaderResources(0, Some(&[source_view, cursor_view]));
        test.context
            .PSSetConstantBuffers(0, Some(&[Some(constant_buffer.unwrap())]));
        test.context.PSSetSamplers(0, Some(&[Some(test.sampler)]));
        test.context.Draw(3, 0);
        test.context.PSSetShaderResources(0, Some(&[None, None]));
        let readback = make_test_texture(
            &test.device,
            target_size.0,
            target_size.1,
            &[],
            0,
            D3D11_USAGE_STAGING,
            D3D11_CPU_ACCESS_READ.0 as u32,
        );
        test.context.CopyResource(&readback, &target_texture);
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        test.context
            .Map(&readback, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
            .unwrap();
        let mut pixels = Vec::with_capacity((target_size.0 * target_size.1) as usize);
        for y in 0..target_size.1 as usize {
            for x in 0..target_size.0 as usize {
                let source = (mapped.pData as *const u8).add(y * mapped.RowPitch as usize + x * 4);
                pixels.push([*source, *source.add(1), *source.add(2), *source.add(3)]);
            }
        }
        test.context.Unmap(&readback, 0);
        pixels
    }

    unsafe fn render_proxy_test(
        shape: DecodedShape,
        cursor: &[u8],
        output_size: (u32, u32),
    ) -> (u32, u32, Vec<[u8; 4]>) {
        let output_width = output_size.0;
        let output_height = output_size.1;
        let width = output_width + 4;
        let height = output_height + 4;
        let test = make_test_device(0);
        let proxy_pixel = compile_shader(
            CURSOR_PROXY_PIXEL_SHADER,
            PCSTR(c"ps_main".as_ptr() as *const u8),
            PCSTR(c"ps_4_0".as_ptr() as *const u8),
        )
        .unwrap();
        let mut proxy_pixel_shader = None;
        test.device
            .CreatePixelShader(
                std::slice::from_raw_parts(
                    proxy_pixel.GetBufferPointer() as *const u8,
                    proxy_pixel.GetBufferSize(),
                ),
                None,
                Some(&mut proxy_pixel_shader),
            )
            .unwrap();
        let cursor_texture = make_test_texture(
            &test.device,
            shape.width,
            shape.height,
            cursor,
            D3D11_BIND_SHADER_RESOURCE.0 as u32,
            D3D11_USAGE_IMMUTABLE,
            0,
        );
        let target_texture = make_test_texture(
            &test.device,
            width,
            height,
            &[],
            D3D11_BIND_RENDER_TARGET.0 as u32,
            D3D11_USAGE_DEFAULT,
            0,
        );
        let mut cursor_view = None;
        test.device
            .CreateShaderResourceView(&cursor_texture, None, Some(&mut cursor_view))
            .unwrap();
        let mut target_view = None;
        test.device
            .CreateRenderTargetView(&target_texture, None, Some(&mut target_view))
            .unwrap();
        let mut constants = CursorProxyConstants {
            cursor_size: [shape.width as f32, shape.height as f32],
            output_size: [output_width as f32, output_height as f32],
            mode: shape.mode,
            clip_count: 1,
            diagnostic_tint: 0,
            reserved: 0,
            padding: [2.0, 2.0],
            alignment: [0.0; 2],
            clips: [[0.0; 4]; CURSOR_PROXY_CLIP_CAPACITY],
        };
        constants.clips[0] = [
            2.0,
            2.0,
            (output_width + 2) as f32,
            (output_height + 2) as f32,
        ];
        let desc = D3D11_BUFFER_DESC {
            ByteWidth: size_of::<CursorProxyConstants>() as u32,
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: D3D11_BIND_CONSTANT_BUFFER.0 as u32,
            CPUAccessFlags: 0,
            MiscFlags: 0,
            StructureByteStride: 0,
        };
        let initial = D3D11_SUBRESOURCE_DATA {
            pSysMem: &constants as *const CursorProxyConstants as *const c_void,
            SysMemPitch: 0,
            SysMemSlicePitch: 0,
        };
        let mut constant_buffer = None;
        test.device
            .CreateBuffer(&desc, Some(&initial), Some(&mut constant_buffer))
            .unwrap();
        let target_view = target_view.unwrap();
        test.context
            .OMSetRenderTargets(Some(&[Some(target_view.clone())]), None);
        test.context.RSSetViewports(Some(&[D3D11_VIEWPORT {
            TopLeftX: 0.0,
            TopLeftY: 0.0,
            Width: width as f32,
            Height: height as f32,
            MinDepth: 0.0,
            MaxDepth: 1.0,
        }]));
        test.context.ClearRenderTargetView(&target_view, &[0.0; 4]);
        test.context
            .IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
        test.context.VSSetShader(&test.vertex_shader, None);
        test.context.PSSetShader(&proxy_pixel_shader.unwrap(), None);
        test.context.PSSetShaderResources(0, Some(&[cursor_view]));
        test.context
            .PSSetConstantBuffers(0, Some(&[constant_buffer]));
        test.context.Draw(3, 0);
        test.context.PSSetShaderResources(0, Some(&[None]));
        test.context
            .OMSetRenderTargets(None, None::<&ID3D11DepthStencilView>);
        let readback = make_test_texture(
            &test.device,
            width,
            height,
            &[],
            0,
            D3D11_USAGE_STAGING,
            D3D11_CPU_ACCESS_READ.0 as u32,
        );
        test.context.CopyResource(&readback, &target_texture);
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        test.context
            .Map(&readback, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
            .unwrap();
        let mut pixels = Vec::with_capacity((width * height) as usize);
        for y in 0..height as usize {
            for x in 0..width as usize {
                let source = (mapped.pData as *const u8).add(y * mapped.RowPitch as usize + x * 4);
                pixels.push([*source, *source.add(1), *source.add(2), *source.add(3)]);
            }
        }
        test.context.Unmap(&readback, 0);
        (width, height, pixels)
    }

    #[test]
    fn benchmark_cursor_requires_benchmark_duration() {
        let lone_fixture = vec![
            "exmonitor-viewer".to_owned(),
            "--benchmark-cursor".to_owned(),
        ];
        assert!(parse_options(&lone_fixture, 1).is_err());
        let benchmark = vec![
            "exmonitor-viewer".to_owned(),
            "--benchmark".to_owned(),
            "20".to_owned(),
            "--benchmark-cursor".to_owned(),
        ];
        let options = parse_options(&benchmark, 1).unwrap();
        assert!(options.benchmark_cursor);
        assert_eq!(options.benchmark_seconds, Some(20));
    }

    #[test]
    fn manual_check_joins_existing_request_instead_of_starting_another() {
        let mut ui = UpdateUi {
            active: Some(UpdateCheck::completed_for_test(
                CheckKind::Automatic,
                Ok(ReleaseInfo {
                    version: update::Version(1, 2, 3),
                }),
            )),
            pending: None,
        };
        ui.start(CheckKind::Manual);
        ui.poll();
        assert!(ui.active.is_none());
        assert_eq!(
            ui.pending.as_ref().map(|(kind, _)| *kind),
            Some(CheckKind::Manual)
        );
    }

    #[test]
    fn check_updates_cli_option_parses_without_ui_options() {
        let args = vec!["exmonitor-viewer".to_owned(), "--check-updates".to_owned()];
        let options = parse_options(&args, 1).unwrap();
        assert!(options.check_updates);
        assert!(options.benchmark_seconds.is_none());
        assert!(options.webex_controls.is_none());
    }

    #[test]
    fn composition_cursor_benchmark_accepts_generic_and_legacy_flags() {
        let invalid = vec![
            "exmonitor-viewer".to_owned(),
            "--benchmark".to_owned(),
            "20".to_owned(),
            "--benchmark-composition-cursor".to_owned(),
        ];
        assert!(parse_options(&invalid, 1).is_err());
        let system_cursor = vec![
            "exmonitor-viewer".to_owned(),
            "--benchmark".to_owned(),
            "20".to_owned(),
            "--dwm-composition".to_owned(),
            "--benchmark-composition-cursor".to_owned(),
        ];
        let options = parse_options(&system_cursor, 1).unwrap();
        assert!(options.benchmark_webex_cursor);
        assert!(!options.benchmark_cursor);
        assert_eq!(options.webex_controls, Some(true));
        let valid = vec![
            "exmonitor-viewer".to_owned(),
            "--benchmark".to_owned(),
            "20".to_owned(),
            "--benchmark-cursor".to_owned(),
            "--dwm-composition".to_owned(),
            "--benchmark-webex-cursor".to_owned(),
        ];
        let options = parse_options(&valid, 1).unwrap();
        assert!(options.benchmark_webex_cursor);
        assert_eq!(options.webex_controls, Some(true));
    }

    #[test]
    fn parses_settings_file_without_changing_benchmark_options() {
        let args = vec![
            "exmonitor-viewer".to_owned(),
            "--settings-file".to_owned(),
            "isolated/settings.json".to_owned(),
            "--monitor".to_owned(),
            "2".to_owned(),
            "--fps".to_owned(),
            "30".to_owned(),
            "--size".to_owned(),
            "800x450".to_owned(),
            "--benchmark".to_owned(),
            "5".to_owned(),
        ];
        let options = parse_options(&args, 2).unwrap();
        assert_eq!(
            options.settings_file.as_deref(),
            Some("isolated/settings.json")
        );
        assert_eq!(options.monitor, Some(1));
        assert_eq!(options.fps, Some(30));
        assert_eq!(options.size, Some((800, 450)));
        assert_eq!(options.benchmark_seconds, Some(5));
    }

    #[test]
    fn generic_and_legacy_cli_flags_explicitly_toggle_composition() {
        let enable = vec![
            "exmonitor-viewer".to_owned(),
            "--dwm-composition".to_owned(),
        ];
        assert_eq!(
            parse_options(&enable, 1).unwrap().webex_controls,
            Some(true)
        );
        let disable = vec![
            "exmonitor-viewer".to_owned(),
            "--dwm-composition".to_owned(),
            "--no-dwm-composition".to_owned(),
        ];
        assert_eq!(
            parse_options(&disable, 1).unwrap().webex_controls,
            Some(false)
        );
        let legacy_enable = vec!["exmonitor-viewer".to_owned(), "--webex-controls".to_owned()];
        assert_eq!(
            parse_options(&legacy_enable, 1).unwrap().webex_controls,
            Some(true)
        );
        let legacy_disable = vec![
            "exmonitor-viewer".to_owned(),
            "--no-webex-controls".to_owned(),
        ];
        assert_eq!(
            parse_options(&legacy_disable, 1).unwrap().webex_controls,
            Some(false)
        );
    }

    #[test]
    fn warp_shader_composites_color_masked_and_monochrome_shapes() {
        unsafe {
            let background = [20, 40, 60, 255];
            let color = render_cursor_test(
                (1, 1),
                (1, 1),
                &background,
                (1, 1),
                &[200, 100, 50, 128],
                [0, 0],
                0,
                0,
            );
            assert_eq!(color[0], [110, 70, 55, 255]);

            let masked = render_cursor_test(
                (2, 1),
                (2, 1),
                &[10, 20, 30, 255, 10, 20, 30, 255],
                (2, 1),
                &[1, 2, 3, 0, 0xFF, 0x00, 0xFF, 255],
                [0, 0],
                2,
                0,
            );
            assert_eq!(masked, [[1, 2, 3, 255], [245, 20, 225, 255]]);

            let monochrome = render_cursor_test(
                (4, 1),
                (4, 1),
                &[
                    0, 0, 0, 255, 255, 255, 255, 255, 90, 90, 90, 255, 90, 90, 90, 255,
                ],
                (4, 1),
                &[
                    0, 0, 0, 0, // AND=0, XOR=0
                    0, 0, 0, 255, // AND=0, XOR=255
                    255, 255, 255, 0, // AND=255, XOR=0
                    255, 255, 255, 255, // AND=255, XOR=255
                ],
                [0, 0],
                1,
                0,
            );
            assert_eq!(
                monochrome,
                [
                    [0, 0, 0, 255],
                    [255, 255, 255, 255],
                    [90, 90, 90, 255],
                    [165, 165, 165, 255],
                ]
            );
        }
    }

    #[test]
    fn warp_shader_clips_cursor_and_rotates_source_pixels() {
        unsafe {
            let source = [
                10, 0, 0, 255, 20, 0, 0, 255, // row 0
                30, 0, 0, 255, 40, 0, 0, 255, // row 1
                50, 0, 0, 255, 60, 0, 0, 255, // row 2
            ];
            let pixels = render_cursor_test(
                (2, 3),
                (3, 2),
                &source,
                (2, 1),
                &[1, 2, 3, 0, 200, 0, 0, 0],
                [-1, 0],
                2,
                1,
            );
            assert_eq!(pixels[0], [200, 0, 0, 255]);
            assert_eq!(pixels[1], [30, 0, 0, 255]);

            let source = [0, 0, 0, 255].repeat(8);
            let scaled = render_cursor_test(
                (4, 2),
                (2, 1),
                &source,
                (1, 1),
                &[255, 0, 0, 0],
                [0, 0],
                2,
                0,
            );
            assert_eq!(scaled[0], [64, 0, 0, 255]);
            assert_eq!(scaled[1], [0, 0, 0, 255]);
        }
    }
}
