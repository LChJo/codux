fn main() {
    #[cfg(windows)]
    embed_app_icon();
    deploy_windows_conpty();
}

#[cfg(windows)]
fn embed_app_icon() {
    let icon = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("runtime-assets")
        .join("icons")
        .join("icon.ico");
    if icon.is_file() {
        let mut resource = winresource::WindowsResource::new();
        resource.set_icon(&icon.display().to_string());
        let _ = resource.compile();
    }
}

/// Deploy the vendored Windows Terminal ConPTY pair (conpty.dll +
/// OpenConsole.exe, Microsoft-signed, MIT) next to every binary we build.
///
/// portable-pty prefers a `conpty.dll` sitting beside the executable
/// (LoadLibrary searches the application directory first) and only falls back
/// to the in-box `kernel32` ConPTY when it is absent. The in-box version is
/// frozen with the OS — older Windows builds carry scrollback-loss/ghosting
/// bugs and a cursor re-serialization artifact — so the desktop app and the
/// agent (they share the profile dir in dev builds) must both find the
/// vendored pair or they silently regress to the old ConPTY.
fn deploy_windows_conpty() {
    const SOURCES: [&str; 2] = [
        "resources/conpty/x64/conpty.dll",
        "resources/conpty/x64/OpenConsole.exe",
    ];
    for source in SOURCES {
        println!("cargo:rerun-if-changed={source}");
    }

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    // Only x64 binaries are vendored; other targets keep the kernel32 fallback.
    if std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() != Ok("x86_64") {
        println!(
            "cargo:warning=no vendored ConPTY for this arch; falling back to in-box ConPTY"
        );
        return;
    }

    let manifest_dir = std::path::PathBuf::from(
        std::env::var_os("CARGO_MANIFEST_DIR")
            .expect("Cargo did not provide CARGO_MANIFEST_DIR"),
    );
    let out_dir =
        std::path::PathBuf::from(std::env::var_os("OUT_DIR").expect("Cargo did not provide OUT_DIR"));
    let profile_dir = out_dir
        .ancestors()
        .nth(3)
        .expect("unexpected Cargo OUT_DIR layout");
    // Test executables run from deps/, so they need the pair as well.
    let deps_dir = profile_dir.join("deps");
    std::fs::create_dir_all(&deps_dir).expect("failed to create deps dir");

    for source in SOURCES {
        let source = manifest_dir.join(source);
        let file_name = source.file_name().expect("conpty source has a file name");
        for dir in [profile_dir, deps_dir.as_path()] {
            deploy_conpty_file(&source, &dir.join(file_name));
        }
    }
}

/// Copy `source` to `destination`, tolerating a destination that is mapped
/// into a live process (running codux instances keep conpty.dll and
/// OpenConsole.exe locked). A locked destination is renamed aside — allowed
/// on Windows even for mapped images — so the copy finds an unlocked,
/// overwritable path.
fn deploy_conpty_file(source: &std::path::Path, destination: &std::path::Path) {
    assert!(
        source.exists(),
        "vendored ConPTY binary missing: {}",
        source.display()
    );

    sweep_parked_files(destination);

    let locked = destination.exists()
        && std::fs::OpenOptions::new()
            .write(true)
            .open(destination)
            .is_err();
    if locked {
        park_locked_file(destination);
    } else {
        let up_to_date = matches!(
            (std::fs::read(source), std::fs::read(destination)),
            (Ok(new), Ok(existing)) if new == existing
        );
        if up_to_date {
            return;
        }
    }

    std::fs::copy(source, destination).unwrap_or_else(|error| {
        panic!(
            "failed to copy {} to {}: {error}",
            source.display(),
            destination.display()
        )
    });
}

/// Rename a mapped file out of the way. The parked copy stays mapped by the
/// running process and becomes sweepable garbage once that process exits.
fn park_locked_file(destination: &std::path::Path) {
    for attempt in 0..16 {
        let parked = parked_path(destination, attempt);
        if std::fs::rename(destination, &parked).is_ok() {
            return;
        }
    }
    panic!(
        "failed to move locked {} aside; close running codux instances and rebuild",
        destination.display()
    );
}

/// Delete leftover parked copies whose owning processes have exited.
fn sweep_parked_files(destination: &std::path::Path) {
    for attempt in 0..16 {
        let _ = std::fs::remove_file(parked_path(destination, attempt));
    }
}

fn parked_path(destination: &std::path::Path, attempt: u32) -> std::path::PathBuf {
    let mut name = destination.as_os_str().to_os_string();
    name.push(format!(".stale-{attempt}"));
    std::path::PathBuf::from(name)
}
