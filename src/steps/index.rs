//! Stage 7 — Generate `docs/_categories.json` + `docs/index.html`.
//!
//! Reads `poc.txt` for `poc/`, walks `poc_dedup/` and `poc_gold_*/` on disk,
//! and writes a single `_categories.json` with per-tier category counts.
//! `index.html` is the static homepage and is copied verbatim (not generated).

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, BufWriter};
use std::path::Path;

use clap::Parser;
use rayon::prelude::*;
use serde::Serialize;
use walkdir::WalkDir;

const OUT_DIR: &str = "docs";

fn discover_gold_dirs(repo_root: &Path) -> Vec<String> {
    let mut dirs: Vec<String> = fs::read_dir(repo_root)
        .ok().into_iter().flatten()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.starts_with("poc_gold_") && n[9..].chars().all(|c| c.is_ascii_digit()))
        .collect();
    dirs.sort();
    dirs
}

#[derive(Parser, Debug)]
#[command(name = "index", about = "Generate docs/_categories.json")]
struct Args {
    #[arg(long, default_value = "poc.txt")]
    poc_txt: String,
    #[arg(long, default_value = ".")]
    repo_root: String,
}

fn ensure_dir(path: &Path) -> anyhow::Result<()> {
    fs::create_dir_all(path)?;
    Ok(())
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> anyhow::Result<()> {
    let file = File::create(path)?;
    let writer = BufWriter::new(file);
    serde_json::to_writer(writer, value)?;
    Ok(())
}

fn is_yaml(fname: &str) -> bool {
    let lower = fname.to_lowercase();
    lower.ends_with(".yaml") || lower.ends_with(".yml")
}

fn parse_poc_txt(txt_path: &Path) -> anyhow::Result<BTreeMap<String, Vec<String>>> {
    let file = File::open(txt_path)?;
    let reader = BufReader::new(file);
    let mut cats: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for line in reader.lines() {
        let line = line?;
        let line = line.trim();
        if line.is_empty() { continue; }
        if let Some((cat, fname)) = line.split_once('/') {
            if is_yaml(fname) {
                cats.entry(cat.to_string()).or_default().push(fname.to_string());
            }
        }
    }
    Ok(cats)
}

fn scan_fs_dir(dir: &Path) -> BTreeMap<String, Vec<String>> {
    let entries: Vec<_> = WalkDir::new(dir)
        .into_iter().filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file() && e.path().extension().map_or(false, |ext| ext.eq_ignore_ascii_case("yaml") || ext.eq_ignore_ascii_case("yml")))
        .collect();
    let pairs: Vec<(String, String)> = entries.par_iter().filter_map(|entry| {
        let path = entry.path();
        let rel = path.strip_prefix(dir).ok()?;
        let cat = rel.parent().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "_root".to_string());
        let fname = path.file_name()?.to_string_lossy().to_string();
        Some((cat, fname))
    }).collect();
    let mut cats: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (cat, fname) in pairs {
        cats.entry(cat).or_default().push(fname);
    }
    for files in cats.values_mut() { files.sort(); }
    cats
}

/// 生成 GitHub Pages 浏览器索引（统一入口）。
pub fn run(argv: Vec<String>) -> anyhow::Result<()> {
    let args = Args::parse_from(&argv);
    let root = Path::new(&args.repo_root);
    let out_dir = root.join(OUT_DIR);
    ensure_dir(&out_dir)?;

    let mut all_data: BTreeMap<String, BTreeMap<String, Vec<String>>> = BTreeMap::new();

    // 1. poc/ from poc.txt
    let poc_txt = root.join(&args.poc_txt);
    if poc_txt.exists() {
        let cats = parse_poc_txt(&poc_txt)?;
        let count: usize = cats.values().map(|v| v.len()).sum();
        println!("  poc/: {} files across {} categories", count, cats.len());
        all_data.insert("poc".to_string(), cats);
    } else {
        eprintln!("  [SKIP] {} not found", args.poc_txt);
    }

    // 2. poc_dedup/
    let dedup_dir = root.join("poc_dedup");
    if dedup_dir.is_dir() {
        let cats = scan_fs_dir(&dedup_dir);
        let count: usize = cats.values().map(|v| v.len()).sum();
        println!("  poc_dedup/: {} files across {} categories", count, cats.len());
        all_data.insert("poc_dedup".to_string(), cats);
    }

    // 3. poc_gold_*
    for dir_name in discover_gold_dirs(root) {
        let dir = root.join(&dir_name);
        if !dir.is_dir() { continue; }
        let cats = scan_fs_dir(&dir);
        let count: usize = cats.values().map(|v| v.len()).sum();
        println!("  {}/: {} files across {} categories", dir_name, count, cats.len());
        all_data.insert(dir_name, cats);
    }

    if all_data.is_empty() {
        eprintln!("WARNING: No POC files found. _categories.json will not be updated.");
        return Ok(());
    }

    // Convert to tier → { cat → count } and write
    let mut all_cats: BTreeMap<String, BTreeMap<String, usize>> = BTreeMap::new();
    for (tier, cats) in &all_data {
        let summary: BTreeMap<String, usize> = cats.iter().map(|(k, v)| (k.clone(), v.len())).collect();
        all_cats.insert(tier.clone(), summary);
    }

    // Clean old per-tier chunk files (generated by previous versions), keep _categories.json + index.html
    if out_dir.exists() {
        for entry in fs::read_dir(&out_dir)? {
            let entry = entry?;
            let path = entry.path();
            let fname = path.file_name().unwrap_or_default().to_string_lossy().to_string();
            if fname == "_categories.json" || fname == "index.html" { continue; }
            if path.is_file() && path.extension().map_or(false, |ext| ext == "json") {
                fs::remove_file(&path).ok();
            }
        }
    }

    write_json(&out_dir.join("_categories.json"), &all_cats)?;
    println!("\nDone! docs/ → _categories.json only");

    Ok(())
}
