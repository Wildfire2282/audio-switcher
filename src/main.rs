// Hides the console window on Windows. MUST be the very first item in the
// crate root (before any other item), per the Rust reference.
#![windows_subsystem = "windows"]

use audio_switcher::{audio, startup, theme, tray};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

use tao::event::{Event, StartCause};
use tao::event_loop::EventLoopProxy;
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tray_icon::menu::MenuEvent;
use tray_icon::TrayIconEvent;
use windows::core::w;
use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, SetWindowsHookExW, UnhookWindowsHookEx, HHOOK, MSLLHOOKSTRUCT, WH_MOUSE_LL,
    WM_MOUSEWHEEL,
};

/// One variant of every event we forward into tao.
#[derive(Debug)]
enum UserEvent {
    Menu(MenuEvent),
    TrayIcon(TrayIconEvent),
    Wheel(i16),
}

static WHEEL_PROXY: OnceLock<EventLoopProxy<UserEvent>> = OnceLock::new();
static HOOK_HANDLE: OnceLock<isize> = OnceLock::new();
static HOVERING: AtomicBool = AtomicBool::new(false);
const WHEEL_DELTA: i16 = 120;

/// Low-level mouse hook. When the wheel turns while the cursor is over our
/// tray icon, forward the wheel delta into the tao event loop.
unsafe extern "system" fn mouse_hook_proc(
    code: i32,
    wparam: windows::Win32::Foundation::WPARAM,
    lparam: windows::Win32::Foundation::LPARAM,
) -> windows::Win32::Foundation::LRESULT {
    if code >= 0 && wparam.0 as u32 == WM_MOUSEWHEEL && HOVERING.load(Ordering::Relaxed) {
        let info = &*(lparam.0 as *const MSLLHOOKSTRUCT);
        let delta = ((info.mouseData >> 16) & 0xffff) as u16 as i16;
        if delta != 0 {
            if let Some(proxy) = WHEEL_PROXY.get() {
                let _ = proxy.send_event(UserEvent::Wheel(delta));
            }
        }
    }
    // First argument is ignored on modern Windows but must be provided.
    CallNextHookEx(None, code, wparam, lparam)
}

/// Install the low-level mouse hook and stash a proxy for forwarding wheel
/// events into the event loop.
fn install_wheel_hook(proxy: EventLoopProxy<UserEvent>) {
    let _ = WHEEL_PROXY.set(proxy);
    unsafe {
        let hook = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook_proc), None, 0);
        if let Ok(h) = hook {
            let _ = HOOK_HANDLE.set(h.0 as isize);
        }
    }
}

fn main() {
    // 1. COM first — before tao, before anything else.
    if unsafe { CoInitializeEx(None, COINIT_MULTITHREADED).ok() }.is_err() {
        eprintln!("CoInitializeEx failed");
        std::process::exit(1);
    }

    // 2. Single-instance guard.
    let already_running = match unsafe { CreateMutexW(None, true, w!("Global\\AudioSwitcherTray")) }
    {
        Ok(_) => (unsafe { GetLastError() }) == ERROR_ALREADY_EXISTS,
        Err(e) => {
            eprintln!("CreateMutexW failed: {e}");
            unsafe { CoUninitialize() };
            std::process::exit(1);
        }
    };
    if already_running {
        unsafe { CoUninitialize() };
        return;
    }

    // 3. Detect theme once at startup; no runtime polling.
    let initial_theme = theme::init();

    let event_loop = EventLoopBuilder::with_user_event().build();

    let menu_proxy = event_loop.create_proxy();
    let tray_proxy = event_loop.create_proxy();
    let wheel_proxy = event_loop.create_proxy();

    MenuEvent::set_event_handler(Some(move |e| {
        let _ = menu_proxy.send_event(UserEvent::Menu(e));
    }));
    TrayIconEvent::set_event_handler(Some(move |e| {
        let _ = tray_proxy.send_event(UserEvent::TrayIcon(e));
    }));
    install_wheel_hook(wheel_proxy);

    let menu = tray::build_menu();
    let icon = tray::make_icon(initial_theme);
    let tray = match tray::build_tray(menu, icon) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("build_tray failed: {e}");
            unsafe { CoUninitialize() };
            std::process::exit(1);
        }
    };

    event_loop.run(move |event, _target, control_flow| {
        *control_flow = ControlFlow::Wait;
        match event {
            Event::NewEvents(StartCause::Init) => {}
            Event::UserEvent(UserEvent::Menu(menu_evt)) => {
                let id = menu_evt.id.0.as_str();
                match tray::classify_menu_id(id) {
                    tray::MenuAction::Switch(device_id) => {
                        let _ = audio::set_default(device_id);
                        if let Some(m) = safe_rebuild() {
                            tray.set_menu(Some(Box::new(m)));
                        }
                    }
                    tray::MenuAction::Refresh => {
                        if let Some(m) = safe_rebuild() {
                            tray.set_menu(Some(Box::new(m)));
                        }
                    }
                    tray::MenuAction::ToggleAutostart => {
                        let current = startup::is_enabled();
                        startup::set_enabled(!current);
                        if let Some(m) = safe_rebuild() {
                            tray.set_menu(Some(Box::new(m)));
                        }
                    }
                    tray::MenuAction::Exit => {
                        *control_flow = ControlFlow::Exit;
                    }
                }
            }
            Event::UserEvent(UserEvent::TrayIcon(icon_evt)) => match icon_evt {
                TrayIconEvent::Enter { .. } => {
                    HOVERING.store(true, Ordering::Relaxed);
                }
                TrayIconEvent::Leave { .. } => {
                    HOVERING.store(false, Ordering::Relaxed);
                }
                _ => {}
            },
            Event::UserEvent(UserEvent::Wheel(delta)) => {
                let notches = (delta as i32) / (WHEEL_DELTA as i32);
                if notches != 0 {
                    let _ = audio::adjust_volume(notches);
                    // Feedback: update tooltip with current volume
                    if let Some(scalar) = audio::get_volume() {
                        let pct = (scalar * 100.0).round() as u32;
                        let tip = format!("音频输出设备切换 - 音量: {}%", pct);
                        let _ = tray.set_tooltip(Some(tip.as_str()));
                    }
                }
            }
            Event::LoopDestroyed => unsafe {
                if let Some(&handle) = HOOK_HANDLE.get() {
                    let _ = UnhookWindowsHookEx(HHOOK(handle as *mut _));
                }
                CoUninitialize();
            },
            _ => {}
        }
    });
}

/// Build a fresh menu, treating any unexpected panic as "no refresh" so a
/// transient COM hiccup never tears down the tray.
fn safe_rebuild() -> Option<tray_icon::menu::Menu> {
    std::panic::catch_unwind(|| tray::build_menu()).ok()
}
