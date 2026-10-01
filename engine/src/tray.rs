//! Ícone na área de notificação (bandeja) com menu: Abrir, Mudo e Sair.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};

use tokio::sync::mpsc::UnboundedSender;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{PCWSTR, w};

#[derive(Debug, Clone, Copy)]
pub enum TrayEvent {
    Open,
    ToggleMute,
    Quit,
}

const WM_TRAY: u32 = WM_APP + 1;
const ID_OPEN: usize = 1;
const ID_MUTE: usize = 2;
const ID_QUIT: usize = 3;

static SENDER: OnceLock<UnboundedSender<TrayEvent>> = OnceLock::new();
/// Estado de mudo para marcar o item do menu (atualizado pelo main).
pub static MUTED: AtomicBool = AtomicBool::new(false);
static HWND_RAW: AtomicIsize = AtomicIsize::new(0);

pub fn spawn(tx: UnboundedSender<TrayEvent>) {
    let _ = SENDER.set(tx);
    let _ = std::thread::Builder::new().name("screamo-tray".into()).spawn(|| unsafe { run() });
}

/// Remove o ícone (senão ele fica "fantasma" na bandeja até passar o mouse).
pub fn shutdown() {
    let raw = HWND_RAW.load(Ordering::SeqCst);
    if raw != 0 {
        unsafe {
            let _ = SendMessageW(HWND(raw as _), WM_CLOSE, None, None);
        }
    }
}

fn emit(ev: TrayEvent) {
    if let Some(tx) = SENDER.get() {
        let _ = tx.send(ev);
    }
}

fn icon_data(hwnd: HWND) -> NOTIFYICONDATAW {
    let mut nid = NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: 1,
        ..Default::default()
    };
    let tip: Vec<u16> = "ScreamO".encode_utf16().collect();
    nid.szTip[..tip.len()].copy_from_slice(&tip);
    nid
}

unsafe fn run() {
    unsafe {
        let Ok(module) = GetModuleHandleW(None) else { return };
        let class = w!("ScreamOTray");
        let wc = WNDCLASSW { lpfnWndProc: Some(wndproc), hInstance: module.into(), lpszClassName: class, ..Default::default() };
        RegisterClassW(&wc);
        let Ok(hwnd) = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            class,
            w!("ScreamO"),
            WINDOW_STYLE::default(),
            0,
            0,
            0,
            0,
            Some(HWND_MESSAGE),
            None,
            Some(module.into()),
            None,
        ) else {
            return;
        };
        HWND_RAW.store(hwnd.0 as isize, Ordering::SeqCst);

        // Ícone embutido pelo build.rs (recurso 1); cai no ícone padrão se faltar.
        let icon = LoadIconW(Some(module.into()), PCWSTR(1 as _)).or_else(|_| LoadIconW(None, IDI_APPLICATION)).unwrap_or_default();
        let mut nid = icon_data(hwnd);
        nid.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
        nid.uCallbackMessage = WM_TRAY;
        nid.hIcon = icon;
        let _ = Shell_NotifyIconW(NIM_ADD, &nid);

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

unsafe fn show_menu(hwnd: HWND) {
    unsafe {
        let Ok(menu) = CreatePopupMenu() else { return };
        let _ = AppendMenuW(menu, MF_STRING, ID_OPEN, w!("Abrir ScreamO"));
        let mute_flags = if MUTED.load(Ordering::Relaxed) { MF_STRING | MF_CHECKED } else { MF_STRING };
        let _ = AppendMenuW(menu, mute_flags, ID_MUTE, w!("Mudo"));
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
        let _ = AppendMenuW(menu, MF_STRING, ID_QUIT, w!("Sair"));
        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        // Necessário para o menu fechar ao clicar fora.
        let _ = SetForegroundWindow(hwnd);
        let _ = TrackPopupMenu(menu, TPM_RIGHTBUTTON | TPM_BOTTOMALIGN, pt.x, pt.y, None, hwnd, None);
        let _ = DestroyMenu(menu);
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_TRAY => {
                match lparam.0 as u32 {
                    WM_LBUTTONUP => emit(TrayEvent::Open),
                    WM_RBUTTONUP | WM_CONTEXTMENU => show_menu(hwnd),
                    _ => {}
                }
                LRESULT(0)
            }
            WM_COMMAND => {
                match wparam.0 & 0xFFFF {
                    ID_OPEN => emit(TrayEvent::Open),
                    ID_MUTE => emit(TrayEvent::ToggleMute),
                    ID_QUIT => emit(TrayEvent::Quit),
                    _ => {}
                }
                LRESULT(0)
            }
            WM_CLOSE => {
                let _ = Shell_NotifyIconW(NIM_DELETE, &icon_data(hwnd));
                let _ = DestroyWindow(hwnd);
                LRESULT(0)
            }
            WM_DESTROY => {
                PostQuitMessage(0);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}
