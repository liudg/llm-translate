//! Start with Windows (DESIGN 5.7): `HKCU\...\Run` value `LLM Translate` =
//! `"<exe>" --minimized`. Always the current user, even when the app runs
//! elevated (tauri-plugin-autostart writes HKLM then). Task Manager's
//! "disabled" flag (`StartupApproved\Run`, odd first byte) counts as off, and
//! enabling clears it.

use windows_registry::{CURRENT_USER, Type};

const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const APPROVED: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";
const NAME: &str = "LLM Translate";
pub const ARG: &str = "--minimized";

/// Task Manager's "enabled" marker.
const APPROVED_ENABLED: [u8; 12] = [2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];

pub fn is_enabled() -> bool {
    let registered = CURRENT_USER.open(RUN).and_then(|k| k.get_string(NAME)).is_ok();
    let disabled = CURRENT_USER
        .open(APPROVED)
        .and_then(|k| k.get_value(NAME))
        .is_ok_and(|v| v.first().is_some_and(|b| b & 1 == 1));
    registered && !disabled
}

pub fn set(enabled: bool) -> anyhow::Result<()> {
    if enabled {
        let exe = std::env::current_exe()?;
        CURRENT_USER.create(RUN)?.set_string(NAME, format!("\"{}\" {ARG}", exe.display()))?;
        // Only touch the approval key when it exists and marks us disabled.
        if let Ok(k) = CURRENT_USER.options().read().write().open(APPROVED)
            && k.get_value(NAME).is_ok()
        {
            k.set_bytes(NAME, Type::Bytes, &APPROVED_ENABLED)?;
        }
    } else if let Ok(k) = CURRENT_USER.options().write().open(RUN) {
        match k.remove_value(NAME) {
            Ok(()) => {}
            Err(e) if e.code() == windows::Win32::Foundation::ERROR_FILE_NOT_FOUND.to_hresult() => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}
