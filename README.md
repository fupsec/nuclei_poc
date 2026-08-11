# Nuclei POC

<a href="https://github.com/adysec/nuclei_poc/stargazers"><img alt="GitHub Repo stars" src="https://img.shields.io/github/stars/adysec/nuclei_poc?color=yellow&logo=riseup&logoColor=yellow&style=flat-square"></a>
<a href="https://github.com/adysec/nuclei_poc/network/members"><img alt="GitHub forks" src="https://img.shields.io/github/forks/adysec/nuclei_poc?color=orange&style=flat-square"></a>
<a href="https://github.com/adysec/nuclei_poc/issues"><img alt="GitHub issues" src="https://img.shields.io/github/issues/adysec/nuclei_poc?color=red&style=flat-square"></a>

Nuclei POC，每日从 400+ 上游仓库自动采集、校验、去重、分级。

[中文](https://github.com/adysec/nuclei_poc/blob/main/README.md) | [English](https://github.com/adysec/nuclei_poc/blob/main/README_EN.md)

## 如何使用

```bash
# 只下载旗舰级 POC（稀疏检出）
git clone --filter=tree:0 --sparse https://github.com/adysec/nuclei_poc
cd nuclei_poc
git sparse-checkout set poc_gold_15

# 扫描目标
nuclei -t poc_gold_15/ -u http://example.com
```

## 流水线

统一命令 `nuclei_poc`，所有阶段通过子命令调用：

| 命令 | 说明 |
|------|------|
| `nuclei_poc all` | 一键执行完整流水线 |
| `nuclei_poc clone` | 增量克隆上游仓库（部分克隆 + state.json 进度续传） |
| `nuclei_poc delete` | SHA-256 内容去重 |
| `nuclei_poc move` | 分类归档 + 非 nuclei 文件预过滤 |
| `nuclei_poc check` | nuclei 结构校验（通过→poc/，未通过→poc_needs_review/） |
| `nuclei_poc dedup` | 多因素评分去重（poc/ → poc_dedup/） |
| `nuclei_poc quality` | 精品分级（poc_dedup/ → poc_gold_*） |
| `nuclei_poc index` | 生成 GitHub Pages 浏览器索引（→ docs/） |

### 增量克隆核心

利用 Git `--filter=blob:none` 部分克隆，配合 `state.json` 记录进度：

- **无记录（首次）** → `git clone <url> <dst>`，全量拉取
- **有记录 + HEAD 未变** → 跳过，零网络
- **有记录 + HEAD 变了 + 缓存不存在** → `git init` + `fetch --filter=blob:none` 只拉元数据和变更文件的 blob
- **有记录 + `.git` 缓存存在** → `fetch --filter=blob:none` 增量

## 目录结构

| 目录 | 说明 |
|------|------|
| `poc/` | nuclei 校验通过的模板（50,647 个） |
| `poc_dedup/` | 多因素去重后的模板（37,383 个） |
| `poc_gold_11~15/` | 5 级精品分级（入门→旗舰） |
| `poc_needs_review/` | nuclei 校验未通过的模板，供人工审核 |
| `poc_excluded/` | 非 nuclei 的 YAML 文件隔离区 |
| `docs/` | GitHub Pages 浏览器索引 + index.html |
| `state.json` | 仓库增量同步状态（HEAD commit 记录） |
| `repo.csv` | 上游仓库列表（输入来源） |
| `src/` | Rust 源码 |
| `clone-templates/` | 上游仓库克隆缓存（运行时生成，被 .gitignore） |

## 评分规则（0-80 分，18 因子）

- 基础结构 (0-7): id, name, severity
- 严重程度 (0-8): critical=8, high=6, medium=4, low=2, info=1
- 协议支持 (0-10): http+matchers=6, requests+matchers=5, tcp/dns=3ea
- 元数据丰富度 (0-16): author/description/tags/reference/classification/remediation ×2
- 检测能力 (0-15): matchers=5, extractors=4, URL数(≤6)
- 漏洞关联 (0-10): CVE=6, CNVD=4
- 格式规范 (0-6): http(非requests)=3, 无severity大小写问题=1, 无废弃network=2
- 文件大小合理性 (0-5): 500B-10KB=5, 200-20KB=3, <200=1
- 多协议加分 (0-3): 2+协议=3

## 技术栈

Rust | GitHub Actions | Git 部分克隆 | nuclei

## 致谢

感谢 [ProjectDiscovery](https://github.com/projectdiscovery/nuclei) 提供的 Nuclei 工具和开源社区支持。
感谢 [TajangSec](https://github.com/TajangSec) 对部分代码的优化和改进建议。
感谢 [重剑无锋](https://github.com/TideSec) 对去重规则的优化和改进建议。
