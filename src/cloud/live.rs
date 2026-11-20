/// Manual: reads the saved sign-in, runs the real connection path (read-only) and waits for the space list.
/// `cargo test live_connect -- --ignored --nocapture`
#[test]
#[ignore]
fn live_connect() {
    let dir = std::path::Path::new(&std::env::var("APPDATA").unwrap()).join("WebOx");
    let s = crate::system::load_settings(&dir);
    let mut token = s["token"].as_str().unwrap_or("").to_string();
    // the saved access token may have expired: refresh it in memory only (nothing is written back)
    if let Some(r) = s["refresh_token"].as_str().filter(|r| !r.is_empty()) {
        let base = s["idbroker_base"].as_str().filter(|b| !b.is_empty()).unwrap_or(crate::cloud::auth::US_BROKER).to_string();
        if let Ok(t) = crate::cloud::auth::refresh(r, &base, "") { token = t.access; }
    }
    let kms = s["refresh_token"].as_str().filter(|r| !r.is_empty())
        .map(|r| (r.to_string(), s["idbroker_base"].as_str().filter(|b| !b.is_empty()).unwrap_or(crate::cloud::auth::US_BROKER).to_string()));
    let c = super::start(token, s["device"].as_str().map(String::from), kms);
    let hs = |b: &[u8]| { use std::hash::{Hash, Hasher}; let mut h = std::collections::hash_map::DefaultHasher::new(); b.hash(&mut h); h.finish() as u32 };
    let mut asked_older: Vec<String> = vec![]; // spaces for which the history was requested twice (read-only paging check)
    let mut gif_frames: Vec<usize> = vec![]; // frames (max 3 counted) of every GIF blob delivered for the list
    let mut atts: Vec<(String, String)> = vec![]; // (key, mime)
    let mut blobs: std::collections::HashMap<String, u32> = Default::default();
    let end = std::time::Instant::now() + std::time::Duration::from_secs(40);
    while let Some(left) = end.checked_duration_since(std::time::Instant::now()) {
        match c.rx.recv_timeout(left) {
            Ok(super::Resp::Spaces(l)) => { println!("SPACES {}", l.len()); for sp in l.iter().take(40) { c.req(super::Req::Messages(sp.id.clone())); } std::thread::sleep(std::time::Duration::from_secs(1)); continue }
            Ok(super::Resp::Status(m)) => println!("status: {m}"),
            Ok(super::Resp::OlderMessages(_, m, p, r)) => println!("OLDER messages now {} prepended {p} reached_start {r}", m.len()),
            Ok(super::Resp::Messages(sid, m)) => { if asked_older.is_empty() && m.len() > 10 { asked_older.push(sid.clone()); c.req(super::Req::MessagesBefore(sid.clone())); c.req(super::Req::MessagesBefore(sid.clone())); } for a in m.iter().flat_map(|x| x.images.iter()) { if !atts.iter().any(|(k, _)| *k == a.key) { atts.push((a.key.clone(), a.mime.clone())); } } if let Some(k) = m.iter().flat_map(|x| x.images.iter()).next().map(|a| a.key.clone()) { c.req(super::Req::FullImage(k)); } println!("messages: {} with images: {} reaction chips: {} mine: {}", m.len(), m.iter().map(|x| x.images.len()).sum::<usize>(), m.iter().map(|x| x.reactions.len()).sum::<usize>(), m.iter().flat_map(|x| x.reactions.iter()).filter(|r| r.mine.is_some()).count()) },
            Ok(super::Resp::SpaceAvatars(v)) => println!("SPACEAVATARS {:?}", v.iter().map(|(_, b)| (b.len(), image::load_from_memory(b).map(|i| (i.width(), i.height())).ok())).collect::<Vec<_>>()),
            Ok(super::Resp::Images(v)) if v.iter().all(|(k, _)| !k.starts_with("full:")) => {
                for (k, b) in &v {
                    blobs.insert(k.clone(), hs(b));
                    if image::guess_format(b).ok() == Some(image::ImageFormat::Gif) {
                        use image::AnimationDecoder;
                        let n = image::codecs::gif::GifDecoder::new(std::io::Cursor::new(b.as_slice())).map(|d| d.into_frames().take(3).count()).unwrap_or(0);
                        gif_frames.push(n);
                    }
                }
            }
            Ok(super::Resp::Images(v)) => println!("images: {} bytes {:?}", v.len(), v.iter().map(|(_, b)| b.len()).collect::<Vec<_>>()),
            Ok(super::Resp::Error(e)) => println!("error: {e}"),
            Ok(super::Resp::AuthFailed(e)) => { println!("authfailed: {e}"); return }
            Ok(_) => {}
            Err(_) => break,
        }
    }
    let uniq_keys: std::collections::HashSet<&String> = atts.iter().map(|a| &a.0).collect();
    let uniq_blobs: std::collections::HashSet<u32> = blobs.values().copied().collect();
    let gif_keys: Vec<&String> = atts.iter().filter(|a| a.1 == "image/gif").map(|a| &a.0).collect();
    let gif_blobs: std::collections::HashSet<u32> = gif_keys.iter().filter_map(|k| blobs.get(*k)).copied().collect();
    println!("GIFCHECK gif blobs {} animated {} single-frame {}", gif_frames.len(), gif_frames.iter().filter(|n| **n > 1).count(), gif_frames.iter().filter(|n| **n == 1).count());
    println!("IMGCHECK attachments {} unique keys {} | downloaded {} unique blobs {} | gifs {} downloaded-gif unique blobs {}",
        atts.len(), uniq_keys.len(), blobs.len(), uniq_blobs.len(), gif_keys.len(), gif_blobs.len());
}
