use anyhow::Context as _;
use clap::{Parser, Subcommand};
use interview_platform::config::{self, PlatformConfig};
use interview_platform::telemetry;
use tokio::io::AsyncBufReadExt as _;

/// AI 面试的服务进程。
#[derive(Debug, Parser)]
#[command(name = "interview-server", version)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// 启动服务（不写子命令时默认做这个）
    Serve,
    /// 只跑数据库迁移
    Migrate,
    /// 把 OpenAPI 文档（JSON）打印到标准输出。前端的接口类型由它生成
    Openapi,
    /// 新建一个 HR 账号。密码从标准输入读一行
    CreateAdmin {
        #[arg(long)]
        email: String,
        /// 显示的名字
        #[arg(long, default_value = "管理员")]
        name: String,
        /// admin 或 recruiter
        #[arg(long, default_value = "admin")]
        role: String,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    if matches!(cli.command, Some(Command::Openapi)) {
        println!("{}", interview_server::serve::openapi_json()?);
        return Ok(());
    }

    config::load_dotenv();
    telemetry::init();
    let config = PlatformConfig::from_env().context("配置不对，对照 .env.example 检查环境变量")?;
    match cli.command.unwrap_or(Command::Serve) {
        Command::Serve => interview_server::serve::serve(config).await,
        Command::Migrate => interview_server::serve::migrate(config).await,
        Command::Openapi => unreachable!("上面已经处理了"),
        Command::CreateAdmin { email, name, role } => {
            eprintln!("输入密码（至少 8 位），回车结束：");
            let mut password = String::new();
            tokio::io::BufReader::new(tokio::io::stdin())
                .read_line(&mut password)
                .await
                .context("读密码失败")?;
            let password = password.trim_end_matches(['\r', '\n']);
            interview_server::serve::create_admin(config, &email, &name, password, &role).await
        }
    }
}
