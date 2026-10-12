use std::ffi::c_void;
use std::mem::size_of;
use std::os::windows::io::AsRawHandle as _;
use std::process::Child;
use std::sync::Once;
use std::time::{Duration, Instant};

use gpui_kit::{App, Window};
use image::RgbaImage;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows::Win32::Foundation::{
    COLORREF, CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, HWND, LPARAM, LRESULT,
    POINT, RECT, WPARAM,
};
use windows::Win32::Graphics::Dwm::{
    DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS, DWMWA_TRANSITIONS_FORCEDISABLED,
    DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND, DwmGetWindowAttribute,
    DwmSetWindowAttribute,
};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CAPTUREBLT, CreateCompatibleDC, CreateDIBSection,
    CreateSolidBrush, DIB_RGB_COLORS, DeleteDC, DeleteObject, EnumDisplayMonitors, GdiFlush, GetDC,
    GetMonitorInfoW, HBITMAP, HDC, HGDIOBJ, HMONITOR, MONITOR_DEFAULTTONEAREST, MONITORINFO,
    MonitorFromPoint, ReleaseDC, SRCCOPY, SelectObject,
};
use windows::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
    SetInformationJobObject, TerminateJobObject,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::{
    CURSOR_SHOWING, CURSORINFO, CreateWindowExW, DI_NORMAL, DefWindowProcW, DestroyWindow,
    DrawIconEx, EnumWindows, GWL_EXSTYLE, GetClassNameW, GetCursorInfo, GetCursorPos,
    GetForegroundWindow, GetIconInfo, GetSystemMetrics, GetWindowLongPtrW, GetWindowRect, HICON,
    ICONINFO, IsIconic, IsWindow, IsWindowVisible, RegisterClassW, SM_CXVIRTUALSCREEN,
    SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN, SW_SHOWNOACTIVATE, SW_SHOWNORMAL,
    SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER, SetForegroundWindow, SetWindowDisplayAffinity,
    SetWindowPos, ShowWindow, WDA_EXCLUDEFROMCAPTURE, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_OVERLAPPED, WS_POPUP,
};
use windows::core::{BOOL, PCWSTR, w};

use crate::geom::RectI;

#[derive(Clone, Copy, Debug)]
pub struct Monitor {
    pub id: u64,
    pub rect: RectI,
    pub scale: f32,
}

/// Holds the named mutex that marks this process as the running copy.
pub struct Instance(#[allow(dead_code)] windows::Win32::Foundation::HANDLE);

/// `None` when another copy still owns the tray icon and the hotkeys after `patience`, the time an update gives the old copy to leave.
pub fn single_instance(patience: Duration) -> Option<Instance> {
    let deadline = Instant::now() + patience;
    loop {
        unsafe {
            let handle = CreateMutexW(None, true, w!("bawkshot-single-instance")).ok()?;
            if GetLastError() != ERROR_ALREADY_EXISTS {
                return Some(Instance(handle));
            }
            let _ = CloseHandle(handle);
        }
        if Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Windows applies the launcher's show state, such as a shortcut set to run minimized, to the first window shown. This spends it on an invisible throwaway window so no capture overlay opens hidden.
pub fn discard_launch_show_state() {
    unsafe {
        let window = CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_LAYERED,
            w!("STATIC"),
            None,
            WS_OVERLAPPED,
            -32000,
            -32000,
            1,
            1,
            None,
            None,
            None,
            None,
        );
        if let Ok(window) = window {
            let _ = ShowWindow(window, SW_SHOWNORMAL);
            let _ = DestroyWindow(window);
        }
    }
}
pub fn virtual_screen() -> RectI {
    unsafe {
        RectI::new(
            GetSystemMetrics(SM_XVIRTUALSCREEN),
            GetSystemMetrics(SM_YVIRTUALSCREEN),
            GetSystemMetrics(SM_CXVIRTUALSCREEN),
            GetSystemMetrics(SM_CYVIRTUALSCREEN),
        )
    }
}

pub fn cursor_position() -> (i32, i32) {
    let mut point = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut point);
    }
    (point.x, point.y)
}

pub fn monitors() -> Vec<Monitor> {
    unsafe extern "system" fn collect(
        monitor: HMONITOR,
        _: HDC,
        _: *mut RECT,
        data: LPARAM,
    ) -> BOOL {
        let handles = data.0 as *mut Vec<HMONITOR>;
        unsafe { (*handles).push(monitor) };
        BOOL(1)
    }

    let mut handles: Vec<HMONITOR> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(
            None,
            None,
            Some(collect),
            LPARAM(&mut handles as *mut _ as isize),
        );
    }
    handles.into_iter().filter_map(monitor_info).collect()
}

pub fn monitor_at(x: i32, y: i32) -> Option<Monitor> {
    monitor_info(unsafe { MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST) })
}

fn monitor_info(handle: HMONITOR) -> Option<Monitor> {
    let mut info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    let (mut dpi_x, mut dpi_y) = (96, 96);
    unsafe {
        if !GetMonitorInfoW(handle, &mut info).as_bool() {
            return None;
        }
        let _ = GetDpiForMonitor(handle, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y);
    }
    Some(Monitor {
        id: handle.0 as u64,
        rect: rect_from(info.rcMonitor),
        scale: dpi_x as f32 / 96.0,
    })
}

fn rect_from(rect: RECT) -> RectI {
    RectI::new(
        rect.left,
        rect.top,
        rect.right - rect.left,
        rect.bottom - rect.top,
    )
}

/// The visible frame of a window, without the invisible resize border Windows adds around it.
fn frame_bounds(hwnd: HWND) -> Option<RectI> {
    let mut rect = RECT::default();
    unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            &mut rect as *mut _ as *mut c_void,
            size_of::<RECT>() as u32,
        )
        .ok()?;
    }
    let rect = rect_from(rect);
    (!rect.is_empty()).then_some(rect)
}

/// A window an area pick can snap to. `window` is its handle, absent when the target is a whole monitor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SnapTarget {
    pub rect: RectI,
    pub window: Option<isize>,
}

pub fn foreground_handle() -> Option<isize> {
    let hwnd = unsafe { GetForegroundWindow() };
    (!hwnd.is_invalid()).then_some(hwnd.0 as isize)
}

pub fn foreground_window() -> Option<RectI> {
    let rect = frame_bounds(HWND(foreground_handle()? as *mut c_void))?;
    let rect = rect.intersect(&virtual_screen());
    (!rect.is_empty()).then_some(rect)
}

/// What a followed window is doing right now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowState {
    Gone,
    Minimized,
    At(RectI),
}

pub fn window_state(window: isize) -> WindowState {
    let hwnd = HWND(window as *mut c_void);
    unsafe {
        if !IsWindow(Some(hwnd)).as_bool() {
            return WindowState::Gone;
        }
        if IsIconic(hwnd).as_bool() || !IsWindowVisible(hwnd).as_bool() {
            return WindowState::Minimized;
        }
    }
    frame_bounds(hwnd).map_or(WindowState::Minimized, WindowState::At)
}

/// Every window an area pick can snap to, front to back.
pub fn snap_targets() -> Vec<SnapTarget> {
    unsafe extern "system" fn collect(hwnd: HWND, data: LPARAM) -> BOOL {
        let targets = data.0 as *mut Vec<SnapTarget>;
        if let Some(rect) = snap_rect(hwnd) {
            let window = Some(hwnd.0 as isize);
            unsafe { (*targets).push(SnapTarget { rect, window }) };
        }
        BOOL(1)
    }

    let mut targets: Vec<SnapTarget> = Vec::new();
    unsafe {
        let _ = EnumWindows(Some(collect), LPARAM(&mut targets as *mut _ as isize));
    }
    targets
}

fn snap_rect(hwnd: HWND) -> Option<RectI> {
    unsafe {
        if !IsWindowVisible(hwnd).as_bool() || IsIconic(hwnd).as_bool() {
            return None;
        }
        let mut cloaked = 0u32;
        let _ = DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED,
            &mut cloaked as *mut _ as *mut c_void,
            size_of::<u32>() as u32,
        );
        if cloaked != 0 {
            return None;
        }
        let mut class = [0u16; 64];
        let len = GetClassNameW(hwnd, &mut class) as usize;
        let class = String::from_utf16_lossy(&class[..len]);
        let desktop = matches!(class.as_str(), "Progman" | "WorkerW");
        let tool = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOOLWINDOW.0 != 0;
        if desktop || (tool && class != "Shell_TrayWnd") {
            return None;
        }
    }
    let rect = frame_bounds(hwnd)?.intersect(&virtual_screen());
    (rect.w >= 24 && rect.h >= 24).then_some(rect)
}

/// Copies screen areas of one fixed size, keeping its bitmap from frame to frame.
pub struct Grabber {
    screen: HDC,
    memory: HDC,
    bitmap: HBITMAP,
    previous: HGDIOBJ,
    bits: *const u8,
    width: i32,
    height: i32,
}

impl Grabber {
    pub fn new(width: i32, height: i32) -> Option<Self> {
        if width <= 0 || height <= 0 {
            return None;
        }
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                biHeight: -height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        unsafe {
            let screen = GetDC(None);
            let memory = CreateCompatibleDC(Some(screen));
            let mut bits: *mut c_void = std::ptr::null_mut();
            let bitmap = CreateDIBSection(Some(memory), &info, DIB_RGB_COLORS, &mut bits, None, 0);
            match bitmap {
                Ok(bitmap) if !bits.is_null() => Some(Self {
                    screen,
                    memory,
                    bitmap,
                    previous: SelectObject(memory, bitmap.into()),
                    bits: bits as *const u8,
                    width,
                    height,
                }),
                _ => {
                    let _ = DeleteDC(memory);
                    ReleaseDC(None, screen);
                    None
                }
            }
        }
    }

    /// The screen at `(x, y)` as top-down BGRA rows. The fourth byte of each pixel is padding, not alpha.
    pub fn frame(&mut self, x: i32, y: i32, with_cursor: bool) -> Option<&[u8]> {
        unsafe {
            BitBlt(
                self.memory,
                0,
                0,
                self.width,
                self.height,
                Some(self.screen),
                x,
                y,
                SRCCOPY | CAPTUREBLT,
            )
            .ok()?;
            if with_cursor {
                draw_cursor(self.memory, x, y);
            }
            let _ = GdiFlush();
            let len = self.width as usize * self.height as usize * 4;
            Some(std::slice::from_raw_parts(self.bits, len))
        }
    }
}

impl Drop for Grabber {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.memory, self.previous);
            let _ = DeleteObject(self.bitmap.into());
            let _ = DeleteDC(self.memory);
            ReleaseDC(None, self.screen);
        }
    }
}

/// Copies a screen area, given in virtual-screen pixels, into an image.
pub fn grab(rect: RectI, with_cursor: bool) -> Option<RgbaImage> {
    let mut grabber = Grabber::new(rect.w, rect.h)?;
    let mut pixels = grabber.frame(rect.x, rect.y, with_cursor)?.to_vec();
    for pixel in pixels.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
        pixel[3] = 255;
    }
    RgbaImage::from_raw(rect.w as u32, rect.h as u32, pixels)
}

unsafe fn draw_cursor(target: HDC, left: i32, top: i32) {
    let mut cursor = CURSORINFO {
        cbSize: size_of::<CURSORINFO>() as u32,
        ..Default::default()
    };
    unsafe {
        if GetCursorInfo(&mut cursor).is_err() || cursor.flags != CURSOR_SHOWING {
            return;
        }
        let icon = HICON(cursor.hCursor.0);
        let mut details = ICONINFO::default();
        if GetIconInfo(icon, &mut details).is_err() {
            return;
        }
        let _ = DrawIconEx(
            target,
            cursor.ptScreenPos.x - details.xHotspot as i32 - left,
            cursor.ptScreenPos.y - details.yHotspot as i32 - top,
            icon,
            0,
            0,
            0,
            None,
            DI_NORMAL,
        );
        let _ = DeleteObject(details.hbmMask.into());
        let _ = DeleteObject(details.hbmColor.into());
    }
}

fn hwnd_of(window: &Window) -> Option<isize> {
    match HasWindowHandle::window_handle(window).ok()?.as_raw() {
        RawWindowHandle::Win32(handle) => Some(handle.hwnd.get()),
        _ => None,
    }
}

/// Runs a Win32 call on the window once GPUI is idle; made mid-update, the messages it sends back are dropped.
fn later(window: &Window, cx: &App, call: impl FnOnce(HWND) + 'static) {
    if let Some(raw) = hwnd_of(window) {
        cx.spawn(async move |_| call(HWND(raw as *mut c_void)))
            .detach();
    }
}

/// Squares a capture overlay's corners, skips its open animation and gives it the keyboard.
pub fn prepare_overlay(window: &Window, cx: &App) {
    later(window, cx, |hwnd| {
        let square = DWMWCP_DONOTROUND;
        let no_transitions = BOOL(1);
        unsafe {
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_WINDOW_CORNER_PREFERENCE,
                &square as *const _ as *const c_void,
                size_of_val(&square) as u32,
            );
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_TRANSITIONS_FORCEDISABLED,
                &no_transitions as *const _ as *const c_void,
                size_of::<BOOL>() as u32,
            );
            let _ = SetForegroundWindow(hwnd);
        }
    });
}

/// Brings one of this app's windows to the front, which a tray click alone does not do.
pub fn bring_to_front(window: &Window, cx: &App) {
    later(window, cx, |hwnd| unsafe {
        let _ = SetForegroundWindow(hwnd);
    });
}

/// Keeps one of this app's windows out of screenshots and recordings, its own included.
pub fn hide_from_capture(window: &Window, cx: &App) {
    later(window, cx, |hwnd| unsafe {
        let _ = SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE);
    });
}

/// Moves one of this app's windows by a number of screen pixels, keeping its size.
pub fn nudge(window: &Window, dx: i32, dy: i32, cx: &App) {
    later(window, cx, move |hwnd| unsafe {
        let mut rect = RECT::default();
        if GetWindowRect(hwnd, &mut rect).is_ok() {
            let flags = SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE;
            let _ = SetWindowPos(hwnd, None, rect.left + dx, rect.top + dy, 0, 0, flags);
        }
    });
}

/// A job object that takes its processes down with it, so a recorder never outlives the app.
pub struct Job(isize);

impl Job {
    pub fn new() -> Option<Self> {
        let job = Self(unsafe { CreateJobObjectW(None, PCWSTR::null()) }.ok()?.0 as isize);
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        unsafe {
            SetInformationJobObject(
                job.handle(),
                JobObjectExtendedLimitInformation,
                &limits as *const _ as *const c_void,
                size_of_val(&limits) as u32,
            )
        }
        .ok()?;
        Some(job)
    }

    fn handle(&self) -> HANDLE {
        HANDLE(self.0 as *mut c_void)
    }

    pub fn adopt(&self, child: &Child) {
        unsafe {
            let _ = AssignProcessToJobObject(self.handle(), HANDLE(child.as_raw_handle()));
        }
    }

    pub fn kill(&self) {
        unsafe {
            let _ = TerminateJobObject(self.handle(), 1);
        }
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.handle());
        }
    }
}

const OUTLINE_CLASS: PCWSTR = w!("bawkshot-outline");
const OUTLINE_COLOR: COLORREF = COLORREF(0x006b_d5f6);

unsafe extern "system" fn outline_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
}

/// A border around a screen area made of four thin windows, so nothing sits on the area itself. Hidden from captures.
pub struct Outline {
    windows: Vec<isize>,
    thickness: i32,
}

impl Outline {
    fn sides(rect: RectI, t: i32) -> [RectI; 4] {
        [
            RectI::new(rect.x - t, rect.y - t, rect.w + t * 2, t),
            RectI::new(rect.x - t, rect.bottom(), rect.w + t * 2, t),
            RectI::new(rect.x - t, rect.y, t, rect.h),
            RectI::new(rect.right(), rect.y, t, rect.h),
        ]
    }

    pub fn around(rect: RectI, thickness: i32) -> Self {
        static REGISTER: Once = Once::new();
        let module = unsafe { GetModuleHandleW(None) }.unwrap_or_default();
        REGISTER.call_once(|| unsafe {
            RegisterClassW(&WNDCLASSW {
                lpfnWndProc: Some(outline_proc),
                hInstance: module.into(),
                hbrBackground: CreateSolidBrush(OUTLINE_COLOR),
                lpszClassName: OUTLINE_CLASS,
                ..Default::default()
            });
        });
        let thickness = thickness.max(1);
        let windows = Self::sides(rect, thickness)
            .into_iter()
            .filter_map(|side| unsafe {
                let window = CreateWindowExW(
                    WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                    OUTLINE_CLASS,
                    None,
                    WS_POPUP,
                    side.x,
                    side.y,
                    side.w,
                    side.h,
                    None,
                    None,
                    Some(module.into()),
                    None,
                )
                .ok()?;
                let _ = SetWindowDisplayAffinity(window, WDA_EXCLUDEFROMCAPTURE);
                let _ = ShowWindow(window, SW_SHOWNOACTIVATE);
                Some(window.0 as isize)
            });
        Self {
            windows: windows.collect(),
            thickness,
        }
    }

    pub fn move_to(&self, rect: RectI) {
        let flags = SWP_NOZORDER | SWP_NOACTIVATE;
        for (window, side) in self.windows.iter().zip(Self::sides(rect, self.thickness)) {
            let hwnd = HWND(*window as *mut c_void);
            unsafe {
                let _ = SetWindowPos(hwnd, None, side.x, side.y, side.w, side.h, flags);
            }
        }
    }
}

impl Drop for Outline {
    fn drop(&mut self) {
        for window in self.windows.drain(..) {
            unsafe {
                let _ = DestroyWindow(HWND(window as *mut c_void));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn placed(window: isize) -> RectI {
        let mut rect = RECT::default();
        unsafe { GetWindowRect(HWND(window as *mut c_void), &mut rect).unwrap() };
        rect_from(rect)
    }

    #[test]
    fn an_outline_frames_the_area_follows_it_and_goes_away_when_dropped() {
        let outline = Outline::around(RectI::new(-30000, -30000, 200, 100), 3);
        let windows = outline.windows.clone();
        assert_eq!(windows.len(), 4);
        assert_eq!(placed(windows[0]), RectI::new(-30003, -30003, 206, 3));
        outline.move_to(RectI::new(-29000, -29500, 200, 100));
        assert_eq!(placed(windows[0]), RectI::new(-29003, -29503, 206, 3));
        assert_eq!(placed(windows[3]), RectI::new(-28800, -29500, 3, 100));
        drop(outline);
        for window in windows {
            assert!(!unsafe { IsWindow(Some(HWND(window as *mut c_void))) }.as_bool());
            assert_eq!(window_state(window), WindowState::Gone);
        }
    }

    #[test]
    fn a_grabber_returns_whole_frames_of_its_size() {
        let mut grabber = Grabber::new(32, 20).expect("the screen can be read");
        let frame = grabber.frame(0, 0, false).expect("a frame");
        assert_eq!(frame.len(), 32 * 20 * 4);
        assert!(Grabber::new(0, 20).is_none());
        let image = grab(RectI::new(0, 0, 16, 8), true).expect("a capture");
        assert_eq!(image.dimensions(), (16, 8));
        assert!(image.pixels().all(|pixel| pixel[3] == 255));
    }
    #[test]
    fn a_job_kills_what_it_adopted() {
        let mut child = std::process::Command::new("cmd")
            .args(["/c", "pause"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let job = Job::new().expect("job objects are available");
        job.adopt(&child);
        assert!(child.try_wait().unwrap().is_none());
        drop(job);
        assert!(child.wait().is_ok());
    }
}
