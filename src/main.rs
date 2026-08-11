//! Nuclei-POC 统一流水线入口。
//!
//! 将原先 `src/bin/` 下的多个独立二进制合并为单一命令，通过子命令执行各阶段，
//! 并提供 `all` 一键依次运行完整流水线（与旧 workflow 顺序一致）。

mod steps;

use clap::{Parser, Subcommand};

/// Nuclei POC 更新整理工具（统一入口）。
#[derive(Parser, Debug)]
#[command(name = "nuclei_poc", version, about = "Nuclei POC 更新整理工具（统一命令）")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// 增量克隆/更新上游仓库（含 state.json 进度续传）
    Clone {
        /// 透传给步骤的参数（如 --depth 1）
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// 对 clone-templates 内 YAML 按内容 hash 去重
    Delete {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// 分类归档 + 预过滤非 nuclei 文件
    Move {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// nuclei 结构校验（通过→poc/，未通过→poc_needs_review/）
    Check {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// 高级去重：poc/ → poc_dedup/
    Dedup {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// 精品分级：poc_dedup/ → poc_gold_*
    Quality {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// 生成 GitHub Pages 浏览器索引 → docs/
    Index {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// 依次执行完整流水线（clone → delete → move → check → dedup → quality → index）
    All,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let prog = env_prog_name();

    match cli.command {
        Command::Clone { args } => steps::clone::run(prepend(prog, args)),
        Command::Delete { args } => steps::delete::run(prepend(prog, args)),
        Command::Move { args } => steps::move_file::run(prepend(prog, args)),
        Command::Check { args } => steps::check::run(prepend(prog, args)),
        Command::Dedup { args } => steps::dedup::run(prepend(prog, args)),
        Command::Quality { args } => steps::quality::run(prepend(prog, args)),
        Command::Index { args } => steps::index::run(prepend(prog, args)),
        Command::All => run_all(prog),
    }
}

/// 完整流水线：与旧 GitHub Actions 工作流步骤一致。
fn run_all(prog: String) -> anyhow::Result<()> {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .try_init();

    // 1. 增量克隆（--filter=blob:none 部分克隆，只拉元数据 + 变更文件 blob）
    println!("\n===== [1/7] clone (部分克隆增量) =====");
    steps::clone::run(vec![prog.clone()])?;

    // 2. 去重
    println!("\n===== [2/7] delete (去重) =====");
    steps::delete::run(vec![prog.clone()])?;

    // 3. 分类移动 + 预过滤
    println!("\n===== [3/7] move (分类) =====");
    steps::move_file::run(vec![prog.clone()])?;

    // 4. nuclei 校验
    println!("\n===== [4/7] check (nuclei 校验) =====");
    steps::check::run(vec![prog.clone()])?;

    // 5. 高级去重
    println!("\n===== [5/7] dedup (poc/ → poc_dedup/) =====");
    steps::dedup::run(vec![
        prog.clone(),
        "--src-dir".into(),
        "poc".into(),
        "--dst-dir".into(),
        "poc_dedup".into(),
        "--threshold".into(),
        "70".into(),
    ])?;

    // 6. 精品分级
    println!("\n===== [6/7] quality (poc_dedup/ → poc_gold_*) =====");
    steps::quality::run(vec![prog.clone()])?;

    // 7. 生成浏览器索引
    println!("\n===== [7/7] index (docs/) =====");
    steps::index::run(vec![prog.clone(), "--repo-root".into(), ".".into()])?;

    Ok(())
}

/// argv[0]（程序名）。
fn env_prog_name() -> String {
    std::env::args().next().unwrap_or_else(|| "nuclei_poc".to_string())
}

/// 在子命令 args 前插入 argv[0]。
fn prepend(prog: String, mut args: Vec<String>) -> Vec<String> {
    args.insert(0, prog);
    args
}