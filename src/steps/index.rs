use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::Path;

use clap::Parser;
use rayon::prelude::*;
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
#[command(name = "index", about = "Generate docs/index.html + _categories.json")]
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

/// Inject JSON data into index.html by replacing the line after `const A = JSON.parse('`.
fn inject_data_into_html(html_path: &Path, json_data: &str) -> anyhow::Result<()> {
    let content = fs::read_to_string(html_path)?;
    if let Some(pos) = content.find("const A = JSON.parse('") {
        let before = &content[..pos];
        let after_marker = &content[pos + "const A = JSON.parse('".len()..];
        // Find the end of the JSON string: `');`
        let end = match after_marker.find("');\n") {
            Some(p) => p + 3,
            None => after_marker.find("');").unwrap_or(after_marker.len()) + 2,
        };
        let rest = &after_marker[end..];
        let new_content = format!("{}const A = JSON.parse('{}');{}", before, json_data.replace('\'', "\\'"), rest);
        fs::write(html_path, new_content)?;
        println!("  index.html: data injected ({}/{} chars replaced)", json_data.len(), json_data.len());
    } else {
        println!("  index.html: marker not found, appending data as separate _categories.json");
        fs::write(html_path.with_file_name("_categories.json"), json_data)?;
    }
    Ok(())
}

pub fn run(argv: Vec<String>) -> anyhow::Result<()> {
    let args = Args::parse_from(&argv);
    let root = Path::new(&args.repo_root);
    let out_dir = root.join(OUT_DIR);
    ensure_dir(&out_dir)?;

    let mut all_data: BTreeMap<String, BTreeMap<String, Vec<String>>> = BTreeMap::new();

    let poc_txt = root.join(&args.poc_txt);
    if poc_txt.exists() {
        let cats = parse_poc_txt(&poc_txt)?;
        let count: usize = cats.values().map(|v| v.len()).sum();
        println!("  poc/: {} files across {} categories", count, cats.len());
        all_data.insert("poc".to_string(), cats);
    }

    let dedup_dir = root.join("poc_dedup");
    if dedup_dir.is_dir() {
        let cats = scan_fs_dir(&dedup_dir);
        let count: usize = cats.values().map(|v| v.len()).sum();
        println!("  poc_dedup/: {} files across {} categories", count, cats.len());
        all_data.insert("poc_dedup".to_string(), cats);
    }

    for dir_name in discover_gold_dirs(root) {
        let dir = root.join(&dir_name);
        if !dir.is_dir() { continue; }
        let cats = scan_fs_dir(&dir);
        let count: usize = cats.values().map(|v| v.len()).sum();
        println!("  {}/: {} files across {} categories", dir_name, count, cats.len());
        all_data.insert(dir_name, cats);
    }

    if all_data.is_empty() {
        eprintln!("WARNING: No POC files found.");
        return Ok(());
    }

    let mut all_cats: BTreeMap<String, BTreeMap<String, usize>> = BTreeMap::new();
    for (tier, cats) in &all_data {
        let summary: BTreeMap<String, usize> = cats.iter().map(|(k, v)| (k.clone(), v.len())).collect();
        all_cats.insert(tier.clone(), summary);
    }

    let json_data = serde_json::to_string(&all_cats)?;

    // Clean old per-tier chunk files, keep index.html + _categories.json
    if out_dir.exists() {
        for entry in fs::read_dir(&out_dir)? {
            let entry = entry?;
            let path = entry.path();
            let fname = path.file_name().unwrap_or_default().to_string_lossy().to_string();
            if fname == "index.html" || fname == "_categories.json" { continue; }
            if path.is_file() && path.extension().map_or(false, |ext| ext == "json") {
                fs::remove_file(&path).ok();
            }
        }
    }

    // Inject data into index.html if it exists
    let html_path = out_dir.join("index.html");
    if html_path.exists() {
        inject_data_into_html(&html_path, &json_data)?;
    } else {
        eprintln!("  WARNING: docs/index.html not found, writing _categories.json only");
    }

    // Also write _categories.json for fallback
    fs::write(&out_dir.join("_categories.json"), &json_data)?;

    println!("\nDone! docs/ → index.html (with embedded data) + _categories.json");
    Ok(())
}