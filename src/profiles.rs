use windows::core::PWSTR;
use windows::Win32::Foundation::{CloseHandle, HWND};
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
    PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};

/// Returns the foreground window handle and its lowercase executable filename
/// (e.g. `(hwnd, "tlou-ii.exe")`), or `None` on failure or if it belongs to `overcap.exe`.
pub fn foreground_target_window() -> Option<(HWND, String)> {
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.0.is_null() {
            return None;
        }

        let mut pid: u32 = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == 0 {
            return None;
        }

        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let query_ok = QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_WIN32,
            PWSTR(buf.as_mut_ptr()),
            &mut len,
        )
        .is_ok();
        let _ = CloseHandle(handle);

        if !query_ok || len == 0 {
            return None;
        }

        let full_path = String::from_utf16_lossy(&buf[..len as usize]);
        let exe_name = full_path
            .rsplit(['\\', '/'])
            .next()?
            .trim()
            .to_lowercase();

        if exe_name.is_empty() || exe_name == "overcap.exe" {
            return None;
        }

        Some((hwnd, exe_name))
    }
}

/// Returns the lowercase executable filename (e.g. `"tlou-ii.exe"`, `"chrome.exe"`)
/// of the current foreground window's process, or `None` on failure or if the
/// foreground window belongs to `overcap.exe` itself.
pub fn foreground_process_name() -> Option<String> {
    foreground_target_window().map(|(_, name)| name)
}
