You are a senior systems and multimedia engineer. Build a cross-platform, open-source Wireless Display (Miracast Sink / Receiver) desktop application in Rust that runs on Linux, macOS, and Windows and renders incoming wireless display streams inside a resizable native desktop window.

### 1. Core Architectural Constraint (Read Carefully)
Do NOT hallucinate native Wi-Fi Direct APIs on macOS—Apple does not expose Wi-Fi P2P / Miracast APIs in CoreWLAN. Instead, design a modular workspace with a clean trait-based transport abstraction so the shared RTSP/media/GUI core works identically across all three OSes:
- **Linux Transport (`transport-linux-p2p`):** Native Wi-Fi Direct Miracast sink via `wpa_supplicant` D-Bus (`fi.w1.wpa_supplicant1`) using a dedicated P2P interface (do NOT tear down NetworkManager or kill the user's primary Wi-Fi connection). Set Wi-Fi Display Information Elements (WFD IEs) declaring a Primary Sink, handle P2P Group Owner (GO) negotiation + WPS PBC/PIN, and run a lightweight embedded DHCPv4 server on the P2P group interface.
- **Windows Transport (`transport-windows-p2p`):** Native Wi-Fi Direct Miracast sink using WinRT (`windows` crate: `Windows::Devices::WiFiDirect::WiFiDirectAdvertisementPublisher`, `WiFiDirectConnectionListener`, and WFD IEs).
- **Cross-Platform Infrastructure Transport (`transport-ms-mice`):** Implement Microsoft's Miracast over Infrastructure ([MS-MICE]) protocol for macOS, Linux, and Windows using mDNS/DNS-SD (`_display._tcp.local.`), TCP port 7250 handshake (SOURCE_READY, STOP_PROJECTION, PIN/security negotiation), and standard LAN routing. On macOS, clearly display in the UI that Infrastructure Mode (MS-MICE) is active and Wi-Fi Direct is unsupported by macOS.

### 2. Workspace Structure
Create a Cargo workspace with these crates:
1. `wfd-core`: Pure-Rust Wi-Fi Display (WFD) RTSP state machine and protocol parser.
   - Implement RTSP 1.0 server/session handling for WFD messages **M1 through M8** (OPTIONS, GET_PARAMETER, SET_PARAMETER, SETUP, PLAY, TEARDOWN, PAUSE) and **M16** (keep-alive).
   - Negotiate `wfd_video_formats` (H.264 Constrained Baseline/High Profile up to 1920x1080@60fps), `wfd_audio_codecs` (LPCM 48kHz 16-bit 2ch + AAC), `wfd_client_rtp_ports`, and `wfd_trigger_method`.
   - Implement optional **UIBC (User Input Back Channel)** over TCP so mouse and keyboard events from the receiver window can be forwarded back to the sender (e.g., Samsung DeX / Windows) when supported.
2. `wfd-transport`: Async `DiscoveryTransport` trait (using `tokio`) that emits `PeerConnected { peer_ip, rtsp_port, device_name }` and `PeerDisconnected` events, with feature-gated implementations for Linux P2P, Windows P2P, and MS-MICE.
3. `wfd-media`: Low-latency media pipeline using **GStreamer** (`gstreamer` + `gstreamer-app` crates) to ingest UDP RTP/MPEG-TS (`udpsrc -> rtpjitterbuffer -> rtpmp2tdepay -> tsdemux`), hardware-accelerate H.264 decoding where available (VA-API / VideoToolbox / D3D11) with software fallback (`avdec_h264`), output decoded RGBA/NV12 video frames to an `appsink`, and route audio (LPCM/AAC) to the system audio sink (`autoaudiosink`).
4. `wfd-app`: Native desktop GUI application using `eframe`/`egui` (or `winit` + `wgpu`) that:
   - Opens a resizable window displaying the receiver's discoverable name, active transport mode, connection status, and an Allow/Deny + optional PIN pairing prompt when a device connects.
   - Renders decoded video frames from `wfd-media` with proper aspect-ratio letterboxing, low-latency texture upload, fullscreen toggle (`F11` / double-click), volume/mute controls, and an FPS/latency debug overlay.
   - Captures mouse/keyboard input inside the video canvas and routes it to the UIBC channel when connected.
   - Includes a "Simulate / Local Test Source" mode (feeding a local `gst-launch-1.0` MPEG-TS RTP stream over localhost) so the entire RTSP + media + window pipeline can be verified without a physical Miracast phone.

### 3. Execution Plan (Work in Phases)
Execute and verify one phase at a time; do not move to the next phase until tests pass:
- **Phase 1:** Scaffold the Cargo workspace, define traits, and implement `wfd-core` (RTSP M1–M8, M16, WFD IE encoding/decoding) with comprehensive unit tests covering a full WFD handshake transcript.
- **Phase 2:** Implement `wfd-media` and `wfd-app` with the local localhost RTP/MPEG-TS test harness so a live H.264+audio stream renders smoothly inside the desktop window.
- **Phase 3:** Implement `transport-ms-mice` (cross-platform mDNS + TCP 7250) and verify discovery on the local network.
- **Phase 4:** Implement `transport-linux-p2p` (D-Bus `wpa_supplicant` + embedded DHCPv4) and `transport-windows-p2p` (WinRT Wi-Fi Direct), guarded by `#[cfg(target_os = "...")]` so the workspace compiles cleanly on all three targets.
- **Phase 5:** Add UIBC mouse/keyboard forwarding, structured logging (`tracing`), and a `README.md` documenting OS prerequisites (GStreamer dev libs, firewall ports, Linux polkit/wpa_supplicant permissions).

Start now with Phase 1: inspect the environment, propose the exact crate versions in `Cargo.toml`, and scaffold the workspace.