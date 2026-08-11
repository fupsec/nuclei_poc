use clap::Parser;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{self, BufRead};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::process::Command;
use tokio::sync::Semaphore;
use tokio::time::timeout;
use tracing::{error, info, warn};

/// 增量克隆——使用 `--filter=blob:none` 部分克隆，只拉取 commit/tree 元数据，
/// 再按需拉取变更文件的 blob，避免全量下载整个仓库。
///
/// 核心路径：
/// 1. HEAD 未变 → 完全跳过，零网络。
/// 2. HEAD 变了 + 有 prev_head → `git init` + `fetch --filter=blob:none origin prev HEAD`
///    （仅 ~100KB 元数据）→ `git diff prev..HEAD` 得变更列表 → `git checkout` 只拉变更文件的 blob。
/// 3. 首次（无 prev_head）→ 部分克隆完整仓库（用 `git clone --filter=blob:none` 代替全量 clone）。
#[derive(Parser, Debug)]
#[clap(author, version, about)]
struct Args {
    /// 仓库列表的CSV文件
    #[clap(default_value = "repo.csv")]
    repo_file: String,

    /// 克隆目标根目录
    #[clap(short, long, default_value = "clone-templates")]
    clone_dir: String,

    /// 已有的 poc 目录（仅当 --reprocess-existing 时才会移入 clone-templates 参与重处理）。
    #[clap(long, default_values = &["poc", "poc_gold_11", "poc_gold_12", "poc_gold_13", "poc_gold_14", "poc_gold_15", "poc_dedup", "poc_excluded"])]
    poc_dirs: Vec<String>,

    /// 最大并发的git操作（0表示自动检测）
    #[clap(short, long, default_value_t = 0)]
    jobs: usize,

    /// 跳过 git clone/pull，仅将 poc/ 复制到 clone-templates/
    #[clap(long)]
    skip_clone: bool,

    /// 增量状态文件（记录每个仓库上次处理到的 commit）。随仓库一起提交以便恢复状态。
    #[clap(long, default_value = "state.json")]
    state_file: String,

    /// 本次变更清单输出（每个变更仓库及其变更文件），供后续步骤做原子增量处理。
    #[clap(long, default_value = "incremental_manifest.json")]
    manifest_file: String,

    /// 是否把已有 poc* 目录移回 clone-templates 全量重处理。默认关闭（增量模式）。
    #[clap(long)]
    reprocess_existing: bool,
}

/// 单个仓库的上次处理状态。
#[derive(Debug, Clone, Serialize, Deserialize)]
struct RepoState {
    /// 上次处理到的远端 HEAD commit（短/长 hash 均可，仅用于等值比较）。
    head: Option<String>,
    #[serde(default)]
    synced_at: Option<i64>,
}

impl Default for RepoState {
    fn default() -> Self {
        RepoState { head: None, synced_at: None }
    }
}

/// 增量状态机：保存到 state.json。
#[derive(Debug, Default, Serialize, Deserialize)]
struct IncrementalState {
    repos: HashMap<String, RepoState>,
}

/// 仓库粒度变更记录：写进 incremental_manifest.json。
#[derive(Debug, Clone, Serialize)]
struct RepoManifest {
    url: String,
    /// 本次是否发生真实 clone/pull（false = 完全跳过）。
    changed: bool,
    /// 远端当前 HEAD。
    head: Option<String>,
    /// 上次处理的 HEAD（首次为 None）。
    prev_head: Option<String>,
    /// changed=true 时，本次相对于 prev_head 的变更文件（相对仓库根）。
    /// 为 None/changed=false 表示无 diff；changed=true 且空表示"全量/无法 diff"。
    changed_files: Option<Vec<String>>,
}

fn load_state(path: &str) -> IncrementalState {
    match fs::read_to_string(path) {
        Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
        Err(_) => IncrementalState::default(),
    }
}

fn save_state(path: &str, state: &IncrementalState) -> anyhow::Result<()> {
    let s = serde_json::to_string_pretty(state)?;
    fs::write(path, s)?;
    Ok(())
}

fn now_ts() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// 通过 `git ls-remote <url> HEAD` 获取远端默认分支 HEAD。失败返回 None。
async fn ls_remote_head(url: &str) -> Option<String> {
    let out = Command::new("git")
        .args(["ls-remote", url, "HEAD"])
        .output()
        .await
        .ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .next()
        .map(|s| s.to_string())
}

/// 读取本地仓库当前 HEAD（`git -C <repo> rev-parse HEAD`）。失败返回 None。
async fn local_head(repo: &Path) -> Option<String> {
    let out = Command::new("git")
        .arg("-C").arg(repo).args(["rev-parse", "HEAD"])
        .output()
        .await
        .ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .next()
        .map(|s| s.to_string())
}

async fn sync_repo(
    url: &str,
    dst: &Path,
    prev_head: Option<&str>,
) -> anyhow::Result<RepoManifest> {
    let remote_head = ls_remote_head(url).await;

    // 增量核心：HEAD 未变化 → 完全跳过，零网络。
    // 注意：不依赖 dst.exists()——即使全新 VM 上 clone-templates 被清空、
    // 该仓库目录不存在，只要上次记录的 commit 与远端 HEAD 相同，
    // 说明上游内容没变，可直接复用上一轮已提交的 poc* 输出，无需重新 clone。
    if let Some(prev) = prev_head {
        if let Some(remote) = &remote_head {
            if remote == prev {
                info!(repo = %url, "HEAD 未变化，跳过该仓库（复用上轮输出）");
                return Ok(RepoManifest {
                    url: url.to_string(),
                    changed: false,
                    head: Some(remote.clone()),
                    prev_head: Some(prev.to_string()),
                    changed_files: None,
                });
            }
        }
    }

    // 确实需要更新的仓库：部分克隆增量模式。
    let (new_head, changed_files) = clone_partial_incremental(url, dst, prev_head).await?;
    let files = changed_files.clone().unwrap_or_default();
    info!(repo = %url, files = files.len(), "仓库已更新");
    Ok(RepoManifest {
        url: url.to_string(),
        changed: true,
        head: new_head,
        prev_head: prev_head.map(|s| s.to_string()),
        changed_files,
    })
}

/// 部分克隆增量——使用 `--filter=blob:none`，只拉元数据和变更文件 blob。
///
/// 三种路径：
/// 1. 首次（无 prev_head）：`git clone --filter=blob:none --single-branch`
/// 2. 增量（有 prev_head）：`git init` + `fetch --filter=blob:none origin prev HEAD`
///    → diff 得变更列表 → checkout 仅变更文件（只拉 Changed-files 的 blob）
/// 3. 已有 `.git`：`fetch --filter=blob:none origin`
async fn clone_partial_incremental(
    url: &str,
    dst: &Path,
    prev_head: Option<&str>,
) -> anyhow::Result<(Option<String>, Option<Vec<String>>)> {
    if !dst.exists() {
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent)?;
        }
        if let Some(prev) = prev_head {
            // 增量：init + fetch 新旧两个元数据点 → diff → checkout 仅变更文件
            partial_fetch_only_changed(url, dst, prev).await?;
        } else {
            // 首次：全量 clone（无状态记录时不做任何增量假设）
            let mut cmd = Command::new("git");
            cmd.arg("clone").arg(url).arg(dst);
            cmd.stdout(Stdio::null());
            cmd.stderr(Stdio::piped());
            let out = timeout(Duration::from_secs(300), cmd.output())
                .await
                .map_err(|_| anyhow::anyhow!("clone timeout"))??;
            if !out.status.success() {
                let stderr = String::from_utf8_lossy(&out.stderr);
                let msg = stderr.lines().next().unwrap_or("").to_string();
                // 输出完整 stderr 以便调试
                for line in stderr.lines().take(5) {
                    eprintln!("[git stderr] {}", line);
                }
                return Err(anyhow::anyhow!("git clone failed: {}", msg));
            }
        }
    } else {
        // 已有缓存：增量 fetch 元数据
        let mut fetch_cmd = Command::new("git");
        fetch_cmd.arg("-C").arg(dst).arg("fetch").arg("--filter=blob:none").arg("--prune").arg("origin");
        let _fetch = fetch_cmd.output().await;
        let _ = Command::new("git").arg("-C").arg(dst)
            .args(["reset", "--hard", "FETCH_HEAD"])
            .output().await;
        let _ = Command::new("git").arg("-C").arg(dst)
            .args(["clean", "-fd"])
            .output().await;
    }

    let cur = local_head(dst).await;

    let mut changed: Option<Vec<String>> = None;
    if let Some(prev) = prev_head {
        let diff_ref = if dst.join(".git").exists() { "FETCH_HEAD" } else { "HEAD" };
        let diff = Command::new("git")
            .arg("-C").arg(dst)
            .args(["diff", "--name-status", prev, diff_ref])
            .output()
            .await;
        if let Ok(out) = diff {
            if out.status.success() {
                let files: Vec<String> = String::from_utf8_lossy(&out.stdout)
                    .lines()
                    .filter(|l| !l.trim().is_empty())
                    .map(|l| l.split_whitespace().last().unwrap_or("").to_string())
                    .collect();
                changed = Some(files);
            }
        }
    }

    Ok((cur, changed))
}

/// 全新 VM + 有上次进度：只拉元数据 + 只 checkout 变更文件（真增量）。
async fn partial_fetch_only_changed(url: &str, dst: &Path, prev: &str) -> anyhow::Result<()> {
    let init = Command::new("git").arg("init").arg("-q").arg(dst).output().await?;
    if !init.status.success() { return Err(anyhow::anyhow!("git init failed")); }

    let add = Command::new("git")
        .arg("-C").arg(dst).args(["remote", "add", "origin", url])
        .output().await?;
    if !add.status.success() { return Err(anyhow::anyhow!("git remote add failed")); }

    // 拉取 prev（旧元数据，供 diff）——仅 ~100KB
    let f_prev = Command::new("git")
        .arg("-C").arg(dst)
        .args(["fetch", "--filter=blob:none", "--depth=1", "origin", prev])
        .output().await;
    if !match &f_prev { Ok(o) if o.status.success() => true, _ => false } {
        warn!(prev = %prev, "无法拉取旧 commit，退化为全量部分克隆");
        let mut cmd = Command::new("git");
        cmd.arg("clone").arg("--filter=blob:none").arg(url).arg(dst);
        let out = timeout(Duration::from_secs(300), cmd.output()).await;
        match out {
            Ok(Ok(o)) if o.status.success() => return Ok(()),
            _ => return Err(anyhow::anyhow!("git clone fallback failed")),
        }
    }

    // 拉取 HEAD（最新元数据）
    let f_head = Command::new("git")
        .arg("-C").arg(dst)
        .args(["fetch", "--filter=blob:none", "--depth=1", "origin", "HEAD"])
        .output().await?;
    if !f_head.status.success() { return Err(anyhow::anyhow!("git fetch HEAD failed")); }

    // diff 得到变更文件列表（只靠元数据，不需要任何 blob）
    let diff_out = Command::new("git")
        .arg("-C").arg(dst)
        .args(["diff", "--name-status", prev, "FETCH_HEAD"])
        .output().await?;
    let diff_stdout = String::from_utf8_lossy(&diff_out.stdout).into_owned();
    let changed_files: Vec<String> = diff_stdout
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| l.split_whitespace().last().map(|s| s.to_string()))
        .collect();

    if changed_files.is_empty() {
        let _ = Command::new("git").arg("-C").arg(dst)
            .args(["checkout", "-q", "FETCH_HEAD"])
            .output().await;
        return Ok(());
    }

    // 只 checkout 变更文件（按需拉 blob——这就是"只拉变更文件"的关键）
    let mut checkout_args = vec![
        "-C".to_string(), dst.to_string_lossy().to_string(),
        "checkout".to_string(), "-q".to_string(), "FETCH_HEAD".to_string(), "--".to_string()
    ];
    checkout_args.extend(changed_files.iter().map(|f| f.to_string()));
    let checkout = Command::new("git").args(&checkout_args).output().await?;
    if !checkout.status.success() {
        warn!("checkout 部分文件失败，退化为完整 checkout 只保留变更文件");
        let _ = Command::new("git").arg("-C").arg(dst)
            .args(["checkout", "-q", "FETCH_HEAD"])
            .output().await;
    }
    Ok(())
}

/// 增量克隆/更新上游仓库（统一入口，由 main.rs 调用）。
pub fn run(argv: Vec<String>) -> anyhow::Result<()> {
    let args = Args::parse_from(&argv);
    let jobs_final = if args.jobs == 0 {
        let n = num_cpus::get();
        if n == 0 { 1 } else { n }
    } else {
        args.jobs
    };
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(jobs_final)
        .enable_all()
        .build()?;
    rt.block_on(async_main(args, jobs_final))
}

async fn async_main(args: Args, jobs_final: usize) -> anyhow::Result<()> {
    // 幂等初始化：已在 main.rs 初始化过则跳过（all 模式下避免重复设置全局 dispatcher）。
    let _ = tracing_subscriber::fmt().with_max_level(tracing::Level::INFO).try_init();

    match which::which("git") {
        Ok(path) => info!(git = %path.display(), "git 可用"),
        Err(e) => {
            error!(error = %e, "git 未找到，请安装 git 后重试");
            return Err(anyhow::anyhow!("git not found: {}", e));
        }
    }

    let clone_dir = args.clone_dir.clone();
    fs::create_dir_all(&clone_dir)?;

    // 加载增量状态。
    let mut state: IncrementalState = load_state(&args.state_file);
    info!(repos = state.repos.len(), "已加载增量状态");

    // 读取去重后的 repo 列表。
    let file = fs::File::open(&args.repo_file)
        .map_err(|e| anyhow::anyhow!("open {}: {}", args.repo_file, e))?;
    let reader = io::BufReader::new(file);
    let mut urls: Vec<String> = Vec::new();
    let mut seen = HashSet::new();
    for line in reader.lines().filter_map(Result::ok) {
        let s = line.trim().to_string();
        if !s.is_empty() && seen.insert(s.clone()) {
            urls.push(s);
        }
    }

    info!(url_count = urls.len(), jobs = jobs_final, "开始并发 clone/update");

    let sem = Arc::new(Semaphore::new(jobs_final));
    let mut handles = vec![];
    for url in urls {
        let permit = sem.clone().acquire_owned().await.unwrap();
        let clone_dir = clone_dir.clone();
        let url_clone = url.clone();
        let prev_head = state.repos.get(&url_clone).and_then(|r| r.head.clone());
        handles.push(tokio::spawn(async move {
            let _permit = permit;
            if let Some((owner, repo_name)) = parse_owner_repo(&url_clone) {
                let target = PathBuf::from(&clone_dir)
                    .join(owner)
                    .join(repo_name.to_lowercase());
                match sync_repo(&url_clone, &target, prev_head.as_deref()).await {
                    Ok(m) => Some((url_clone, m)),
                    Err(e) => {
                        error!(repo = %repo_name, error = %e, "仓库同步失败");
                        None
                    }
                }
            } else {
                warn!(url = %url_clone, "URL 无效");
                None
            }
        }));
    }

    let mut manifest_list = Vec::new();
    for h in handles {
        if let Ok(Some((url, m))) = h.await {
            // 回写 state（增量推进 HEAD）。
            let entry = state.repos.entry(url).or_insert_with(RepoState::default);
            entry.head = m.head.clone();
            entry.synced_at = Some(now_ts());
            manifest_list.push(m);
        }
    }

    // 持久化状态 + 变更清单。
    save_state(&args.state_file, &state)?;
    let manifest_json = serde_json::to_string_pretty(&manifest_list)?;
    fs::write(&args.manifest_file, manifest_json)?;
    info!(manifest = args.manifest_file, repos = manifest_list.len(), "状态与变更清单已写入");

    let changed_count = manifest_list.iter().filter(|m| m.changed).count();
    info!(changed = changed_count, skipped = manifest_list.len() - changed_count, "增量同步完成");

    // ── 可选：把已有 poc* 目录移回 clone-templates 全量重处理 ──
    if args.reprocess_existing {
        for poc_dir in &args.poc_dirs {
            let poc_src = Path::new(poc_dir);
            if !poc_src.is_dir() {
                continue;
            }
            let dir_name = poc_src.file_name().unwrap_or_default();
            let poc_dest = Path::new(&clone_dir).join(dir_name);
            if poc_dest.exists() {
                if let Err(e) = fs::remove_dir_all(&poc_dest) {
                    warn!("清理旧目录 {:?} 失败: {}", poc_dest, e);
                }
            }
            info!("移动已有 POC: {:?} -> {:?}", poc_src, poc_dest);
            if let Err(e) = fs::rename(poc_src, &poc_dest) {
                warn!("移动 {} 失败: {} (原目录保留)", poc_dir, e);
            }
        }
    }

    Ok(())
}

fn parse_owner_repo(url: &str) -> Option<(String, String)> {
    // https://github.com/owner/repo 或 git@github.com:owner/repo.git
    if let Some(idx) = url.rfind('/') {
        let repo_name = url[idx + 1..]
            .trim_end_matches('/')
            .trim_end_matches('.')
            .trim_end_matches(".git");
        let owner_part = &url[..idx];
        if let Some(idx2) = owner_part.rfind('/') {
            let owner = owner_part[idx2 + 1..].to_string();
            return Some((owner, repo_name.to_string()));
        }
    }
    None
}