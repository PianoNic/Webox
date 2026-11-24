//! Where secret plugin settings (`password` fields) live: the OS keyring. Windows: the Credential Manager (generic credentials
//! "Webox/plugin/<plugin>/<setting>", local machine). Linux: the Secret Service through `secret-tool` (libsecret) when it is
//! installed. macOS: the login Keychain (generic password, service "Webox", account "Webox/plugin/<plugin>/<setting>").
//! Elsewhere, or when the keyring refuses, the store falls back to the settings file and the page says so.
pub trait Secrets: Send {
    fn get(&self, plugin: &str, id: &str) -> Option<String>;
    fn set(&mut self, plugin: &str, id: &str, value: &str) -> Result<(), String>;
    fn delete(&mut self, plugin: &str, id: &str);
}

fn target(plugin: &str, id: &str) -> String { format!("Webox/plugin/{plugin}/{id}") }

/// The OS keyring.
pub struct OsSecrets;

#[cfg(windows)]
impl Secrets for OsSecrets {
    fn get(&self, plugin: &str, id: &str) -> Option<String> {
        use windows::Win32::Security::Credentials::{CredFree, CredReadW, CREDENTIALW, CRED_TYPE_GENERIC};
        let name: Vec<u16> = target(plugin, id).encode_utf16().chain([0]).collect();
        let mut c: *mut CREDENTIALW = std::ptr::null_mut();
        unsafe {
            CredReadW(windows::core::PCWSTR(name.as_ptr()), CRED_TYPE_GENERIC, 0, &mut c).ok()?;
            let blob = std::slice::from_raw_parts((*c).CredentialBlob, (*c).CredentialBlobSize as usize).to_vec();
            CredFree(c as *const _);
            String::from_utf8(blob).ok()
        }
    }
    fn set(&mut self, plugin: &str, id: &str, value: &str) -> Result<(), String> {
        use windows::Win32::Security::Credentials::{CredWriteW, CREDENTIALW, CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC};
        let mut name: Vec<u16> = target(plugin, id).encode_utf16().chain([0]).collect();
        let mut user: Vec<u16> = "Webox".encode_utf16().chain([0]).collect();
        let mut blob = value.as_bytes().to_vec();
        let c = CREDENTIALW {
            Type: CRED_TYPE_GENERIC, TargetName: windows::core::PWSTR(name.as_mut_ptr()), UserName: windows::core::PWSTR(user.as_mut_ptr()),
            CredentialBlobSize: blob.len() as u32, CredentialBlob: blob.as_mut_ptr(), Persist: CRED_PERSIST_LOCAL_MACHINE, ..Default::default()
        };
        unsafe { CredWriteW(&c, 0) }.map_err(|e| format!("Anmeldeinformationsverwaltung: {e}"))
    }
    fn delete(&mut self, plugin: &str, id: &str) {
        use windows::Win32::Security::Credentials::{CredDeleteW, CRED_TYPE_GENERIC};
        let name: Vec<u16> = target(plugin, id).encode_utf16().chain([0]).collect();
        let _ = unsafe { CredDeleteW(windows::core::PCWSTR(name.as_ptr()), CRED_TYPE_GENERIC, 0) };
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
impl Secrets for OsSecrets {
    fn get(&self, plugin: &str, id: &str) -> Option<String> {
        let o = crate::system::restore_env(std::process::Command::new("secret-tool").args(["lookup", "application", "webox", "plugin", plugin, "setting", id]))
            .stderr(std::process::Stdio::null()).output().ok()?;
        (o.status.success() && !o.stdout.is_empty()).then(|| String::from_utf8_lossy(&o.stdout).into_owned())
    }
    fn set(&mut self, plugin: &str, id: &str, value: &str) -> Result<(), String> {
        use std::io::Write;
        let label = format!("Webox: {}", target(plugin, id));
        let mut c = crate::system::restore_env(std::process::Command::new("secret-tool").args(["store", "--label", &label, "application", "webox", "plugin", plugin, "setting", id]))
            .stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null())
            .spawn().map_err(|_| "secret-tool nicht gefunden (libsecret-tools)".to_string())?;
        c.stdin.take().ok_or("secret-tool")?.write_all(value.as_bytes()).map_err(|e| e.to_string())?;
        if c.wait().map_err(|e| e.to_string())?.success() { Ok(()) } else { Err("Schlüsselbund nicht erreichbar".into()) }
    }
    fn delete(&mut self, plugin: &str, id: &str) {
        let _ = crate::system::restore_env(std::process::Command::new("secret-tool").args(["clear", "application", "webox", "plugin", plugin, "setting", id]))
            .stderr(std::process::Stdio::null()).status();
    }
}

#[cfg(target_os = "macos")]
impl Secrets for OsSecrets {
    fn get(&self, plugin: &str, id: &str) -> Option<String> { crate::platform::macos::keychain::get(&target(plugin, id)) }
    fn set(&mut self, plugin: &str, id: &str, value: &str) -> Result<(), String> { crate::platform::macos::keychain::set(&target(plugin, id), value) }
    fn delete(&mut self, plugin: &str, id: &str) { crate::platform::macos::keychain::delete(&target(plugin, id)) }
}

#[cfg(not(any(windows, unix)))]
impl Secrets for OsSecrets {
    fn get(&self, _: &str, _: &str) -> Option<String> { None }
    fn set(&mut self, _: &str, _: &str, _: &str) -> Result<(), String> { Err("kein Schlüsselbund".into()) }
    fn delete(&mut self, _: &str, _: &str) {}
}

/// In memory only: tests and --sample runs (nothing reaches the real keyring). `fail` makes `set` refuse (file fallback).
#[derive(Default)]
pub struct MemSecrets { pub map: std::collections::HashMap<String, String>, pub fail: bool }
impl Secrets for MemSecrets {
    fn get(&self, plugin: &str, id: &str) -> Option<String> { self.map.get(&target(plugin, id)).cloned() }
    fn set(&mut self, plugin: &str, id: &str, value: &str) -> Result<(), String> {
        if self.fail { return Err("kein Schlüsselbund".into()); }
        self.map.insert(target(plugin, id), value.into());
        Ok(())
    }
    fn delete(&mut self, plugin: &str, id: &str) { self.map.remove(&target(plugin, id)); }
}
