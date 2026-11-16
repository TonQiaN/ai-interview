//! 日志。

use std::io::{self, Write};

use tracing_subscriber::EnvFilter;

/// 初始化日志输出。级别用环境变量 `AI_INTERVIEW_LOG` 控制，写法和 `RUST_LOG` 一样，
/// 比如 `info`、`debug`、`interview_room=debug,info`。默认 `info`。
///
/// 重复调用没有副作用（测试里可能多次调）。
pub fn init() {
    let filter = std::env::var("AI_INTERVIEW_LOG")
        .ok()
        .and_then(|raw| EnvFilter::try_new(raw).ok())
        .unwrap_or_else(|| EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(true)
        .with_writer(|| DropOnError(io::stdout()))
        .try_init();
}

/// 写不进去就把内容丢掉的输出。
///
/// 日志写到标准输出。跑服务的终端被关掉（或者 SSH 断开）以后，标准输出和标准错误都写不进去了。
/// 日志库写失败时会往标准错误报一句，那一句也写不进去就会让进程出错，服务来不及正常收尾。
/// 日志丢了可以接受，不能因为写日志把服务弄坏。
struct DropOnError<W>(W);

impl<W: Write> Write for DropOnError<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self.0.write(buf) {
            // 被信号打断的交给调用方重试
            Err(err) if err.kind() != io::ErrorKind::Interrupted => Ok(buf.len()),
            other => other,
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        let _ = self.0.flush();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 终端没了以后的标准输出：怎么写都报错。
    struct Gone;

    impl Write for Gone {
        fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
            Err(io::Error::from(io::ErrorKind::BrokenPipe))
        }

        fn flush(&mut self) -> io::Result<()> {
            Err(io::Error::from(io::ErrorKind::BrokenPipe))
        }
    }

    #[test]
    fn a_log_line_that_cannot_be_written_is_dropped() {
        let mut out = DropOnError(Gone);
        out.write_all("一行日志\n".as_bytes())
            .expect("写不进去也不报错");
        out.flush().expect("刷新也不报错");
    }

    #[test]
    fn what_can_be_written_is_written() {
        let mut out = DropOnError(Vec::new());
        out.write_all(b"ok\n").unwrap();
        assert_eq!(out.0, b"ok\n");
    }
}
