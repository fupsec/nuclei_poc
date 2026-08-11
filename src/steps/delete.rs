use nuclei_poc::core::hash;
use std::collections::HashMap;
use std::fs;
use walkdir::WalkDir;

fn find_and_remove_duplicates(directory: &str) -> anyhow::Result<()> {
    let mut hash_map: HashMap<String, String> = HashMap::new();
    for entry in WalkDir::new(directory).into_iter().filter_map(|e| e.ok()) {
        if entry.file_type().is_file() {
            let filename = entry.file_name().to_str().unwrap_or_default();
            if filename.ends_with(".yml") || filename.ends_with(".yaml") {
                let path = entry.path();
                let file_hash = hash::hash_file(path)?;
                if let Some(_prev) = hash_map.get(&file_hash) {
                    println!("删除重复文件: {:?}", path);
                    fs::remove_file(path)?;
                } else {
                    hash_map.insert(file_hash, path.to_string_lossy().into_owned());
                }
            }
        }
    }
    Ok(())
}

/// 对目录内 YAML 做内容 hash 去重（统一入口）。
pub fn run(argv: Vec<String>) -> anyhow::Result<()> {
    let base_directory = argv.get(1).map(|s| s.as_str()).unwrap_or("clone-templates");
    find_and_remove_duplicates(base_directory)
}

