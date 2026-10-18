use sha2::{Digest, Sha256};
use std::{
    io::Read,
    path::{Path, PathBuf},
};

fn option(args: &[String], name: &str) -> Option<String> {
    let mut value = None;
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--" {
            break;
        }
        if args[i] == name && i + 1 < args.len() {
            value = Some(args[i + 1].clone());
            i += 2;
        } else {
            if let Some(v) = args[i].strip_prefix(&format!("{name}=")) {
                value = Some(v.into());
            }
            i += 1;
        }
    }
    value
}
fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}
fn home() -> Option<PathBuf> {
    env("HOME")
        .or_else(|| env("USERPROFILE"))
        .map(PathBuf::from)
}
fn expand(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/").or_else(|| path.strip_prefix("~\\"))
        && let Some(home) = home()
    {
        return home.join(rest);
    }
    PathBuf::from(path)
}
fn user_config(args: &[String]) -> Option<PathBuf> {
    if let Some(path) = option(args, "--config")
        .or_else(|| env("CODEWHALE_CONFIG_PATH"))
        .or_else(|| env("DEEPSEEK_CONFIG_PATH"))
    {
        return Some(expand(path.trim()));
    }
    if let Some(root) = env("CODEWHALE_HOME") {
        return Some(PathBuf::from(root).join("config.toml"));
    }
    let home = home()?;
    let primary = home.join(".codewhale/config.toml");
    let legacy = home.join(".deepseek/config.toml");
    Some(if primary.exists() || !legacy.exists() {
        primary
    } else {
        legacy
    })
}
fn table(path: &Path) -> Result<toml::Table, String> {
    if !path.exists() {
        return Ok(toml::Table::new());
    }
    let mut text = String::new();
    std::fs::File::open(path)
        .map_err(|_| "Unable to open CodeWhale configuration")?
        .take(8 * 1024 * 1024 + 1)
        .read_to_string(&mut text)
        .map_err(|_| "Unable to read CodeWhale configuration")?;
    if text.len() > 8 * 1024 * 1024 {
        return Err("CodeWhale configuration is too large".into());
    }
    text.trim_start_matches('\u{feff}')
        .parse()
        .map_err(|_| "Unable to parse CodeWhale configuration".into())
}
fn instructions(table: &toml::Table) -> Result<Option<Vec<toml::Value>>, String> {
    table
        .get("instructions")
        .map(|value| {
            value
                .as_array()
                .filter(|a| a.iter().all(|v| v.is_str()))
                .cloned()
                .ok_or_else(|| "CodeWhale instructions must be an array of paths".into())
        })
        .transpose()
}
fn write_atomic(path: &Path, content: &[u8]) -> Result<(), String> {
    if std::fs::read(path).is_ok_and(|bytes| bytes == content) {
        return Ok(());
    }
    let temp = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    std::fs::write(&temp, content).map_err(|_| "Unable to write CodeWhale context")?;
    if std::fs::rename(&temp, path).is_err() {
        let _ = std::fs::remove_file(&temp);
        return Err("Unable to publish CodeWhale context".into());
    }
    Ok(())
}
pub(super) fn prepare(
    context_file: &str,
    args: &[String],
    context: &str,
) -> Result<PathBuf, String> {
    let context_file =
        std::fs::canonicalize(context_file).map_err(|_| "Managed context is missing")?;
    let parent = context_file
        .parent()
        .ok_or("Managed context has no parent")?;
    let user = user_config(args)
        .map(|path| table(&path))
        .transpose()?
        .unwrap_or_default();
    let mut inherited = instructions(&user)?;
    if let Some(profile) = option(args, "--profile")
        && let Some(value) = user.get("profiles").and_then(|v| v.get(&profile))
    {
        let profile = value.as_table().ok_or("Invalid CodeWhale profile")?;
        if let Some(paths) = instructions(profile)? {
            inherited = Some(paths);
        }
    }
    let managed_path =
        env("DEEPSEEK_MANAGED_CONFIG_PATH").ok_or("CodeWhale managed configuration is missing")?;
    let mut managed = table(&expand(&managed_path))?;
    let mut paths = instructions(&managed)?.or(inherited).unwrap_or_default();
    // CodeWhale caps each instruction file at 100 KiB. Split large contexts on
    // UTF-8 boundaries instead of silently losing hard rules after that cap.
    if context.len() <= 96 * 1024 {
        paths.push(toml::Value::String(
            dunce::simplified(&context_file).to_string_lossy().into(),
        ));
    } else {
        let digest = format!("{:x}", Sha256::digest(context.as_bytes()));
        let mut remaining = context;
        let mut index = 0;
        while !remaining.is_empty() {
            let mut end = remaining.len().min(96 * 1024);
            while !remaining.is_char_boundary(end) {
                end -= 1;
            }
            let path = parent.join(format!("codewhale-context-{}-{index}.md", &digest[..16]));
            write_atomic(&path, remaining[..end].as_bytes())?;
            paths.push(toml::Value::String(
                dunce::simplified(&path).to_string_lossy().into(),
            ));
            remaining = &remaining[end..];
            index += 1;
        }
    }
    managed.insert("instructions".into(), toml::Value::Array(paths));
    let text = toml::to_string(&managed).map_err(|_| "Unable to encode CodeWhale configuration")?;
    let digest = format!("{:x}", Sha256::digest(text.as_bytes()));
    let output = parent.join(format!("codewhale-{}.toml", &digest[..16]));
    write_atomic(&output, text.as_bytes())?;
    Ok(dunce::simplified(&output).to_path_buf())
}
