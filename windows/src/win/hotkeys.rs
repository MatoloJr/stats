//! Global keyboard shortcuts (`RegisterHotKey`): they work from any app.
//! Each command in `stats_core::hotkey::Command` gets id `index + 1`. A
//! combination another app already owns fails to register; that is logged
//! and shown in Settings, and the rest keep working.

use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;

use super::app::{save_setting, with_app, App};
use super::{bar, dashboard, line, util};
use stats_core::hotkey::{Command, Hotkey, Key, Mods};

fn id(command: Command) -> i32 {
    command.index() as i32 + 1
}

/// Virtual-key code for a key.
pub fn vk_for(key: Key) -> Option<u32> {
    Some(match key {
        Key::F(n) => VK_F1 as u32 + (n as u32 - 1),
        Key::Char(c) if c.is_ascii_uppercase() || c.is_ascii_digit() => c as u32,
        Key::Char(',') => VK_OEM_COMMA as u32,
        Key::Char('.') => VK_OEM_PERIOD as u32,
        Key::Char('/') => VK_OEM_2 as u32,
        Key::Char(';') => VK_OEM_1 as u32,
        Key::Char('-') => VK_OEM_MINUS as u32,
        Key::Char('=') => VK_OEM_PLUS as u32,
        Key::Char('[') => VK_OEM_4 as u32,
        Key::Char(']') => VK_OEM_6 as u32,
        Key::Char('\'') => VK_OEM_7 as u32,
        Key::Char(_) => return None,
    })
}

/// The reverse, for recording a new shortcut from a key press.
pub fn key_for(vk: u32) -> Option<Key> {
    let k = vk as u16;
    Some(match k {
        0x41..=0x5A | 0x30..=0x39 => Key::Char(k as u8 as char),
        _ if (VK_F1..=VK_F24).contains(&k) => Key::F((k - VK_F1) as u8 + 1),
        _ if k == VK_OEM_COMMA => Key::Char(','),
        _ if k == VK_OEM_PERIOD => Key::Char('.'),
        _ if k == VK_OEM_2 => Key::Char('/'),
        _ if k == VK_OEM_1 => Key::Char(';'),
        _ if k == VK_OEM_MINUS => Key::Char('-'),
        _ if k == VK_OEM_PLUS => Key::Char('='),
        _ if k == VK_OEM_4 => Key::Char('['),
        _ if k == VK_OEM_6 => Key::Char(']'),
        _ if k == VK_OEM_7 => Key::Char('\''),
        _ => return None,
    })
}

/// Modifiers held right now (for recording).
pub fn mods_down() -> Mods {
    let down = |vk: VIRTUAL_KEY| unsafe { GetKeyState(vk as i32) } < 0;
    Mods { ctrl: down(VK_CONTROL), alt: down(VK_MENU), shift: down(VK_SHIFT), win: down(VK_LWIN) || down(VK_RWIN) }
}

pub fn unregister_all(main: HWND) {
    for command in Command::ALL {
        unsafe { UnregisterHotKey(main, id(command)) };
    }
}

/// (Re)register every shortcut from the settings. Records which worked.
pub fn register_all(app: &mut App) {
    unregister_all(app.main);
    for command in Command::ALL {
        let i = command.index();
        app.hotkey_ok[i] = None;
        let Some(hotkey) = app.config.hotkeys[i] else { continue };
        let Some(vk) = vk_for(hotkey.key) else { continue };
        let mut mods = MOD_NOREPEAT;
        if hotkey.mods.ctrl {
            mods |= MOD_CONTROL;
        }
        if hotkey.mods.alt {
            mods |= MOD_ALT;
        }
        if hotkey.mods.shift {
            mods |= MOD_SHIFT;
        }
        if hotkey.mods.win {
            mods |= MOD_WIN;
        }
        let ok = unsafe { RegisterHotKey(app.main, id(command), mods, vk) } != 0;
        if !ok {
            util::log(&format!("shortcut {} for '{}' is in use by another app", hotkey.label(), command.label()));
        }
        app.hotkey_ok[i] = Some(ok);
    }
}

/// Save a new combination (or none) for `command` and re-register.
pub fn set(app: &mut App, command: Command, hotkey: Option<Hotkey>) {
    app.config.hotkeys[command.index()] = hotkey;
    let text = app.config.hotkey_text(command);
    save_setting(app, command.key(), &text);
    register_all(app);
}

/// WM_HOTKEY. Runs with the app released (it may show windows).
pub fn handle(raw_id: usize) {
    let Some(command) = Command::ALL.into_iter().find(|&c| id(c) as usize == raw_id) else { return };
    match command {
        Command::Dashboard => {
            with_app(|app| {
                if app.dash_visible() {
                    dashboard::hide(app);
                } else {
                    dashboard::show_tab(app, dashboard::Tab::Overview);
                }
            });
        }
        Command::Settings => {
            with_app(|app| dashboard::show_tab(app, dashboard::Tab::Settings));
        }
        Command::Readings => {
            with_app(|app| {
                line::wake_island(app);
                bar::toggle(app);
            });
        }
        Command::Pin => {
            with_app(|app| {
                let pinned = !app.config.bar_pinned;
                line::wake_island(app);
                bar::set_pinned(app, pinned);
            });
        }
        Command::Line => {
            with_app(|app| {
                app.config.bar_visible = !app.config.bar_visible;
                let value = app.config.bar_visible.to_string();
                save_setting(app, "bar-visible", &value);
                app.update_overlay();
            });
        }
    }
}
