use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::unix::{
        fs::{MetadataExt, OpenOptionsExt},
        io::AsRawFd,
    },
    path::{Path, PathBuf},
};

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct Lease {
    pub resource: String,
    pub token: String,
    pub uid: u32,
    pub session: String,
    pub mode: String,
    pub expires: DateTime<Utc>,
}
#[derive(Serialize, Deserialize, Default)]
struct Store {
    version: u32,
    leases: Vec<Lease>,
}
fn production_path() -> PathBuf {
    PathBuf::from("/var/lib/hosthog/leases")
}
#[cfg(test)]
static TEST_PATH: std::sync::OnceLock<std::sync::Mutex<Option<PathBuf>>> =
    std::sync::OnceLock::new();
fn store_path() -> Result<PathBuf, String> {
    #[cfg(test)]
    if let Some(p) = TEST_PATH
        .get_or_init(|| std::sync::Mutex::new(None))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
    {
        return Ok(p);
    }
    if let Some(s) = std::env::var_os("HOSTHOG_LEASE_STATE") {
        if unsafe { libc::geteuid() } == 0
            || unsafe { libc::getuid() } != unsafe { libc::geteuid() }
        {
            return Err("invalid".into());
        }
        let p = PathBuf::from(s);
        if !p.is_absolute() {
            return Err("invalid".into());
        }
        return Ok(p);
    }
    Ok(production_path())
}
fn validate(s: &str) -> Result<(), String> {
    if s.is_empty() || s.len() > 256 || s.chars().any(char::is_control) {
        Err("invalid".into())
    } else {
        Ok(())
    }
}
fn open_secure(path: &Path, write: bool, uid: u32) -> Result<File, String> {
    let mut o = OpenOptions::new();
    o.read(true)
        .write(write)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    let f = o.open(path).map_err(|_| "state".to_string())?;
    let m = f.metadata().map_err(|_| "state".to_string())?;
    if !m.is_file() || m.uid() != uid || (m.mode() & 0o077) != 0 {
        return Err("state".into());
    }
    Ok(f)
}
fn with_store<T>(f: impl FnOnce(&mut Store) -> Result<T, String>) -> Result<T, String> {
    let path = store_path()?;
    let parent = path.parent().ok_or("invalid")?;
    let uid = unsafe { libc::geteuid() } as u32;
    if path == production_path() {
        if uid != 0 {
            return Err("state".into());
        }
        fs::create_dir_all(parent).map_err(|_| "state")?;
        let m = fs::metadata(parent).map_err(|_| "state")?;
        if !m.is_dir() || m.uid() != 0 || m.mode() & 0o022 != 0 {
            return Err("state".into());
        }
    } else {
        fs::create_dir_all(parent).map_err(|_| "state")?;
    }
    let lockpath = path.with_extension("lock");
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(&lockpath)
        .map_err(|_| "state")?;
    let lm = lock.metadata().map_err(|_| "state")?;
    if !lm.is_file() || lm.uid() != uid || lm.mode() & 0o077 != 0 {
        return Err("state".into());
    }
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX) } != 0 {
        return Err("state".into());
    }
    let mut store = if path.exists() {
        let mut t = String::new();
        open_secure(&path, false, uid)?
            .read_to_string(&mut t)
            .map_err(|_| "state")?;
        let v: Store = serde_json::from_str(&t).map_err(|_| "corrupt".to_string())?;
        if v.version != 1 {
            return Err("corrupt".into());
        }
        v
    } else {
        Store {
            version: 1,
            leases: vec![],
        }
    };
    store.leases.retain(|x| x.expires > Utc::now());
    let value = f(&mut store)?;
    let tmp = parent.join(format!(
        ".hosthog-leases-{}-{}.tmp",
        std::process::id(),
        random_token()?
    ));
    let mut out = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(&tmp)
        .map_err(|_| "state")?;
    let write_result = out
        .write_all(
            serde_json::to_string(&store)
                .map_err(|_| "state")?
                .as_bytes(),
        )
        .and_then(|_| out.sync_all());
    if write_result.is_err() {
        let _ = fs::remove_file(&tmp);
        return Err("state".into());
    }
    drop(out);
    fs::rename(&tmp, &path).map_err(|_| {
        let _ = fs::remove_file(&tmp);
        "state"
    })?;
    File::open(parent)
        .and_then(|d| d.sync_all())
        .map_err(|_| "state")?;
    Ok(value)
}
fn random_token() -> Result<String, String> {
    let mut b = [0; 32];
    File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut b))
        .map_err(|_| "state")?;
    Ok(b.iter().map(|x| format!("{x:02x}")).collect())
}
pub fn dispatch(
    op: &str,
    resource: &str,
    mode: &str,
    token: &str,
    session: &str,
    seconds: i64,
) -> Result<serde_json::Value, String> {
    validate(resource)?;
    validate(session)?;
    let uid = unsafe { libc::getuid() } as u32;
    with_store(|s| match op {
        "acquire" => {
            if !["shared", "exclusive"].contains(&mode) || seconds <= 0 || seconds > 31_536_000 {
                return Err("invalid".into());
            }
            if s.leases
                .iter()
                .any(|l| l.resource == resource && (l.mode == "exclusive" || mode == "exclusive"))
            {
                return Err("busy".into());
            }
            let l = Lease {
                resource: resource.into(),
                token: random_token()?,
                uid,
                session: session.into(),
                mode: mode.into(),
                expires: Utc::now() + chrono::Duration::seconds(seconds),
            };
            s.leases.push(l.clone());
            Ok(serde_json::json!({"lease":l}))
        }
        "status" => Ok(
            serde_json::json!({"leases":s.leases.iter().filter(|l|l.resource==resource).map(|l| serde_json::json!({"resource":l.resource,"uid":l.uid,"session":l.session,"mode":l.mode,"expires":l.expires})).collect::<Vec<_>>()}),
        ),
        "renew" | "release" => {
            let ix = s
                .leases
                .iter()
                .position(|l| l.resource == resource && l.token == token && l.uid == uid)
                .ok_or("notowner")?;
            if op == "release" {
                s.leases.remove(ix);
                Ok(serde_json::json!({"released":true}))
            } else {
                if seconds <= 0 || seconds > 31_536_000 {
                    return Err("invalid".into());
                }
                s.leases[ix].expires = Utc::now() + chrono::Duration::seconds(seconds);
                Ok(serde_json::json!({"lease":s.leases[ix]}))
            }
        }
        _ => Err("invalid".into()),
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Mutex,
    };
    static SERIAL: Mutex<()> = Mutex::new(());
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    fn isolated<T>(f: impl FnOnce() -> T) -> T {
        let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let root = std::env::temp_dir().join(format!(
            "hosthog-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();
        *TEST_PATH
            .get_or_init(|| std::sync::Mutex::new(None))
            .lock()
            .unwrap() = Some(root.join("leases.json"));
        let r = f();
        *TEST_PATH.get().unwrap().lock().unwrap() = None;
        let _ = fs::remove_dir_all(root);
        r
    }
    fn call(
        op: &str,
        res: &str,
        mode: &str,
        tok: &str,
        secs: i64,
    ) -> Result<serde_json::Value, String> {
        dispatch(op, res, mode, tok, "test-session", secs)
    }
    #[test]
    fn shared_exclusive_and_independent() {
        isolated(|| {
            let a = call("acquire", "a", "shared", "", 300).unwrap();
            call("acquire", "a", "shared", "", 300).unwrap();
            assert_eq!(
                call("acquire", "a", "exclusive", "", 300).unwrap_err(),
                "busy"
            );
            call("acquire", "b", "exclusive", "", 300).unwrap();
            assert_eq!(
                call("acquire", "a", "exclusive", "", 300).unwrap_err(),
                "busy"
            );
            assert!(a["lease"]["token"].as_str().unwrap().len() == 64);
        });
    }
    #[test]
    fn token_uid_and_expiry() {
        isolated(|| {
            let a = call("acquire", "r", "exclusive", "", 300).unwrap();
            let t = a["lease"]["token"].as_str().unwrap();
            assert_eq!(
                call("release", "r", "shared", "bad", 0).unwrap_err(),
                "notowner"
            );
            call("renew", "r", "shared", t, 600).unwrap();
            assert!(call("release", "r", "shared", t, 0).is_ok());
        });
    }
    #[test]
    fn concurrency_single_exclusive() {
        isolated(|| {
            let mut ts = vec![];
            for _ in 0..12 {
                ts.push(std::thread::spawn(|| {
                    call("acquire", "race", "exclusive", "", 300).is_ok()
                }))
            }
            let wins = ts
                .into_iter()
                .map(|t| t.join().unwrap())
                .filter(|x| *x)
                .count();
            assert_eq!(wins, 1);
        });
    }
    #[test]
    fn expired_lease_is_pruned_and_cannot_be_renewed() {
        isolated(|| {
            let v = call("acquire", "expired", "exclusive", "", 1).unwrap();
            let p = store_path().unwrap();
            let mut store: Store = serde_json::from_slice(&fs::read(&p).unwrap()).unwrap();
            store.leases[0].expires = Utc::now() - chrono::Duration::seconds(1);
            fs::write(&p, serde_json::to_vec(&store).unwrap()).unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&p, fs::Permissions::from_mode(0o600)).unwrap();
            }
            let token = v["lease"]["token"].as_str().unwrap().to_owned();
            assert_eq!(
                call("renew", "expired", "exclusive", &token, 10).unwrap_err(),
                "notowner"
            );
            assert!(
                call("status", "expired", "shared", "", 0).unwrap()["leases"]
                    .as_array()
                    .unwrap()
                    .is_empty()
            );
        });
    }
    #[test]
    fn corrupt_state_fails_closed() {
        isolated(|| {
            let p = store_path().unwrap();
            fs::write(&p, "broken").unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&p, fs::Permissions::from_mode(0o600)).unwrap();
            }
            assert_eq!(call("status", "r", "shared", "", 0).unwrap_err(), "corrupt");
        });
    }
    #[test]
    fn json_shape_and_atomic_state() {
        isolated(|| {
            let v = call("acquire", "r", "shared", "", 300).unwrap();
            assert!(v["lease"]["expires"].is_string());
            let p = store_path().unwrap();
            let raw = fs::read_to_string(&p).unwrap();
            assert_eq!(serde_json::from_str::<Store>(&raw).unwrap().version, 1);
            assert_eq!(fs::metadata(p).unwrap().mode() & 0o077, 0);
        });
    }
}
