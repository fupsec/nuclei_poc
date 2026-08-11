# Nuclei POC

<a href="https://github.com/adysec/nuclei_poc/stargazers"><img alt="GitHub Repo stars" src="https://img.shields.io/github/stars/adysec/nuclei_poc?color=yellow&logo=riseup&logoColor=yellow&style=flat-square"></a>
<a href="https://github.com/adysec/nuclei_poc/network/members"><img alt="GitHub forks" src="https://img.shields.io/github/forks/adysec/nuclei_poc?color=orange&style=flat-square"></a>
<a href="https://github.com/adysec/nuclei_poc/issues"><img alt="GitHub issues" src="https://img.shields.io/github/issues/adysec/nuclei_poc?color=red&style=flat-square"></a>

Nuclei POC — automatically collected, validated, deduplicated, and quality-ranked daily from 400+ upstream repositories.

[中文](https://github.com/adysec/nuclei_poc/blob/main/README.md) | [English](https://github.com/adysec/nuclei_poc/blob/main/README_EN.md)

## Usage

```bash
# Sparse checkout — flagship tier only
git clone --filter=tree:0 --sparse https://github.com/adysec/nuclei_poc
cd nuclei_poc
git sparse-checkout set poc_gold_15

# Scan target
nuclei -t poc_gold_15/ -u http://example.com
```

## Pipeline

Single command `nuclei_poc` with subcommands:

| Command | Description |
|---------|-------------|
| `nuclei_poc all` | Run full pipeline sequentially |
| `nuclei_poc clone` | Incremental clone via partial clone + state.json progress tracking |
| `nuclei_poc delete` | SHA-256 content dedup |
| `nuclei_poc move` | Categorize + filter non-nuclei files |
| `nuclei_poc check` | nuclei validation (pass→poc/, fail→poc_needs_review/) |
| `nuclei_poc dedup` | Multi-factor dedup (poc/ → poc_dedup/) |
| `nuclei_poc quality` | Quality tiers (poc_dedup/ → poc_gold_*) |
| `nuclei_poc index` | GitHub Pages index generation (→ docs/) |

### Incremental Clone

Uses Git `--filter=blob:none` partial clone with `state.json` for progress:

- **No record (first run)** → `git clone <url> <dst>`, full pull
- **Recorded + HEAD unchanged** → skipped, zero network
- **Recorded + HEAD changed + no cache** → `git init` + `fetch --filter=blob:none` — metadata + changed file blobs only
- **Recorded + `.git` cached** → `fetch --filter=blob:none` incremental

## Directory Layout

| Directory | Description |
|-----------|-------------|
| `poc/` | Validated templates (50,647) |
| `poc_dedup/` | Deduplicated templates (37,383) |
| `poc_gold_11~15/` | 5 quality tiers (Entry→Flagship) |
| `poc_needs_review/` | Failed validation, for manual review |
| `poc_excluded/` | Non-nuclei YAML files (audit trail) |
| `docs/` | GitHub Pages index + index.html |
| `state.json` | Incremental sync progress (HEAD commit records) |
| `repo.csv` | Upstream repository list |
| `src/` | Rust source code |
| `clone-templates/` | Clone cache (runtime, gitignored) |

## Scoring (0–80, 18 factors)

- Basic structure (0-7): id, name, severity
- Severity (0-8): critical=8, high=6, medium=4, low=2, info=1
- Protocol support (0-10): http+matchers=6, requests+matchers=5, tcp/dns=3
- Metadata richness (0-16): author/description/tags/reference/classification/remediation ×2
- Detection capability (0-15): matchers=5, extractors=4, URLs(≤6)
- Vulnerability association (0-10): CVE=6, CNVD=4
- Format quality (0-6): http(not requests)=3, severity casing=1, no deprecated network=2
- File size (0-5): 500B-10KB=5, 200-20KB=3, <200=1
- Multi-protocol bonus (0-3): 2+ protocols=3

## Tech Stack

Rust | GitHub Actions | Git Partial Clone | nuclei

## Acknowledgements

- [ProjectDiscovery](https://github.com/projectdiscovery/nuclei) — the Nuclei engine and community
- [TajangSec](https://github.com/TajangSec) — code optimizations
- [重剑无锋](https://github.com/TideSec) — dedup rule optimizations
