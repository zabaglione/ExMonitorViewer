use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub schema_version: u32,
    pub monitor: Option<String>,
    pub fps: u32,
    pub topmost: bool,
    pub window: WindowSettings,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WindowSettings {
    pub left: i32,
    pub top: i32,
    pub width: u32,
    pub height: u32,
    pub maximized: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            monitor: None,
            fps: 60,
            topmost: false,
            window: WindowSettings {
                left: 100,
                top: 100,
                width: 1000,
                height: 600,
                maximized: false,
            },
        }
    }
}

impl Settings {
    pub fn validate(self) -> Result<Self, String> {
        if self.schema_version != SCHEMA_VERSION {
            return Err("unsupported settings schema".into());
        }
        if ![0, 30, 60].contains(&self.fps) {
            return Err("invalid FPS setting".into());
        }
        if self.monitor.as_ref().is_some_and(|name| {
            name.len() > 64 || !name.starts_with("\\\\.\\DISPLAY") || !name.is_ascii()
        }) {
            return Err("invalid monitor setting".into());
        }
        let w = self.window;
        if !(320..=8000).contains(&w.width)
            || !(240..=4500).contains(&w.height)
            || w.left.unsigned_abs() > 100_000
            || w.top.unsigned_abs() > 100_000
        {
            return Err("invalid window geometry".into());
        }
        Ok(self)
    }
}

pub fn default_path() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA")
        .map(|root| root.into())
        .map(|root: PathBuf| root.join("ExMonitorViewer").join("settings.json"))
}

pub fn choose_monitor(
    explicit_index: Option<usize>,
    saved_name: Option<&str>,
    monitors: &[(String, bool)],
) -> usize {
    if monitors.is_empty() {
        return 0;
    }
    if let Some(index) = explicit_index {
        return index.min(monitors.len() - 1);
    }
    saved_name
        .and_then(|name| monitors.iter().position(|(monitor, _)| monitor == name))
        .or_else(|| monitors.iter().position(|(_, primary)| !primary))
        .unwrap_or(0)
}

pub fn load(path: &Path) -> Result<Settings, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("settings read failed: {e}"))?;
    let settings: Settings =
        serde_json::from_slice(&bytes).map_err(|e| format!("settings parse failed: {e}"))?;
    settings.validate()
}

pub fn save(path: &Path, settings: &Settings) -> Result<(), String> {
    settings.clone().validate()?;
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent).map_err(|e| format!("settings directory failed: {e}"))?;
    let file_name = path.file_name().ok_or("settings path has no file name")?;
    let mut temp_name = file_name.to_os_string();
    temp_name.push(format!(".{}.tmp", std::process::id()));
    let temp = parent.join(temp_name);
    let bytes =
        serde_json::to_vec_pretty(settings).map_err(|e| format!("settings encode failed: {e}"))?;
    use std::io::Write;
    let mut file =
        std::fs::File::create(&temp).map_err(|e| format!("settings create failed: {e}"))?;
    file.write_all(&bytes)
        .map_err(|e| format!("settings write failed: {e}"))?;
    file.sync_all()
        .map_err(|e| format!("settings flush failed: {e}"))?;
    drop(file);
    replace_file(&temp, path).map_err(|e| {
        let _ = std::fs::remove_file(&temp);
        format!("settings replace failed: {e}")
    })
}

#[cfg(windows)]
fn replace_file(temp: &Path, destination: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };
    let from = temp
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let to = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    unsafe {
        MoveFileExW(
            windows::core::PCWSTR(from.as_ptr()),
            windows::core::PCWSTR(to.as_ptr()),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
        .map_err(std::io::Error::from)
    }
}

#[cfg(not(windows))]
fn replace_file(temp: &Path, destination: &Path) -> std::io::Result<()> {
    std::fs::rename(temp, destination)
}

pub fn clamp_rect(
    left: i32,
    top: i32,
    width: u32,
    height: u32,
    work: (i32, i32, i32, i32),
) -> WindowSettings {
    let (wl, wt, wr, wb) = work;
    let max_w = (wr - wl).max(320) as u32;
    let max_h = (wb - wt).max(240) as u32;
    let width = width.clamp(320, max_w.min(8000));
    let height = height.clamp(240, max_h.min(4500));
    let min_left = wl;
    let max_left = wr.saturating_sub(width as i32).max(wl);
    let min_top = wt;
    let max_top = wb.saturating_sub(height as i32).max(wt);
    WindowSettings {
        left: left.clamp(min_left, max_left),
        top: top.clamp(min_top, max_top),
        width,
        height,
        maximized: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_schema_and_ranges() {
        let settings = Settings {
            fps: 45,
            ..Settings::default()
        };
        assert!(settings.validate().is_err());
        let settings = Settings {
            schema_version: 2,
            ..Settings::default()
        };
        assert!(settings.validate().is_err());
        let settings = Settings {
            monitor: Some("\\\\.\\DISPLAY2".into()),
            ..Settings::default()
        };
        assert!(settings.validate().is_ok());
    }

    #[test]
    fn missing_saved_monitor_falls_back_to_secondary_then_primary() {
        let monitors = vec![
            ("\\\\.\\DISPLAY1".into(), true),
            ("\\\\.\\DISPLAY2".into(), false),
        ];
        assert_eq!(
            choose_monitor(None, Some("\\\\.\\DISPLAY999"), &monitors),
            1
        );
        assert_eq!(
            choose_monitor(Some(0), Some("\\\\.\\DISPLAY2"), &monitors),
            0
        );
        assert_eq!(choose_monitor(None, None, &monitors[..1]), 0);
    }

    #[test]
    fn clamps_negative_and_offscreen_rectangles_to_work_area() {
        let rect = clamp_rect(-3000, 4000, 1920, 1080, (-1920, 0, 0, 1040));
        assert!(rect.left < 0);
        assert_eq!(rect.left, -1920);
        assert_eq!(rect.top, 0);
        assert_eq!((rect.width, rect.height), (1920, 1040));
    }

    #[test]
    fn round_trips_and_rejects_corrupt_json() {
        let root =
            std::env::temp_dir().join(format!("exmonitor-settings-test-{}", std::process::id()));
        let path = root.join("settings.json");
        let settings = Settings::default();
        save(&path, &settings).unwrap();
        assert_eq!(load(&path).unwrap(), settings);
        std::fs::write(&path, b"{broken").unwrap();
        assert!(load(&path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"{broken");
        let _ = std::fs::remove_dir_all(root);
    }
}
