//! Live state shared between the engine (server/client tasks) and whatever shows it: the GUI
//! window, the tray, or nobody (CLI). Also routes pairing questions to the terminal or a dialog.

use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use tokio::sync::oneshot;

use crate::geom::{Screen, Side};
use crate::proto::Os;

/// Who answers "allow this pairing?" questions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Approval {
    Terminal,
    Gui,
}

#[derive(Clone, Debug)]
pub struct ClientInfo {
    /// Identifies the link (a reconnect gets a new one).
    pub conn: u64,
    pub name: String,
    pub os: Os,
    pub addr: IpAddr,
    pub screen: Screen,
    pub side: Side,
    pub since: Instant,
    pub rtt: Rtt,
}

/// Round-trip time of a link in milliseconds, updated by its heartbeat (0 = not measured yet).
#[derive(Clone, Debug, Default)]
pub struct Rtt(Arc<AtomicU32>);

impl Rtt {
    pub fn set(&self, ms: u32) {
        self.0.store(ms.max(1), Ordering::Relaxed);
    }

    pub fn get(&self) -> Option<u32> {
        Some(self.0.load(Ordering::Relaxed)).filter(|v| *v > 0)
    }
}

#[derive(Clone, Debug, Default)]
pub enum LinkState {
    #[default]
    Idle,
    /// Client without a paired server: waiting for the user to pair.
    Unpaired,
    Searching {
        server: String,
        since: Instant,
        error: Option<String>,
    },
    Connected {
        server: String,
        addr: SocketAddr,
        since: Instant,
        rtt: Rtt,
    },
}

/// A server heard on the LAN.
#[derive(Clone, Debug)]
pub struct Discovered {
    pub name: String,
    pub addr: SocketAddr,
    pub key: Vec<u8>,
    pub seen: Instant,
}

#[derive(Clone, Debug)]
pub struct ClipEvent {
    pub text: String,
    pub at: Instant,
}

#[derive(Clone, Debug)]
pub struct Transfer {
    pub label: String,
    pub done: Arc<AtomicU64>,
    pub total: u64,
}

#[derive(Clone, Debug, Default)]
pub struct Status {
    pub running: bool,
    /// Why the engine stopped, if it failed.
    pub error: Option<String>,
    /// Things that limit what works (missing permissions and the like).
    pub warnings: Vec<String>,
    pub local_screen: Option<Screen>,
    /// Server: whether this machine's keyboard and mouse are being captured.
    pub capturing: bool,
    pub clients: Vec<ClientInfo>,
    /// Server: the client that has the cursor.
    pub active: Option<String>,
    pub link: LinkState,
    pub discovered: Vec<Discovered>,
    pub clipboard: Option<ClipEvent>,
    pub transfers: Vec<Transfer>,
    /// A one-off message for the user (pairing finished, ...).
    pub notice: Option<(String, Instant)>,
}

/// A pairing code waiting for the user's verdict.
#[derive(Clone, Debug)]
pub struct PairPrompt {
    pub id: u64,
    pub peer: String,
    pub os: Os,
    pub addr: IpAddr,
    pub code: String,
    /// True on the server (another machine asks to pair); false on the machine that asked.
    pub incoming: bool,
}

type Listener = Arc<dyn Fn() + Send + Sync>;

pub struct Hub {
    /// Signalled when the config file changed in ways a running engine can apply live.
    pub reload: tokio::sync::Notify,
    state: Mutex<Status>,
    listener: Mutex<Option<Listener>>,
    pub approval: Approval,
    prompts: Mutex<Vec<(PairPrompt, Option<oneshot::Sender<bool>>)>>,
    next_prompt: AtomicU64,
}

impl Hub {
    pub fn new(approval: Approval) -> Arc<Hub> {
        Arc::new(Hub { reload: tokio::sync::Notify::new(), state: Mutex::default(), listener: Mutex::new(None), approval, prompts: Mutex::default(), next_prompt: AtomicU64::new(1) })
    }

    /// Called after every change (the GUI uses it to repaint).
    pub fn set_listener(&self, f: Option<Listener>) {
        *self.listener.lock().unwrap() = f;
    }

    fn notify(&self) {
        let l = self.listener.lock().unwrap().clone();
        if let Some(l) = l {
            l();
        }
    }

    pub fn update(&self, f: impl FnOnce(&mut Status)) {
        f(&mut self.state.lock().unwrap());
        self.notify();
    }

    pub fn snapshot(&self) -> Status {
        self.state.lock().unwrap().clone()
    }

    pub fn reset(&self) {
        *self.state.lock().unwrap() = Status::default();
        self.prompts.lock().unwrap().clear();
        self.notify();
    }

    pub fn clipboard_event(&self, text: String) {
        self.update(|s| s.clipboard = Some(ClipEvent { text, at: Instant::now() }));
    }

    /// Registers a running transfer; it disappears from the status when the guard drops.
    pub fn transfer(self: &Arc<Self>, label: String, total: u64) -> TransferGuard {
        let done = Arc::new(AtomicU64::new(0));
        self.update(|s| s.transfers.push(Transfer { label, done: done.clone(), total }));
        TransferGuard { hub: self.clone(), done }
    }

    /// Asks the user to approve a pairing code; resolves to false if the prompt is dismissed.
    pub fn ask_pair(&self, mut prompt: PairPrompt) -> oneshot::Receiver<bool> {
        let (tx, rx) = oneshot::channel();
        prompt.id = self.next_prompt.fetch_add(1, Ordering::Relaxed);
        self.prompts.lock().unwrap().push((prompt, Some(tx)));
        self.notify();
        rx
    }

    /// Open prompts. Ones whose pairing attempt has ended (peer gone, timed out) drop out.
    pub fn pair_prompts(&self) -> Vec<PairPrompt> {
        let mut prompts = self.prompts.lock().unwrap();
        prompts.retain(|(_, tx)| tx.as_ref().is_some_and(|t| !t.is_closed()));
        prompts.iter().map(|(p, _)| p.clone()).collect()
    }

    pub fn answer_pair(&self, id: u64, accept: bool) {
        let mut prompts = self.prompts.lock().unwrap();
        if let Some(i) = prompts.iter().position(|(p, _)| p.id == id) {
            let (_, tx) = prompts.remove(i);
            if let Some(tx) = tx {
                let _ = tx.send(accept);
            }
        }
        drop(prompts);
        self.notify();
    }

}

pub struct TransferGuard {
    hub: Arc<Hub>,
    done: Arc<AtomicU64>,
}

impl TransferGuard {
    pub fn add(&self, n: u64) {
        self.done.fetch_add(n, Ordering::Relaxed);
    }
}

impl Drop for TransferGuard {
    fn drop(&mut self) {
        let done = self.done.clone();
        self.hub.update(|s| s.transfers.retain(|t| !Arc::ptr_eq(&t.done, &done)));
    }
}
