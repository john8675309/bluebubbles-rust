#[test]
#[ignore = "Run using tests/notification_smoke.py under dbus-run-session"]
fn desktop_notification_reaches_session_bus() {
    bluebubbles_linux::push::test_notification().unwrap();
}
