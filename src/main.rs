//! 命令行入口：把双语 EPUB 拆成两本单语 EPUB。
//!
//! 实现全在库里（模块分工见 `src/lib.rs`），这里只负责参数、编排和打印。
//!
//! ```text
//! cargo run                               # 自动使用当前目录下的 .epub
//! cargo run -- <path.epub>                # 指定源 EPUB
//! cargo run -- <path.epub> --zh-title 中文书名
//! cargo run -- <path.epub> --strict       # 把警告也当成失败
//! ```
//!
//! 输出 `<源文件名>.ja.epub` 与 `<源文件名>.zh.epub`。

use std::error::Error;
use std::path::PathBuf;

use novelia_epub_bilingual_splitter::audit::{self, Report};
use novelia_epub_bilingual_splitter::epub;
use novelia_epub_bilingual_splitter::lang::Lang;
use novelia_epub_bilingual_splitter::metadata::{self, Plan};
use novelia_epub_bilingual_splitter::paths;
use novelia_epub_bilingual_splitter::pipeline::{self, BookStats};

/// 命令行参数。
#[derive(Default)]
struct Args {
    source: Option<PathBuf>,
    ja_title: Option<String>,
    zh_title: Option<String>,
    /// 把「疑似但大概正常」的告警也提升为失败。
    strict: bool,
}

fn main() -> Result<(), Box<dyn Error>> {
    let args = parse_args()?;
    let source = paths::resolve_source(args.source.clone())?;

    println!("源 EPUB: {}", source.path.display());
    for other in &source.alternatives {
        println!("  （同时发现 {}，可用参数指定）", other.display());
    }

    // 源文件通常只有几 MB，整体读进内存最简单，也便于写两遍。
    let entries = epub::read_path(&source.path)?;
    println!("读入 {} 个条目", entries.len());

    let plan = build_plan(&entries, &args, &source.path);
    println!(
        "书名: 日文版「{}」 / 中文版「{}」",
        plan.title(Lang::Ja),
        plan.title(Lang::Zh)
    );

    let mut failed = false;
    for lang in Lang::ALL {
        let (archive, stats) = pipeline::build(&entries, lang, &plan);
        let out_path = paths::output_path(&source.path, lang);
        epub::write_path(&out_path, &archive)?;

        println!("\n=== [{}] {} ===", lang.tag(), out_path.display());
        print_stats(&stats);

        let report = audit::inspect_path(&out_path, lang)?;
        print_audit(&report);

        for problem in report.fatal_issues() {
            println!("  ✗ {problem}");
        }
        let mut warnings = report.warnings();
        if !stats.empty_pages.is_empty() {
            warnings.push(format!(
                "以下页面在本语言下一段正文都没有，请确认是有意为之：{}",
                stats.empty_pages.join(", ")
            ));
        }
        for warning in &warnings {
            println!("  ⚠ {warning}");
        }

        if !report.fatal_issues().is_empty() || (args.strict && !warnings.is_empty()) {
            failed = true;
        }
    }

    if failed {
        return Err("产物未通过自检".into());
    }
    println!("\n完成。");
    Ok(())
}

/// 组装书名与书籍 ID：源书有的用源书的，缺的用文件名兜底，命令行可以覆盖。
fn build_plan(entries: &[epub::Entry], args: &Args, source: &std::path::Path) -> Plan {
    let meta = metadata::SourceMeta::extract(entries);
    // 实测有一本源书的 dc:title 是空元素，只能退回文件名。
    let fallback = paths::title_from_filename(source);
    let base_title = if meta.title.trim().is_empty() {
        fallback.as_str()
    } else {
        meta.title.trim()
    };

    let mut plan = Plan::new(base_title, &meta.identifier);
    if let Some(title) = &args.ja_title {
        plan = plan.with_ja_title(title.clone());
    }
    if let Some(title) = &args.zh_title {
        plan = plan.with_zh_title(title.clone());
    }
    plan
}

/// 手工解析参数：参数很少，不值得引入依赖。
fn parse_args() -> Result<Args, Box<dyn Error>> {
    let mut args = Args::default();
    let mut iter = std::env::args_os().skip(1);

    while let Some(argument) = iter.next() {
        let text = argument.to_string_lossy().into_owned();
        match text.as_str() {
            "--strict" => args.strict = true,
            "--ja-title" | "--zh-title" => {
                let value = iter
                    .next()
                    .ok_or_else(|| format!("{text} 后面缺少书名"))?
                    .to_string_lossy()
                    .into_owned();
                if text == "--ja-title" {
                    args.ja_title = Some(value);
                } else {
                    args.zh_title = Some(value);
                }
            }
            other if other.starts_with('-') => {
                return Err(format!(
                    "未知参数 {other}\n用法: [源.epub] [--ja-title 书名] [--zh-title 书名] [--strict]"
                )
                .into());
            }
            _ => {
                if args.source.is_some() {
                    return Err("只能指定一个源 EPUB".into());
                }
                args.source = Some(PathBuf::from(argument));
            }
        }
    }
    Ok(args)
}

fn print_stats(stats: &BookStats) {
    let p = &stats.paragraphs;
    println!(
        "  保留 日文段 {:>5} / 中文段 {:>5} / 中立段 {:>4} / 丢弃 {:>5}",
        p.jp_kept, p.zh_kept, p.neutral_kept, p.dropped
    );
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
        "  正文 {} 段（另有图片/空行 {} 段）；残留淡化样式 {} 段；疑似混入另一种语言 {} 段",
        report.content_paragraphs,
        report.neutral_paragraphs,
        report.fading_style_leaks,
        report.suspect_count
    );
}
