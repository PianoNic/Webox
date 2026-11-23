#[test]
fn extract_code() {
    assert_eq!(super::extract("webox-code:ABC123._~-xyz").as_deref(), Some("ABC123._~-xyz"));
    assert_eq!(super::extract("https://x/#webox-code:AB%2DCD%7Eef").as_deref(), Some("AB-CD~ef"));
    assert_eq!(super::extract("webox-code:abc&state=1").as_deref(), Some("abc"));
    assert_eq!(super::extract("nothing here"), None);
}

#[test]
fn capture_true_mode() {
    // A real-shaped OOB code (base64url_PE93_uuid), ≥20 chars.
    let code = "OTF1OTgxMjM0NTY3ODkw_PE93_1a2b3c4d5e6f7a8b9c0d";
    // Navigation URL (query and fragment), our marker, raw JSON, and a host-bridge string.
    assert_eq!(super::capture(&format!("https://idbroker-eu.webex.com/idb/oauth2/v1/authorize/callback?code={code}&state=s")).as_deref(), Some(code));
    assert_eq!(super::capture(&format!("urn:ietf:wg:oauth:2.0:oob#code={code}")).as_deref(), Some(code));
    assert_eq!(super::capture(&format!("webox-code:{code}")).as_deref(), Some(code));
    assert_eq!(super::capture(&format!("{{\"type\":\"idbroker.authorize.response\",\"code\":\"{code}\"}}")).as_deref(), Some(code));
    // idbroker's JSON key shapes.
    assert_eq!(super::capture(&format!("{{\"messageType\":\"idbroker.authorize.response\",\"authorizationCode\":\"{code}\"}}")).as_deref(), Some(code));
    assert_eq!(super::capture(&format!("{{\"authCode\":\"{code}\",\"AuthResult\":0}}")).as_deref(), Some(code));
    assert_eq!(super::capture(&format!("{{\"RedirectUri\":\"urn:ietf:wg:oauth:2.0:oob?code={code}\"}}")).as_deref(), Some(code));
    // Must NOT match short decoy params.
    assert_eq!(super::capture("https://x/?country_code=CH&error_code=invalid_request"), None);
    assert_eq!(super::capture("no code here"), None);
}

#[test]
fn only_codes_are_accepted() {
    // A ready-made token from a page is never taken (no PKCE behind it); the code marker still is.
    assert!(super::classify("webox-token:{\"access_token\":\"x\"}").is_none());
    assert!(matches!(super::classify("webox-code:ABCDEFGHIJKL"), Some(super::Captured::Code(c)) if c == "ABCDEFGHIJKL"));
}

#[test]
fn log_redaction() {
    let code = "OTF1OTgxMjM0NTY3ODkw_PE93_1a2b3c4d5e6f7a8b9c0d";
    for s in [format!("webox-code:{code}"), format!("{{\"authorizationCode\":\"{code}\"}}"), format!("<p hidden=\"\"> code={code}</p>"),
              format!("x?code={}", code.replace('_', "%5F"))] {
        let r = super::redact(&s);
        assert!(!r.contains(code) && !r.contains("OTF1OTgxMjM0NTY3ODkw"), "{r}");
    }
    assert_eq!(super::redact("webox-log:page:{\"bodyLen\":12}"), "webox-log:page:{\"bodyLen\":12}");
    assert_eq!(super::log_url("https://idbroker.webex.com/cb?code=abc#webox-code:abc"), "https://idbroker.webex.com/cb");
}
