// Hides the console window on Windows. MUST be the very first item in the
// crate root (before any other item), per the Rust reference.
#![windows_subsystem = "windows"]

use audio_switcher::{audio, startup, theme, tray};

use tao::event::{Event, StartCause};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tray_icon::menu::MenuEvent;
use windows::core::w;
use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};
use windows::Win32::System::Threading::CreateMutexW;

/// One variant of every event we forward into tao.
///
/// Only menu events are needed: the tray icon handles right-click menu
/// popup itself, and left-click does nothing (disabled in `build_tray`).
#[derive(Debug)]
enum UserEvent {
    Menu(MenuEvent),
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

    // Only the menu event handler is needed. Left-click is intentionally
    // a no-op (`build_tray` sets `with_menu_on_left_click(false)`).
    let menu_proxy = event_loop.create_proxy();
    MenuEvent::set_event_handler(Some(move |e| {
        let _ = menu_proxy.send_event(UserEvent::Menu(e));
    }));

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
            Event::LoopDestroyed => unsafe {
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
