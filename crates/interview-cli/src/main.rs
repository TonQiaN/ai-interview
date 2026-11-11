use anyhow::Result;
use clap::{Parser, Subcommand};
use interview_tts::{ENGINE_ENV_VAR, EngineId};

/// AI 面试。
#[derive(Debug, Parser)]
#[command(name = "ai-interview", version)]
struct Cli {
    /// 用哪个 TTS 引擎：index2、index25、breeze2
    #[arg(long, global = true, env = ENGINE_ENV_VAR, default_value_t = EngineId::DEFAULT)]
    tts_engine: EngineId,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// 语音合成相关
    Tts {
        #[command(subcommand)]
        command: TtsCommand,
    },
}

#[derive(Debug, Subcommand)]
enum TtsCommand {
    /// 列出三个引擎，标出当前选中的那个
    Engines,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Tts {
            command: TtsCommand::Engines,
        } => print!("{}", engines_table(cli.tts_engine)),
    }
    Ok(())
}

fn engines_table(selected: EngineId) -> String {
    let mut out = String::new();
    for id in EngineId::ALL {
        let spec = id.spec();
        let marker = if id == selected { "*" } else { " " };
        let languages: Vec<&str> = spec.languages.iter().map(|l| l.as_str()).collect();
        let license = if spec.non_commercial_only {
            "，权重仅限非商业使用"
        } else {
            ""
        };
        out.push_str(&format!(
            "{marker} {:<8} {}（语言：{}{license}）\n",
            id.as_str(),
            spec.name,
            languages.join(" ")
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn flag_selects_the_engine() {
        let cli =
            Cli::try_parse_from(["ai-interview", "tts", "engines", "--tts-engine", "breeze2"])
                .unwrap();
        assert_eq!(cli.tts_engine, EngineId::Breeze2);
    }

    #[test]
    fn unknown_engine_is_rejected() {
        let result =
            Cli::try_parse_from(["ai-interview", "--tts-engine", "other", "tts", "engines"]);
        assert!(result.is_err());
    }

    #[test]
    fn table_marks_only_the_selected_engine() {
        let table = engines_table(EngineId::Index25);
        let marked: Vec<&str> = table.lines().filter(|line| line.starts_with('*')).collect();
        assert_eq!(marked.len(), 1);
        assert!(marked[0].contains("index25"));
        assert_eq!(table.lines().count(), EngineId::ALL.len());
    }
}
