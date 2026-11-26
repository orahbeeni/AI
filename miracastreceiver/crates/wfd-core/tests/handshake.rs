//! Full Wi-Fi Display handshake transcripts against the sink state machine.

use std::time::{Duration, Instant};

use wfd_core::audio::AudioFormat;
use wfd_core::params;
use wfd_core::rtsp::{Decoder, Message};
use wfd_core::session::{Output, SessionEvent, SinkConfig, SinkSession, SinkState};
use wfd_core::source::{SourceConfig, SourceEvent, SourceOutput, SourceSession, SourceState};
use wfd_core::uibc::InputType;

fn parse(raw: &str) -> Message {
    let mut d = Decoder::new();
    d.feed(raw.as_bytes());
    d.next_message().unwrap().expect("complete message")
}

fn sent(outs: &[Output]) -> Vec<&Message> {
    outs.iter().filter_map(|o| if let Output::Send(m) = o { Some(m) } else { None }).collect()
}

fn events(outs: &[Output]) -> Vec<&SessionEvent> {
    outs.iter().filter_map(|o| if let Output::Event(e) = o { Some(e) } else { None }).collect()
}

fn text(m: &Message) -> String {
    String::from_utf8(m.to_bytes()).unwrap()
}

fn body_len(s: &str) -> usize {
    s.len()
}

/// Byte-level transcript modelled on an Android source (Samsung Smart View
/// sends the same shape of messages).
#[test]
fn android_style_transcript_m1_to_m16() {
    let t0 = Instant::now();
    let mut cfg = SinkConfig::new("Living Room PC", 19000);
    cfg.source_host = "192.168.173.80".into();
    let mut sink = SinkSession::new(cfg, t0);

    // M1: source OPTIONS
    let outs = sink.handle(parse("OPTIONS * RTSP/1.0\r\nDate: Thu, 26 Nov 2026 10:00:00 +0000\r\nServer: stagefright/1.2 (Linux;Android 14)\r\nCSeq: 1\r\nRequire: org.wfa.wfd1.0\r\n\r\n"), t0);
    let s = sent(&outs);
    assert_eq!(s.len(), 2, "M1 reply + M2 request");
    assert_eq!(text(s[0]), "RTSP/1.0 200 OK\r\nCSeq: 1\r\nPublic: org.wfa.wfd1.0, GET_PARAMETER, SET_PARAMETER\r\n\r\n");
    assert_eq!(text(s[1]), "OPTIONS * RTSP/1.0\r\nCSeq: 1\r\nRequire: org.wfa.wfd1.0\r\n\r\n");
    assert_eq!(sink.state(), SinkState::Capabilities);

    // M2 reply from source
    let outs = sink.handle(
        parse("RTSP/1.0 200 OK\r\nCSeq: 1\r\nPublic: org.wfa.wfd1.0, SETUP, TEARDOWN, PLAY, PAUSE, GET_PARAMETER, SET_PARAMETER\r\n\r\n"),
        t0,
    );
    assert!(outs.is_empty());

    // M3: capability query, including parameters we don't know
    let m3_body = "wfd_content_protection\r\nwfd_video_formats\r\nwfd_audio_codecs\r\nwfd_client_rtp_ports\r\nwfd_uibc_capability\r\nwfd_vnd_sec_max_buffer_length\r\nmicrosoft_cursor\r\nwfd_idr_request_capability\r\n";
    let m3 = format!(
        "GET_PARAMETER rtsp://localhost/wfd1.0 RTSP/1.0\r\nDate: Thu, 26 Nov 2026 10:00:00 +0000\r\nServer: stagefright/1.2 (Linux;Android 14)\r\nCSeq: 2\r\nContent-Type: text/parameters\r\nContent-Length: {}\r\n\r\n{m3_body}",
        body_len(m3_body)
    );
    let outs = sink.handle(parse(&m3), t0);
    let s = sent(&outs);
    assert_eq!(s.len(), 1);
    let Message::Response(r) = s[0] else { panic!("expected response") };
    assert_eq!(r.status, 200);
    assert_eq!(r.cseq(), Some(2));
    assert_eq!(r.headers.get("Content-Type"), Some("text/parameters"));
    let got = params::parse_params(r.body_str());
    let names: Vec<&str> = got.iter().map(|(k, _)| k.as_str()).collect();
    assert_eq!(
        names,
        vec![
            "wfd_content_protection",
            "wfd_video_formats",
            "wfd_audio_codecs",
            "wfd_client_rtp_ports",
            "wfd_uibc_capability",
            "wfd_idr_request_capability"
        ],
        "answers in request order and leaves unknown parameters out"
    );
    assert_eq!(params::get(&got, "wfd_content_protection"), Some("none"));
    assert!(params::get(&got, "wfd_video_formats").unwrap().starts_with("40 00 03 10 0001bdeb"));
    assert_eq!(params::get(&got, "wfd_audio_codecs"), Some("LPCM 00000003 00, AAC 00000001 00"));
    assert_eq!(params::get(&got, "wfd_client_rtp_ports"), Some("RTP/AVP/UDP;unicast 19000 0 mode=play"));
    assert!(params::get(&got, "wfd_uibc_capability").unwrap().starts_with("input_category_list=GENERIC, HIDC;"));
    assert_eq!(params::get(&got, "wfd_idr_request_capability"), Some("1"));

    // M4: source's selection (1080p30 CHP, AAC, UIBC HIDC on port 7239)
    let m4_body = "wfd_video_formats: 00 00 02 10 00000080 00000000 00000000 00 0000 0000 00 none none\r\n\
wfd_audio_codecs: AAC 00000001 00\r\n\
wfd_presentation_URL: rtsp://192.168.173.80/wfd1.0/streamid=0 none\r\n\
wfd_client_rtp_ports: RTP/AVP/UDP;unicast 19000 0 mode=play\r\n\
wfd_uibc_capability: input_category_list=HIDC;generic_cap_list=none;hidc_cap_list=Keyboard/USB, Mouse/USB;port=7239\r\n\
wfd_uibc_setting: enable\r\n";
    let m4 = format!(
        "SET_PARAMETER rtsp://localhost/wfd1.0 RTSP/1.0\r\nCSeq: 3\r\nContent-Type: text/parameters\r\nContent-Length: {}\r\n\r\n{m4_body}",
        m4_body.len()
    );
    let outs = sink.handle(parse(&m4), t0);
    assert_eq!(text(sent(&outs)[0]), "RTSP/1.0 200 OK\r\nCSeq: 3\r\n\r\n");
    let ev = events(&outs);
    let SessionEvent::Negotiated(n) = ev[0] else { panic!("expected Negotiated, got {ev:?}") };
    assert_eq!(n.video.unwrap().to_string(), "1920x1080p30");
    assert_eq!(n.audio.unwrap().format, AudioFormat::Aac);
    assert_eq!(n.presentation_url.as_deref(), Some("rtsp://192.168.173.80/wfd1.0/streamid=0"));
    let u = n.uibc.as_ref().unwrap();
    assert_eq!(u.port, Some(7239));
    assert!(u.hidc_path(InputType::Mouse).is_some());
    assert!(n.uibc_enabled);
    assert_eq!(sink.state(), SinkState::Negotiated);

    // M5: trigger SETUP -> 200, PrepareMedia, then M6 SETUP
    let m5_body = "wfd_trigger_method: SETUP\r\n";
    let m5 = format!(
        "SET_PARAMETER rtsp://localhost/wfd1.0 RTSP/1.0\r\nCSeq: 4\r\nContent-Type: text/parameters\r\nContent-Length: {}\r\n\r\n{m5_body}",
        m5_body.len()
    );
    let outs = sink.handle(parse(&m5), t0);
    assert_eq!(events(&outs), vec![&SessionEvent::PrepareMedia { rtp_port: 19000 }]);
    let s = sent(&outs);
    assert_eq!(text(s[0]), "RTSP/1.0 200 OK\r\nCSeq: 4\r\n\r\n");
    assert_eq!(
        text(s[1]),
        "SETUP rtsp://192.168.173.80/wfd1.0/streamid=0 RTSP/1.0\r\nCSeq: 2\r\nTransport: RTP/AVP/UDP;unicast;client_port=19000\r\n\r\n"
    );

    // M6 reply -> M7 PLAY with the session id
    let outs = sink.handle(
        parse("RTSP/1.0 200 OK\r\nCSeq: 2\r\nSession: 6B8B4567;timeout=30\r\nTransport: RTP/AVP/UDP;unicast;client_port=19000-19001;server_port=1990-1991\r\n\r\n"),
        t0,
    );
    assert_eq!(text(sent(&outs)[0]), "PLAY rtsp://192.168.173.80/wfd1.0/streamid=0 RTSP/1.0\r\nCSeq: 3\r\nSession: 6B8B4567\r\n\r\n");

    // M7 reply -> playing
    let outs = sink.handle(parse("RTSP/1.0 200 OK\r\nCSeq: 3\r\nSession: 6B8B4567\r\n\r\n"), t0);
    assert_eq!(events(&outs), vec![&SessionEvent::Playing { session_id: "6B8B4567".into() }]);
    assert_eq!(sink.state(), SinkState::Playing);
    // keep-alive deadline = timeout (30 s) + grace
    assert_eq!(sink.deadline(), Some(t0 + Duration::from_secs(45)));

    // M16 keep-alive at t+25s
    let t1 = t0 + Duration::from_secs(25);
    let outs = sink.handle(parse("GET_PARAMETER rtsp://localhost/wfd1.0 RTSP/1.0\r\nCSeq: 5\r\nSession: 6B8B4567\r\n\r\n"), t1);
    assert_eq!(text(sent(&outs)[0]), "RTSP/1.0 200 OK\r\nCSeq: 5\r\nSession: 6B8B4567\r\n\r\n");
    assert!(sink.on_timer(t0 + Duration::from_secs(50)).is_empty(), "keep-alive pushed the deadline out");

    // M13 IDR request
    let outs = sink.request_idr();
    assert_eq!(
        text(sent(&outs)[0]),
        "SET_PARAMETER rtsp://localhost/wfd1.0 RTSP/1.0\r\nCSeq: 4\r\nSession: 6B8B4567\r\nContent-Type: text/parameters\r\nContent-Length: 17\r\n\r\nwfd_idr_request\r\n"
    );
    assert!(sink.handle(parse("RTSP/1.0 200 OK\r\nCSeq: 4\r\n\r\n"), t1).is_empty());

    // Source-triggered teardown: M5 TEARDOWN -> M8
    let body = "wfd_trigger_method: TEARDOWN\r\n";
    let m5t = format!(
        "SET_PARAMETER rtsp://localhost/wfd1.0 RTSP/1.0\r\nCSeq: 6\r\nContent-Type: text/parameters\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    let outs = sink.handle(parse(&m5t), t1);
    let s = sent(&outs);
    assert_eq!(text(s[1]), "TEARDOWN rtsp://192.168.173.80/wfd1.0/streamid=0 RTSP/1.0\r\nCSeq: 5\r\nSession: 6B8B4567\r\n\r\n");
    let outs = sink.handle(parse("RTSP/1.0 200 OK\r\nCSeq: 5\r\n\r\n"), t1);
    assert_eq!(events(&outs), vec![&SessionEvent::Closed { reason: "teardown complete".into() }]);
    assert_eq!(sink.state(), SinkState::Closed);
}

#[test]
fn windows_style_m3_gets_intel_names() {
    let t0 = Instant::now();
    let mut sink = SinkSession::new(SinkConfig::new("Desk", 19000), t0);
    sink.handle(parse("OPTIONS * RTSP/1.0\r\nCSeq: 1\r\nRequire: org.wfa.wfd1.0\r\n\r\n"), t0);
    let body = "wfd_video_formats\r\nintel_friendly_name\r\nintel_sink_manufacturer_name\r\nintel_sink_model_name\r\nintel_sink_device_URL\r\nmicrosoft_latency_management_capability\r\nmicrosoft_format_change_capability\r\n";
    let m3 = format!(
        "GET_PARAMETER rtsp://localhost/wfd1.0 RTSP/1.0\r\nCSeq: 2\r\nContent-Type: text/parameters\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    let outs = sink.handle(parse(&m3), t0);
    let Message::Response(r) = sent(&outs)[0] else { panic!() };
    let got = params::parse_params(r.body_str());
    assert_eq!(params::get(&got, "intel_friendly_name"), Some("Desk"));
    assert_eq!(params::get(&got, "intel_sink_device_URL"), Some("none"));
    assert!(params::get(&got, "microsoft_latency_management_capability").is_none());
    assert_eq!(got.len(), 5);
}

#[test]
fn keepalive_timeout_closes_session() {
    let t0 = Instant::now();
    let (mut sink, _) = negotiate_with_simulated_source(SourceConfig::default(), t0);
    assert_eq!(sink.state(), SinkState::Playing);
    // simulated source uses timeout=30 -> close after 45 s of silence
    assert!(sink.on_timer(t0 + Duration::from_secs(44)).is_empty());
    let outs = sink.on_timer(t0 + Duration::from_secs(46));
    assert_eq!(events(&outs), vec![&SessionEvent::Closed { reason: "source stopped sending keep-alives".into() }]);
}

#[test]
fn negotiation_timeout_closes_session() {
    let t0 = Instant::now();
    let mut sink = SinkSession::new(SinkConfig::new("x", 19000), t0);
    assert!(sink.on_timer(t0 + Duration::from_secs(29)).is_empty());
    let outs = sink.on_timer(t0 + Duration::from_secs(31));
    assert!(matches!(events(&outs)[0], SessionEvent::Closed { .. }));
}

#[test]
fn renegotiation_while_playing() {
    let t0 = Instant::now();
    let (mut sink, _) = negotiate_with_simulated_source(SourceConfig::default(), t0);
    let body = "wfd_video_formats: 00 00 01 01 00000000 00000008 00000000 00 0000 0000 00 none none\r\n";
    let m4 = format!(
        "SET_PARAMETER rtsp://localhost/wfd1.0 RTSP/1.0\r\nCSeq: 20\r\nContent-Type: text/parameters\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    let outs = sink.handle(parse(&m4), t0);
    let SessionEvent::Negotiated(n) = events(&outs)[0] else { panic!() };
    assert_eq!(n.video.unwrap().to_string(), "1024x768p60");
    assert_eq!(sink.state(), SinkState::Playing, "re-negotiation keeps the session playing");
}

#[test]
fn rejected_setup_closes() {
    let t0 = Instant::now();
    let mut sink = SinkSession::new(SinkConfig::new("x", 19000), t0);
    sink.handle(parse("OPTIONS * RTSP/1.0\r\nCSeq: 1\r\n\r\n"), t0);
    let b = "wfd_trigger_method: SETUP\r\n";
    let outs =
        sink.handle(parse(&format!("SET_PARAMETER rtsp://x/wfd1.0 RTSP/1.0\r\nCSeq: 2\r\nContent-Length: {}\r\n\r\n{b}", b.len())), t0);
    let setup_cseq = sent(&outs)[1].cseq().unwrap();
    let outs = sink.handle(parse(&format!("RTSP/1.0 461 Unsupported Transport\r\nCSeq: {setup_cseq}\r\n\r\n")), t0);
    assert!(matches!(events(&outs)[0], SessionEvent::Closed { reason } if reason.contains("461")));
}

/// Pumps messages between a sink and the simulated source until PLAY.
fn negotiate_with_simulated_source(cfg: SourceConfig, t0: Instant) -> (SinkSession, Vec<SourceEvent>) {
    let mut sink = SinkSession::new(SinkConfig::new("Sim Sink", 19500), t0);
    let mut source = SourceSession::new(cfg, t0);
    let mut to_sink: Vec<Message> = Vec::new();
    let mut to_source: Vec<Message> = Vec::new();
    let mut source_events = Vec::new();
    let push_source = |outs: Vec<SourceOutput>, to_sink: &mut Vec<Message>, evs: &mut Vec<SourceEvent>| {
        for o in outs {
            match o {
                SourceOutput::Send(m) => to_sink.push(m),
                SourceOutput::Event(e) => evs.push(e),
            }
        }
    };
    push_source(source.start(), &mut to_sink, &mut source_events);
    for _ in 0..50 {
        for m in std::mem::take(&mut to_sink) {
            // serialize + parse to exercise the wire format both ways
            let m = parse(&String::from_utf8(m.to_bytes()).unwrap());
            for o in sink.handle(m, t0) {
                if let Output::Send(m) = o {
                    to_source.push(m);
                }
            }
        }
        for m in std::mem::take(&mut to_source) {
            let m = parse(&String::from_utf8(m.to_bytes()).unwrap());
            push_source(source.handle(m, t0), &mut to_sink, &mut source_events);
        }
        if sink.state() == SinkState::Playing && to_sink.is_empty() && to_source.is_empty() {
            break;
        }
    }
    assert_eq!(source.state(), SourceState::Playing);
    (sink, source_events)
}

#[test]
fn sink_and_simulated_source_reach_playing() {
    let t0 = Instant::now();
    let (sink, evs) = negotiate_with_simulated_source(SourceConfig { uibc_port: Some(7239), ..Default::default() }, t0);
    let n = sink.negotiated();
    assert_eq!(n.video.unwrap().to_string(), "1280x720p30", "source limits to 720p30 by default");
    assert_eq!(n.audio.unwrap().format, AudioFormat::Aac);
    assert_eq!(n.client_rtp_port, Some(19500));
    assert_eq!(n.uibc.as_ref().unwrap().port, Some(7239));
    assert!(n.uibc_enabled);
    assert!(evs.contains(&SourceEvent::Setup { sink_rtp_port: 19500 }));
    assert!(evs.contains(&SourceEvent::Play));
}

#[tokio::test]
async fn tcp_drivers_full_session() {
    use tokio::net::{TcpListener, TcpStream};
    use tokio::sync::mpsc;
    use wfd_core::driver::{SinkCommand, SourceCommand, run_sink, run_source};

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (src_ev_tx, mut src_ev) = mpsc::unbounded_channel();
    let (src_cmd_tx, src_cmd_rx) = mpsc::unbounded_channel();
    let source_cfg = SourceConfig { keepalive_interval: Duration::from_millis(200), uibc_port: Some(7239), ..Default::default() };
    let source = tokio::spawn(async move {
        let (s, _) = listener.accept().await.unwrap();
        run_source(s, source_cfg, src_ev_tx, src_cmd_rx).await
    });

    let (ev_tx, mut ev) = mpsc::unbounded_channel();
    let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
    let stream = TcpStream::connect(addr).await.unwrap();
    let sink = tokio::spawn(run_sink(stream, SinkConfig::new("TCP Sink", 19600), ev_tx, cmd_rx));

    assert!(matches!(next_event(&mut ev).await, SessionEvent::Negotiated(_)));
    assert_eq!(next_event(&mut ev).await, SessionEvent::PrepareMedia { rtp_port: 19600 });
    assert!(matches!(next_event(&mut ev).await, SessionEvent::Playing { .. }));

    // a few keep-alives go by without closing the session
    tokio::time::sleep(Duration::from_millis(700)).await;
    cmd_tx.send(SinkCommand::RequestIdr).unwrap();
    let mut saw_idr = false;
    let mut keepalive_ok = true;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    while let Ok(Some(e)) = tokio::time::timeout_at(deadline, src_ev.recv()).await {
        if e == SourceEvent::IdrRequested {
            saw_idr = true;
            break;
        }
        if matches!(e, SourceEvent::Closed { .. }) {
            keepalive_ok = false;
        }
    }
    assert!(saw_idr && keepalive_ok);

    // source-initiated teardown ends both sides cleanly
    src_cmd_tx.send(SourceCommand::Teardown).unwrap();
    assert_eq!(next_event(&mut ev).await, SessionEvent::Closed { reason: "teardown complete".into() });
    sink.await.unwrap().unwrap();
    source.await.unwrap().unwrap();
}

async fn next_event(rx: &mut tokio::sync::mpsc::UnboundedReceiver<SessionEvent>) -> SessionEvent {
    tokio::time::timeout(Duration::from_secs(5), rx.recv()).await.expect("event in time").expect("channel open")
}
