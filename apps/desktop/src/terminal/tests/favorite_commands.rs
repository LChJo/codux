use super::super::*;

#[test]
fn trigger_message_submits_once_without_splitting_multiline_prompt() {
    let mut model = TerminalModel::new_for_test(20, 4, 100);
    model.process_output_bytes_for_test(b"\x1b[?2004h");
    model
        .write_automation_input("first\n第二行", true, true)
        .unwrap();
    assert_eq!(
        model.written_bytes_for_test(),
        "\x1b[200~first\n第二行\x1b[201~\r".as_bytes()
    );
}

#[test]
fn trigger_message_rejects_exited_remote_owned_and_unsupported_multiline() {
    let mut model = TerminalModel::new_for_test(20, 4, 100);
    assert!(
        model
            .write_automation_input("first\nsecond", true, true)
            .is_err()
    );
    assert!(model.write_automation_input("hello", false, true).is_err());
    model.exited = true;
    assert!(model.write_automation_input("hello", true, true).is_err());
    assert!(model.written_bytes_for_test().is_empty());
}

#[test]
fn favorite_commands_plain_right_click_opens_menu_even_with_mouse_tracking() {
    assert!(!terminal_right_click_reports(false, true));
    assert!(!terminal_right_click_reports(false, false));
}

#[test]
fn favorite_commands_shift_right_click_reports_to_mouse_application() {
    assert!(terminal_right_click_reports(true, true));
    assert!(!terminal_right_click_reports(true, false));
}

#[test]
fn favorite_commands_paste_plain_chinese_without_submitting() {
    let state = TerminalModel::new_for_test(20, 4, 100);
    state.paste_favorite_command("请检查代码", true).unwrap();
    assert_eq!(state.written_bytes_for_test(), "请检查代码".as_bytes());
}

#[test]
fn favorite_commands_multiline_requires_live_bracketed_paste() {
    let mut state = TerminalModel::new_for_test(20, 4, 100);
    assert_eq!(
        state.paste_favorite_command("first\nsecond", true),
        Err(FavoriteCommandPasteError::MultilineUnsupported)
    );
    assert!(state.written_bytes_for_test().is_empty());
    state.process_output_bytes_for_test(b"\x1b[?2004h");
    assert!(!state.handle.input_mode().bracketed_paste);
    state.paste_favorite_command("first\n第二行", true).unwrap();
    assert_eq!(
        state.written_bytes_for_test(),
        "\x1b[200~first\n第二行\x1b[201~".as_bytes()
    );
}

#[test]
fn favorite_commands_unavailable_target_receives_no_input() {
    let mut state = TerminalModel::new_for_test(20, 4, 100);
    assert_eq!(
        state.paste_favorite_command("command", false),
        Err(FavoriteCommandPasteError::Unavailable)
    );
    state.exited = true;
    assert_eq!(
        state.paste_favorite_command("command", true),
        Err(FavoriteCommandPasteError::Unavailable)
    );
    assert!(state.written_bytes_for_test().is_empty());
}

#[test]
fn favorite_commands_writer_failure_is_reported_as_unavailable() {
    struct FailedWriter;
    impl std::io::Write for FailedWriter {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::ErrorKind::BrokenPipe.into())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut state = TerminalModel::new_for_test(20, 4, 100);
    state.stdin_writer = Arc::new(Mutex::new(Box::new(FailedWriter)));
    assert_eq!(
        state.paste_favorite_command("command", true),
        Err(FavoriteCommandPasteError::Unavailable)
    );
}

#[test]
fn favorite_commands_first_shift_drag_produces_selected_text() {
    let mut state = TerminalModel::new_for_test(40, 4, 100);
    state.process_bytes(b"command snippet");
    state.handle.publish_snapshot();
    state.extend_selection(TerminalSelectionPoint { line: 0, col: 0 });
    state.update_selection(TerminalSelectionPoint { line: 0, col: 7 });
    assert_eq!(state.selected_text().as_deref(), Some("command"));
}
