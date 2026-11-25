use super::*;

/// An SCR seals the file so that the receiving side (`jose::gcm_open` with the SCR's key / iv / aad / tag, as images.rs does
/// it) gets the bytes back, and its JSON has the SCR fields.
#[test]
fn scr_round_trip_and_shape() {
    let scr = NewScr::new("2026-11-16T10:00:00.000Z");
    let mut data = b"hello file".to_vec();
    let tag = scr.seal(&mut data).unwrap();
    assert_ne!(data, b"hello file");
    assert_eq!(data.len(), 10); // no tag appended: it lives in the SCR
    let j = scr.json("https://files-api-a.wbx2.com/v1/spaces/x/contents/y/versions/z/bytes", &tag);
    let keys: Vec<&str> = j.as_object().unwrap().keys().map(String::as_str).collect();
    for k in ["enc", "key", "iv", "aad", "loc", "tag"] { assert!(keys.contains(&k), "{k}"); }
    assert_eq!(j["enc"], "A256GCM");
    let (k, iv, t) = (jose::unb64(j["key"].as_str().unwrap()).unwrap(), jose::unb64(j["iv"].as_str().unwrap()).unwrap(), jose::unb64(j["tag"].as_str().unwrap()).unwrap());
    assert_eq!((k.len(), iv.len(), t.len()), (32, 12, 16));
    assert_eq!(jose::gcm_open(&k, &iv, j["aad"].as_str().unwrap().as_bytes(), &data, &t).unwrap(), b"hello file");
    // The encrypted SCR has the received shape: dir/A256GCM JWE without kid, opened with the space key.
    let space_key = [3u8; 32];
    let jwe = jose::encrypt_dir_nokid(&space_key, j.to_string().as_bytes()).unwrap();
    assert_eq!(jose::header(&jwe), json!({"alg": "dir", "enc": "A256GCM"}));
    let back: Value = serde_json::from_slice(&jose::decrypt_dir(&space_key, &jwe).unwrap()).unwrap();
    assert_eq!(back["loc"], j["loc"]);
}

#[test]
fn upload_session_bodies() {
    assert_eq!(init_body(179483, Some(".pdf")), json!({"uploadProtocol": "content-length", "fileSize": 179483, "claimedFileType": ".pdf"}));
    assert_eq!(init_body(10, None), json!({"uploadProtocol": "content-length", "fileSize": 10})); // thumbnail: no claimedFileType
    assert_eq!(finish_body(3, &sha256_hex(b"abc")), json!({"fileSize": 3, "fileHash": "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"}));
    assert_eq!(claimed_type("Bericht.final.PDF"), ".PDF");
    assert_eq!(claimed_type("README"), "README");
    assert_eq!(with_query("https://f/upload_sessions", TRANSCODE), "https://f/upload_sessions?transcode=true");
    assert_eq!(with_query("https://f/finish?x=1", TRANSCODE), "https://f/finish?x=1&transcode=true");
}

#[test]
fn categories_like_the_sdk() {
    let v = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    assert_eq!(content_category(&v(&["image/png", "image/jpeg"])), "images");
    assert_eq!(content_category(&v(&["video/mp4"])), "videos");
    assert_eq!(content_category(&v(&["image/png", "application/pdf"])), "documents");
    assert_eq!(content_category(&v(&["application/pdf"])), "documents");
    assert_eq!(content_category(&v(&["image/png", "video/mp4"])), "documents");
}

#[test]
fn share_activity_shape() {
    let it = Item { name_enc: "ENC(a.png)".into(), size: 5, mime: "image/png".into(), url: "https://files/d".into(), scr_enc: "SCR".into(), dims: Some((800, 600)), thumb: Some((640, 480, "https://files/t".into(), "TSCR".into())) };
    let f = file_item("kms://k/keys/1", &it);
    assert_eq!(f, json!({"objectType": "file", "displayName": "ENC(a.png)", "fileSize": 5, "mimeType": "image/png", "url": "https://files/d", "scr": "SCR",
        "encryptionKeyUrl": "kms://k/keys/1", "width": 800, "height": 600, "image": {"width": 640, "height": 480, "url": "https://files/t", "scr": "TSCR"}}));
    let text = json!({"objectType": "comment", "displayName": "ENC(hi)"});
    let b = share_body("kms://k/keys/1", Some(text), vec![f], &["image/png".into()], Some("root-1"));
    assert_eq!(b["verb"], "share");
    assert_eq!(b["object"]["objectType"], "content"); // the text's "comment" type is not taken over
    assert_eq!(b["object"]["displayName"], "ENC(hi)");
    assert_eq!(b["object"]["contentCategory"], "images");
    assert_eq!(b["parent"], json!({"id": "root-1", "type": "reply"}));
    let b = share_body("k", None, vec![], &["application/pdf".into()], None);
    assert!(b["object"].get("displayName").is_none() && b.get("parent").is_none());
    assert_eq!(activities::GIF_SHARE_PATH, "/content?transcode=true&async=false");
}

#[test]
fn thumbnails_fit_the_sdk_box() {
    let big = image::DynamicImage::new_rgb8(1600, 900);
    let mut png = std::io::Cursor::new(Vec::new());
    big.write_to(&mut png, image::ImageFormat::Png).unwrap();
    let (t, tw, th, w, h) = thumbnail(&png.into_inner()).unwrap();
    assert_eq!((w, h), (1600, 900));
    assert!(tw <= THUMB_MAX_W && th <= THUMB_MAX_H && tw == 640, "{tw}x{th}");
    assert_eq!(image::guess_format(&t).unwrap(), image::ImageFormat::Png);
    assert!(thumbnail(b"%PDF-1.7 not an image").is_none());
    let small = image::DynamicImage::new_rgb8(100, 50);
    let mut png = std::io::Cursor::new(Vec::new());
    small.write_to(&mut png, image::ImageFormat::Png).unwrap();
    assert_eq!(thumbnail(&png.into_inner()).map(|x| (x.1, x.2)), Some((100, 50))); // never enlarged
}

#[test]
fn refusals_in_the_originals_words() {
    assert_eq!(refusal(413, ""), crate::trq!("File too large"));
    assert_eq!(refusal(403, "{\"message\":\"File extension blocked\"}"), crate::trq!("Blocked file"));
    assert_eq!(refusal(500, "x"), "http 500");
}
