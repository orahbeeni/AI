# Miracast Receiver

A desktop **wireless display (Miracast / Wi-Fi Display) receiver** written in Rust. Cast a
Samsung phone (Smart View or **Samsung DeX**), any Android phone, or a Windows PC (Win + K)
into a resizable window. With the input back-channel, the mouse and keyboard over that
window control the device.

| | Linux (Ubuntu) | Windows | macOS |
|---|---|---|---|
| Wi-Fi Direct (phones, DeX, Windows) | wpa_supplicant D-Bus, **built and tested against a mock** | WinRT, **compiles, untested** | not possible (Apple exposes no Wi-Fi P2P API) |
| Infrastructure / MS-MICE (Windows over your LAN) | yes | yes (compiles) | yes (compiles) |
| Video, audio, window, UIBC | yes, tested | compiles | compiles |

## Quick start (Ubuntu)

```bash
scripts/setup-linux.sh        # one time: packages, permissions, install, menu entry
miracast-receiver             # or "Miracast Receiver" in the app menu
```

Then, on the phone: **Quick Settings → Smart View** (or the **DeX** tile) → pick the receiver's name.
On Windows: **Win + K** → pick the name.

No phone at hand? Click **Test with a simulated phone** in the window, or run `miracast-receiver --simulate`.

## What the setup script changes (and why)

`scripts/setup-linux.sh` asks for your sudo password once:

1. **Packages:** `libgstreamer1.0-dev`, `libgstreamer-plugins-base1.0-dev` and the GStreamer plugins (`good`, `bad`, `ugly`, `libav`).
2. **Install:** builds `target/release/miracast-receiver` and copies it to `~/.local/bin`.
3. **Capabilities:** `setcap cap_net_admin,cap_net_bind_service,cap_net_raw+ep` on that copy. These let it put an
   address on the Wi-Fi Direct group interface and serve DHCP on UDP 67 to the phone. Re-run the script after
   rebuilding, because a new file loses its capabilities.
4. **wpa_supplicant access:** writes `/etc/dbus-1/system.d/miracast-receiver.conf`, letting your user call
   `fi.w1.wpa_supplicant1`. This is the same access the `netdev` group gives, and it applies without logging out.
5. **App menu entry** with an icon.

`scripts/setup-linux.sh --uninstall` removes the binary, the menu entry and the D-Bus policy.
NetworkManager keeps running and your normal Wi-Fi stays connected. The app uses only the separate
P2P device (`p2p-dev-<wifi>`) that wpa_supplicant creates. When the app quits, it restores the device
name, WFD IEs and WPS settings it changed on that device.

`miracast-receiver doctor` checks everything: decoders, D-Bus access, capabilities and port 7250.

### Firewall

If `ufw` is enabled:

```bash
sudo ufw allow 7250/tcp                          # MS-MICE (Windows over the LAN)
sudo ufw allow 8765/tcp                          # file sharing
sudo ufw allow 19000:19010/udp                   # video/audio RTP (19000 is tried first)
sudo ufw allow 5353/udp                          # mDNS
sudo ufw allow in on p2p-+ to any port 67 proto udp   # DHCP on Wi-Fi Direct links
```

### Other platforms

* **Windows:** install the GStreamer MSVC runtime + development packages (gstreamer.freedesktop.org) and set
  `PKG_CONFIG_PATH`, then `cargo build --release -p wfd-app`. Windows' own "Wireless Display" feature uses the same
  Wi-Fi Direct receiver role. If it is installed and running, turn it off.
* **macOS:** `brew install gstreamer` and build. Only Infrastructure mode is offered, and the window says so.
  Windows finds MS-MICE receivers through Wi-Fi Direct probe responses, which a Mac can't send, so in practice
  Windows has to be pointed at the Mac some other way.

## Using it

| | |
|---|---|
| **F11** / double-click | full screen (double-click when input isn't going to the device) |
| **F3** / Stats | overlay: decoder, fps, bitrate, packet loss, decode-to-display latency |
| click the picture | locks the mouse to the device (relative HID mouse); **Ctrl+Alt** releases |
| Refresh | ask the device for a fresh key frame (if the picture is smeared) |
| Settings | name, Wi-Fi Direct / MS-MICE, auto-accept, input on/off, max quality, decoder, latency vs. smooth |

On desktops where NetworkManager manages Wi-Fi, it makes the Wi-Fi Direct connection when you click Allow (it deletes
groups it did not start). That path supports push-button pairing only and connects without extra network capabilities. With `cap_net_admin`
(granted by the setup script) the app also turns Wi-Fi power saving off on the link, which otherwise adds lag.

**Files.** Miracast carries no files, so while a device is connected the app serves a small page on the link to it
(shown under **Files**, usually `http://192.168.49.200:8765`). Drag files onto the window, then open that address in
the device's web browser to download them, or to send files back (saved in `~/Downloads/Miracast`, never
overwriting). Only the connected device is answered, only the files you added can be downloaded, the list is cleared
when the session ends, and Settings can turn it off. The transfer is plain HTTP over the encrypted Wi-Fi Direct link
(over your own network with MS-MICE it is readable by others on that network). On Wayland desktops the window runs
through XWayland, because drag and drop doesn't work otherwise.

**Lag.** If your laptop's Wi-Fi is connected on one channel and the Wi-Fi Direct link is on another, the single radio
has to alternate between them. Connecting the phone to the same Wi-Fi network as the laptop often makes it pick the
same channel. The mouse pointer over the picture can be changed in Settings.

When a device connects you get an **Allow / Deny** prompt. For push-button pairing, "Always allow" skips it in future.
Over Wi-Fi Direct on Linux the receiver offers push-button pairing only, so phones connect with one tap. If a device
still insists on a PIN, the prompt shows the PIN to type on it, or asks for the PIN it shows.

Command line:

```
miracast-receiver [--name NAME] [--simulate] [--connect IP:PORT] [--no-wifi-direct] [--no-infrastructure]
                  [--decoder auto|software|hardware] [--fullscreen] [--selftest shot.png]
miracast-receiver sim-source [--listen 127.0.0.1:7236] [--width 1280 --height 720 --fps 30] [--gst-launch]
miracast-receiver selftest      # headless end-to-end check
miracast-receiver doctor        # system check
```

Logs: `RUST_LOG=debug miracast-receiver` (RTSP messages at `trace`).

## How it works

```
crates/
  wfd-core       pure Rust protocol: RTSP codec, M1–M8/M13/M16 sink state machine, WFD parameters
                 (video tables, audio codecs, UIBC capability), WFD IE subelements, UIBC packets,
                 a simulated *source* (for tests/simulator), tokio drivers
  wfd-transport  DiscoveryTransport trait + transports:
                   linux_p2p    wpa_supplicant D-Bus, GO negotiation, PBC/PIN, netlink, DHCP
                   windows_p2p  WinRT WiFiDirectAdvertisementPublisher / ConnectionListener
                   mice         MS-MICE: TCP 7250 + _display._tcp (Avahi or built-in mDNS)
                   direct       a known IP:port (simulator, --connect)
  wfd-media      GStreamer: udpsrc → rtpjitterbuffer → rtpmp2tdepay → tsdemux → h264parse →
                 HW decoder (VA-API / NVDEC / D3D11 / VideoToolbox) or avdec_h264 → I420 appsink;
                 audio → decodebin → volume → autoaudiosink. Also the test source.
  wfd-app        eframe/egui window, engine (tokio), simulator, I420→RGB GL shader
```

The **source** (phone/PC) is the RTSP server; the receiver connects to it, answers its capability query
(M3), accepts its choice (M4), then sends SETUP/PLAY. Video arrives as H.264 in MPEG-TS over RTP/UDP.
The receiver advertises H.264 CBP/CHP up to 1080p60 (level 4.2), LPCM and AAC audio, and UIBC
(generic and HID).

**Wi-Fi Direct on Linux:** WFD IEs (primary sink, port 7236, UIBC bit) plus device type "Display/TV" go on the P2P
device, and P2P `Find` keeps it discoverable. A phone's GO-negotiation or provision-discovery request opens the
Allow prompt. We then `Connect` with GO intent 15. Android sources use intent 0, so we become group owner, put
192.168.173.1/24 on the group interface and hand the phone an address over DHCP. The lease tells us where the
phone's RTSP server is. If the source insists on being GO, we take an address from it instead (DHCP client, or
the address wpa_supplicant got in EAPOL).

**Input (UIBC):** HIDC is used when the source allows it: a relative 3-button mouse with wheel, and a boot
keyboard, so right-click, scrolling, shortcuts and the Super/Windows key (DeX app drawer) all work. If HIDC
isn't allowed, Generic input is used (absolute pointer, left button, ASCII keys).

## Testing

```bash
cargo test --workspace                 # protocol, transports, media loopback, mock wpa_supplicant
scripts/cross-check.sh                 # type-check everything for Windows and macOS
miracast-receiver selftest             # simulator → RTSP → decode → UIBC, no window
miracast-receiver --selftest shot.png  # same with the window; saves a screenshot
```

* `wfd-core/tests/handshake.rs`: byte-level Android-style transcript M1–M16, Windows-style `intel_*` queries,
  keep-alive and negotiation timeouts, renegotiation, a full session over TCP.
* `wfd-transport/tests/mice.rs`: a fake Windows source on TCP 7250, plus our `_display._tcp` record found by an
  independent mDNS browser. MS-MICE codec tests use the spec's own hex examples and PIN hash vectors.
* `wfd-transport/tests/linux_p2p_mock.rs`: the Wi-Fi Direct transport against a mock wpa_supplicant on a
  private D-Bus. It checks the exact D-Bus types written, PBC and PIN pairing, group start/finish, refusing
  a second phone, and that settings are restored.
* `wfd-media/tests/loopback.rs`: test source → RTP → decode, checking colours, at 720p30 (software) and
  1080p60 (hardware decoder when present).
* `scripts/test-source.sh`: a plain `gst-launch-1.0` stream for checking the media path on its own.

## Not done / known limits

* **No real phone has been tested yet.** Wi-Fi Direct on Linux has been tested against a mock wpa_supplicant,
  not a Samsung phone. The first real DeX connection is the next thing to try. Logs with `RUST_LOG=debug`
  will show where it stops if it does.
* **Windows Wi-Fi Direct** (WinRT) and all macOS/Windows builds have only been compile-checked.
* **HDCP** isn't supported (`wfd_content_protection: none`), so protected video apps may refuse to cast.
* **MS-MICE** without DTLS/PIN: the receiver advertises neither, so Windows uses an unencrypted session.
* One source at a time. A second device is refused while one is connected.
