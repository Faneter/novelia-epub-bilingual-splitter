//! 命令行入口：把双语 EPUB 拆成两本单语 EPUB。
//!
//! 实现全在库里（模块分工见 `src/lib.rs`），这里只负责参数、编排和打印。
//!
//! ```text
//! cargo run                 # 自动使用当前目录下的 .epub
//! cargo run -- <path.epub>  # 指定源 EPUB
//! ```
//!
//! 输出 `<源文件名>.ja.epub` 与 `<源文件名>.zh.epub`。

use std::error::Error;
use std::path::PathBuf;

use novelia_epub_bilingual_splitter::audit::{self, Report};
use novelia_epub_bilingual_splitter::epub;
use novelia_epub_bilingual_splitter::lang::Lang;
use novelia_epub_bilingual_splitter::paths;
use novelia_epub_bilingual_splitter::pipeline::{self, BookStats};

fn main() -> Result<(), Box<dyn Error>> {
    let explicit = std::env::args_os().nth(1).map(PathBuf::from);
    let source = paths::resolve_source(explicit)?;

    println!("源 EPUB: {}", source.path.display());
    for other in &source.alternatives {
        println!("  （同时发现 {}，可用参数指定）", other.display());
    }
    println!();

    // 源文件只有 2 MB，整体读进内存最简单，也便于写两遍。
    let entries = epub::read_path(&source.path)?;
    println!("读入 {} 个条目", entries.len());

    for lang in Lang::ALL {
        let (archive, stats) = pipeline::build(&entries, lang);
        let out_path = paths::output_path(&source.path, lang);
        epub::write_path(&out_path, &archive)?;

        println!("\n=== [{}] {} ===", lang.tag(), out_path.display());
        print_stats(&stats);

        // 自检不过就以失败退出，别让一本坏了书默默留在磁盘上。
        let report = audit::inspect_path(&out_path, lang)?;
        print_audit(&report);
        if !report.is_spec_compliant() || !report.is_clean() {
            return Err(format!("[{}] 产物未通过自检", lang.tag()).into());
        }
    }

    println!("\n完成。");
    Ok(())
}

fn print_stats(stats: &BookStats) {
    let p = &stats.paragraphs;
    println!(
        "  保留 日文段 {:>5} / 中文段 {:>5} / 中立段 {:>4} / 丢弃 {:>5}",
        p.jp_kept, p.zh_kept, p.neutral_kept, p.dropped
    );
    for page in &stats.empty_pages {
        println!("  ⚠ 该语言下没有正文的页面: {page}");
    }
}

fn print_audit(report: &Report) {
    println!(
        "  产物 {} 个条目，首条目 {}({}) {}",
        report.entries,
        report.first_entry,
        if report.first_entry_is_stored {
            "Stored"
        } else {
            "Deflated"
        },
        if report.is_spec_compliant() {
            "✓ 符合规范"
        } else {
            "✗ 不合规"
        }
    );
    println!(
        "  正文 {} 段（另有图片/空行 {} 段）；残留淡化样式 {} 处；混入日文 {} 段",
        report.content_paragraphs,
        report.neutral_paragraphs,
        report.fading_style_leaks,
        report.kana_leaks
    );
}
