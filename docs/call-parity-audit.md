# Call parity audit: Webox compared with the official Webex client, function by function

Audited 2026-11-27 against the official Windows client **46.11.0.36194**. The Webox baseline is `main` (edca2be) plus open PR #49 (`fix/call-media-shape`, head 073cc71). All Webox `file:line` references point at that PR head.

**Short answer to "is the call logic 1:1 like the decompile?": no.** Signalling for a plain 1:1 audio call (join, ROAP OFFER/ANSWER/OK, keep-alive, mute flag, leave) is close. Media transport, SDP content, recovery and the mid-call features are not. This document makes "1:1" measurable as a parity score per stage, and it lists the fixes in PR-sized batches.

## Summary

| # | Stage | Rows | Match | Score | Main gaps |
|---|---|---|---|---|---|
| 1 | Setup: join, ringing, answer, decline | 16 | 9 | **56 %** | reachability `joinCookie`, `device` fields, alert `sequence`, call waiting |
| 2 | SDP offer | 17 | 5 | **29 %** | line directions, Opus/H.264 fmtp, RTX/FEC, extmaps, multistream |
| 3 | ROAP sequencing | 12 | 6 | **50 %** | OFFER_REQUEST, WAIT_RX_OK, glare, error types, re-OFFER on direction change |
| 4 | ICE / DTLS / transport | 11 | 3 | **27 %** | no TCP/TLS-443 fallback, different recovery (no 5-try cap), no network-change trigger, no reachability |
| 5 | Mute, camera, camera switch | 7 | 3 | **43 %** | no direction renegotiation, no video call / answer with video |
| 6 | Share | 8 | 3 | **38 %** | floor body unconfirmed, no share type hints, slides line always sendrecv, receive without CSI |
| 7 | Mid-call | 10 | 2 | **20 %** | hold/resume, escalation, call waiting, move media; several controls only in webex-js-sdk shape |
| 8 | End and cleanup | 6 | 2 | **33 %** | leave `correlationId`, no "end after N failed reconnects", floor not released on end |
| 9 | Stats and metrics to the server | – | – | n/a | Webox sends none, by design (listed in §9) |
| | **All stages (1–8)** | **87** | **33** | **38 %** | |

How the score works:

* Score = rows marked **match** / rows in the stage.
* "Differs", "missing" and "unconfirmed" all count as not matching.
* A row counts only if the official behaviour is backed by a string or a decompiled function, listed in the evidence column.
* Webox's own extras with no official counterpart (for example the one-retry leave) are not rows.

The biggest real-call risks are all in stage 4:

* **No transport fallback.** On a network that blocks UDP 5004/9000, Webox calls have no media. The official client goes UDP, then TCP 5004, then TLS 443 (through a proxy if needed).
* **Different recovery.**
  * Webox keeps ICE-restarting the same peer connection forever. Whether Webex media servers (ICE lite) answer an ICE-restart OFFER has **not been confirmed**.
  * The official client tears the media connection down and rebuilds it. It stops after 5 failed attempts and ends the call with a media-drop report.

## Evidence base and method

* **Ghidra.**
  * Ghidra 11.4.3, headless, on a private copy of the `webox` project. The copy was deleted afterwards.
  * `spark-windows-desktop-ui.dll` was run `-readOnly -noanalysis`. `mediasession.dll` was imported `-noanalysis`.
  * The `RefsToString.java` script finds the code that loads a string (RIP-relative LEA) and decompiles the function around it.
  * The UI DLL in the project is not auto-analysed, so its decompiles are fragments with wrong function boundaries. Function names below are Ghidra's `FUN_<VA>`.
  * `spark-windows-media.dll` evidence is the decompile already cited in PR #49: `Connection::initializeMedia` 0x1fa190, line builder 0x2051f0, `Connection::renegotiateSdp` 0x205470, `LocusManager::createKeepAliveTimer` 0x184b5d990.
  * `wmeclient.dll` was not needed.
* **Binary strings.** Read from the installed DLLs with their RVAs. Image base is 0x180000000, so VA = 0x180000000 + RVA. Covered DLLs: `spark-windows-desktop-ui.dll` (UI), `spark-windows-media.dll` (SWM), `mediasession.dll` (MS), `wrtp.dll`.
  * The Locus DTO key table sits at file offset ~0xBE9_xxxx of the UI DLL. It lists the JSON keys the client (de)serialises: `deviceCapabilities`, `respOnlySdp`, `allowMultiDevice`, `isUsingHMS`, `supportsNativeLobby`, `keepAliveUrl`, `keepAliveSecs`, `mediaId`, `usingResource`, `callPreferences`, `requestedMedia`, `seq`, `holding`, `serverComposed`, `mediaShares`, `suggestedMedia`, `mediaConnections`, `floorType`, `endMeetingReason`.
* **Earlier offline analysis.** `checks/call-flow-spec.md`, `checks/protocol_coverage.md` and `docs/calling-gap.md`.
* **Official client logs.** The client's own media logs (`%LOCALAPPDATA%\CiscoSpark\Logs\media`, copied read-only) contain **no SDP of a real cloud call**. The Opus values come from the drone-session offer in `call-flow-spec.md` §4.

Confidence marks in the evidence column:

* **D** = decompiled function
* **S** = string at that address (the behaviour is inferred from it)
* **L** = seen in the official client's logs (`call-flow-spec.md`)
* **U** = **not confirmed in the decompile**: the shape is webex-js-sdk's or a guess

Impact means the effect on a real call: high / med / low.

---

## 1. Outgoing and incoming setup

| # | Official behaviour (evidence) | Webox now | Verdict | Impact | Fix |
|---|---|---|---|---|---|
| 1.1 | Local offer first (WME `CreateOffer`), then the join with the ROAP OFFER inside. `<join_call>: roap flow, callId [` UI 0xbd92018, used in FUN_184b07556 (D-fragment); `<join_call>: normal flow` 0xbd91fb0 is the medialess variant (S) | `session.rs:174` (dial), `:189` (answer): `prepare_media` before any request | match | – | – |
| 1.2 | Outgoing call is `POST {locus}/loci/call` (`/loci/call` UI 0xbf16f68, S) | `session.rs:178` `/loci/call?alternateRedirect=true` | match (the `alternateRedirect` query is **U**) | low | – |
| 1.3 | `device:{url, deviceType, countryCode, regionCode}` (L, the official client's own bodies); deviceType `WINDOWS` | `session.rs:103` `{url, deviceType}`; the type comes from the registration, `session.rs:146` `DESKTOP` | differs | low | add `countryCode` / `regionCode` from the WDM registration answer; keep `DESKTOP` unless a test shows Locus treats `WINDOWS` differently |
| 1.4 | One `correlationId` per call, logged with every `update_media` (`], correlationId [` UI 0xbd92000, S) | new uuid per join (`session.rs:176`), and a **new** one again for leave (`session.rs:468`), alert (`:640`) and DTMF (`:90`) | differs (U: whether the official client reuses it for leave/alert) | low | keep `c.correlation_id` for leave, alert and media requests |
| 1.5 | `respOnlySdp`, `allowMultiDevice`, `supportsNativeLobby` (DTO key table, S) | `session.rs:686-691` sends all three, plus `usingResource:null`, `moveMediaToResource:false`, `ensureConversation:false` | match (`supportsNativeLobby` sent as `1`; the official type is U) | – | – |
| 1.6 | `clientMediaPreferences{preferTranscoding, joinCookie, ipver}`. The join cookie is the calliope reachability result: `[reachability] join meeting with report-version:1 reachability:` UI 0xbe977d0, `event.reachability.joinCookie` 0xbe88eb0, `[reachability] trigger=just-in-time` 0xbd413e0 (S, L) | `session.rs:689` `{preferTranscoding:true}` only; no calliope discovery anywhere | **missing** | med (Locus picks the media cluster blind; worse cluster = more latency / loss) | Batch 4: calliope `POST /discovery/v1/clusters`, STUN to UDP 5004 of each cluster, send `joinCookie` + `ipver:4` |
| 1.7 | `deviceCapabilities` in the join (DTO key, S; contents U) | not sent | missing (U) | low | only after a trace shows the content |
| 1.8 | `localSdp` = JSON string `{roapMessage:{messageType:OFFER, version:"2", seq, tieBreaker, sdps:[sdp]}, audioMuted, videoMuted}` (`, tieBreaker: ` UI 0xbe984f8, S; L) | `mod.rs:506-513`: seq 1, tieBreaker 4294967294, both flags | match | – | – |
| 1.9 | Incoming: within ~80 ms `PUT {locusUrl}/participant/alert {correlationId, device, sequence}` (L) | `session.rs:640-641` `{device, correlationId}`, **no `sequence`** | differs | low | add `sequence` from the locus that rang |
| 1.10 | Before answering: `HEAD https://locus-…` "About to warm up host" (L) | none | missing | low | optional: warm the TLS connection while ringing |
| 1.11 | Answer = `POST {locusUrl}/participant` with the OFFER (L) | `session.rs:193` | match | – | – |
| 1.12 | Decline = `PUT …/participant/decline` (L; body U) | `session.rs:466` `{device}` | match (body U) | – | – |
| 1.13 | Answered on another device stops ringing (`joinedOnThisDevice`, S) | `session.rs:489-493` | match | – | – |
| 1.14 | Inline remote SDP in the join answer is ignored when ROAP is on: `roap is enabled, ignore remote sdp` UI 0xbe0c3f0 (S) | `session.rs:272` applies `mediaConnections[].remoteSdp` if it holds a ROAP message | differs | low | ignore inline remote SDP; the answer comes by `locus.message.roap` |
| 1.15 | No ROAP OK without mediaId: `media id is null, don't send roap ok` UI 0xbe0c528 (S, ref in FUN_185008910) | `mod.rs:524-528`: top level, else `self.devices[].mediaConnections[]`; `session.rs:280` refuses without it | match | – | – |
| 1.16 | Second incoming call while busy: "Hold and Answer", "End Call and Accept" (S, `calling-gap.md` B10) | one `calls.cur` slot; a new incoming locus is ignored (`session.rs:631-642`) | missing | med (caller hears ringing until timeout) | Batch 6 |

## 2. The SDP offer

| # | Official behaviour (evidence) | Webox now | Verdict | Impact | Fix |
|---|---|---|---|---|---|
| 2.1 | Four m-lines once, in this order: audio/main, video/main, audio/slides, video/slides (SWM `initializeMedia` 0x1fa190 → builder 0x2051f0, D; PR #49). `audio-slide` config MS 0x438a48 (S) | `media.rs:562-565`, `:682-709` | match | – | – |
| 2.2 | Lines not in use are offered **inactive**. Turning video or share on is a direction re-OFFER (`Connection::renegotiateSdp` SWM 0x205470 "Update media tracks. old media type = … new media type =", D). Locus showed video/main and both slides lines `inactive` in an audio call (L) | video/main and video/slides are **sendrecv from the first offer**; only audio/slides is inactive (`media.rs:586-589`, `:695`, `:703`) | differs (deliberate in #49) | med (the server may reserve video bandwidth or encoders for every call; the far end's UI may show "video on" from `videoStatus`) | Batch 3: either follow the official model (inactive, then a direction re-OFFER on camera / share) or confirm in a live trace that Locus' `videoStatus` follows `videoMuted` only |
| 2.3 | Opus **PT 101** `maxplaybackrate=48000;maxaveragebitrate=64000;stereo=1;sprop-stereo=0`, session `b=TIAS:64000` (L drone offer; MS keys `opus maxaveragebitrate` 0x477c48, `opus sprop-stereo` 0x477bb8, S) | PT 111 `minptime=10;useinbandfec=1` (`media.rs:129-134`), no TIAS | differs | low (transcoded) | Batch 3: copy the official fmtp and TIAS; keep PT 111 or move to 101 (the server answers in the offer's numbers) |
| 2.4 | Audio fallbacks G722, iLBC, PCMA, G729 annexb, G7221 24k/32k, PCMU, CN (L) | Opus only | missing | low | not needed for cloud calls; skip |
| 2.5 | `telephone-event/8000` PT 100 `0-15` (L; MS `telephone-event` 0x4786a8) | not offered; DTMF goes by `POST {locus}/sendDtmf` (`session.rs:85-93`) | missing | low | Batch 3: offer telephone-event (cheap); keep `/sendDtmf` until a trace shows which path the official client uses |
| 2.6 | Cisco FEC `x-ulpfecuc` and RTX `mari-rtx` / `rtx-time` (MS 0x478690, 0x478680, 0x478d60; "require RTX enabled for enabling hybrid resilience" 0x4780d8, S) | none (webrtc-rs registers no RTX/FEC, `media.rs:463-465`) | missing | med (no loss recovery on lossy Wi-Fi beyond NACK) | Batch 8 |
| 2.7 | H.264 fmtp with `profile-level-id`, `packetization-mode`, `level-asymmetry-allowed`, `max-mbps`, `max-fs`, `max-fps`, `max-br`, `max-dpb` (MS 0x477ac0, 0x477ad8, 0x477a60, 0x435780, 0x4357a8, 0x4357a0, 0x4357b4, 0x477a58; builders FUN_1800a0240, FUN_1800a064d, FUN_1800a10e0, FUN_1800a4283, D-fragment). **Numeric defaults not confirmed** | `42e028;packetization-mode=1;level-asymmetry-allowed=1`, no `max-*` (`media.rs:141-147`) | differs | med (the server may pick the level-4.0 maximum or a low default; one profile for camera and share) | Batch 3: add `max-fs`, `max-mbps`, `max-fps`, `max-br` matching what Webox encodes; per-line values (camera vs share) |
| 2.8 | AV1 offered (`AddAV1Codec`, `libav1enc.dll`, MS 0x43ed70, S) | not offered | missing | low | skip |
| 2.9 | Multistream: `sprop-source`, `sprop-simul`, `x-ssrc-range`, `a=label`, CSI per line, `a=rid` simulcast (MS 0x478f70, 0x478c30, 0x478498, 0x4789a0, S; the server answer carries them, L) | none; one stream per direction relying on `preferTranscoding` (`media.rs:21`) | missing | low for 1:1, **med–high for group / space meetings** (no per-participant video, no active-speaker layout) | Batch 8 |
| 2.10 | RTP header extensions: `ssrc-audio-level`, `toffset`, `abs-send-time`, `abs-capture-time`, transport-wide-cc, `video-orientation`, Cisco `timestamp#100us`, `virtualid`, `namedmediagroup`, `framemarking`, `priority` (`wrtp.dll` 0x18bcf0…0x18c080, S) | only transport-cc from webrtc-rs' default interceptors (`media.rs:465`) | differs | low (med for group calls: no audio level for active speaker) | Batch 3: register `ssrc-audio-level`, `abs-send-time`, `toffset` |
| 2.11 | rtcp-fb `* nack`, `nack pli`, `ccm fir`, `ccm tmmbr`, `ccm cisco-e2ed`, `ccm cisco-scr[-v2..v5]`, transport-cc (MS 0x43a328–0x43a3a0, 0x478270, S) | `nack`, `nack pli`, `ccm fir` (`media.rs:146`) plus transport-cc | differs | low | Batch 3: add `ccm tmmbr`; Cisco SCR/e2ed only with multistream |
| 2.12 | `a=rtcp-xr:rcvr-rtt=all voip-metrics` (MS 0x433248, ref in FUN_1800cb7a8, D-fragment), `cisco-mari-hybrid-resilience` (L) | none | missing | low | skip |
| 2.13 | BUNDLE + rtcp-mux (MS `BUNDLE` 0x4780a4, `rtcp-mux` 0x4789a8, S; answer `a=group:BUNDLE 0 1 2 3`, L) | webrtc-rs max-bundle + rtcp-mux | match | – | – |
| 2.14 | `a=ice-options:cisco+tls+fqdn` (MS 0x43a318, S; L) | not sent | missing | low (only matters together with TLS, see 4.2) | with Batch 2 |
| 2.15 | DTLS-SRTP, `setup:actpass` + fingerprint (`IsDtlsSRTPEnabled`, MS `actpass` 0x4784e0, S) | webrtc-rs | match | – | – |
| 2.16 | `a=content:main` / `a=content:slides` labels (L) | `media.rs:152-164` | match | – | – |
| 2.17 | All candidates in the offer, no trickle (L) | `media.rs:572-581` waits for gathering | match | – | – |

## 3. ROAP

| # | Official behaviour (evidence) | Webox now | Verdict | Impact | Fix |
|---|---|---|---|---|---|
| 3.1 | OFFER seq 1 in the join (L) | `session.rs:689` | match | – | – |
| 3.2 | ANSWER as Mercury `locus.message.roap` (UI 0xbe98758; `<ROAP_EVENT>: roap event arrived` 0xbe985d0, S) | `mod.rs:516-520`, `session.rs:545-553` (filtered by correlationId) | match | – | – |
| 3.3 | OK with the same seq on `PUT {participant}/media` with mediaId (`ROAP_STATE_WAIT_TX_OK` UI 0xbf6e4c8, S; L) | `session.rs:277-284`, `:305` | match | – | – |
| 3.4 | No OK without mediaId (1.15) | `session.rs:280` | match | – | – |
| 3.5 | A repeated ANSWER is answered with OK again (libroap state machine, S) | `session.rs:293-298` | match | – | – |
| 3.6 | Server OFFER → client ANSWER with the server's seq → wait for the server's OK (`ROAP_STATE_WAIT_RX_OK` 0xbf6e3c8, `ReceiveOkCallback` 0xbf6e198, S) | ANSWER sent (`session.rs:320-336`); the OK is ignored, so no "waiting for OK" state | differs | low | track WAIT_RX_OK; no own OFFER while it is open |
| 3.7 | `OFFER_REQUEST`: the server asks the client for an offer (`TransmitCallback(OFFERREQUEST)` 0xbf6e2d0, `ROAP_STATE_WAIT_TX_OFFER` 0xbf6e418, S) | ignored (`session.rs:346` `_ => {}`) | missing | med (a server-driven renegotiation would stall the media leg) | Batch 5: answer OFFER_REQUEST with a fresh OFFER (seq+1) |
| 3.8 | ERROR goes to `ReceiveErrorCallback` (0xbf6e270, ref in FUN_18634dc70), it does not hang up (S) | `session.rs:338-345`, the call is kept | match | – | – |
| 3.9 | Glare resolution with tieBreaker: "glare we lose", "glare everyone loses", "missing tieBreaker field" (UI 0xc0e8d70, S) | fixed maximum tieBreaker, no glare logic | differs | low (Webox rarely offers mid-call) | Batch 5: random tieBreaker per OFFER and the libroap comparison |
| 3.10 | Error types `CONFLICT`, `DOUBLE_CONFLICT`, `NOMATCH`, `OUT_OF_ORDER`, … (UI 0xdbbcdb8, 0xdbbcc58, 0xdbbcd18, 0xdbbcd58, S) | always `errorType:"FAILED"` (`mod.rs:511`) | differs | low | Batch 5: OUT_OF_ORDER for an old seq, CONFLICT for glare |
| 3.11 | Retransmit / timeout of an unanswered OFFER (libroap; timer values **U**) | 10 s `ROAP_TIMEOUT`, then a user notice, no retransmit (`mod.rs:477`, `:483`) | unconfirmed | low | confirm libroap timers first |
| 3.12 | Mid-call direction re-OFFER on video / share (`<update_media>: roap flow` 0xbd8ef50 in FUN_184b1cfb6 and FUN_184b846a1, D-fragment) | never; the only re-OFFER is the ICE restart (`session.rs:392-400`) | differs (follows from 2.2) | med | Batch 3, together with 2.2 |

## 4. ICE, DTLS, transport

| # | Official behaviour (evidence) | Webox now | Verdict | Impact | Fix |
|---|---|---|---|---|---|
| 4.1 | Host candidates on up to 8 local IPs ("we only support at most 8 IP addresses as candidate" MS 0x42b100, S) | one UDP socket on the routed interface (`media.rs:272-289`, `ice_socket.rs`) | differs | low | fine as is (the route IP is the one that works) |
| 4.2 | UDP, then TCP 5004, then TLS 443: `collectCandidatesProtocolFirst UDP/TCP/TLS` MS 0x42ac30/70/b0, `ICELIB_addLocalCandidate TCP:/TLS:` 0x42abc8/0x42ac10, `EnableUDPFallbackTCP` SWM 0x4d7710, `EnhanceUDPSwitchTCP` 0x4d7fd8, `ConnectionState::TransportSwitched` SWM 0x52fa20 (S) | UDP only; the server's TCP / xTLS candidates are dropped (`sdp.rs:75-84`, `media.rs:879`) | **missing** | **high** on networks that block UDP (hotel / guest Wi-Fi, strict corporate firewalls): no audio at all | Batch 2 |
| 4.3 | TLS through an HTTP proxy (`DisableTLSProxy` MS 0x42da38 in FUN_18005f629, `CCmConnectorHttpProxyT`, S) | none | missing | med (proxied enterprise networks) | Batch 2 |
| 4.4 | NoListen sockets for cloud calls: `SetICENoListen(true)` SWM 0x529b78 (S) | unconnected UDP socket on one IP | differs | low | optional: connect the socket to the selected pair |
| 4.5 | Server is ICE lite, the client is controlling (L) | webrtc-rs controlling | match | – | – |
| 4.6 | DTLS-SRTP (S, 2.15) | webrtc-rs | match | – | – |
| 4.7 | **Media recovery.** On ICE failure: "time to reconnect media for ICE Failure during meeting" → `remove media connection [` → new connection. Capped at **5** (`if (*(ulong*)(call+0x2818) < 5)`), then "Media fail to reconnect after Terminated for 5 times, end meeting/call and report media drop events". Offline: "Network is offline, wait for network restored, before triggering media reconnect"; on hold: "Call held. do not attempt to reconnect media". All in FUN_184ef4ff4 (UI RVA 0x4ef4ff4, D; strings 0xbde59f8, 0xbde6330, 0xbde5990, 0xbde5930, 0xbde5870). WME also reconnects itself: `RtcpBasedReconnect`, `ReconnectByEvent`, `LogKeepAliveReconnect` (SWM/MS, S) | ICE restart on the **same** peer connection after 10 s down, every 15 s, **never gives up** (`session.rs:363-421`, `mod.rs:463-469`); no offline wait | differs | **high** (if Webex media servers don't honour an ICE-restart OFFER, which is **unconfirmed**, Webox shows "Reconnecting…" forever instead of recovering or ending) | Batch 1 |
| 4.8 | Network change triggers reconnect and reachability: `network/ip-address-changed` UI 0xbdea698, `[reachability] retry reachability for reason: LocalNetworkChanged` 0xbde8120, `FastReconnectOnNwChg` SWM 0x4d7ff0 (S) | no network-change hook; waits until ICE notices | missing | med–high (Wi-Fi ↔ LAN, VPN on/off: 10–60 s of silence) | Batch 1 |
| 4.9 | Locus keep-alive `GET keepAliveUrl` every `(keepAliveSecs-1)*330 ms` while joined without media (`createKeepAliveTimer` 0x184b5d990, D; PR #49) | `session.rs:438-454`, `mod.rs:538-554` | match | – | – |
| 4.10 | Reachability (calliope cluster discovery, STUN UDP 5004) (L, 1.6) | none | missing | med | Batch 4 |
| 4.11 | No-media timers `setupNoMediaTimers`, `onNoMedia` (SWM 0x536428), `CheckSessionInactive` (MS 0x43d540): network warning in the UI (S) | `media_stall` notices (`mod.rs:481-489`), "Reconnecting…" after 5 s (`session.rs:522-528`) | differs (similar idea, different timers) | low | align once the official timer values are known (U) |

## 5. Mute, camera, camera switch

| # | Official behaviour (evidence) | Webox now | Verdict | Impact | Fix |
|---|---|---|---|---|---|
| 5.1 | Mute = `PUT {participant}/media` without ROAP ("`<update_media>: normal flow, callId [`" UI 0xbd91030 in FUN_184b84902, D-fragment), `localSdp {audioMuted, videoMuted}` | `session.rs:49`, body `mod.rs:231-235` | match | – | – |
| 5.2 | Muted by someone else → mute locally, never auto-unmute ("muteAudio() when RemoteMuted and LocalAudioUnmuted" UI 0xbada730, S) | `mod.rs:222-226`, `session.rs:70-73` | match | – | – |
| 5.3 | Unmute after being muted by others | `PATCH {self.url}/controls {audio:{muted:false}}` (`session.rs:41`, `mod.rs:238`) | unconfirmed (webex-js-sdk shape) | low | confirm in a trace |
| 5.4 | Camera on/off: `Connection::muteVideo()` SWM/UI 0xbfeac50, "Video track not ready when muteVideo" 0xbfeac28; direction changes go through `renegotiateSdp` (2.2) | only the `videoMuted` flag on an always-sendrecv line (`video.rs:152-162`) | differs | med (see 2.2) | Batch 3 |
| 5.5 | Camera switch without renegotiation: `Connection::restartVideoTracks()` 0xbff23b8 (S) | `video.rs:101-104`, a new feed on the same line | match | – | – |
| 5.6 | Video call button and "Answer calls with my video on" (`calling-gap.md` A2/B3) | the camera only after connect; no video dial or answer (`video.rs:86`) | missing | med | Batch 3 (UI + `video_want` before `Dial`/`Answer`) |
| 5.7 | Step away: `Connection::muteVideoStepAway()` 0xbfeac80 (S) | none | missing | low | later |

## 6. Share

| # | Official behaviour (evidence) | Webox now | Verdict | Impact | Fix |
|---|---|---|---|---|---|
| 6.1 | "Connect on spark device before requesting floor" UI 0xbd65258 (S) | `share.rs:84-89` | match | – | – |
| 6.2 | Floor request: "Share perform floor request" 0xbd65340 (ref in FUN_1849aafb8), `call_floorRequest` 0xbda17f0; DTO keys `floorType`, `beneficiary`, `disposition` (S). **The request body is not visible in the decompile (U)** | `PUT {mediaShares[content].url}` `floor{disposition:GRANTED, requester{url}, beneficiary{url, devices[{deviceType,url}]}}` (`share.rs:68-71`, `:190`), no `floorType` | unconfirmed / differs | med (a wrong body means no share) | Batch 7: capture one official floor request (or confirm on a live Webox share, which the #20 work did for 1:1) and add `floorType` |
| 6.3 | Granted with no Mercury DTO in time is treated as granted ("Send floor granted notification event due to not receive floor granted from mercury dto", S) | `share.rs:164-167` (10 s) | match | – | – |
| 6.4 | Release tolerates 404 / 409 ("Skip 409 conflict failure caused by floor release request", S) | `share.rs:125-131` | match | – | – |
| 6.5 | video/slides inactive until sharing, then a direction re-OFFER (2.2; "whiteboard is about to start, delay modify media until whiteboard floor granted" 0xbd64820, S) | sendrecv from the join (`media.rs:703`) | differs | med | Batch 3 |
| 6.6 | Share type and optimisation: `SHARE_DESKTOP` / `SHARE_APPLICATION` …, `shareOptimizeForText` / `…Video` (normal vs HFPS profile) (S, `call-flow-spec.md` §6) | one H.264 profile; fps from the picker (`share.rs:110`) | missing | low | Batch 7 |
| 6.7 | Computer audio on audio/slides (`enableShareAudio`, S) | audio/slides always inactive (`media.rs:588`) | missing | low | Batch 7 |
| 6.8 | Receive: floor GRANTED to another participant → subscribe to the slides CSI (`client.media.share.csi.changed` UI, S) | floor read from the locus (`share.rs:228-253`, `share_rx.rs:66-81`); pictures on video/slides; undeclared SSRC declared locally (`media.rs:758-771`) | differs | med (works in the loopback tests; live with Webex media servers **unconfirmed**) | verify live with the #49 call log; CSI handling comes with multistream (Batch 8) |

## 7. Mid-call

| # | Official behaviour (evidence) | Webox now | Verdict | Impact | Fix |
|---|---|---|---|---|---|
| 7.1 | Hold / resume of a Locus call: `[holdCall]` UI 0xbf6acb0, `[resumeCall]` 0xbf6acc0, `holdCallAttempt` / `…Success`, flag `desktop-locus-hold-resume-enabled` 0xc037440, DTO key `holding` (S). Request shape **U** | none (shortcut stub) | missing | med | Batch 6 |
| 7.2 | Transfer, blind and consult ("About to blind transfer call…" 0xbd45e48, S); mostly Webex Calling | none | missing | low (WxC) | out of scope |
| 7.3 | Add participant `call_addParticipant` (S) | `PUT {locus}/participant {invitees, alertIfActive}` (`controls.rs:111`) | unconfirmed (webex-js-sdk shape) | low | confirm in a trace |
| 7.4 | Escalate 1:1 to a meeting (`escalateToMeeting`, S) | none | missing | med | later (needs meetings, A8) |
| 7.5 | Roster with mute, hand, role (S) | `controls.rs:72-87` | match | – | – |
| 7.6 | Mute others, raise / lower hand, remove, end for all (S) | `controls.rs:105-114` | unconfirmed (webex-js-sdk shapes) | low | confirm in a trace |
| 7.7 | Device change mid-call ("device restart audio track reset timer", SWM 0x5489f0, S) | `session.rs:11-14` reopens the devices | match | – | – |
| 7.8 | Locus resync after a Mercury reconnect (`client.locus.sync.complete`, `locus.sync_*`, S) | delta merge with a GET on desync (`mod.rs:129-140`, `session.rs:582-591`) and a 15 / 30 s poll | differs | low | on a Mercury reconnect during a call: one immediate locus GET |
| 7.9 | Move media / call pull (`client.call.move-media`, `moveMediaToResource`, S) | always `moveMediaToResource:false` | missing | low | later |
| 7.10 | Call waiting (1.16) | one call slot | missing | med | Batch 6 |

Network change and reconnect: see 4.7 and 4.8.

## 8. End and cleanup

| # | Official behaviour (evidence) | Webox now | Verdict | Impact | Fix |
|---|---|---|---|---|---|
| 8.1 | Leave: `PUT …/participant/{id}/leave` (`/leave` UI 0xbf1ef4c, S; `client.locus.leave.request` / `.response`, S). Body **U**; the DTO has `usingResource`, `endMeetingReason` | `{device, usingResource:null, correlationId:<new uuid>}` (`session.rs:468`) | differs (1.4) | low | the call's correlationId |
| 8.2 | Local media stops whatever the server answers | `session.rs:464` (`end_call` before the request) | match | – | – |
| 8.3 | 1:1 ends when the other side declined or left (`shouldDisconnectOne2OneCall`, S) | `mod.rs:284-307`, with an 8 s devices-gone grace (`session.rs:500-510`) | match | – | – |
| 8.4 | Leave reason (`endMeetingReason`, U) | none | unconfirmed | low | – |
| 8.5 | Ends the call after 5 failed media reconnects and reports a media drop (4.7, D) | never ends on media loss | missing | high (the call can hang without audio) | Batch 1 |
| 8.6 | Floor released when the sharer leaves (`floor_release` 0xbd62fd0, S; behaviour on leave U) | `end_call` stops the capture without a `RELEASED` request (`session.rs:127`) | differs | low (Locus releases the floor with the leave) | release the floor before the leave if sharing |

## 9. Stats and metrics the official client reports (for reference; Webox needs none)

None of these are sent by Webox. `call.log` (PR #49) and `media_stats` are local only.

* **Call Analyzer / diagnostics events** (UI strings, `client.*`). Each event is one row of the server-side call timeline:
  * Call lifecycle: `client.call.initiated`, `.created`, `.prepare`, `.displayed`, `.declined`, `.aborted`, `.leave`, `.remote-started`, `.remote-ended`, `.move-media`, `.fallback`, `.diagnostics`, `.skip-locus-join`, `client.alert.displayed` / `.removed`.
  * Locus requests: `client.locus.join.request` / `.response`, `client.locus.media.request` / `.response`, `client.locus.leave.request` / `.response`, `client.locus.decline.request`, `client.locus.sync.complete`.
  * ROAP and media engine: `client.roap-message.received`, `client.media-engine.local-sdp-generate[d]`, `.remote-sdp-received`, `.ready`, `.lost`, `.crash`.
  * ICE and media: `client.ice.start` / `.end`, `client.media.reachability`, `.capabilities`, `.rx.start` / `.stop`, `.tx.start` / `.stop`, `.render.start` / `.stop`, `.reconnecting`, `.recovered`, `client.media-status`.
  * Share: `client.share.initiated`, `.floor-grant.request`, `.floor-granted.local`, `.stopped`, `.paused`, `client.media.share.csi.changed`.
  * Network: `client.network.changed` / `.lost` / `.restored`, `client.mercury.connection.*`.
* **Media quality (MQE).** `client.mediaquality.event` with interval metadata, from `EnableGetMediaStateMQEMetrics[V2]`, `MediaFlowStateMQE` and `MediaFlowMQEMeeting`.
  * Per-stream counters in MS include `rtxPackets`, `rtxBitrate`, `rtxRecovered` and the Opus DRED counters.
  * The counters are sent to the metrics service (`/clientmetrics`, `/clientmetrics-prelogin` in the UI strings).
* **Reachability report.** `event.reachability.*` (joinCookie used, first STUN test time).
* **Media status diagnostics.** "About to send media status diagnostics", `[media metrics] SetAudioSendEvent`.

---

## Not confirmed in the decompile

These need a live trace of the official client, or a successful live Webox call that logs the server's reaction. Until then they stay "unconfirmed":

1. The exact JSON bodies of the floor request and release, leave, decline and alert.
   * The DTO key table only proves which keys exist.
   * Whether leave, alert and DTMF reuse the call's `correlationId`.
2. The roster controls (mute others, hand, remove, invite, end for all) and unmute-after-remote-mute. All are webex-js-sdk shapes.
3. The hold / resume request.
4. Numeric H.264 fmtp defaults (`max-fs`, `max-mbps`, `max-fps`, `max-br`) and the payload types of a real cloud-call offer.
   * The Opus values come from a local drone session only.
   * The official client's logs contain no cloud-call SDP.
5. libroap retransmit and timeout timers.
6. Whether Webex media servers (Homer/Linus) answer an **ICE-restart** OFFER. Webox's whole recovery path (4.7) depends on this.
7. The `deviceCapabilities` content, the type of `supportsNativeLobby`, and the `alternateRedirect` query.
8. Whether the official client sends silence or stops the audio track while muted.
9. What Locus does with Webox's sendrecv video lines when `videoMuted` is true (2.2): whether `videoStatus` shows SENDRECV to the other side.

The UI DLL's Ghidra project is not auto-analysed, so the cited `FUN_…` decompiles are fragments around the string reference.

* The facts taken from them are only the string-to-function links and the one counter check (`< 5` reconnects in FUN_184ef4ff4).
* For exact bodies, run full analysis on that DLL (several hours) and follow the DTO serialiser calls.

## Prioritised fix list (PR-sized batches, highest real-call impact first)

**Batch 1: media recovery like the official client** (4.7, 4.8, 8.5). High.

1. First confirm, from `call.log` on a real call with a forced network drop, whether the server answers the ICE-restart OFFER (ANSWER vs ERROR / nothing).
2. If it does not: implement the official "reconnect media".
   * Close the peer connection and build a new `Media` (new ICE + DTLS).
   * Send a new ROAP OFFER (seq+1) on `PUT {participant}/media`.
   * Handle the ANSWER / OK as at join.
3. In both cases:
   * Cap at 5 attempts. Then leave and end the call with a "media lost" reason.
   * Wait while the network is offline. Skip while on hold.
4. A network-change hook: re-check `route_ip` every tick. If the local IP changed, reconnect now instead of waiting for ICE.

**Batch 2: TCP and TLS-443 media fallback** (4.2, 4.3, 2.14). High on restrictive networks.

* webrtc-rs has no ICE-TCP or TLS. This needs either:
  * our own TCP / TLS transport with RFC 4571 framing, behind the existing `ice_socket` mux; or
  * a media stack that has it.
* Order UDP, then TCP 5004, then TLS 443. Add HTTP CONNECT for proxied networks, and `a=ice-options:cisco+tls+fqdn`.

**Batch 3: SDP and direction fidelity** (2.2, 2.3, 2.5, 2.7, 2.10, 2.11, 3.12, 5.4, 5.6, 6.5). Med.

* Decide on 2.2 with a live trace (open item 9). If it goes the official way: video/main and video/slides inactive at join, a direction re-OFFER for camera on / share start, an answer that keeps the lines.
* Codec and attribute details:
  * Opus fmtp as official and `b=TIAS:64000`; telephone-event.
  * H.264 `max-*`.
  * Extmaps `ssrc-audio-level`, `abs-send-time`, `toffset`; `ccm tmmbr`.
* Video call / answer with video in the UI.

**Batch 4: join body and reachability** (1.3, 1.4, 1.6, 1.9, 1.14, 4.10, 8.1). Med.

* Calliope discovery and STUN test, then `joinCookie` + `ipver` in `clientMediaPreferences`.
* `countryCode` / `regionCode` in `device`.
* One correlationId per call for leave, alert and media.
* `sequence` in the alert body.
* Ignore inline `remoteSdp` while ROAP is on.

**Batch 5: ROAP robustness** (3.6, 3.7, 3.9, 3.10). Med–low.

* Answer OFFER_REQUEST with an OFFER.
* A WAIT_RX_OK state.
* A random tieBreaker and glare rules.
* OUT_OF_ORDER / CONFLICT error types.

**Batch 6: hold / resume and call waiting** (7.1, 1.16, 7.10). Med.

* First capture the official hold request (open item 3).
* Then add a second call slot with "hold and answer" / "end and answer".

**Batch 7: share fidelity** (6.2, 6.6, 6.7, 8.6). Med–low.

* Confirm the floor body and add `floorType`.
* Share type and optimise-for-text/video hints.
* Computer audio on audio/slides.
* Release the floor before leaving.

**Batch 8: resilience codecs and multistream** (2.6, 2.9, 6.8). Med; high for group meetings.

* `mari-rtx` / `x-ulpfecuc`.
* `sprop-source` / `sprop-simul` / CSI subscription for per-participant video and active speaker in space meetings.

### Top 10 fixes by real-call impact

1. Media reconnect the official way: new connection, at most 5 tries, then end (Batch 1).
2. Reconnect immediately when the local network or IP changes (Batch 1).
3. ICE over TCP 5004 and TLS 443 (Batch 2).
4. TLS media through an HTTP proxy (Batch 2).
5. Calliope reachability and `joinCookie` in the join (Batch 4).
6. Line directions: settle sendrecv vs inactive plus re-OFFER for camera and share (Batch 3).
7. H.264 `max-fs` / `max-mbps` / `max-fps` / `max-br` and per-line profiles (Batch 3).
8. Handle ROAP `OFFER_REQUEST` (Batch 5).
9. Hold / resume and call waiting (Batch 6).
10. RTX / FEC for lossy networks (Batch 8).
