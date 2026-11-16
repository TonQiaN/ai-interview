//! 评测里候选人的话先过一遍「耳朵」：读出来再识别回去，引擎拿到的是带识别错误的文字。
//!
//! 设了环境变量 `AI_INTERVIEW_EVAL_HEARD_CMD` 才会这样做，值是一条命令：从标准输入一行读一个 JSON
//! 字符串，往标准输出对应写一行 JSON 字符串。`scripts/asr_roundtrip.py` 就是这样一条命令，怎么装见
//! 它开头的说明。没设的时候候选人写的字原样交给引擎。

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use anyhow::{Context, Result, bail};

const COMMAND_ENV: &str = "AI_INTERVIEW_EVAL_HEARD_CMD";

/// 开着耳朵时加在扮演候选人的提示词后面，让模型写出来的话像口头说的，而不是书面的。
const SPOKEN_STYLE: &str = "\n\n你是在口头说话，不是写文章：会带「嗯」「那个」「就是说」这类口头语（说英语时是 um、you know、I mean），偶尔说到一半改口重说。想词或者回忆的时候会停一下，停的地方写「……」。数字按嘴上说的写，不分点，不用括号。";

struct Process {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

pub struct Ears {
    process: Option<Process>,
}

impl Ears {
    /// 环境变量没设就什么都不做，[`Ears::hear`] 原样返回。
    pub fn from_env() -> Result<Self> {
        let Some(command) = std::env::var(COMMAND_ENV)
            .ok()
            .filter(|command| !command.trim().is_empty())
        else {
            return Ok(Self { process: None });
        };
        let mut child = Command::new("sh")
            .arg("-c")
            .arg(&command)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .with_context(|| format!("起不来 {COMMAND_ENV} 指的命令"))?;
        let stdin = child.stdin.take().context("拿不到命令的标准输入")?;
        let stdout = BufReader::new(child.stdout.take().context("拿不到命令的标准输出")?);
        println!("候选人的话会先读出来再识别回去（{COMMAND_ENV}）\n");
        Ok(Self {
            process: Some(Process {
                child,
                stdin,
                stdout,
            }),
        })
    }

    /// 加在扮演候选人的提示词后面的话。没开耳朵时是空的，评测和以前一样。
    pub fn style(&self) -> &'static str {
        if self.process.is_some() {
            SPOKEN_STYLE
        } else {
            ""
        }
    }

    /// 候选人说了 `said`，识别出来是什么。什么都没识别出来时返回空字符串。
    pub fn hear(&mut self, said: &str) -> Result<String> {
        let Some(process) = &mut self.process else {
            return Ok(said.to_owned());
        };
        writeln!(process.stdin, "{}", serde_json::to_string(said)?)?;
        process.stdin.flush()?;
        let mut line = String::new();
        if process.stdout.read_line(&mut line)? == 0 {
            let status = process.child.wait()?;
            bail!("{COMMAND_ENV} 指的命令退出了（{status}）");
        }
        let heard: String = serde_json::from_str(line.trim())
            .with_context(|| format!("{COMMAND_ENV} 指的命令输出的不是 JSON 字符串"))?;
        Ok(heard.trim().to_owned())
    }
}

impl Drop for Ears {
    fn drop(&mut self) {
        if let Some(mut process) = self.process.take() {
            drop(process.stdin);
            let _ = process.child.wait();
        }
    }
}
