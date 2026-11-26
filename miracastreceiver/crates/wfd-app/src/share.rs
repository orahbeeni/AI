//! File sharing with the connected device. Miracast itself carries no files, so
//! while a session runs we serve a small web page on the link to the device:
//! open it in the device's browser to download the files dropped on our window,
//! or to send files back.
//!
//! Access: only requests coming from the connected device's address are
//! answered, only files the user added can be downloaded (by number, never by
//! path), uploads land in one folder under a cleaned-up name, and the `Host`
//! header must be our own address (so another web page can't reach us by
//! rebinding a DNS name). Uploads use `PUT`, which browsers don't allow other
//! sites to send without our permission.

use std::io;
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;
use tracing::{debug, warn};

pub const PREFERRED_PORT: u16 = 8765;
const MAX_HEAD: usize = 16 * 1024;
const IDLE: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, PartialEq)]
pub struct SharedFile {
    pub id: u64,
    pub path: PathBuf,
    pub name: String,
    pub size: u64,
}

/// The files offered to the device.
#[derive(Debug, Default)]
pub struct FileList {
    files: Vec<SharedFile>,
    next_id: u64,
}

impl FileList {
    pub fn add(&mut self, path: &Path) -> Result<(), String> {
        let meta = std::fs::metadata(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        if !meta.is_file() {
            return Err(format!("{} is not a file (folders can't be shared; zip it first)", path.display()));
        }
        if self.files.iter().any(|f| f.path == path) {
            return Ok(());
        }
        let name = path.file_name().map_or_else(|| "file".into(), |n| n.to_string_lossy().into_owned());
        self.next_id += 1;
        self.files.push(SharedFile { id: self.next_id, path: path.to_path_buf(), name, size: meta.len() });
        Ok(())
    }

    pub fn remove(&mut self, id: u64) {
        self.files.retain(|f| f.id != id);
    }

    pub fn clear(&mut self) {
        self.files.clear();
    }

    pub fn files(&self) -> &[SharedFile] {
        &self.files
    }
}

pub type Files = Arc<Mutex<FileList>>;
/// Called with the saved path and size of each received file.
pub type OnReceived = Arc<dyn Fn(PathBuf, u64) + Send + Sync>;

#[derive(Clone)]
struct Context {
    host: String,
    files: Files,
    folder: PathBuf,
    on_received: OnReceived,
}

/// A running server; dropping it closes the port and cuts transfers.
pub struct ShareServer {
    pub url: String,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for ShareServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl ShareServer {
    /// Serves on `bind`, answering only `peer`.
    pub async fn start(bind: IpAddr, peer: IpAddr, files: Files, folder: PathBuf, on_received: OnReceived) -> io::Result<Self> {
        let listener = match TcpListener::bind(SocketAddr::new(bind, PREFERRED_PORT)).await {
            Ok(l) => l,
            Err(_) => TcpListener::bind(SocketAddr::new(bind, 0)).await?,
        };
        let host = listener.local_addr()?.to_string();
        let url = format!("http://{host}");
        let context = Context { host, files, folder, on_received };
        let task = tokio::spawn(async move {
            // owned by this task, so aborting it also aborts open transfers
            let mut connections = JoinSet::new();
            loop {
                tokio::select! {
                    accepted = listener.accept() => {
                        let Ok((stream, from)) = accepted else {
                            tokio::time::sleep(Duration::from_millis(200)).await;
                            continue;
                        };
                        if from.ip() != peer {
                            warn!(%from, "file sharing: refused an address that isn't the connected device");
                            continue;
                        }
                        let context = context.clone();
                        connections.spawn(async move {
                            if let Err(e) = serve(stream, context).await {
                                debug!("file sharing: {e}");
                            }
                        });
                    }
                    Some(_) = connections.join_next() => {}
                }
            }
        });
        Ok(Self { url, task })
    }
}

/// Where received files are saved.
pub fn download_folder() -> PathBuf {
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from).unwrap_or_default();
    home.join("Downloads").join("Miracast")
}

struct Head {
    method: String,
    path: String,
    host: String,
    length: Option<u64>,
    expects_continue: bool,
}

fn parse_head(text: &str) -> Option<Head> {
    let mut lines = text.split("\r\n");
    let mut first = lines.next()?.split(' ');
    let (method, path) = (first.next()?.to_string(), first.next()?.to_string());
    let mut head = Head { method, path, host: String::new(), length: None, expects_continue: false };
    for line in lines {
        let Some((name, value)) = line.split_once(':') else { continue };
        let value = value.trim();
        match name.to_ascii_lowercase().as_str() {
            "host" => head.host = value.to_string(),
            "content-length" => head.length = Some(value.parse().ok()?),
            "expect" => head.expects_continue = value.eq_ignore_ascii_case("100-continue"),
            _ => {}
        }
    }
    Some(head)
}

async fn read_some(stream: &mut TcpStream, buf: &mut [u8]) -> io::Result<usize> {
    tokio::time::timeout(IDLE, stream.read(buf)).await.map_err(|_| io::Error::from(io::ErrorKind::TimedOut))?
}

async fn serve(mut stream: TcpStream, context: Context) -> io::Result<()> {
    let mut buf = Vec::new();
    let mut chunk = vec![0u8; 64 * 1024];
    let end = loop {
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i;
        }
        if buf.len() > MAX_HEAD {
            return respond(&mut stream, "431 Request Header Fields Too Large", "text/plain", b"").await;
        }
        let n = read_some(&mut stream, &mut chunk).await?;
        if n == 0 {
            return Ok(());
        }
        buf.extend_from_slice(&chunk[..n]);
    };
    let rest = buf.split_off(end + 4);
    let Some(head) = parse_head(&String::from_utf8_lossy(&buf[..end])) else {
        return respond(&mut stream, "400 Bad Request", "text/plain", b"").await;
    };
    if head.host != context.host {
        return respond(&mut stream, "403 Forbidden", "text/plain", b"use the address shown by the receiver").await;
    }
    match head.method.as_str() {
        "GET" if head.path == "/" => {
            let page = PAGE.replace("<!--files-->", &file_rows(&context.files));
            respond(&mut stream, "200 OK", "text/html; charset=utf-8", page.as_bytes()).await
        }
        "GET" if head.path == "/list" => {
            let rows = file_rows(&context.files);
            respond(&mut stream, "200 OK", "text/html; charset=utf-8", rows.as_bytes()).await
        }
        "GET" if head.path.starts_with("/f/") => {
            let id: Option<u64> = head.path[3..].parse().ok();
            let file = context.files.lock().unwrap().files().iter().find(|f| Some(f.id) == id).cloned();
            match file {
                Some(file) => download(&mut stream, &file).await,
                None => respond(&mut stream, "404 Not Found", "text/plain", b"no longer shared").await,
            }
        }
        "PUT" if head.path.starts_with("/u/") => {
            let Some(length) = head.length else {
                return respond(&mut stream, "411 Length Required", "text/plain", b"").await;
            };
            if head.expects_continue {
                stream.write_all(b"HTTP/1.1 100 Continue\r\n\r\n").await?;
            }
            let name = clean_name(&percent_decode(&head.path[3..]));
            match receive(&mut stream, &context.folder, &name, length, rest).await {
                Ok(path) => {
                    (context.on_received)(path, length);
                    respond(&mut stream, "201 Created", "text/plain", b"saved").await
                }
                Err(e) => {
                    warn!(%name, "file sharing: upload failed: {e}");
                    respond(&mut stream, "500 Internal Server Error", "text/plain", b"could not save the file").await
                }
            }
        }
        _ => respond(&mut stream, "405 Method Not Allowed", "text/plain", b"").await,
    }
}

async fn respond(stream: &mut TcpStream, status: &str, kind: &str, body: &[u8]) -> io::Result<()> {
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nCache-Control: no-store\r\n\
         X-Content-Type-Options: nosniff\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes()).await?;
    stream.write_all(body).await?;
    stream.shutdown().await
}

async fn download(stream: &mut TcpStream, file: &SharedFile) -> io::Result<()> {
    let Ok(handle) = tokio::fs::File::open(&file.path).await else {
        return respond(stream, "404 Not Found", "text/plain", b"the file is gone").await;
    };
    let length = handle.metadata().await?.len();
    let head = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: {}\r\nContent-Length: {length}\r\n\
         Content-Disposition: attachment; filename*=UTF-8''{}\r\nCache-Control: no-store\r\n\
         X-Content-Type-Options: nosniff\r\nConnection: close\r\n\r\n",
        content_type(&file.name),
        percent_encode(&file.name)
    );
    stream.write_all(head.as_bytes()).await?;
    tokio::io::copy(&mut handle.take(length), stream).await?;
    stream.shutdown().await
}

/// Streams an upload into a temporary file, then gives it a free name.
async fn receive(stream: &mut TcpStream, folder: &Path, name: &str, length: u64, early: Vec<u8>) -> io::Result<PathBuf> {
    tokio::fs::create_dir_all(folder).await?;
    static SERIAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let serial = SERIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let partial = folder.join(format!(".upload-{}-{serial}.part", std::process::id()));
    // removes the partial file however we leave, including on cancellation
    let guard = RemoveOnDrop(partial.clone());
    let mut file = tokio::fs::File::create(&partial).await?;
    let first = early.len().min(length as usize);
    file.write_all(&early[..first]).await?;
    let mut left = length - first as u64;
    let mut chunk = vec![0u8; 64 * 1024];
    while left > 0 {
        let want = left.min(chunk.len() as u64) as usize;
        let n = read_some(stream, &mut chunk[..want]).await?;
        if n == 0 {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "the transfer stopped early"));
        }
        file.write_all(&chunk[..n]).await?;
        left -= n as u64;
    }
    file.flush().await?;
    drop(file);
    // claim a name that doesn't exist yet, never overwriting
    for attempt in 0..1000 {
        let target = folder.join(numbered(name, attempt));
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&target) {
            Ok(_) => {
                std::fs::rename(&partial, &target)?;
                drop(guard);
                return Ok(target);
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::new(io::ErrorKind::AlreadyExists, "too many files with this name"))
}

struct RemoveOnDrop(PathBuf);

impl Drop for RemoveOnDrop {
    fn drop(&mut self) {
        // already renamed away after a successful upload
        let _ = std::fs::remove_file(&self.0);
    }
}

fn numbered(name: &str, attempt: u32) -> String {
    if attempt == 0 {
        return name.to_string();
    }
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => format!("{stem} ({attempt}).{ext}"),
        _ => format!("{name} ({attempt})"),
    }
}

/// A plain file name that is safe on every platform.
fn clean_name(raw: &str) -> String {
    let last = raw.rsplit(['/', '\\']).next().unwrap_or("");
    let kept: String = last.chars().filter(|c| !c.is_control()).map(|c| if "<>:\"|?*".contains(c) { '_' } else { c }).collect();
    let mut name = kept.trim_start_matches(['.', ' ']).trim_end_matches(['.', ' ']).to_string();
    // leave room for a " (999)" suffix within the usual 255-byte limit
    while name.len() > 200 {
        let cut = name.rfind('.').filter(|i| name.len() - i <= 16);
        match cut {
            Some(dot) => {
                let ext = name.split_off(dot);
                name.pop();
                name.push_str(&ext);
            }
            None => {
                name.pop();
            }
        }
    }
    if name.is_empty() { "file".into() } else { name }
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let hex = (b[i] == b'%').then(|| b.get(i + 1..i + 3)).flatten();
        match hex.and_then(|h| std::str::from_utf8(h).ok()).and_then(|h| u8::from_str_radix(h, 16).ok()) {
            Some(v) => {
                out.push(v);
                i += 3;
            }
            None => {
                out.push(b[i]);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn percent_encode(s: &str) -> String {
    s.bytes()
        .map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") })
        .collect()
}

/// Lets the device open common files with the right app.
fn content_type(name: &str) -> &'static str {
    match name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).as_deref() {
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("png") => "image/png",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("pdf") => "application/pdf",
        Some("mp4") => "video/mp4",
        Some("mp3") => "audio/mpeg",
        Some("txt") => "text/plain; charset=utf-8",
        Some("zip") => "application/zip",
        Some("apk") => "application/vnd.android.package-archive",
        _ => "application/octet-stream",
    }
}

pub fn human_size(bytes: u64) -> String {
    let mut v = bytes as f64;
    for unit in ["B", "KB", "MB", "GB"] {
        if v < 1024.0 || unit == "GB" {
            return if unit == "B" { format!("{bytes} B") } else { format!("{v:.1} {unit}") };
        }
        v /= 1024.0;
    }
    unreachable!()
}

fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn file_rows(files: &Files) -> String {
    let list = files.lock().unwrap();
    if list.files().is_empty() {
        return "<p class=\"none\">Nothing yet. Drag files onto the receiver window.</p>".into();
    }
    list.files()
        .iter()
        .map(|f| {
            format!(
                "<a class=\"file\" href=\"/f/{}\" download><b>{}</b><span>{}</span></a>",
                f.id,
                escape_html(&f.name),
                human_size(f.size)
            )
        })
        .collect()
}

const PAGE: &str = r#"<!doctype html>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Shared files</title>
<style>
body{font:16px system-ui,sans-serif;background:#14171c;color:#e8eaed;max-width:640px;margin:0 auto;padding:16px}
h2{font-size:18px;margin:24px 0 8px}
.file{display:flex;justify-content:space-between;gap:12px;background:#222732;color:inherit;text-decoration:none;padding:14px;border-radius:10px;margin:8px 0;overflow-wrap:anywhere}
.file span,.none,#status{color:#9aa3b2;white-space:nowrap}
label{display:block;background:#2f6fed;text-align:center;padding:14px;border-radius:10px;font-weight:600}
input{display:none}
#status{white-space:normal}
</style>
<h2>From the computer</h2>
<div id="list"><!--files--></div>
<h2>To the computer</h2>
<label>Choose files to send<input id="pick" type="file" multiple></label>
<p id="status">They are saved in Downloads/Miracast on the computer.</p>
<script>
const list = document.getElementById('list'), status = document.getElementById('status');
setInterval(async () => { try { const r = await fetch('/list'); if (r.ok) list.innerHTML = await r.text(); } catch (e) {} }, 3000);
function send(file, label) {
  return new Promise((ok, fail) => {
    const x = new XMLHttpRequest();
    x.open('PUT', '/u/' + encodeURIComponent(file.name));
    x.upload.onprogress = e => status.textContent = label + ' ' + Math.floor(100 * e.loaded / (e.total || 1)) + '%';
    x.onload = () => x.status === 201 ? ok() : fail();
    x.onerror = fail;
    x.send(file);
  });
}
document.getElementById('pick').onchange = async e => {
  const files = [...e.target.files];
  try {
    for (const [i, f] of files.entries()) await send(f, 'Sending ' + f.name + ' (' + (i + 1) + '/' + files.length + ')');
    status.textContent = 'Sent ' + files.length + ' file(s).';
  } catch (err) { status.textContent = 'Sending failed. Is the cast still connected?'; }
  e.target.value = '';
};
</script>
"#;

#[cfg(test)]
mod tests {
    use super::*;

    async fn request(addr: &str, raw: Vec<u8>) -> String {
        let mut s = TcpStream::connect(addr).await.unwrap();
        s.write_all(&raw).await.unwrap();
        let mut out = Vec::new();
        s.read_to_end(&mut out).await.unwrap();
        String::from_utf8_lossy(&out).into_owned()
    }

    fn put(host: &str, path: &str, body: &str) -> Vec<u8> {
        format!("PUT {path} HTTP/1.1\r\nHost: {host}\r\nContent-Length: {}\r\n\r\n{body}", body.len()).into_bytes()
    }

    #[test]
    fn names_are_made_safe() {
        assert_eq!(clean_name("../../.ssh/authorized_keys"), "authorized_keys");
        assert_eq!(clean_name("..\\..\\boot.ini"), "boot.ini");
        assert_eq!(clean_name(".bashrc"), "bashrc");
        assert_eq!(clean_name(".."), "file");
        assert_eq!(clean_name("a\nb:c?.txt"), "ab_c_.txt");
        let long = clean_name(&format!("{}.jpg", "é".repeat(300)));
        assert!(long.len() <= 200 && long.ends_with(".jpg"));
        assert_eq!(numbered("a.tar.gz", 2), "a.tar (2).gz");
        assert_eq!(percent_decode("my%20photo%C3%A9.jpg%"), "my photoé.jpg%");
    }

    #[tokio::test]
    async fn share_and_receive() {
        let dir = std::env::temp_dir().join(format!("wfd-share-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("hello <1>.txt");
        std::fs::write(&source, "hello phone").unwrap();
        let files = Files::default();
        files.lock().unwrap().add(&source).unwrap();
        assert!(files.lock().unwrap().add(&dir).is_err(), "folders are refused");

        let received = Arc::new(Mutex::new(Vec::new()));
        let log = received.clone();
        let local: IpAddr = "127.0.0.1".parse().unwrap();
        let inbox = dir.join("inbox");
        let server = ShareServer::start(local, local, files.clone(), inbox.clone(), Arc::new(move |p, n| log.lock().unwrap().push((p, n))))
            .await
            .unwrap();
        let host = server.url.strip_prefix("http://").unwrap().to_string();
        let get = |path: &str| format!("GET {path} HTTP/1.1\r\nHost: {host}\r\n\r\n").into_bytes();

        let page = request(&host, get("/")).await;
        assert!(page.starts_with("HTTP/1.1 200") && page.contains("hello &lt;1&gt;.txt"), "{page}");
        let file = request(&host, get("/f/1")).await;
        assert!(file.contains("attachment; filename*=UTF-8''hello%20%3C1%3E.txt") && file.ends_with("hello phone"), "{file}");
        assert!(request(&host, get("/f/2")).await.starts_with("HTTP/1.1 404"));
        assert!(request(&host, get("/f/../../etc/passwd")).await.starts_with("HTTP/1.1 404"));

        // a different host name (DNS rebinding) is refused
        let rebound = b"GET / HTTP/1.1\r\nHost: evil.example\r\n\r\n".to_vec();
        assert!(request(&host, rebound).await.starts_with("HTTP/1.1 403"));
        // cross-site preflight is refused, so other pages can't PUT
        let preflight = format!("OPTIONS /u/x HTTP/1.1\r\nHost: {host}\r\n\r\n").into_bytes();
        assert!(request(&host, preflight).await.starts_with("HTTP/1.1 405"));

        assert!(request(&host, put(&host, "/u/..%2F..%2Fnote.txt", "from phone")).await.starts_with("HTTP/1.1 201"));
        assert!(request(&host, put(&host, "/u/note.txt", "second")).await.starts_with("HTTP/1.1 201"));
        assert_eq!(std::fs::read_to_string(inbox.join("note.txt")).unwrap(), "from phone");
        assert_eq!(std::fs::read_to_string(inbox.join("note (1).txt")).unwrap(), "second");
        assert_eq!(received.lock().unwrap().len(), 2);

        // an interrupted upload leaves nothing behind
        let mut s = TcpStream::connect(&host).await.unwrap();
        s.write_all(format!("PUT /u/half.bin HTTP/1.1\r\nHost: {host}\r\nContent-Length: 100\r\n\r\nabc").as_bytes()).await.unwrap();
        s.shutdown().await.unwrap();
        let mut reply = Vec::new();
        s.read_to_end(&mut reply).await.unwrap();
        assert_eq!(std::fs::read_dir(&inbox).unwrap().count(), 2);

        files.lock().unwrap().remove(1);
        assert!(request(&host, get("/f/1")).await.starts_with("HTTP/1.1 404"));
        drop(server);
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(TcpStream::connect(&host).await.is_err(), "port closed with the session");

        // only the connected device is answered
        let other: IpAddr = "192.0.2.1".parse().unwrap();
        let server = ShareServer::start(local, other, files, inbox, Arc::new(|_, _| {})).await.unwrap();
        let host = server.url.strip_prefix("http://").unwrap().to_string();
        let mut s = TcpStream::connect(&host).await.unwrap();
        let _ = s.write_all(format!("GET / HTTP/1.1\r\nHost: {host}\r\n\r\n").as_bytes()).await;
        let mut out = Vec::new();
        let _ = s.read_to_end(&mut out).await;
        assert!(out.is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }
}
