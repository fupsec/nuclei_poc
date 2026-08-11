//! 流水线各阶段（统一入口库函数）。
//!
//! 每个模块对应原 `src/bin/` 下的一个独立二进制，现改为 `pub fn run(argv)` 供
//! 主程序 `src/main.rs` 按子命令调用，从而避免"多个二进制分步运行"。

pub mod check;
pub mod clone;
pub mod dedup;
pub mod delete;
pub mod index;
pub mod move_file;
pub mod quality;
