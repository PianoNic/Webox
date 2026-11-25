//! Generic passwords in the user's login Keychain (Security framework), for the secret plugin settings: service "Webox",
//! account `Webox/plugin/<plugin>/<setting>` (the Credential Manager target name on Windows). The SecKeychain calls are the
//! small C API that takes plain strings; they still work on current macOS.
use std::ffi::c_void;

const SERVICE: &str = "Webox";
const ERR_SEC_ITEM_NOT_FOUND: i32 = -25300;

#[link(name = "Security", kind = "framework")]
extern "C" {
    fn SecKeychainFindGenericPassword(kc: *const c_void, slen: u32, service: *const u8, alen: u32, account: *const u8,
        plen: *mut u32, pdata: *mut *mut c_void, item: *mut *mut c_void) -> i32;
    fn SecKeychainAddGenericPassword(kc: *const c_void, slen: u32, service: *const u8, alen: u32, account: *const u8,
        plen: u32, pdata: *const c_void, item: *mut *mut c_void) -> i32;
    fn SecKeychainItemModifyAttributesAndData(item: *mut c_void, attrs: *const c_void, len: u32, data: *const c_void) -> i32;
    fn SecKeychainItemDelete(item: *mut c_void) -> i32;
    fn SecKeychainItemFreeContent(attrs: *mut c_void, data: *mut c_void) -> i32;
}
#[link(name = "CoreFoundation", kind = "framework")]
extern "C" { fn CFRelease(cf: *const c_void); }

/// The item for `account` and, when `want_data`, its password. Err: the Keychain's status code.
fn find(account: &str, want_data: bool) -> Result<(*mut c_void, Option<Vec<u8>>), i32> {
    let (mut len, mut data, mut item) = (0u32, std::ptr::null_mut(), std::ptr::null_mut());
    let st = unsafe {
        SecKeychainFindGenericPassword(std::ptr::null(), SERVICE.len() as u32, SERVICE.as_ptr(), account.len() as u32, account.as_ptr(),
            if want_data { &mut len } else { std::ptr::null_mut() }, if want_data { &mut data } else { std::ptr::null_mut() }, &mut item)
    };
    if st != 0 { return Err(st); }
    let bytes = (!data.is_null()).then(|| unsafe {
        let v = std::slice::from_raw_parts(data as *const u8, len as usize).to_vec();
        SecKeychainItemFreeContent(std::ptr::null_mut(), data);
        v
    });
    Ok((item, bytes))
}

pub fn get(account: &str) -> Option<String> {
    let (item, data) = find(account, true).ok()?;
    if !item.is_null() { unsafe { CFRelease(item) }; }
    String::from_utf8(data?).ok()
}

pub fn set(account: &str, value: &str) -> Result<(), String> {
    let st = match find(account, false) {
        Ok((item, _)) => unsafe {
            let st = SecKeychainItemModifyAttributesAndData(item, std::ptr::null(), value.len() as u32, value.as_ptr() as *const c_void);
            CFRelease(item);
            st
        },
        Err(ERR_SEC_ITEM_NOT_FOUND) => unsafe {
            SecKeychainAddGenericPassword(std::ptr::null(), SERVICE.len() as u32, SERVICE.as_ptr(), account.len() as u32, account.as_ptr(),
                value.len() as u32, value.as_ptr() as *const c_void, std::ptr::null_mut())
        },
        Err(e) => e,
    };
    if st == 0 { Ok(()) } else { Err(crate::trq!("Keychain not reachable (error {{0}})", st)) }
}

pub fn delete(account: &str) {
    if let Ok((item, _)) = find(account, false) {
        unsafe { SecKeychainItemDelete(item); CFRelease(item); }
    }
}

#[cfg(test)]
mod tests {
    /// Writes, reads, overwrites and deletes one item. Needs an unlocked login Keychain (a desktop session; CI runners have
    /// none): `cargo test --bin webox keychain -- --ignored` on a Mac.
    #[test]
    #[ignore]
    fn keychain_roundtrip() {
        let acc = format!("Webox/plugin/test.roundtrip/{}", std::process::id());
        super::set(&acc, "geheim").unwrap();
        assert_eq!(super::get(&acc).as_deref(), Some("geheim"));
        super::set(&acc, "neu").unwrap();
        assert_eq!(super::get(&acc).as_deref(), Some("neu"));
        super::delete(&acc);
        assert_eq!(super::get(&acc), None);
    }
}
