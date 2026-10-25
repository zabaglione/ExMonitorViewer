use std::env;
use std::ffi::c_void;
use std::fs;
use std::mem::size_of;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use windows::core::{Interface, BOOL, PCSTR, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Direct3D::Fxc::{D3DCompile, D3DCOMPILE_ENABLE_STRICTNESS};
use windows::Win32::Graphics::Direct3D::{
    ID3DBlob, D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST, D3D_DRIVER_TYPE_UNKNOWN,
};
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::*;
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, ScreenToClient, HMONITOR, MONITORINFOEXW,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{
    SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows::Win32::UI::WindowsAndMessaging::*;

const CLASS_NAME: PCWSTR = windows::core::w!("MotionSourceWindow");
const WINDOW_TITLE: PCWSTR = windows::core::w!("Motion Source");
static CURSOR_TEST: AtomicBool = AtomicBool::new(false);
const SHADER: &[u8] = br#"
cbuffer Params : register(b0) { float time; float width; float height; float pad; };
struct VertexOut { float4 position : SV_POSITION; float2 uv : TEXCOORD0; };
VertexOut VSMain(uint id : SV_VertexID) {
    float2 p = id == 0 ? float2(-1, -1) : (id == 1 ? float2(-1, 3) : float2(3, -1));
    VertexOut o;
    o.position = float4(p, 0, 1);
    o.uv = float2(p.x * 0.5 + 0.5, 0.5 - p.y * 0.5);
    return o;
}
float4 PSMain(VertexOut input) : SV_TARGET {
    float2 uv = input.uv;
    float3 color = float3(0.021, 0.026, 0.038);
    if (pad > 0.5) {
        float band = min(min(uv.x, 1.0 - uv.x), min(abs(uv.x - 0.25), min(abs(uv.x - 0.5), abs(uv.x - 0.75))));
        color = uv.x < 0.25 ? float3(0.035, 0.055, 0.075) :
                (uv.x < 0.5 ? float3(0.055, 0.035, 0.070) :
                (uv.x < 0.75 ? float3(0.070, 0.055, 0.025) : float3(0.035, 0.070, 0.040)));
        if (band < 0.0015) color = float3(0.35, 0.4, 0.45);
    }
    float lane = abs(uv.y - 0.5);
    if (lane < 0.0022) color = float3(0.16, 0.2, 0.24);
    float x = frac(time * 0.22);
    float distanceToBar = abs(uv.x - x);
    distanceToBar = min(distanceToBar, 1.0 - distanceToBar);
    if (distanceToBar < 0.025 && lane < 0.06) {
        float pulse = 0.65 + 0.12 * sin(time * 2.0);
        float3 tint = uv.x < x ? float3(0.0, 0.62, 0.78) : float3(0.64, 0.34, 0.06);
        color = lerp(color, tint, pulse);
    }
    float markA = abs(uv.x - 0.08);
    float markB = abs(uv.x - 0.92);
    if ((markA < 0.0015 || markB < 0.0015) && lane < 0.08) color = float3(0.2, 0.28, 0.36);
    return float4(color, 1.0);
}
"#;

#[derive(Debug)]
struct Options {
    display: String,
    seconds: u64,
    report: Option<PathBuf>,
    list_monitors: bool,
    cursor_test: bool,
}

#[derive(Clone, Copy)]
struct MonitorBounds {
    rect: RECT,
}

#[derive(Default)]
struct MonitorCollection {
    entries: Vec<(String, MonitorBounds)>,
}

#[derive(Default)]
struct RunStats {
    presents: u64,
    elapsed: Duration,
    adapter_name: String,
}

struct Renderer {
    context: ID3D11DeviceContext,
    swap_chain: IDXGISwapChain1,
    render_target: ID3D11RenderTargetView,
    vertex_shader: ID3D11VertexShader,
    pixel_shader: ID3D11PixelShader,
    constants: ID3D11Buffer,
    viewport_width: u32,
    viewport_height: u32,
}

struct OutputBinding {
    adapter: IDXGIAdapter1,
    adapter_name: String,
}

fn parse_options() -> Result<Options, String> {
    let mut display = None;
    let mut seconds = 30u64;
    let mut report = None;
    let mut list_monitors = false;
    let mut cursor_test = false;
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--display" => display = args.next(),
            "--seconds" => {
                let value = args.next().ok_or("missing seconds value")?;
                seconds = value.parse().map_err(|_| "invalid seconds value")?;
            }
            "--report" => report = args.next().map(PathBuf::from),
            "--list-monitors" => list_monitors = true,
            "--cursor-test" => cursor_test = true,
            "--help" | "-h" => {
                println!(
                    r"Usage: motion-source --display \\.\DISPLAY2 [--seconds 30] [--report path] [--cursor-test]"
                );
                println!("       motion-source --list-monitors");
                println!("       --cursor-test divides the screen into Arrow, I-beam, Crosshair, and hidden cursor zones");
                std::process::exit(0);
            }
            _ => return Err("unknown argument".into()),
        }
    }
    if seconds == 0 || seconds > 86_400 {
        return Err("seconds must be between 1 and 86400".into());
    }
    let display = display.unwrap_or_default();
    if list_monitors {
        return Ok(Options {
            display,
            seconds,
            report,
            list_monitors,
            cursor_test,
        });
    }
    if display.is_empty() {
        return Err("--display is required".into());
    }
    if !display.starts_with(r"\\.\DISPLAY") {
        return Err(r"display must use the \\.\DISPLAYn form".into());
    }
    Ok(Options {
        display,
        seconds,
        report,
        list_monitors,
        cursor_test,
    })
}

unsafe extern "system" fn collect_monitor(
    monitor: HMONITOR,
    _hdc: windows::Win32::Graphics::Gdi::HDC,
    _rect: *mut RECT,
    data: LPARAM,
) -> BOOL {
    let monitors = &mut *(data.0 as *mut MonitorCollection);
    let mut info = MONITORINFOEXW::default();
    info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
    if GetMonitorInfoW(monitor, &mut info.monitorInfo).as_bool() {
        let end = info
            .szDevice
            .iter()
            .position(|value| *value == 0)
            .unwrap_or(info.szDevice.len());
        let name = String::from_utf16_lossy(&info.szDevice[..end]);
        monitors.entries.push((
            name,
            MonitorBounds {
                rect: info.monitorInfo.rcMonitor,
            },
        ));
    }
    BOOL(1)
}

fn monitor_bounds(display: &str) -> Result<MonitorBounds, String> {
    let mut monitors = MonitorCollection::default();
    unsafe {
        if !EnumDisplayMonitors(
            None,
            None,
            Some(collect_monitor),
            LPARAM(&mut monitors as *mut _ as isize),
        )
        .as_bool()
        {
            return Err("failed to enumerate display monitors".into());
        }
    }
    monitors
        .entries
        .into_iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(display))
        .map(|(_, bounds)| bounds)
        .ok_or_else(|| "requested display was not found".into())
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

fn ascii_safe(text: &str) -> String {
    text.chars()
        .map(|ch| if ch.is_ascii() { ch } else { '?' })
        .collect()
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_SETCURSOR if CURSOR_TEST.load(Ordering::Relaxed) => {
            let hit_test = (lparam.0 as u32 & 0xffff) as u16;
            if hit_test == HTCLIENT as u16 {
                let mut point = POINT::default();
                if GetCursorPos(&mut point).is_ok() && ScreenToClient(hwnd, &mut point).as_bool() {
                    let mut rect = RECT::default();
                    if GetClientRect(hwnd, &mut rect).is_ok() {
                        let width = rect.right - rect.left;
                        if let Some(zone) = cursor_zone(point.x, width) {
                            let cursor = match zone {
                                0 => Some(IDC_ARROW),
                                1 => Some(IDC_IBEAM),
                                2 => Some(IDC_CROSS),
                                _ => None,
                            };
                            match cursor {
                                Some(id) => {
                                    if let Ok(cursor) = LoadCursorW(None, id) {
                                        let _ = SetCursor(Some(cursor));
                                    }
                                }
                                None => {
                                    let _ = SetCursor(None);
                                }
                            }
                            return LRESULT(1);
                        }
                    }
                }
            }
            DefWindowProcW(hwnd, message, wparam, lparam)
        }
        WM_KEYDOWN if wparam.0 as u32 == 27 => {
            let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
            LRESULT(0)
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, message, wparam, lparam),
    }
}

fn cursor_zone(x: i32, width: i32) -> Option<usize> {
    if width <= 0 || x < 0 || x >= width {
        return None;
    }
    Some(((x as i64 * 4) / width as i64) as usize)
}

fn create_window(bounds: MonitorBounds) -> Result<HWND, String> {
    unsafe {
        let instance = GetModuleHandleW(None).map_err(|_| "failed to get module handle")?;
        let class = WNDCLASSW {
            hCursor: LoadCursorW(None, IDC_ARROW).map_err(|_| "failed to load cursor")?,
            hInstance: instance.into(),
            lpszClassName: CLASS_NAME,
            lpfnWndProc: Some(window_proc),
            ..Default::default()
        };
        if RegisterClassW(&class) == 0 {
            return Err("failed to register window class".into());
        }
        let rect = bounds.rect;
        let width = rect.right - rect.left;
        let height = rect.bottom - rect.top;
        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST,
            CLASS_NAME,
            WINDOW_TITLE,
            WS_POPUP,
            rect.left,
            rect.top,
            width,
            height,
            None,
            None,
            Some(instance.into()),
            None,
        )
        .map_err(|_| "failed to create motion pattern window")?;
        let _ = ShowWindow(hwnd, SW_SHOW);
        Ok(hwnd)
    }
}

fn find_output(display: &str) -> Result<OutputBinding, String> {
    unsafe {
        let factory: IDXGIFactory1 =
            CreateDXGIFactory1().map_err(|_| "failed to create DXGI factory")?;
        let mut index = 0;
        loop {
            let adapter = match factory.EnumAdapters1(index) {
                Ok(adapter) => adapter,
                Err(_) => break,
            };
            let mut output_index = 0;
            loop {
                let output = match adapter.EnumOutputs(output_index) {
                    Ok(output) => output,
                    Err(_) => break,
                };
                let desc = output
                    .GetDesc()
                    .map_err(|_| "failed to get display output description")?;
                let end = desc
                    .DeviceName
                    .iter()
                    .position(|value| *value == 0)
                    .unwrap_or(desc.DeviceName.len());
                let name = String::from_utf16_lossy(&desc.DeviceName[..end]);
                if name.eq_ignore_ascii_case(display) {
                    let adapter_desc = adapter
                        .GetDesc1()
                        .map_err(|_| "failed to get adapter description")?;
                    let adapter_end = adapter_desc
                        .Description
                        .iter()
                        .position(|value| *value == 0)
                        .unwrap_or(adapter_desc.Description.len());
                    let adapter_name =
                        String::from_utf16_lossy(&adapter_desc.Description[..adapter_end]);
                    return Ok(OutputBinding {
                        adapter,
                        adapter_name,
                    });
                }
                output_index += 1;
            }
            index += 1;
        }
    }
    Err("requested display output was not found on a DXGI adapter".into())
}

fn list_monitors() -> Result<(), String> {
    let mut monitors = MonitorCollection::default();
    unsafe {
        if !EnumDisplayMonitors(
            None,
            None,
            Some(collect_monitor),
            LPARAM(&mut monitors as *mut _ as isize),
        )
        .as_bool()
        {
            return Err("failed to enumerate display monitors".into());
        }
    }
    for (name, _) in monitors.entries {
        match find_output(&name) {
            Ok(binding) => println!(
                "display={} adapter={}",
                name,
                ascii_safe(&binding.adapter_name)
            ),
            Err(_) => println!("display={} adapter=unavailable", name),
        }
    }
    Ok(())
}

fn compile_shader(entry: &'static [u8], target: &'static [u8]) -> Result<ID3DBlob, String> {
    unsafe {
        let mut bytecode = None;
        let mut errors = None;
        D3DCompile(
            SHADER.as_ptr().cast::<c_void>(),
            SHADER.len(),
            PCSTR::null(),
            None,
            None,
            PCSTR(entry.as_ptr()),
            PCSTR(target.as_ptr()),
            D3DCOMPILE_ENABLE_STRICTNESS,
            0,
            &mut bytecode,
            Some(&mut errors),
        )
        .map_err(|_| "failed to compile Direct3D shader")?;
        bytecode.ok_or_else(|| "shader compiler returned no bytecode".into())
    }
}

fn create_renderer(
    hwnd: HWND,
    binding: OutputBinding,
    width: u32,
    height: u32,
) -> Result<Renderer, String> {
    unsafe {
        let mut device = None;
        let mut context = None;
        let adapter_for_device = binding
            .adapter
            .cast::<IDXGIAdapter>()
            .map_err(|_| "failed to query DXGI adapter")?;
        D3D11CreateDevice(
            &adapter_for_device,
            D3D_DRIVER_TYPE_UNKNOWN,
            windows::Win32::Foundation::HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            None,
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            Some(&mut context),
        )
        .map_err(|_| "failed to create Direct3D 11 device")?;
        let device = device.ok_or("Direct3D device was not created")?;
        let context = context.ok_or("Direct3D device context was not created")?;

        let device_dxgi = device
            .cast::<IDXGIDevice>()
            .map_err(|_| "failed to query DXGI device")?;
        let factory: IDXGIFactory2 = device_dxgi
            .GetAdapter()
            .and_then(|adapter| adapter.GetParent())
            .map_err(|_| "failed to get DXGI factory")?;
        let desc = DXGI_SWAP_CHAIN_DESC1 {
            Width: width,
            Height: height,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            Stereo: BOOL(0),
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
            BufferCount: 2,
            Scaling: DXGI_SCALING_STRETCH,
            SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
            AlphaMode: DXGI_ALPHA_MODE_IGNORE,
            Flags: 0,
        };
        let swap_chain: IDXGISwapChain1 = factory
            .CreateSwapChainForHwnd(&device, hwnd, &desc, None, None)
            .map_err(|_| "failed to create window swap chain")?;
        let back_buffer: ID3D11Texture2D = swap_chain
            .GetBuffer(0)
            .map_err(|_| "failed to get swap-chain buffer")?;
        let mut render_target = None;
        device
            .CreateRenderTargetView(&back_buffer, None, Some(&mut render_target))
            .map_err(|_| "failed to create render target")?;
        let render_target = render_target.ok_or("render target was not created")?;

        let vertex = compile_shader(b"VSMain\0", b"vs_5_0\0")?;
        let pixel = compile_shader(b"PSMain\0", b"ps_5_0\0")?;
        let vertex_bytes = std::slice::from_raw_parts(
            vertex.GetBufferPointer().cast::<u8>(),
            vertex.GetBufferSize(),
        );
        let pixel_bytes = std::slice::from_raw_parts(
            pixel.GetBufferPointer().cast::<u8>(),
            pixel.GetBufferSize(),
        );
        let mut vertex_shader = None;
        device
            .CreateVertexShader(vertex_bytes, None, Some(&mut vertex_shader))
            .map_err(|_| "failed to create vertex shader")?;
        let vertex_shader = vertex_shader.ok_or("vertex shader was not created")?;
        let mut pixel_shader = None;
        device
            .CreatePixelShader(pixel_bytes, None, Some(&mut pixel_shader))
            .map_err(|_| "failed to create pixel shader")?;
        let pixel_shader = pixel_shader.ok_or("pixel shader was not created")?;

        let constant_desc = D3D11_BUFFER_DESC {
            ByteWidth: 16,
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: D3D11_BIND_CONSTANT_BUFFER.0 as u32,
            CPUAccessFlags: 0,
            MiscFlags: 0,
            StructureByteStride: 0,
        };
        let mut constants = None;
        device
            .CreateBuffer(&constant_desc, None, Some(&mut constants))
            .map_err(|_| "failed to create shader parameter buffer")?;
        let constants = constants.ok_or("shader parameter buffer was not created")?;
        let viewport = D3D11_VIEWPORT {
            TopLeftX: 0.0,
            TopLeftY: 0.0,
            Width: width as f32,
            Height: height as f32,
            MinDepth: 0.0,
            MaxDepth: 1.0,
        };
        context.RSSetViewports(Some(&[viewport]));

        Ok(Renderer {
            context,
            swap_chain,
            render_target,
            vertex_shader,
            pixel_shader,
            constants,
            viewport_width: width,
            viewport_height: height,
        })
    }
}

impl Renderer {
    fn render(&self, time: f32, cursor_test: bool) -> Result<(), String> {
        unsafe {
            let targets = [Some(self.render_target.clone())];
            self.context.OMSetRenderTargets(Some(&targets), None);
            self.context.VSSetShader(&self.vertex_shader, None);
            self.context.PSSetShader(&self.pixel_shader, None);
            self.context
                .IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            self.context
                .PSSetConstantBuffers(0, Some(&[Some(self.constants.clone())]));
            let params = [
                time,
                self.viewport_width as f32,
                self.viewport_height as f32,
                f32::from(cursor_test),
            ];
            self.context.UpdateSubresource(
                &self.constants,
                0,
                None,
                params.as_ptr().cast::<c_void>(),
                0,
                0,
            );
            self.context.Draw(3, 0);
            self.swap_chain
                .Present(1, DXGI_PRESENT(0))
                .ok()
                .map_err(|_| "swap-chain presentation failed")?;
        }
        Ok(())
    }
}

fn run(options: &Options) -> Result<RunStats, String> {
    CURSOR_TEST.store(options.cursor_test, Ordering::Relaxed);
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    let bounds = monitor_bounds(&options.display)?;
    let rect = bounds.rect;
    let width = (rect.right - rect.left) as u32;
    let height = (rect.bottom - rect.top) as u32;
    if width == 0 || height == 0 {
        return Err("selected display has an invalid size".into());
    }
    let hwnd = create_window(bounds)?;
    let binding = find_output(&options.display)?;
    let mut stats = RunStats {
        adapter_name: binding.adapter_name.clone(),
        ..RunStats::default()
    };
    let renderer = create_renderer(hwnd, binding, width, height)?;
    let start = Instant::now();
    let timeout = Duration::from_secs(options.seconds);
    let mut message = MSG::default();
    let mut title_update = Instant::now();
    loop {
        unsafe {
            while PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() {
                if message.message == WM_QUIT {
                    stats.elapsed = start.elapsed();
                    return Ok(stats);
                }
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        let elapsed = start.elapsed();
        if elapsed >= timeout {
            stats.elapsed = elapsed;
            break;
        }
        renderer.render(elapsed.as_secs_f32(), options.cursor_test)?;
        stats.presents += 1;
        stats.elapsed = start.elapsed();
        if title_update.elapsed() >= Duration::from_millis(500) {
            let title = wide(&format!(
                "Motion Source | {:.2} FPS",
                stats.presents as f64 / stats.elapsed.as_secs_f64()
            ));
            unsafe {
                let _ = SetWindowTextW(hwnd, PCWSTR(title.as_ptr()));
            }
            title_update = Instant::now();
        }
    }
    unsafe {
        let _ = DestroyWindow(hwnd);
    }
    Ok(stats)
}

fn write_report(path: &PathBuf, options: &Options, stats: &RunStats) -> Result<(), String> {
    let elapsed_ms = stats.elapsed.as_millis();
    let fps = if elapsed_ms == 0 {
        0.0
    } else {
        stats.presents as f64 * 1000.0 / elapsed_ms as f64
    };
    let text = format!(
        "display={}\nadapter={}\nelapsed_ms={}\nsuccessful_present_count={}\naverage_successful_present_fps={:.3}\n",
        ascii_safe(&options.display), ascii_safe(&stats.adapter_name), elapsed_ms, stats.presents, fps
    );
    fs::write(path, text).map_err(|_| "failed to write report file".into())
}

fn main() {
    let options = match parse_options() {
        Ok(options) => options,
        Err(error) => {
            eprintln!("ERROR: {error}");
            std::process::exit(2);
        }
    };
    if options.list_monitors {
        if let Err(error) = list_monitors() {
            eprintln!("ERROR: {error}");
            std::process::exit(1);
        }
        return;
    }
    let stats = match run(&options) {
        Ok(stats) => stats,
        Err(error) => {
            eprintln!("ERROR: {error}");
            std::process::exit(1);
        }
    };
    println!(
        "display={} adapter={} successful_present_count={} average_fps={:.3}",
        ascii_safe(&options.display),
        ascii_safe(&stats.adapter_name),
        stats.presents,
        if stats.elapsed.is_zero() {
            0.0
        } else {
            stats.presents as f64 / stats.elapsed.as_secs_f64()
        }
    );
    if let Some(path) = &options.report {
        if let Err(error) = write_report(path, &options, &stats) {
            eprintln!("ERROR: {error}");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::cursor_zone;

    #[test]
    fn cursor_zones_cover_equal_width_regions() {
        assert_eq!(cursor_zone(0, 1920), Some(0));
        assert_eq!(cursor_zone(479, 1920), Some(0));
        assert_eq!(cursor_zone(480, 1920), Some(1));
        assert_eq!(cursor_zone(960, 1920), Some(2));
        assert_eq!(cursor_zone(1440, 1920), Some(3));
        assert_eq!(cursor_zone(1919, 1920), Some(3));
    }

    #[test]
    fn cursor_zone_rejects_points_outside_the_client_width() {
        assert_eq!(cursor_zone(-1, 1920), None);
        assert_eq!(cursor_zone(1920, 1920), None);
        assert_eq!(cursor_zone(0, 0), None);
    }
}
