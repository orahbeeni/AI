//! JNI bridge between the Kotlin app (`com.omar.wifishare.Native`) and the wifishare engine
//! (`wifishare::mobile`). Everything crosses as strings: results and the state are JSON, and a
//! returned `null` means success where only an error can come back.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::{Arc, OnceLock};
use std::time::Instant;

use jni::JNIEnv;
use jni::objects::{JClass, JString};
use jni::sys::{jboolean, jlong, jstring};
use serde_json::{Value, json};
use wifishare::files::{Job, JobKind, JobState};
use wifishare::mobile::{Mobile, Setup};
use wifishare::status::LinkState;

static MOBILE: OnceLock<Arc<Mobile>> = OnceLock::new();

fn mobile() -> anyhow::Result<&'static Arc<Mobile>> {
    MOBILE.get().ok_or_else(|| anyhow::anyhow!("the engine has not started"))
}

fn text(env: &mut JNIEnv, s: &JString) -> String {
    if s.is_null() {
        return String::new();
    }
    env.get_string(s).map(Into::into).unwrap_or_default()
}

fn opt_text(env: &mut JNIEnv, s: &JString) -> Option<String> {
    (!s.is_null()).then(|| text(env, s)).filter(|t| !t.is_empty())
}

fn out(env: &mut JNIEnv, s: String) -> jstring {
    env.new_string(s).map(|s| s.into_raw()).unwrap_or(std::ptr::null_mut())
}

/// Runs `f`, turning a panic into an error string rather than unwinding into the JVM.
fn guard(env: &mut JNIEnv, f: impl FnOnce(&mut JNIEnv) -> anyhow::Result<Option<String>>) -> jstring {
    let result = match catch_unwind(AssertUnwindSafe(|| f(env))) {
        Ok(Ok(None)) => return std::ptr::null_mut(),
        Ok(Ok(Some(s))) => s,
        Ok(Err(e)) => format!("{e:#}"),
        Err(_) => "the engine crashed; see the log".into(),
    };
    out(env, result)
}

/// `{"ok": value}` or `{"error": "..."}`.
fn answer(env: &mut JNIEnv, f: impl FnOnce(&mut JNIEnv) -> anyhow::Result<Value>) -> jstring {
    guard(env, |env| {
        Ok(Some(match f(env) {
            Ok(v) => json!({ "ok": v }).to_string(),
            Err(e) => json!({ "error": format!("{e:#}") }).to_string(),
        }))
    })
}

fn paths(json: &str) -> anyhow::Result<Vec<PathBuf>> {
    Ok(serde_json::from_str::<Vec<String>>(json)?.into_iter().map(PathBuf::from).collect())
}

/// A copy's address, without the tag Android may keep in a pointer's top byte, so it stays a
/// positive number in Kotlin.
fn job_id(job: &Arc<Job>) -> u64 {
    Arc::as_ptr(job) as usize as u64 & ((1 << 56) - 1)
}

fn job_json(job: &Arc<Job>) -> Value {
    let (state, detail) = match job.state() {
        JobState::Running => ("running", String::new()),
        JobState::Waiting(why) => ("waiting", why),
        JobState::Done => ("done", String::new()),
        JobState::Failed(why) => ("failed", why),
        JobState::Cancelled => ("cancelled", String::new()),
    };
    json!({
        "id": job_id(job),
        "kind": if job.kind == JobKind::Download { "download" } else { "upload" },
        "peer": job.peer,
        "what": job.what,
        "place": job.place,
        "done": job.done.load(Ordering::Relaxed),
        "total": job.total.load(Ordering::Relaxed),
        "state": state,
        "detail": detail,
    })
}

fn state_json(m: &Mobile) -> Value {
    let st = m.status();
    let link = match &st.link {
        LinkState::Idle => json!({ "state": "idle" }),
        LinkState::Unpaired => json!({ "state": "unpaired" }),
        LinkState::Searching { server, since, error } => {
            json!({ "state": "searching", "computer": server, "secs": since.elapsed().as_secs(), "error": error })
        }
        LinkState::Connected { server, addr, since, rtt } => json!({
            "state": "connected", "computer": server, "addr": addr.ip().to_string(),
            "secs": since.elapsed().as_secs(), "rtt": rtt.get(),
        }),
    };
    let computers = m.computers();
    let discovered: Vec<Value> = st
        .discovered
        .iter()
        .map(|d| {
            let key = hex_key(&d.key);
            json!({ "name": d.name, "addr": d.addr.to_string(), "paired": computers.iter().any(|c| c.key == key) })
        })
        .collect();
    let prompts: Vec<Value> =
        m.hub.pair_prompts().iter().map(|p| json!({ "id": p.id, "peer": p.peer, "code": p.code, "addr": p.addr.to_string() })).collect();
    let notice = st
        .notice
        .as_ref()
        .filter(|(_, at)| at.elapsed().as_secs() < 8)
        .map(|(text, at)| json!({ "text": text, "id": at.duration_since(*START.get_or_init(Instant::now)).as_millis() as u64 }));
    json!({
        "link": link,
        "computers": computers.iter().map(|c| json!({ "name": c.name, "current": c.current, "lastAddr": c.last_addr })).collect::<Vec<_>>(),
        "discovered": discovered,
        "prompts": prompts,
        "jobs": m.jobs().iter().map(job_json).collect::<Vec<_>>(),
        "notice": notice,
    })
}

/// A fixed point notices are numbered from, so the app shows each one once.
static START: OnceLock<Instant> = OnceLock::new();

fn hex_key(key: &[u8]) -> String {
    key.iter().map(|b| format!("{b:02x}")).collect()
}

/// Starts the engine once per process; later calls only apply the settings.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_omar_wifishare_Native_start(
    mut env: JNIEnv,
    _: JClass,
    dir: JString,
    name: JString,
    storage: JString,
    allow_add: jboolean,
) -> jstring {
    guard(&mut env, |env| {
        let (dir, name, storage) = (text(env, &dir), text(env, &name), opt_text(env, &storage).map(PathBuf::from));
        if let Some(m) = MOBILE.get() {
            m.configure(&name, storage, allow_add != 0)?;
            return Ok(None);
        }
        #[cfg(target_os = "android")]
        android_logger::init_once(
            android_logger::Config::default().with_max_level(log::LevelFilter::Info).with_tag("wifishare"),
        );
        START.get_or_init(Instant::now);
        let m = Mobile::start(Setup { dir: PathBuf::from(dir), name, storage, allow_add: allow_add != 0 })?;
        let _ = MOBILE.set(Arc::new(m));
        log::info!("engine started");
        Ok(None)
    })
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_omar_wifishare_Native_state(mut env: JNIEnv, _: JClass) -> jstring {
    guard(&mut env, |_| Ok(Some(state_json(mobile()?).to_string())))
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_omar_wifishare_Native_pair(mut env: JNIEnv, _: JClass, target: JString) -> jstring {
    guard(&mut env, |env| {
        let target = text(env, &target);
        mobile()?.pair(target.trim().to_string());
        Ok(None)
    })
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_omar_wifishare_Native_answer(mut env: JNIEnv, _: JClass, id: jlong, yes: jboolean) -> jstring {
    guard(&mut env, |_| {
        mobile()?.hub.answer_pair(id as u64, yes != 0);
        Ok(None)
    })
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_omar_wifishare_Native_follow(mut env: JNIEnv, _: JClass, name: JString) -> jstring {
    guard(&mut env, |env| {
        let name = opt_text(env, &name);
        mobile()?.follow(name.as_deref())?;
        Ok(None)
    })
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_omar_wifishare_Native_forget(mut env: JNIEnv, _: JClass, name: JString) -> jstring {
    guard(&mut env, |env| {
        let name = text(env, &name);
        mobile()?.forget(&name)?;
        Ok(None)
    })
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_omar_wifishare_Native_shares(mut env: JNIEnv, _: JClass) -> jstring {
    answer(&mut env, |_| {
        let shares = mobile()?.shares()?;
        Ok(shares.iter().map(|s| json!({ "name": s.name, "write": s.write })).collect())
    })
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_omar_wifishare_Native_list(mut env: JNIEnv, _: JClass, share: JString, path: JString) -> jstring {
    answer(&mut env, |env| {
        let (share, path) = (text(env, &share), text(env, &path));
        let entries = mobile()?.list(&share, &path)?;
        Ok(entries.iter().map(|e| json!({ "name": e.name, "dir": e.dir, "size": e.size, "modified": e.modified })).collect())
    })
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_omar_wifishare_Native_mkdir(mut env: JNIEnv, _: JClass, share: JString, path: JString) -> jstring {
    guard(&mut env, |env| {
        let (share, path) = (text(env, &share), text(env, &path));
        mobile()?.mkdir(&share, &path)?;
        Ok(None)
    })
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_omar_wifishare_Native_download(
    mut env: JNIEnv,
    _: JClass,
    share: JString,
    dir: JString,
    names: JString,
    dest: JString,
) -> jstring {
    guard(&mut env, |env| {
        let (share, dir, names, dest) = (text(env, &share), text(env, &dir), text(env, &names), text(env, &dest));
        let names: Vec<String> = serde_json::from_str(&names)?;
        mobile()?.download(&share, &dir, names, PathBuf::from(dest))?;
        Ok(None)
    })
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_omar_wifishare_Native_upload(mut env: JNIEnv, _: JClass, share: JString, dir: JString, files: JString) -> jstring {
    guard(&mut env, |env| {
        let (share, dir, files) = (text(env, &share), text(env, &dir), text(env, &files));
        mobile()?.upload(&share, &dir, paths(&files)?)?;
        Ok(None)
    })
}

/// Sends files to the computer's inbox; answers `{"ok": job id}`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_omar_wifishare_Native_send(mut env: JNIEnv, _: JClass, files: JString) -> jstring {
    answer(&mut env, |env| {
        let files = text(env, &files);
        let job = mobile()?.send(paths(&files)?)?;
        Ok(json!(job_id(&job)))
    })
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_omar_wifishare_Native_cancel(mut env: JNIEnv, _: JClass, id: jlong) -> jstring {
    guard(&mut env, |_| {
        if let Some(job) = mobile()?.jobs().into_iter().find(|j| job_id(j) == id as u64) {
            job.cancel();
        }
        Ok(None)
    })
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_omar_wifishare_Native_clearFinished(mut env: JNIEnv, _: JClass) -> jstring {
    guard(&mut env, |_| {
        mobile()?.clear_finished();
        Ok(None)
    })
}
