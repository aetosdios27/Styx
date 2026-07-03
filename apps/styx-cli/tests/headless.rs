use styx_app::MemoryRuntime;
use styx_cli::headless::{run_headless_once, HeadlessOptions};

#[test]
fn headless_once_writes_json_lines() {
    let runtime = MemoryRuntime::default();
    let mut output = Vec::new();

    run_headless_once(runtime, &mut output, HeadlessOptions::default()).unwrap();

    let text = String::from_utf8(output).unwrap();
    let lines = text.lines().collect::<Vec<_>>();
    assert_eq!(lines.len(), 2);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(lines[0]).unwrap()["type"],
        "daemon_started"
    );
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(lines[1]).unwrap()["type"],
        "snapshot"
    );
}
