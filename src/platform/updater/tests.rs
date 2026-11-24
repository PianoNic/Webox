//! Offline tests: version compare, release parsing, asset choice, checksum files, the zip reader and the swap / rollback on
//! temporary folders. Nothing here touches the network or the real program.
use super::release::{self, Asset};
use super::{net, swap, zip};
use std::io::Write;
use std::path::PathBuf;

fn tmp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("webox-updater-test-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn asset(name: &str) -> Asset { Asset { name: name.into(), url: format!("https://github.com/PianoNic/Webox/releases/download/v0.2.0/{name}"), size: 10 } }

/// A zip with the given (name, bytes, deflate?) entries, laid out like Compress-Archive writes it.
fn make_zip(files: &[(&str, &[u8], bool)]) -> Vec<u8> {
    let (mut out, mut central) = (Vec::new(), Vec::new());
    for (name, data, deflate) in files {
        let body = if *deflate {
            let mut e = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
            e.write_all(data).unwrap();
            e.finish().unwrap()
        } else { data.to_vec() };
        let mut crc = flate2::Crc::new();
        crc.update(data);
        let (method, off) = (if *deflate { 8u16 } else { 0 }, out.len() as u32);
        let head = |sig: u32, central: bool| {
            let mut h = sig.to_le_bytes().to_vec();
            if central { h.extend(20u16.to_le_bytes()); }
            h.extend(20u16.to_le_bytes()); h.extend(0u16.to_le_bytes()); h.extend(method.to_le_bytes());
            h.extend(0u32.to_le_bytes()); h.extend(crc.sum().to_le_bytes());
            h.extend((body.len() as u32).to_le_bytes()); h.extend((data.len() as u32).to_le_bytes());
            h.extend((name.len() as u16).to_le_bytes()); h.extend(0u16.to_le_bytes());
            // comment length, disk, internal attributes (2 each), external attributes (4), local header offset (4)
            if central { h.extend([0u8; 6]); h.extend(0u32.to_le_bytes()); h.extend(off.to_le_bytes()); }
            h.extend(name.as_bytes());
            h
        };
        out.extend(head(0x0403_4b50, false));
        out.extend(&body);
        central.extend(head(0x0201_4b50, true));
    }
    let cd_off = out.len() as u32;
    out.extend(&central);
    out.extend(0x0605_4b50u32.to_le_bytes()); out.extend([0u8; 4]);
    out.extend((files.len() as u16).to_le_bytes()); out.extend((files.len() as u16).to_le_bytes());
    out.extend((central.len() as u32).to_le_bytes()); out.extend(cd_off.to_le_bytes()); out.extend(0u16.to_le_bytes());
    out
}

#[test]
fn versions() {
    let v = |s: &str| release::parse_version(s).unwrap();
    assert_eq!(v("v0.2.0"), v("0.2.0"));
    assert!(release::is_newer("0.1.9", &v("v0.2.0")));
    assert!(release::is_newer("0.2.0", &v("0.10.0")));
    assert!(!release::is_newer("0.2.0", &v("0.2.0")));
    assert!(!release::is_newer("46.10.0", &v("0.2.0")));
    assert!(!release::is_newer("0.1.0", &v("0.2.0-beta.1")), "pre-releases are never offered");
    assert!(!release::is_newer("not a version", &v("1.0.0")));
}

#[test]
fn parse_release() {
    let j = r###"{"tag_name":"v0.3.0","draft":false,"prerelease":false,"html_url":"https://github.com/PianoNic/Webox/releases/tag/v0.3.0",
        "body":"## Features\n- Added a thing\n- Fixed another","assets":[{"name":"webox-0.3.0-windows-x64.zip","browser_download_url":"https://github.com/x.zip","size":42}]}"###;
    let r = release::parse(j).unwrap().unwrap();
    assert_eq!(r.version.to_string(), "0.3.0");
    assert_eq!(r.assets[0].size, 42);
    assert!(r.page.ends_with("v0.3.0"));
    assert_eq!(release::excerpt(&r.notes, 10, 500), "Features\n• Added a thing\n• Fixed another");
    assert!(release::parse(&j.replace(r#""draft":false"#, r#""draft":true"#)).unwrap().is_none());
    assert!(release::parse(&j.replace(r#""prerelease":false"#, r#""prerelease":true"#)).unwrap().is_none());
    assert!(release::parse("not json").is_err());
}

#[test]
fn asset_choice() {
    let list = vec![asset("webox-0.2.0-linux-x86_64.tar.gz"), asset("webox-0.2.0-linux-x86_64.tar.gz.sha256"), asset("Webox-Setup-0.2.0-x64.exe"),
        asset("webox-0.2.0-macos-universal.zip"), asset("webox-0.2.0-macos-universal.zip.sha256"),
        asset("webox-0.2.0-windows-x64.zip"), asset("webox-0.2.0-windows-x64.zip.sha256")];
    assert_eq!(release::platform_asset(&list, "windows", "x86_64").unwrap().name, "webox-0.2.0-windows-x64.zip");
    assert_eq!(release::platform_asset(&list, "linux", "x86_64").unwrap().name, "webox-0.2.0-linux-x86_64.tar.gz");
    assert!(release::platform_asset(&list, "linux", "aarch64").is_none());
    // macOS: the universal zip on Apple Silicon and Intel alike; no other platform ever picks it
    assert_eq!(release::platform_asset(&list, "macos", "aarch64").unwrap().name, "webox-0.2.0-macos-universal.zip");
    assert_eq!(release::platform_asset(&list, "macos", "x86_64").unwrap().name, "webox-0.2.0-macos-universal.zip");
    assert!(release::platform_asset(&list, "windows", "aarch64").is_none_or(|a| !a.name.contains("macos")));
    assert!(["windows", "linux"].iter().all(|os| release::platform_asset(&list, os, "x86_64").is_some_and(|a| !a.name.contains("macos"))));
    assert_eq!(release::checksum_asset(&list, "webox-0.2.0-macos-universal.zip").unwrap().name, "webox-0.2.0-macos-universal.zip.sha256");
    assert!(release::platform_asset(&[asset("webox-0.2.0-macos-arm64.zip")], "macos", "aarch64").is_none()); // the old per-arch name
    assert_eq!(release::checksum_asset(&list, "webox-0.2.0-windows-x64.zip").unwrap().name, "webox-0.2.0-windows-x64.zip.sha256");
    let sums = vec![asset("webox-0.2.0-windows-x64.zip"), asset("SHA256SUMS")];
    assert_eq!(release::checksum_asset(&sums, "webox-0.2.0-windows-x64.zip").unwrap().name, "SHA256SUMS");
    assert!(release::checksum_asset(&[asset("webox-0.2.0-windows-x64.zip")], "webox-0.2.0-windows-x64.zip").is_none());
}

#[test]
fn checksum_files() {
    let h = "AB".repeat(32);
    assert_eq!(release::parse_checksum(&format!("{h}  a.zip\n"), "a.zip").unwrap(), h.to_lowercase());
    assert_eq!(release::parse_checksum(&format!("{h} *a.zip"), "a.zip").unwrap(), h.to_lowercase());
    assert_eq!(release::parse_checksum(&h, "a.zip").unwrap(), h.to_lowercase());
    assert!(release::parse_checksum(&format!("{h}  b.zip"), "a.zip").is_none());
    assert!(release::parse_checksum("1234  a.zip", "a.zip").is_none());
}

#[test]
fn http_errors() {
    assert_eq!(net::status_fail(404, None), net::Fail::NoRelease);
    assert_eq!(net::status_fail(403, Some("0")), net::Fail::RateLimited);
    assert_eq!(net::status_fail(429, None), net::Fail::RateLimited);
    assert_eq!(net::status_fail(403, Some("12")), net::Fail::Status(403));
}

#[test]
fn zip_reader() {
    let big: Vec<u8> = (0..50_000u32).map(|i| (i % 251) as u8).collect();
    let z = make_zip(&[("LICENSE", b"MIT", false), ("webox.exe", &big, true)]);
    assert_eq!(zip::extract(&z, "webox.exe", 1 << 20).unwrap(), big);
    assert_eq!(zip::extract(&z, "license", 10).unwrap(), b"MIT");
    assert!(zip::extract(&z, "missing.exe", 1 << 20).is_err());
    assert!(zip::extract(&z, "webox.exe", 1000).is_err(), "size cap");
    let nested = make_zip(&[("webox-0.2.0-windows-x64/webox.exe", b"new", true)]);
    assert_eq!(zip::extract(&nested, "webox.exe", 100).unwrap(), b"new");
    let mut broken = z.clone();
    let at = broken.windows(3).position(|w| w == b"MIT").unwrap();
    broken[at] = b'X';
    assert!(zip::extract(&broken, "LICENSE", 10).is_err(), "CRC mismatch");
}

#[test]
fn checksum_mismatch_aborts() {
    let d = tmp("sha");
    let exe = d.join("webox.exe");
    std::fs::write(&exe, b"old").unwrap();
    let zp = d.join("u.zip");
    std::fs::write(&zp, make_zip(&[("webox.exe", b"new", true)])).unwrap();
    let probed = std::cell::Cell::new(false);
    let err = swap::stage(&zp, &"0".repeat(64), &exe, |_| { probed.set(true); Ok(()) }).unwrap_err();
    assert!(err.contains("Prüfsumme"));
    assert!(!probed.get() && !swap::new_path(&exe).exists());
    assert_eq!(std::fs::read(&exe).unwrap(), b"old");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn stage_swap_and_cleanup() {
    let d = tmp("swap");
    let exe = d.join("webox.exe");
    std::fs::write(&exe, b"old").unwrap();
    std::fs::create_dir_all(d.join("plugins")).unwrap();
    std::fs::write(d.join("plugins/mine.dll"), b"user plugin").unwrap();
    let zp = d.join("u.zip");
    std::fs::write(&zp, make_zip(&[("LICENSE", b"MIT", false), ("webox.exe", b"new", true)])).unwrap();
    let sha = swap::sha256_file(&zp).unwrap();
    let new = swap::stage(&zp, &sha.to_uppercase(), &exe, |p| if std::fs::read(p).unwrap() == b"new" { Ok(()) } else { Err("x".into()) }).unwrap();
    swap::swap(&exe, &new).unwrap();
    assert_eq!(std::fs::read(&exe).unwrap(), b"new");
    assert_eq!(std::fs::read(swap::old_path(&exe)).unwrap(), b"old");
    assert!(!new.exists() && !d.join("LICENSE").exists(), "only the program file is unpacked");
    assert_eq!(std::fs::read(d.join("plugins/mine.dll")).unwrap(), b"user plugin");
    assert!(swap::cleanup(&exe) && !swap::old_path(&exe).exists());
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn failed_probe_leaves_the_old_program() {
    let d = tmp("probe");
    let exe = d.join("webox.exe");
    std::fs::write(&exe, b"old").unwrap();
    let zp = d.join("u.zip");
    std::fs::write(&zp, make_zip(&[("webox.exe", b"broken", true)])).unwrap();
    let sha = swap::sha256_file(&zp).unwrap();
    assert!(swap::stage(&zp, &sha, &exe, |_| Err("does not start".into())).is_err());
    assert!(!swap::new_path(&exe).exists());
    assert_eq!(std::fs::read(&exe).unwrap(), b"old");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn swap_failure_rolls_back() {
    let d = tmp("rollback");
    let exe = d.join("webox.exe");
    std::fs::write(&exe, b"old").unwrap();
    // The staged file is missing, so moving it in fails after the old program was renamed: the rename is undone.
    let err = swap::swap(&exe, &d.join("gone.new")).unwrap_err();
    assert!(err.contains("Neue Version"));
    assert_eq!(std::fs::read(&exe).unwrap(), b"old");
    assert!(!swap::old_path(&exe).exists());
    // A restart that fails after a good swap: rollback() brings the old program back.
    std::fs::write(swap::new_path(&exe), b"new").unwrap();
    swap::swap(&exe, &swap::new_path(&exe)).unwrap();
    swap::rollback(&exe).unwrap();
    assert_eq!(std::fs::read(&exe).unwrap(), b"old");
    assert!(swap::rollback(&exe).is_err(), "nothing left to roll back");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn stale_backup_is_replaced() {
    let d = tmp("stale");
    let exe = d.join("webox.exe");
    std::fs::write(&exe, b"v2").unwrap();
    std::fs::write(swap::old_path(&exe), b"v1").unwrap();
    std::fs::write(swap::new_path(&exe), b"v3").unwrap();
    swap::swap(&exe, &swap::new_path(&exe)).unwrap();
    assert_eq!(std::fs::read(&exe).unwrap(), b"v3");
    assert_eq!(std::fs::read(swap::old_path(&exe)).unwrap(), b"v2");
    assert!(swap::folder_writable(&exe));
    let _ = std::fs::remove_dir_all(&d);
}

#[cfg(windows)]
#[test]
fn install_location_match() {
    let exe = std::path::Path::new(r"C:\Users\someone\AppData\Local\Programs\Webox\webox.exe");
    assert!(super::registry::same_folder(r"C:\Users\someone\AppData\Local\Programs\Webox\", exe));
    assert!(super::registry::same_folder(r"c:\users\someone\appdata\local\programs\webox", exe));
    assert!(!super::registry::same_folder(r"C:\Other\Webox\", exe));
}
