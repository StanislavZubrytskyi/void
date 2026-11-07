use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::{mem::size_of, os::windows::ffi::OsStrExt, ptr};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM},
    Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow},
    System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_DWORD, REG_SZ,
        RRF_RT_REG_DWORD, RegCloseKey, RegDeleteKeyValueW, RegDeleteValueW, RegGetValueW,
        RegOpenKeyExW, RegQueryValueExW, RegSetKeyValueW, RegSetValueExW,
    },
    UI::{
        Shell::{
            DefSubclassProc, NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_INFO, NIM_ADD,
            NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW, RemoveWindowSubclass, SetWindowSubclass,
            Shell_NotifyIconW,
        },
        WindowsAndMessaging::*,
    },
};
use windows_sys::core::PCWSTR;

const TRAY_MESSAGE: u32 = WM_APP + 42;
const TRAY_ID: usize = 1;

pub struct Tray {
    hwnd: HWND,
    icon: HICON,
}

impl Tray {
    pub fn install(window: &impl HasWindowHandle) -> Option<Self> {
        let RawWindowHandle::Win32(window) = window.window_handle().ok()?.as_raw() else {
            return None;
        };
        let hwnd = window.hwnd.get() as HWND;
        // The app and subclass share winit's UI thread. The tray owns the icon
        // until Drop, and the subclass receives only its handle as reference data.
        unsafe {
            let icon = cat_icon();
            if icon.is_null() {
                return None;
            }
            let tray = Self { hwnd, icon };
            if SetWindowSubclass(hwnd, Some(window_proc), TRAY_ID, icon as usize) == 0 {
                return None;
            }
            let visible = IsWindowVisible(hwnd) != 0;
            if visible {
                ShowWindow(hwnd, SW_HIDE);
            }
            let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
            SetWindowLongPtrW(
                hwnd,
                GWL_EXSTYLE,
                (style & !(WS_EX_APPWINDOW as isize)) | WS_EX_TOOLWINDOW as isize,
            );
            SetWindowPos(
                hwnd,
                ptr::null_mut(),
                0,
                0,
                0,
                0,
                SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOZORDER,
            );
            Shell_NotifyIconW(NIM_ADD, &icon_data(hwnd, icon));
            if visible {
                ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            }
            Some(tray)
        }
    }

    /// A standard Windows notification from the tray icon, unless the user turned them off.
    pub fn notify(&self, title: &str, body: &str) {
        if !notifications_enabled() {
            return;
        }
        let mut data = icon_data(self.hwnd, self.icon);
        data.uFlags = NIF_INFO;
        data.dwInfoFlags = NIIF_INFO;
        let last = data.szInfoTitle.len() - 1;
        for (slot, unit) in data.szInfoTitle[..last]
            .iter_mut()
            .zip(title.encode_utf16())
        {
            *slot = unit;
        }
        let last = data.szInfo.len() - 1;
        for (slot, unit) in data.szInfo[..last].iter_mut().zip(body.encode_utf16()) {
            *slot = unit;
        }
        unsafe {
            Shell_NotifyIconW(NIM_MODIFY, &data);
        }
    }

    pub fn dock(&self) {
        unsafe {
            let monitor = MonitorFromWindow(self.hwnd, MONITOR_DEFAULTTONEAREST);
            let mut info = MONITORINFO {
                cbSize: size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            let mut window = std::mem::zeroed();
            if GetMonitorInfoW(monitor, &mut info) != 0
                && GetWindowRect(self.hwnd, &mut window) != 0
            {
                SetWindowPos(
                    self.hwnd,
                    ptr::null_mut(),
                    (info.rcWork.right - (window.right - window.left) - 12).max(info.rcWork.left),
                    (info.rcWork.bottom - (window.bottom - window.top) - 12).max(info.rcWork.top),
                    0,
                    0,
                    SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOZORDER,
                );
            }
        }
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        unsafe {
            RemoveWindowSubclass(self.hwnd, Some(window_proc), TRAY_ID);
            Shell_NotifyIconW(NIM_DELETE, &icon_data(self.hwnd, self.icon));
            DestroyIcon(self.icon);
        }
    }
}

fn icon_data(hwnd: HWND, icon: HICON) -> NOTIFYICONDATAW {
    let mut data = NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: TRAY_ID as u32,
        uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP,
        uCallbackMessage: TRAY_MESSAGE,
        hIcon: icon,
        ..Default::default()
    };
    for (slot, unit) in data.szTip.iter_mut().zip("Void".encode_utf16()) {
        *slot = unit;
    }
    data
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _: usize,
    icon: usize,
) -> LRESULT {
    unsafe {
        // Winit can recompute the extended styles on resize or decoration
        // changes. Keep the tool-window style through those updates.
        if message == WM_STYLECHANGING && wparam as i32 == GWL_EXSTYLE && lparam != 0 {
            let style = &mut *(lparam as *mut STYLESTRUCT);
            style.styleNew = (style.styleNew & !WS_EX_APPWINDOW) | WS_EX_TOOLWINDOW;
        }
        if message == TRAY_MESSAGE {
            match lparam as u32 {
                WM_LBUTTONUP => {
                    let command = if IsWindowVisible(hwnd) != 0 {
                        SW_HIDE
                    } else {
                        SW_SHOWNOACTIVATE
                    };
                    ShowWindow(hwnd, command);
                }
                WM_RBUTTONUP => show_menu(hwnd),
                _ => {}
            }
            return 0;
        }
        if message == WM_NCDESTROY {
            Shell_NotifyIconW(NIM_DELETE, &icon_data(hwnd, icon as HICON));
        } else if message >= 0xC000
            && message == RegisterWindowMessageW(windows_sys::core::w!("TaskbarCreated"))
        {
            Shell_NotifyIconW(NIM_ADD, &icon_data(hwnd, icon as HICON));
        }
        DefSubclassProc(hwnd, message, wparam, lparam)
    }
}

unsafe fn show_menu(hwnd: HWND) {
    unsafe {
        let menu = CreatePopupMenu();
        if menu.is_null() {
            return;
        }
        let toggle = if IsWindowVisible(hwnd) != 0 {
            windows_sys::core::w!("Сховати кота")
        } else {
            windows_sys::core::w!("Показати кота")
        };
        let autostart = autostart_enabled();
        AppendMenuW(menu, MF_STRING, 1, toggle);
        AppendMenuW(
            menu,
            if autostart {
                MF_STRING | MF_CHECKED
            } else {
                MF_STRING
            },
            3,
            windows_sys::core::w!("Запускати з Windows"),
        );
        let notifications = notifications_enabled();
        AppendMenuW(
            menu,
            if notifications {
                MF_STRING | MF_CHECKED
            } else {
                MF_STRING
            },
            4,
            windows_sys::core::w!("Сповіщення"),
        );
        AppendMenuW(menu, MF_STRING, 2, windows_sys::core::w!("Вийти з Void"));
        let mut point = POINT::default();
        GetCursorPos(&mut point);
        SetForegroundWindow(hwnd);
        let action = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON,
            point.x,
            point.y,
            0,
            hwnd,
            ptr::null(),
        );
        DestroyMenu(menu);
        PostMessageW(hwnd, WM_NULL, 0, 0);
        match action {
            1 => {
                ShowWindow(
                    hwnd,
                    if IsWindowVisible(hwnd) != 0 {
                        SW_HIDE
                    } else {
                        SW_SHOWNOACTIVATE
                    },
                );
            }
            2 => {
                PostMessageW(hwnd, WM_CLOSE, 0, 0);
            }
            3 => set_autostart(!autostart),
            4 => set_notifications(!notifications),
            _ => {}
        }
    }
}

const RUN_KEY: PCWSTR = windows_sys::core::w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
const RUN_VALUE: PCWSTR = windows_sys::core::w!("Void");

unsafe fn open_run_key(access: u32) -> Option<HKEY> {
    let mut key = ptr::null_mut();
    (unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, RUN_KEY, 0, access, &mut key) } == 0).then_some(key)
}

fn autostart_enabled() -> bool {
    unsafe {
        let Some(key) = open_run_key(KEY_QUERY_VALUE) else {
            return false;
        };
        let found = RegQueryValueExW(
            key,
            RUN_VALUE,
            ptr::null(),
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
        ) == 0;
        RegCloseKey(key);
        found
    }
}

// Registry errors are ignored: the menu shows the real state on the next open.
fn set_autostart(on: bool) {
    unsafe {
        let Some(key) = open_run_key(KEY_SET_VALUE) else {
            return;
        };
        if on {
            if let Ok(exe) = std::env::current_exe() {
                let command: Vec<u16> = std::iter::once(b'"' as u16)
                    .chain(exe.as_os_str().encode_wide())
                    .chain([b'"' as u16, 0])
                    .collect();
                RegSetValueExW(
                    key,
                    RUN_VALUE,
                    0,
                    REG_SZ,
                    command.as_ptr().cast(),
                    (command.len() * 2) as u32,
                );
            }
        } else {
            RegDeleteValueW(key, RUN_VALUE);
        }
        RegCloseKey(key);
    }
}

unsafe fn cat_icon() -> HICON {
    // A small BGRA cat silhouette with a separate monochrome transparency mask.
    const SIDE: usize = 32;
    let mut mask = [255_u8; SIDE * 4];
    let mut rgba = [0_u8; SIDE * SIDE * 4];
    for y in 0..SIDE {
        for x in 0..SIDE {
            let dx = x as f32 - 15.5;
            let dy = y as f32 - 17.0;
            let ear_x = (dx.abs() - 8.5).abs();
            let head = dx * dx / 160.0 + dy * dy / 105.0 <= 1.0;
            let ear = (4..=14).contains(&y) && ear_x <= (y - 3) as f32 * 0.45;
            if head || ear {
                let eye = (dx.abs() - 4.5).abs() < 1.2 && (16..=18).contains(&y);
                let color = if eye {
                    [59, 54, 57, 255]
                } else {
                    [222, 220, 225, 255]
                };
                let row = SIDE - 1 - y;
                let i = (row * SIDE + x) * 4;
                rgba[i..i + 4].copy_from_slice(&color);
                mask[row * 4 + x / 8] &= !(0x80 >> (x % 8));
            }
        }
    }
    unsafe {
        CreateIcon(
            ptr::null_mut(),
            SIDE as i32,
            SIDE as i32,
            1,
            32,
            mask.as_ptr(),
            rgba.as_ptr(),
        )
    }
}

const SETTINGS_KEY: PCWSTR = windows_sys::core::w!("Software\\Void");
const NOTIFICATIONS: PCWSTR = windows_sys::core::w!("Notifications");

/// On by default; turning them off stores Notifications = 0 under HKCU\Software\Void.
fn notifications_enabled() -> bool {
    let mut value = 1_u32;
    let mut size = size_of::<u32>() as u32;
    let found = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            SETTINGS_KEY,
            NOTIFICATIONS,
            RRF_RT_REG_DWORD,
            ptr::null_mut(),
            (&mut value as *mut u32).cast(),
            &mut size,
        )
    } == 0;
    !found || value != 0
}

fn set_notifications(on: bool) {
    unsafe {
        if on {
            RegDeleteKeyValueW(HKEY_CURRENT_USER, SETTINGS_KEY, NOTIFICATIONS);
        } else {
            let off = 0_u32;
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                SETTINGS_KEY,
                NOTIFICATIONS,
                REG_DWORD,
                (&off as *const u32).cast(),
                size_of::<u32>() as u32,
            );
        }
    }
}
