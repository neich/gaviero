//! Windows input: raw console records, read with `ReadConsoleInputW` and
//! forwarded untouched, so every key (AltGr, dead keys, key-up) reaches the
//! session exactly as the console reported it.

use super::HostEvent;
use super::encode::{KeyRecord, MouseRecord};
use tokio::sync::mpsc::UnboundedSender;
use windows_sys::Win32::System::Console::{
    GetStdHandle, INPUT_RECORD, KEY_EVENT, MOUSE_EVENT, ReadConsoleInputW, STD_INPUT_HANDLE,
    WINDOW_BUFFER_SIZE_EVENT,
};

/// Read console input records on a std thread until the console read fails
/// or the host loop is gone.
pub(crate) fn spawn_console_reader(tx: UnboundedSender<HostEvent>) {
    let spawned = std::thread::Builder::new()
        .name("session-host-console".into())
        .spawn(move || read_loop(tx));
    if let Err(e) = spawned {
        tracing::error!("could not start the console reader: {e}");
    }
}

fn read_loop(tx: UnboundedSender<HostEvent>) {
    // SAFETY: plain Win32 calls; `buf` outlives every call that writes it and
    // `n` never exceeds its length.
    let handle = unsafe { GetStdHandle(STD_INPUT_HANDLE) };
    let mut buf: Vec<INPUT_RECORD> = vec![unsafe { std::mem::zeroed() }; 128];
    loop {
        let mut n: u32 = 0;
        let ok = unsafe { ReadConsoleInputW(handle, buf.as_mut_ptr(), buf.len() as u32, &mut n) };
        if ok == 0 {
            tracing::warn!(
                "ReadConsoleInputW failed: {}",
                std::io::Error::last_os_error()
            );
            return;
        }
        for rec in &buf[..n as usize] {
            let Some(event) = to_event(rec) else {
                continue;
            };
            if tx.send(event).is_err() {
                return;
            }
        }
    }
}

fn to_event(rec: &INPUT_RECORD) -> Option<HostEvent> {
    // SAFETY: the union field read matches `EventType`.
    match rec.EventType as u32 {
        KEY_EVENT => {
            let k = unsafe { rec.Event.KeyEvent };
            Some(HostEvent::Key(KeyRecord {
                vk: k.wVirtualKeyCode,
                scan: k.wVirtualScanCode,
                uchar: unsafe { k.uChar.UnicodeChar },
                key_down: k.bKeyDown != 0,
                ctrl_state: k.dwControlKeyState,
                repeat: k.wRepeatCount,
            }))
        }
        MOUSE_EVENT => {
            let m = unsafe { rec.Event.MouseEvent };
            Some(HostEvent::Mouse(MouseRecord {
                col: m.dwMousePosition.X.max(0) as u16,
                row: m.dwMousePosition.Y.max(0) as u16,
                buttons: m.dwButtonState,
                ctrl_state: m.dwControlKeyState,
                flags: m.dwEventFlags,
            }))
        }
        WINDOW_BUFFER_SIZE_EVENT => Some(HostEvent::Resize),
        _ => None,
    }
}
