use serde::Deserialize;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;
use std::time::{Duration, Instant};

pub const RELEASE_PAGE: &str = "https://github.com/zabaglione/ExMonitorViewer/releases/latest";
const API_HOST: &str = "api.github.com";
const API_PATH: &str = "/repos/zabaglione/ExMonitorViewer/releases/latest";
const MAX_BODY: usize = 64 * 1024;
const REQUEST_DEADLINE: Duration = Duration::from_secs(12);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReleaseInfo {
    pub version: Version,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct Version(pub u64, pub u64, pub u64);

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.0, self.1, self.2)
    }
}

#[derive(Deserialize)]
struct LatestRelease {
    tag_name: String,
    draft: bool,
    prerelease: bool,
}

pub fn current_version() -> Version {
    parse_version(env!("CARGO_PKG_VERSION")).expect("package version must be stable x.y.z")
}

pub fn parse_version(input: &str) -> Option<Version> {
    let input = input.strip_prefix('v').unwrap_or(input);
    let mut parts = input.split('.');
    let parse = |part: Option<&str>| -> Option<u64> {
        let part = part?;
        if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        if part.len() > 1 && part.starts_with('0') {
            return None;
        }
        part.parse().ok()
    };
    let version = Version(
        parse(parts.next())?,
        parse(parts.next())?,
        parse(parts.next())?,
    );
    if parts.next().is_some() {
        return None;
    }
    Some(version)
}

pub fn parse_release_json(bytes: &[u8]) -> Result<ReleaseInfo, String> {
    if bytes.len() > MAX_BODY {
        return Err("response exceeds 64 KiB".into());
    }
    let release: LatestRelease = serde_json::from_slice(bytes)
        .map_err(|_| "release response has missing or invalid fields".to_owned())?;
    if release.draft || release.prerelease {
        return Err("latest release is not stable".into());
    }
    let version = parse_version(&release.tag_name)
        .ok_or_else(|| "release tag is not a stable three-part version".to_owned())?;
    Ok(ReleaseInfo { version })
}

pub fn is_newer(latest: Version, current: Version) -> bool {
    latest > current
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CheckKind {
    Automatic,
    Manual,
}

#[derive(Debug, Eq, PartialEq)]
pub enum Notification {
    Silent,
    Discard,
    UpdateAvailable(Version),
    UpToDate(Version),
    ManualError(String),
    AutomaticError(String),
}

pub fn classify_result(
    kind: CheckKind,
    result: Result<ReleaseInfo, String>,
    current: Version,
    startup_enabled: bool,
) -> Notification {
    if kind == CheckKind::Automatic && !startup_enabled {
        return Notification::Discard;
    }
    match result {
        Ok(release) if is_newer(release.version, current) => {
            Notification::UpdateAvailable(release.version)
        }
        Ok(_) if kind == CheckKind::Manual => Notification::UpToDate(current),
        Ok(_) => Notification::Silent,
        Err(error) if kind == CheckKind::Manual => Notification::ManualError(error),
        Err(error) => Notification::AutomaticError(error),
    }
}

pub fn release_page_after_confirmation(confirmed: bool) -> Option<&'static str> {
    confirmed.then_some(RELEASE_PAGE)
}

pub struct UpdateCheck {
    receiver: Receiver<Result<ReleaseInfo, String>>,
    pub kind: CheckKind,
}

impl UpdateCheck {
    pub fn start(kind: CheckKind) -> Option<Self> {
        Self::start_with(kind, check_latest)
    }

    fn start_with(kind: CheckKind, fetch: fn() -> Result<ReleaseInfo, String>) -> Option<Self> {
        let (sender, receiver) = mpsc::channel();
        thread::Builder::new()
            .name("update-check".into())
            .spawn(move || {
                let _ = sender.send(fetch());
            })
            .ok()?;
        Some(Self { receiver, kind })
    }

    #[cfg(test)]
    pub(crate) fn completed_for_test(kind: CheckKind, result: Result<ReleaseInfo, String>) -> Self {
        let (sender, receiver) = mpsc::channel();
        let _ = sender.send(result);
        Self { receiver, kind }
    }

    pub fn try_finish(&self) -> Result<Option<Result<ReleaseInfo, String>>, String> {
        match self.receiver.try_recv() {
            Ok(result) => Ok(Some(result)),
            Err(TryRecvError::Empty) => Ok(None),
            Err(TryRecvError::Disconnected) => Err("update worker stopped".into()),
        }
    }
}

pub fn check_latest() -> Result<ReleaseInfo, String> {
    #[cfg(windows)]
    {
        fetch_latest_winhttp()
    }
    #[cfg(not(windows))]
    {
        Err("update checks require Windows WinHTTP".into())
    }
}

#[cfg(windows)]
struct InternetHandle(*mut std::ffi::c_void);

#[cfg(windows)]
impl InternetHandle {
    fn new(raw: *mut std::ffi::c_void) -> Result<Self, String> {
        if raw.is_null() {
            Err("WinHTTP handle creation failed".into())
        } else {
            Ok(Self(raw))
        }
    }
}

#[cfg(windows)]
impl Drop for InternetHandle {
    fn drop(&mut self) {
        unsafe {
            let _ = windows::Win32::Networking::WinHttp::WinHttpCloseHandle(self.0);
        }
    }
}

#[cfg(windows)]
fn fetch_latest_winhttp() -> Result<ReleaseInfo, String> {
    use windows::core::{w, PCWSTR};
    use windows::Win32::Networking::WinHttp::*;

    let started = Instant::now();
    let check_deadline = || {
        if started.elapsed() >= REQUEST_DEADLINE {
            Err("update request timed out".to_owned())
        } else {
            Ok(())
        }
    };
    let agent = format!("ExMonitorViewer/{}", env!("CARGO_PKG_VERSION"));
    let agent: Vec<u16> = agent.encode_utf16().chain(Some(0)).collect();
    let session = unsafe {
        InternetHandle::new(WinHttpOpen(
            PCWSTR(agent.as_ptr()),
            WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
            PCWSTR::null(),
            PCWSTR::null(),
            0,
        ))?
    };
    unsafe { WinHttpSetTimeouts(session.0, 3000, 3000, 3000, 3000) }
        .map_err(|_| "WinHTTP timeout configuration failed".to_owned())?;
    let no_default_auth = WINHTTP_AUTOLOGON_SECURITY_LEVEL_HIGH.to_ne_bytes();
    unsafe {
        WinHttpSetOption(
            Some(session.0),
            WINHTTP_OPTION_AUTOLOGON_POLICY,
            Some(&no_default_auth),
        )
    }
    .map_err(|_| "WinHTTP authentication policy configuration failed".to_owned())?;
    let single_connection_attempt = 1u32.to_ne_bytes();
    unsafe {
        WinHttpSetOption(
            Some(session.0),
            WINHTTP_OPTION_CONNECT_RETRIES,
            Some(&single_connection_attempt),
        )
    }
    .map_err(|_| "WinHTTP connection policy configuration failed".to_owned())?;
    check_deadline()?;
    let host: Vec<u16> = API_HOST.encode_utf16().chain(Some(0)).collect();
    let connection = unsafe {
        InternetHandle::new(WinHttpConnect(
            session.0,
            PCWSTR(host.as_ptr()),
            INTERNET_DEFAULT_HTTPS_PORT,
            0,
        ))?
    };
    let path: Vec<u16> = API_PATH.encode_utf16().chain(Some(0)).collect();
    let request = unsafe {
        InternetHandle::new(WinHttpOpenRequest(
            connection.0,
            w!("GET"),
            PCWSTR(path.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            std::ptr::null(),
            WINHTTP_FLAG_SECURE,
        ))?
    };
    let never = WINHTTP_OPTION_REDIRECT_POLICY_NEVER.to_ne_bytes();
    unsafe {
        WinHttpSetOption(
            Some(request.0),
            WINHTTP_OPTION_REDIRECT_POLICY,
            Some(&never),
        )
    }
    .map_err(|_| "WinHTTP redirect policy configuration failed".to_owned())?;
    let headers: Vec<u16> =
        "Accept: application/vnd.github+json\r\nX-GitHub-Api-Version: 2022-11-28\r\n"
            .encode_utf16()
            .collect();
    unsafe { WinHttpAddRequestHeaders(request.0, &headers, WINHTTP_ADDREQ_FLAG_ADD) }
        .map_err(|_| "WinHTTP request header setup failed".to_owned())?;
    check_deadline()?;
    unsafe { WinHttpSendRequest(request.0, None, None, 0, 0, 0) }
        .map_err(|_| "update request send failed".to_owned())?;
    check_deadline()?;
    unsafe { WinHttpReceiveResponse(request.0, std::ptr::null_mut()) }
        .map_err(|_| "update response receive failed".to_owned())?;
    check_deadline()?;
    let mut status = 0u32;
    unsafe {
        WinHttpQueryHeaders(
            request.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            PCWSTR::null(),
            Some((&mut status as *mut u32).cast()),
            &mut (std::mem::size_of::<u32>() as u32),
            std::ptr::null_mut(),
        )
    }
    .map_err(|_| "HTTP status query failed".to_owned())?;
    if status != 200 {
        return Err(format!("GitHub API returned HTTP {status}"));
    }
    let mut body = Vec::with_capacity(4096);
    loop {
        check_deadline()?;
        let mut available = 0u32;
        unsafe { WinHttpQueryDataAvailable(request.0, &mut available) }
            .map_err(|_| "update response read failed".to_owned())?;
        if available == 0 {
            break;
        }
        if body.len().saturating_add(available as usize) > MAX_BODY {
            return Err("response exceeds 64 KiB".into());
        }
        let mut chunk = vec![0u8; available.min(8192) as usize];
        let mut read = 0u32;
        unsafe {
            WinHttpReadData(
                request.0,
                chunk.as_mut_ptr().cast(),
                chunk.len() as u32,
                &mut read,
            )
        }
        .map_err(|_| "update response read failed".to_owned())?;
        if read == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..read as usize]);
    }
    check_deadline()?;
    parse_release_json(&body)
}

pub fn cli_result_json(current: Version, result: Result<ReleaseInfo, &str>) -> String {
    match result {
        Ok(latest) => format!(
            "{{\"current\":\"{current}\",\"latest\":\"{}\",\"status\":\"{}\"}}",
            latest.version,
            if is_newer(latest.version, current) {
                "update_available"
            } else {
                "up_to_date"
            }
        ),
        Err(error) => format!(
            "{{\"current\":\"{current}\",\"status\":\"error\",\"error\":\"{}\"}}",
            ascii_json(error)
        ),
    }
}

fn ascii_json(text: &str) -> String {
    text.chars()
        .map(|c| {
            if c.is_ascii() && c != '"' && c != '\\' {
                c
            } else {
                '?'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn parses_only_stable_three_part_versions() {
        assert_eq!(parse_version("v1.2.3"), Some(Version(1, 2, 3)));
        assert_eq!(parse_version("1.2.3"), Some(Version(1, 2, 3)));
        for invalid in [
            "v1.2",
            "1.2.3.4",
            "v1.2.3-beta",
            "1.2.3+meta",
            "01.2.3",
            "1.2.18446744073709551616",
        ] {
            assert_eq!(parse_version(invalid), None, "{invalid}");
        }
    }

    #[test]
    fn requires_exact_types_for_required_release_fields() {
        let valid = br#"{"tag_name":"v0.1.6","draft":false,"prerelease":false,"body":42}"#;
        assert_eq!(parse_release_json(valid).unwrap().version, Version(0, 1, 6));
        for invalid in [
            br#"{"tag_name":"v0.1.6","draft":false}"#.as_slice(),
            br#"{"tag_name":1,"draft":false,"prerelease":false}"#,
            br#"{"tag_name":"v0.1.6","draft":"false","prerelease":false}"#,
            br#"{"tag_name":"v0.1.6","draft":false,"prerelease":true}"#,
            br#"{"tag_name":"v0.1.6-rc1","draft":false,"prerelease":false}"#,
        ] {
            assert!(parse_release_json(invalid).is_err());
        }
        assert!(parse_release_json(&vec![b' '; MAX_BODY + 1]).is_err());
    }

    #[test]
    fn update_comparison_and_cli_result_are_deterministic() {
        assert!(is_newer(Version(0, 2, 0), Version(0, 1, 9)));
        assert!(!is_newer(Version(0, 1, 5), Version(0, 1, 5)));
        assert_eq!(
            cli_result_json(
                Version(0, 1, 5),
                Ok(ReleaseInfo {
                    version: Version(0, 1, 6)
                })
            ),
            r#"{"current":"0.1.5","latest":"0.1.6","status":"update_available"}"#
        );
    }

    #[test]
    fn notification_policy_covers_manual_auto_off_error_and_decline() {
        let current = Version(0, 1, 5);
        let newer = Ok(ReleaseInfo {
            version: Version(0, 1, 6),
        });
        assert_eq!(
            classify_result(CheckKind::Manual, newer.clone(), current, true),
            Notification::UpdateAvailable(Version(0, 1, 6))
        );
        assert_eq!(
            classify_result(
                CheckKind::Manual,
                Ok(ReleaseInfo { version: current }),
                current,
                true
            ),
            Notification::UpToDate(current)
        );
        assert_eq!(
            classify_result(CheckKind::Automatic, newer.clone(), current, false),
            Notification::Discard
        );
        assert_eq!(
            classify_result(CheckKind::Automatic, newer, current, true),
            Notification::UpdateAvailable(Version(0, 1, 6))
        );
        assert_eq!(
            classify_result(CheckKind::Automatic, Err("offline".into()), current, true),
            Notification::AutomaticError("offline".into())
        );
        assert_eq!(
            classify_result(CheckKind::Manual, Err("offline".into()), current, true),
            Notification::ManualError("offline".into())
        );
        assert_eq!(release_page_after_confirmation(false), None);
        assert_eq!(release_page_after_confirmation(true), Some(RELEASE_PAGE));
    }

    static FETCHES: AtomicUsize = AtomicUsize::new(0);
    fn test_fetch() -> Result<ReleaseInfo, String> {
        FETCHES.fetch_add(1, Ordering::SeqCst);
        Ok(ReleaseInfo {
            version: Version(0, 1, 6),
        })
    }

    #[test]
    fn update_worker_delivers_once_and_receiver_drop_is_safe() {
        FETCHES.store(0, Ordering::SeqCst);
        let worker = UpdateCheck::start_with(CheckKind::Manual, test_fetch).unwrap();
        assert_eq!(worker.kind, CheckKind::Manual);
        for _ in 0..100 {
            if let Ok(Some(result)) = worker.try_finish() {
                assert_eq!(result.unwrap().version, Version(0, 1, 6));
                assert!(!matches!(worker.try_finish(), Ok(Some(_))));
                assert_eq!(FETCHES.load(Ordering::SeqCst), 1);
                return;
            }
            thread::sleep(Duration::from_millis(2));
        }
        panic!("worker did not deliver result");
    }

    #[test]
    fn worker_send_after_receiver_drop_does_not_panic() {
        fn independent_fetch() -> Result<ReleaseInfo, String> {
            Ok(ReleaseInfo {
                version: Version(9, 9, 9),
            })
        }
        drop(UpdateCheck::start_with(CheckKind::Automatic, independent_fetch).unwrap());
        thread::sleep(Duration::from_millis(5));
    }
}
