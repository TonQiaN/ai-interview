//! `tts/worker/run.sh` 用假引擎走一遍：启动、看状态、给后端的环境变量、停掉。
//!
//! 假引擎不加载模型，有 bash、python3 和 curl 的机器都能跑，CI 里也跑。

use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// 一次测试用的环境：自己的目录、自己的端口，结束时把 worker 停掉、目录删掉。
struct Sandbox {
    home: PathBuf,
    index_port: u16,
    breeze_port: u16,
}

impl Sandbox {
    fn new(name: &str) -> Self {
        let home = std::env::temp_dir().join(format!("ai-interview-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&home).expect("建不了临时目录");
        Self {
            home,
            index_port: free_port(),
            breeze_port: free_port(),
        }
    }

    fn run(&self, args: &[&str]) -> Output {
        let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tts/worker/run.sh");
        Command::new("bash")
            .arg(script)
            .args(args)
            .env("AI_INTERVIEW_TTS_HOME", &self.home)
            .env("AI_INTERVIEW_TTS_WORKER_FAKE", "1")
            .env("AI_INTERVIEW_TTS_INDEX_PORT", self.index_port.to_string())
            .env("AI_INTERVIEW_TTS_BREEZE_PORT", self.breeze_port.to_string())
            .env_remove("AI_INTERVIEW_TTS_WORKER_HOST")
            .env_remove("AI_INTERVIEW_TTS_WORKER_TOKEN")
            .output()
            .expect("跑不了 run.sh：这个测试需要 bash")
    }

    /// 跑一条命令，要求它成功，返回标准输出。
    fn ok(&self, args: &[&str]) -> String {
        let output = self.run(args);
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        assert!(
            output.status.success(),
            "run.sh {} 没成功：\n{stdout}\n{}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
        stdout
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        // 中途断言失败也要把 worker 停掉，不留进程
        let _ = self.run(&["stop"]);
        let _ = std::fs::remove_dir_all(&self.home);
    }
}

/// 让系统挑一个空闲端口。拿到以后马上放掉，留给 worker 用。
fn free_port() -> u16 {
    TcpListener::bind(("127.0.0.1", 0))
        .and_then(|listener| listener.local_addr())
        .expect("拿不到空闲端口")
        .port()
}

fn listening(port: u16) -> bool {
    TcpStream::connect(("127.0.0.1", port)).is_ok()
}

#[test]
fn the_script_starts_reports_and_stops_fake_workers() {
    let sandbox = Sandbox::new("worker-script");
    let (index, breeze) = (sandbox.index_port, sandbox.breeze_port);

    // 不写起哪个，只起 IndexTTS 的
    let started = sandbox.ok(&["start"]);
    assert!(started.contains("index 准备好了"), "{started}");
    assert!(listening(index), "start 返回时 worker 应该已经能连上");
    assert!(!listening(breeze));

    // 再起一遍不会多出一个进程；all 把另一个补上
    let again = sandbox.ok(&["start", "all"]);
    assert!(again.contains("index 已经在跑"), "{again}");
    assert!(again.contains("breeze 准备好了"), "{again}");
    assert!(listening(breeze));

    let status = sandbox.ok(&["status"]);
    for expected in [
        "index：在跑",
        "breeze：在跑",
        "\"index2\"",
        "\"breeze2\"",
        "\"waiting\": 0",
    ] {
        assert!(
            status.contains(expected),
            "status 里没有 {expected}：\n{status}"
        );
    }

    assert_eq!(
        sandbox.ok(&["env"]),
        format!(
            "export AI_INTERVIEW_TTS_BACKEND=worker\n\
             export AI_INTERVIEW_TTS_WORKER_URLS=http://127.0.0.1:{index},http://127.0.0.1:{breeze}\n"
        )
    );

    // 只停一个，另一个不受影响
    sandbox.ok(&["stop", "breeze"]);
    assert!(!listening(breeze));
    assert!(listening(index));

    sandbox.ok(&["stop"]);
    assert!(!listening(index), "stop 以后端口上不该还有进程在听");
    let status = sandbox.ok(&["status"]);
    assert!(status.contains("index：没在跑"), "{status}");
    assert!(
        !sandbox.run(&["env"]).status.success(),
        "没有 worker 在跑时 env 应该报错，免得后端被指到一个连不上的地址"
    );
}

#[test]
fn the_script_explains_itself_when_the_command_is_wrong() {
    let sandbox = Sandbox::new("worker-script-usage");
    let output = sandbox.run(&["restart"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stdout).contains("run.sh start"));

    let output = sandbox.run(&["start", "index3"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(!listening(sandbox.index_port));
}
