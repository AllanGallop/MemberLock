use std::sync::Arc;

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::{
    ShellExecuteW, Shell_NotifyIconW, NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_INFO,
    NIM_ADD, NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateIconFromResourceEx, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyWindow,
    DispatchMessageW, GetCursorPos, GetMessageW, GetWindowLongPtrW, InsertMenuW, PostMessageW,
    PostQuitMessage, RegisterClassW, SetForegroundWindow, SetTimer, SetWindowLongPtrW,
    TrackPopupMenu, TranslateMessage, CREATESTRUCTW, GWLP_USERDATA, HICON, HMENU, MF_GRAYED,
    MF_STRING, MSG, SW_SHOW, TPM_BOTTOMALIGN, TPM_RIGHTALIGN, WM_COMMAND, WM_CONTEXTMENU,
    WM_CREATE, WM_DESTROY, WM_LBUTTONUP, WM_NULL, WM_RBUTTONUP, WM_TIMER, WNDCLASSW,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_POPUP,
};

use crate::state::{App, Status};

const WM_TRAY: u32 = 0x8000 + 1;
const CMD_LOCK: usize = 1;
const CMD_CONFIG: usize = 2;
const CMD_LOG: usize = 3;
const CMD_QUIT: usize = 4;

struct Ui {
    app: Arc<App>,
    icon_locked: HICON,
    icon_open: HICON,
    showing_open: bool,
    config_path: Vec<u16>,
    log_path: Vec<u16>,
}

pub fn run(app: Arc<App>, config_path: &std::path::Path, log_path: &std::path::Path) -> windows::core::Result<()> {
    let ui = Box::new(Ui {
        app,
        icon_locked: make_icon(true)?,
        icon_open: make_icon(false)?,
        showing_open: false,
        config_path: wide(config_path.display().to_string()),
        log_path: wide(log_path.display().to_string()),
    });
    let ui = Box::into_raw(ui);

    unsafe {
        let instance = GetModuleHandleW(None)?;
        let class_name = w!("MemberLockTray");
        let class = WNDCLASSW {
            lpfnWndProc: Some(wnd_proc),
            hInstance: instance.into(),
            lpszClassName: class_name,
            ..Default::default()
        };
        RegisterClassW(&class);

        let hwnd = CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            class_name,
            w!("MemberLock"),
            WS_POPUP,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(instance.into()),
            Some(ui.cast()),
        )?;

        let mut nid = notify_data(hwnd, (*ui).icon_locked);
        nid.szTip = tip_array("MemberLock — laser locked");
        Shell_NotifyIconW(NIM_ADD, &nid).ok()?;
        SetTimer(Some(hwnd), 1, 500, None);

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).into() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
        let _ = DestroyWindow(hwnd);
        drop(Box::from_raw(ui));
    }
    Ok(())
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_CREATE {
        let create = &*(lparam.0 as *const CREATESTRUCTW);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
        return LRESULT(0);
    }
    if msg == WM_DESTROY {
        PostQuitMessage(0);
        return LRESULT(0);
    }
    let ui = userdata(hwnd);
    if ui.is_null() {
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }
    let ui = &mut *ui;

    match msg {
        WM_TRAY => {
            let event = lparam.0 as u32;
            if event == WM_RBUTTONUP || event == WM_LBUTTONUP || event == WM_CONTEXTMENU {
                let _ = show_menu(hwnd, ui);
            }
            LRESULT(0)
        }
        WM_TIMER => {
            refresh(hwnd, ui);
            LRESULT(0)
        }
        WM_COMMAND => {
            match wparam.0 & 0xFFFF {
                CMD_LOCK => {
                    ui.app.lock("locked from the menu");
                    ui.app.balloon("Laser locked", "The tube is off until the next valid badge.");
                }
                CMD_CONFIG => open(&ui.config_path),
                CMD_LOG => open(&ui.log_path),
                CMD_QUIT => {
                    ui.app.lock("MemberLock is closing");
                    let nid = notify_data(hwnd, ui.icon_locked);
                    let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
                    PostQuitMessage(0);
                }
                _ => {}
            }
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

fn refresh(hwnd: HWND, ui: &mut Ui) {
    let status = ui.app.status_snapshot();
    let open = status.unlocked;
    if open != ui.showing_open {
        ui.showing_open = open;
    }
    let icon = if open { ui.icon_open } else { ui.icon_locked };
    let mut nid = notify_data(hwnd, icon);
    nid.szTip = tip_array(&clip_tip(&status.summary(&ui.app.cfg.laser.name)));
    if let Some(balloon) = ui.app.take_balloon() {
        nid.uFlags |= NIF_INFO;
        nid.szInfoTitle = title_array(&clip(&balloon.title, 60));
        nid.szInfo = info_array(&clip(&balloon.body, 240));
        nid.dwInfoFlags = NIIF_INFO;
    }
    unsafe {
        let _ = Shell_NotifyIconW(NIM_MODIFY, &nid);
    }
}

unsafe fn show_menu(hwnd: HWND, ui: &Ui) -> windows::core::Result<()> {
    let status = ui.app.status_snapshot();
    let menu = CreatePopupMenu()?;
    insert(&menu, 0, &status_label(&status), 100, true)?;
    insert(&menu, 1, "Lock now", CMD_LOCK, !status.unlocked)?;
    insert(&menu, 2, "Open config", CMD_CONFIG, false)?;
    insert(&menu, 3, "Open log", CMD_LOG, false)?;
    insert(&menu, 4, "Quit", CMD_QUIT, false)?;

    let mut point = windows::Win32::Foundation::POINT::default();
    let _ = GetCursorPos(&mut point);
    let _ = SetForegroundWindow(hwnd);
    let _ = TrackPopupMenu(
        menu,
        TPM_RIGHTALIGN | TPM_BOTTOMALIGN,
        point.x,
        point.y,
        Some(0),
        hwnd,
        None,
    );
    let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
    Ok(())
}

unsafe fn insert(menu: &HMENU, pos: u32, text: &str, id: usize, gray: bool) -> windows::core::Result<()> {
    let wide = wide(text);
    let flags = if gray { MF_STRING | MF_GRAYED } else { MF_STRING };
    InsertMenuW(*menu, pos, flags, id, PCWSTR(wide.as_ptr()))?;
    Ok(())
}

fn status_label(status: &Status) -> String {
    if status.unlocked {
        format!("Enabled: {}", status.holder_name)
    } else {
        "Locked — tube off".into()
    }
}

fn open(path: &[u16]) {
    unsafe {
        // ShellExecuteW returns a value <= 32 on failure. HINSTANCE is a pointer-sized code.
        let _ = ShellExecuteW(
            None,
            w!("open"),
            PCWSTR(path.as_ptr()),
            None,
            None,
            SW_SHOW,
        );
    }
}

fn userdata(hwnd: HWND) -> *mut Ui {
    unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Ui }
}

fn notify_data(hwnd: HWND, icon: HICON) -> NOTIFYICONDATAW {
    let nid = NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: 1,
        uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP,
        uCallbackMessage: WM_TRAY,
        hIcon: icon,
        ..Default::default()
    };
    nid
}

fn make_icon(locked: bool) -> windows::core::Result<HICON> {
    let blob = icon_dib(locked);
    unsafe {
        CreateIconFromResourceEx(
            &blob,
            true,
            0x0003_0000,
            32,
            32,
            windows::Win32::UI::WindowsAndMessaging::LR_DEFAULTCOLOR,
        )
    }
}

/// 32×32 32-bit DIB icon: circle with a small padlock, red when locked, green when open.
fn icon_dib(locked: bool) -> Vec<u8> {
    const N: usize = 32;
    let mut xor = vec![0u8; N * N * 4];
    let (r, g, b) = if locked { (176u8, 48, 48) } else { (36u8, 140, 78) };
    for y in 0..N {
        for x in 0..N {
            let dx = x as i32 - 16;
            let dy = y as i32 - 16;
            if dx * dx + dy * dy > 15 * 15 {
                continue;
            }
            let mut pr = r;
            let mut pg = g;
            let mut pb = b;
            if padlock_pixel(x, y) {
                pr = 245;
                pg = 245;
                pb = 245;
            }
            // Bottom-up.
            let row = N - 1 - y;
            let i = (row * N + x) * 4;
            xor[i] = pb;
            xor[i + 1] = pg;
            xor[i + 2] = pr;
            xor[i + 3] = 255;
        }
    }
    // AND mask, 32 bits per row, all zero (alpha carries transparency).
    let and_mask = vec![0u8; N * 4];

    let mut dib = Vec::with_capacity(40 + xor.len() + and_mask.len());
    dib.extend_from_slice(&40u32.to_le_bytes());
    dib.extend_from_slice(&32i32.to_le_bytes());
    dib.extend_from_slice(&64i32.to_le_bytes()); // height includes the mask
    dib.extend_from_slice(&1u16.to_le_bytes());
    dib.extend_from_slice(&32u16.to_le_bytes());
    dib.extend_from_slice(&0u32.to_le_bytes());
    dib.extend_from_slice(&(xor.len() as u32).to_le_bytes());
    dib.extend_from_slice(&[0u8; 16]);
    dib.extend(xor);
    dib.extend(and_mask);
    dib
}

fn padlock_pixel(x: usize, y: usize) -> bool {
    let body = (12..21).contains(&x) && (15..24).contains(&y);
    let shackle = ((11..13).contains(&x) || (19..21).contains(&x)) && (10..16).contains(&y)
        || (13..19).contains(&x) && (9..11).contains(&y);
    body || shackle
}

fn clip_tip(text: &str) -> String {
    clip(text, 120)
}

fn clip(text: &str, max: usize) -> String {
    let mut out = String::new();
    for ch in text.chars() {
        if out.len() + ch.len_utf8() > max {
            break;
        }
        out.push(ch);
    }
    out
}

fn title_array(text: &str) -> [u16; 64] {
    let mut out = [0u16; 64];
    for (i, unit) in text.encode_utf16().take(63).enumerate() {
        out[i] = unit;
    }
    out
}

fn tip_array(text: &str) -> [u16; 128] {
    let mut out = [0u16; 128];
    for (i, unit) in text.encode_utf16().take(127).enumerate() {
        out[i] = unit;
    }
    out
}

fn info_array(text: &str) -> [u16; 256] {
    let mut out = [0u16; 256];
    for (i, unit) in text.encode_utf16().take(255).enumerate() {
        out[i] = unit;
    }
    out
}

fn wide(text: impl AsRef<str>) -> Vec<u16> {
    text.as_ref().encode_utf16().chain(std::iter::once(0)).collect()
}
