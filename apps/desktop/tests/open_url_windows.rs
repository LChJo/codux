#![cfg(target_os = "windows")]

use gpui::AppContext;
use std::cell::Cell;
use std::process::Command;
use std::rc::Rc;

/// ShellExecuteExW can dispatch Windows messages while a GPUI callback still
/// holds the AppCell borrow. A runtime-only browser probe cannot catch this.
#[test]
#[ignore = "runs native GPUI and opens the default browser; use an interactive Windows desktop"]
fn opening_a_url_does_not_reenter_gpui() {
    // GPUI's native Windows event loop ends with ExitProcess, and a nested
    // panic aborts at the Win32 boundary. Keep both outcomes in a child so the
    // test runner can check the exit status AND the assertions inside GPUI.
    if std::env::var_os("CODUX_TEST_GPUI_URL_CHILD").is_none() {
        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--ignored",
                "--exact",
                "opening_a_url_does_not_reenter_gpui",
                "--nocapture",
            ])
            .env("CODUX_TEST_GPUI_URL_CHILD", "1")
            .output()
            .expect("run the native GPUI probe in a separate process");
        assert!(
            output.status.success(),
            "GPUI URL probe crashed: {}\n{}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains("GPUI_URL_PROBE_OK"));
        return;
    }

    std::thread::spawn(|| {
        std::thread::sleep(std::time::Duration::from_secs(30));
        eprintln!("GPUI URL probe timed out");
        std::process::exit(1);
    });
    let callback_ran = Rc::new(Cell::new(false));
    gpui_platform::application().run(move |cx| {
        let entity = cx.new(|_| ());
        // Queue a real foreground update before launching the browser. It must
        // run after this callback releases its mutable GPUI application borrow.
        let observed_callback = callback_ran.clone();
        cx.spawn(async move |cx| {
            entity.update(cx, |_, _| observed_callback.set(true));
        })
        .detach();

        codux_runtime::app_info::open_url(concat!(
            "http://127.0.0.1:1/#code=0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            "&worktree=11111111-2222-3333-4444-555555555555",
            "&task=aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee"
        ))
        .expect("launch the default browser from inside a GPUI callback");

        cx.spawn(async move |cx| {
            cx.update(|cx| {
                assert!(callback_ran.get(), "the queued GPUI update must run");
                println!("GPUI_URL_PROBE_OK");
                cx.quit();
            });
        })
        .detach();
    });
}
