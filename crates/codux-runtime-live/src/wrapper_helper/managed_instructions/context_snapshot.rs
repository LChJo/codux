use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs::{File, OpenOptions, TryLockError},
    io::{Read, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

pub(super) const MAX_CONTEXT: u64 = 8 * 1024 * 1024;

#[derive(Deserialize, Serialize)]
struct Manifest {
    terminal: String,
    context: String,
}

pub(super) fn read_text(path: &Path, limit: u64) -> Result<String, String> {
    let mut body = String::new();
    File::open(path)
        .and_then(|file| file.take(limit + 1).read_to_string(&mut body))
        .map_err(|_| "Unable to read Managed project instructions")?;
    if body.len() as u64 > limit {
        return Err("Managed project instructions are too large".into());
    }
    Ok(body)
}

pub(super) fn private_file(path: &Path) -> Result<File, String> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path).map_err(|error| error.to_string())
}

pub(super) fn update_config(
    requested_path: &Path,
    empty: &str,
    update: impl FnOnce(&str) -> Result<Option<String>, String>,
) -> Result<(), String> {
    std::fs::create_dir_all(
        requested_path
            .parent()
            .ok_or("Managed configuration has no directory")?,
    )
    .map_err(|error| error.to_string())?;
    // Preserve a user's symlink rather than replacing it with a regular file.
    let path = match std::fs::canonicalize(&requested_path) {
        Ok(path) => path,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if std::fs::symlink_metadata(&requested_path).is_ok() {
                return Err("Managed hooks configuration points to a missing file".into());
            }
            requested_path.to_path_buf()
        }
        Err(error) => return Err(error.to_string()),
    };
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path.with_extension("codux.lock"))
        .map_err(|error| error.to_string())?;
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match lock.try_lock() {
            Ok(()) => break,
            Err(TryLockError::WouldBlock) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(error) => {
                return Err(format!(
                    "Unable to lock Managed hooks configuration: {error}"
                ));
            }
        }
    }
    let original = match File::open(&path) {
        Ok(file) => {
            let mut body = String::new();
            file.take(MAX_CONTEXT + 1)
                .read_to_string(&mut body)
                .map_err(|error| error.to_string())?;
            if body.len() as u64 > MAX_CONTEXT {
                return Err("Managed hooks configuration is too large".into());
            }
            body
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => empty.to_string(),
        Err(error) => return Err(error.to_string()),
    };
    let Some(updated) = update(&original)? else {
        return Ok(());
    };
    let temporary = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut file = private_file(&temporary)?;
        if let Ok(metadata) = std::fs::metadata(&path) {
            file.set_permissions(metadata.permissions())
                .map_err(|error| error.to_string())?;
        }
        file.write_all(updated.as_bytes())
            .and_then(|_| file.sync_all())
            .map_err(|error| error.to_string())?;
        drop(file);
        std::fs::rename(&temporary, &path).map_err(|error| error.to_string())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(temporary);
    }
    result
}

pub(super) fn capture(context_file: &str, context: &str, prefix: &str) -> Result<PathBuf, String> {
    let source = std::fs::canonicalize(context_file)
        .map_err(|_| "Managed project instructions are missing")?;
    let manifest = source
        .parent()
        .ok_or("Managed project instructions have no directory")?
        .join(format!("{prefix}-context-{}.json", uuid::Uuid::new_v4()));
    let content = serde_json::to_vec(&Manifest {
        terminal: std::env::var("DMUX_SESSION_ID").unwrap_or_default(),
        context: context.into(),
    })
    .map_err(|error| error.to_string())?;
    private_file(&manifest)?
        .write_all(&content)
        .map_err(|error| error.to_string())?;
    Ok(manifest)
}

pub(super) fn print_once(
    manifest_env: &str,
    session_field: &str,
    make_output: fn(String) -> Value,
) -> Result<(), String> {
    let path = std::env::var_os(manifest_env).filter(|path| !path.is_empty());
    let Some(path) = path else {
        return std::io::stdout()
            .lock()
            .write_all(b"{}")
            .map_err(|error| error.to_string());
    };
    let path = PathBuf::from(path);
    // JSON quoting can expand an 8 MiB context; retain a bounded 6x envelope.
    let manifest: Manifest = serde_json::from_str(&read_text(&path, MAX_CONTEXT * 6 + 1024)?)
        .map_err(|_| "Invalid Managed instruction snapshot")?;
    if manifest.terminal != std::env::var("DMUX_SESSION_ID").unwrap_or_default()
        || manifest.context.len() as u64 > MAX_CONTEXT
    {
        return Err("Managed instruction snapshot does not match this terminal".into());
    }
    let mut input = String::new();
    std::io::stdin()
        .take(1024 * 1024 + 1)
        .read_to_string(&mut input)
        .map_err(|error| error.to_string())?;
    if input.len() > 1024 * 1024 {
        return Err("Managed hook input is too large".into());
    }
    let input: Value = serde_json::from_str(&input).map_err(|_| "Invalid Managed hook input")?;
    let conversation = input
        .get(session_field)
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty() && id.len() <= 1024)
        .ok_or("Managed conversation identity is missing")?;
    let marker = path.with_extension(format!(
        "{:x}.applied",
        Sha256::digest(conversation.as_bytes())
    ));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    let _marker = match options.open(&marker) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            return std::io::stdout()
                .lock()
                .write_all(b"{}")
                .map_err(|error| error.to_string());
        }
        Err(error) => return Err(error.to_string()),
    };
    let output = make_output(manifest.context);
    let result =
        serde_json::to_writer(std::io::stdout().lock(), &output).map_err(|error| error.to_string());
    if result.is_err() {
        drop(_marker);
        let _ = std::fs::remove_file(marker);
    }
    result
}
