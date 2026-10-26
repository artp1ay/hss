use std::io::Write;
use std::process::{Command, Stdio};

#[test]
fn verbose_stdio_logs_to_stderr_without_corrupting_protocol() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_hss"))
        .args(["--mcp-stdio", "-v"])
        .env("HSS_KEYSTORE", "file")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    writeln!(
        stdin,
        "{{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\"}}"
    )
    .unwrap();
    drop(stdin);
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    let response: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
    assert_eq!(response["id"], 1);
    assert_eq!(response["result"]["serverInfo"]["name"], "hss");
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("credential storage: file"));
    assert!(stderr.contains("ready; waiting"));
    assert!(stderr.contains("received initialize"));
    assert!(stderr.contains("finished initialize"));
    assert!(stderr.contains("stdin closed; exiting"));
}
