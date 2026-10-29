//! Settings (`config.toml`), paired machines (`peers.toml`) and this machine's key pair.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::geom::{Rect, Side};

pub const DEFAULT_PORT: u16 = 24900;

#[derive(Clone, Debug)]
pub struct Paths {
    pub config_dir: PathBuf,
    pub cache_dir: PathBuf,
}

impl Paths {
    /// `override_dir` (from `--config-dir`) keeps everything, cache included, under one folder.
    pub fn new(override_dir: Option<PathBuf>) -> Result<Paths> {
        let (config_dir, cache_dir) = match override_dir {
            Some(dir) => (dir.clone(), dir.join("cache")),
            None => (
                dirs::config_dir().context("no config directory on this system")?.join("wifishare"),
                dirs::cache_dir().context("no cache directory on this system")?.join("wifishare"),
            ),
        };
        fs::create_dir_all(&config_dir).with_context(|| format!("creating {}", config_dir.display()))?;
        Ok(Paths { config_dir, cache_dir })
    }

    pub fn config_file(&self) -> PathBuf {
        self.config_dir.join("config.toml")
    }

    pub fn peers_file(&self) -> PathBuf {
        self.config_dir.join("peers.toml")
    }

    pub fn identity_file(&self) -> PathBuf {
        self.config_dir.join("identity.key")
    }

    pub fn incoming_dir(&self) -> PathBuf {
        self.cache_dir.join("incoming")
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// What the window starts as: sharing this machine's input, or being controlled.
    pub mode: Option<Mode>,
    /// How this machine introduces itself; the server matches layout entries on it.
    pub name: String,
    /// TCP port the server listens on; UDP discovery uses the same number.
    pub port: u16,
    pub network: NetworkCfg,
    pub server: ServerCfg,
    pub client: ClientCfg,
    pub clipboard: ClipboardCfg,
    pub audio: AudioCfg,
    pub files: FilesCfg,
    pub gui: GuiCfg,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FilesCfg {
    /// Folders the other computer may browse and copy from.
    pub shares: Vec<ShareCfg>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShareCfg {
    /// What the other computer sees; unique among the shares.
    pub name: String,
    pub path: PathBuf,
    /// Let the other computer add files and folders (never overwrite or delete).
    #[serde(default)]
    pub write: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AudioCfg {
    /// Play this computer's sound on the computer it is connected to.
    pub send: bool,
    /// While sending, keep playing the sound here as well.
    pub play_here: bool,
    /// Play sound that the other computer sends.
    pub receive: bool,
    /// Buffering on the receiving side; more survives worse Wi-Fi, less is snappier.
    pub buffer_ms: u32,
}

impl Default for AudioCfg {
    fn default() -> Self {
        AudioCfg { send: false, play_here: false, receive: true, buffer_ms: 40 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Server,
    Client,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct GuiCfg {
    /// Start sharing as soon as the window opens.
    pub start_sharing: bool,
    /// Closing the window leaves wifishare running in the tray (where there is one).
    pub keep_running: bool,
}

impl Default for GuiCfg {
    fn default() -> Self {
        GuiCfg { start_sharing: true, keep_running: true }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct NetworkCfg {
    /// How often each side sends a heartbeat.
    pub heartbeat_ms: u64,
    /// A link that has been silent this long is treated as dead and re-established.
    pub timeout_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ServerCfg {
    /// Edge used for clients without a `[[server.clients]]` entry.
    pub default_side: Side,
    /// Swap Cmd and Ctrl when the two machines differ (one macOS, one not).
    pub swap_cmd_ctrl: bool,
    pub clients: Vec<ClientLayout>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClientLayout {
    pub name: String,
    pub side: Side,
    #[serde(default = "one")]
    pub speed: f64,
    /// Overrides `server.swap_cmd_ctrl` for this client.
    pub swap_cmd_ctrl: Option<bool>,
}

fn one() -> f64 {
    1.0
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ClientCfg {
    /// Host name or IP of the server. Optional: discovery finds paired servers on the LAN.
    pub server: Option<String>,
    /// Monitor rectangles `[x, y, w, h]`, when automatic detection gets them wrong.
    pub screen: Option<Vec<[i32; 4]>>,
    /// Multiplier for scrolling injected on this machine.
    pub scroll_speed: Option<f64>,
}

impl ClientCfg {
    pub fn screen_override(&self) -> Option<Vec<Rect>> {
        let rects = self.screen.as_ref()?;
        Some(rects.iter().map(|r| Rect::new(r[0], r[1], r[2], r[3])).collect())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ClipboardCfg {
    pub enabled: bool,
    /// Copied files are sent ahead of time when their total size is at most this.
    pub max_files_mb: u64,
    pub max_text_mb: u64,
    pub max_image_mb: u64,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            mode: None,
            name: default_name(),
            port: DEFAULT_PORT,
            network: NetworkCfg::default(),
            server: ServerCfg::default(),
            client: ClientCfg::default(),
            clipboard: ClipboardCfg::default(),
            audio: AudioCfg::default(),
            files: FilesCfg::default(),
            gui: GuiCfg::default(),
        }
    }
}

impl Default for NetworkCfg {
    fn default() -> Self {
        NetworkCfg { heartbeat_ms: 1000, timeout_ms: 4000 }
    }
}

impl Default for ServerCfg {
    fn default() -> Self {
        ServerCfg { default_side: Side::Right, swap_cmd_ctrl: true, clients: Vec::new() }
    }
}

impl Default for ClipboardCfg {
    fn default() -> Self {
        ClipboardCfg { enabled: true, max_files_mb: 2048, max_text_mb: 32, max_image_mb: 128 }
    }
}

pub fn default_name() -> String {
    let name = gethostname::gethostname().to_string_lossy().into_owned();
    let name = name.strip_suffix(".local").unwrap_or(&name).to_string();
    if name.is_empty() { "wifishare".into() } else { name }
}

impl Config {
    pub fn load(paths: &Paths) -> Result<Config> {
        let file = paths.config_file();
        if !file.exists() {
            return Ok(Config::default());
        }
        let text = fs::read_to_string(&file).with_context(|| format!("reading {}", file.display()))?;
        toml::from_str(&text).with_context(|| format!("parsing {}", file.display()))
    }

    pub fn save(&self, paths: &Paths) -> Result<()> {
        write_atomic(&paths.config_file(), toml::to_string_pretty(self)?.as_bytes())
    }

    /// Sets where a client sits, adding a layout entry for it if needed.
    pub fn set_side(&mut self, name: &str, side: Side) {
        match self.server.clients.iter_mut().find(|c| c.name.eq_ignore_ascii_case(name)) {
            Some(c) => c.side = side,
            None => self.server.clients.push(ClientLayout { name: name.into(), side, speed: 1.0, swap_cmd_ctrl: None }),
        }
    }

    pub fn layout_for(&self, name: &str) -> Option<&ClientLayout> {
        self.server.clients.iter().find(|c| c.name.eq_ignore_ascii_case(name))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Server,
    Client,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Peer {
    pub name: String,
    /// Hex Noise static public key.
    pub key: String,
    /// The role the *peer* plays for us.
    pub role: Role,
    /// Last address a connection to this peer succeeded on (servers only).
    #[serde(default)]
    pub last_addr: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Peers {
    #[serde(default, rename = "peer")]
    pub peers: Vec<Peer>,
}

impl Peers {
    pub fn load(paths: &Paths) -> Result<Peers> {
        let file = paths.peers_file();
        if !file.exists() {
            return Ok(Peers::default());
        }
        let text = fs::read_to_string(&file).with_context(|| format!("reading {}", file.display()))?;
        toml::from_str(&text).with_context(|| format!("parsing {}", file.display()))
    }

    pub fn save(&self, paths: &Paths) -> Result<()> {
        write_atomic(&paths.peers_file(), toml::to_string_pretty(self)?.as_bytes())
    }

    pub fn by_key(&self, key: &[u8]) -> Option<&Peer> {
        let hex = hex::encode(key);
        self.peers.iter().find(|p| p.key == hex)
    }

    /// Records a newly paired peer, replacing an older entry with the same key or name.
    pub fn pin(&mut self, name: &str, key: &[u8], role: Role) {
        let hex = hex::encode(key);
        self.peers.retain(|p| p.key != hex && !(p.name == name && p.role == role));
        self.peers.push(Peer { name: name.into(), key: hex, role, last_addr: None });
    }

    pub fn servers(&self) -> impl Iterator<Item = &Peer> {
        self.peers.iter().filter(|p| p.role == Role::Server)
    }
}

/// This machine's long-term Noise key pair.
#[derive(Clone)]
pub struct Identity {
    pub private: Vec<u8>,
    pub public: Vec<u8>,
}

impl Identity {
    pub fn load_or_create(paths: &Paths) -> Result<Identity> {
        let file = paths.identity_file();
        if let Ok(text) = fs::read_to_string(&file) {
            let mut lines = text.lines();
            let (Some(private), Some(public)) = (lines.next(), lines.next()) else { bail!("{} is malformed", file.display()) };
            return Ok(Identity { private: hex::decode(private.trim())?, public: hex::decode(public.trim())? });
        }
        let pair = snow::Builder::new(crate::wire::NOISE_PARAMS.parse()?).generate_keypair()?;
        let id = Identity { private: pair.private, public: pair.public };
        write_atomic(&file, format!("{}\n{}\n", hex::encode(&id.private), hex::encode(&id.public)).as_bytes())?;
        restrict_permissions(&file);
        log::info!("created a new identity in {}", file.display());
        Ok(id)
    }

    /// Short human-readable form of the public key.
    pub fn fingerprint(&self) -> String {
        fingerprint(&self.public)
    }
}

pub fn fingerprint(key: &[u8]) -> String {
    key.iter().take(8).map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(":")
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, bytes).with_context(|| format!("writing {}", tmp.display()))?;
    fs::rename(&tmp, path).with_context(|| format!("replacing {}", path.display()))?;
    Ok(())
}

fn restrict_permissions(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
    }
    #[cfg(not(unix))]
    let _ = path;
}

/// A commented starting point written by `wifishare init`.
pub const SAMPLE_CONFIG: &str = r#"# wifishare settings. Every key is optional.

# mode = "server"          # what the window starts as: "server" or "client"
# name = "my-laptop"       # defaults to the host name
# port = 24900             # TCP for links, UDP for discovery

[network]
# heartbeat_ms = 1000      # heartbeat interval on each link
# timeout_ms = 4000        # silence after which a link is dropped and re-dialled

[server]
# default_side = "right"   # where clients without an entry below sit
# swap_cmd_ctrl = true     # Cmd<->Ctrl when exactly one side is a Mac

# [[server.clients]]
# name = "omar-ThinkPad-P50"
# side = "left"            # left, right, top or bottom of this machine's desktop
# speed = 1.0              # pointer speed multiplier on that client
# swap_cmd_ctrl = true

[client]
# server = "192.168.1.25"  # optional; paired servers are found automatically
# screen = [[0, 0, 1920, 1080], [1920, 0, 1920, 1080]]   # override monitor detection
# scroll_speed = 1.0

[clipboard]
# enabled = true
# max_files_mb = 2048      # larger file copies are not sent
# max_text_mb = 32
# max_image_mb = 128

[audio]
# send = false             # play this computer's sound on the other computer (Linux so far)
# play_here = false        # while sending, keep playing it here too
# receive = true           # play sound the other computer sends
# buffer_ms = 40           # receiving buffer: more rides out bad Wi-Fi, less is snappier

[files]
# Folders the other computer may browse and copy from (the window's Files tab adds these).
# [[files.shares]]
# name = "Documents"
# path = "/home/me/Documents"
# write = false            # true lets the other computer add files and folders

[gui]
# start_sharing = true     # start sharing when the window opens
# keep_running = true      # closing the window keeps wifishare running in the tray
"#;
