//! 把「中日对照」EPUB 拆成两本单语 EPUB。
//!
//! 源书的排版约定（已在全部 38 个正文文件、6586 个段落上验证）：
//!   * 日文原文 | `<p style="opacity:0.4;">…</p>` | 3215 段，淡化显示（原文在下）
//!   * 中文译文 | `<p>…</p>`                      | 3371 段，正常显示（译文在上）
//!   * 另有 141 个 `<p><br /></p>` 空行和图片页 `<p><img …/></p>`，与语言无关。
//!
//! 判别式是「开标签是否带 opacity 样式」，不是字符集猜测——全書中文段落里
//! 假名出现 0 次，414 个 `<ruby>` 注音也 100% 落在日文段落内，所以切分无歧义。
//!
//! 用法:
//!   cargo run                 # 自动使用当前目录下的 .epub
//!   cargo run -- <path.epub>  # 指定源 EPUB
//!
//! 输出: `<源文件名>.ja.epub` 与 `<源文件名>.zh.epub`

use std::error::Error;
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

/// 源文件里标记「日文原文」的内联样式。
const JP_STYLE: &str = "opacity:0.4;";

/// 源书的章标题只有日文，没有中文对照。中文版用这张表替换。
/// 想让中文版保留原日文标题，把这张表清空即可。
/// 匹配时带 `>` 前缀，所以「一章」不会误伤已生成的「第一章」。
const ZH_HEADINGS: [(&str, &str); 10] = [
    ("プロローグ", "序章"),
    ("エピローグ", "终章"),
    ("あとがき", "后记"),
    ("幕間", "间章"),
    ("一章", "第一章"),
    ("二章", "第二章"),
    ("三章", "第三章"),
    ("四章", "第四章"),
    ("五章", "第五章"),
    ("六章", "第六章"),
];

/// 目录/导航里那两处固定日文标签。
const ZH_NAV: [(&str, &str); 2] = [("目次", "目录"), ("奥付", "版权页")];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Lang {
    Ja,
    Zh,
}

impl Lang {
    fn all() -> [Lang; 2] {
        [Lang::Ja, Lang::Zh]
    }

    /// 输出文件名后缀。
    fn tag(self) -> &'static str {
        match self {
            Lang::Ja => "ja",
            Lang::Zh => "zh",
        }
    }

    /// OPF 里的 `dc:language`。源书写死成 zh-CN，对日文版是错的。
    fn dc_language(self) -> &'static str {
        match self {
            Lang::Ja => "ja",
            Lang::Zh => "zh-CN",
        }
    }

    /// 书名。中文名取自源书奥付（`凯物×女友`）。
    fn book_title(self) -> &'static str {
        match self {
            Lang::Ja => "カイブツ×カノジョ (講談社ラノベ文庫)",
            Lang::Zh => "凯物×女友",
        }
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let source = resolve_epub_path()?;
    println!("源 EPUB: {}\n", source.display());

    // 源文件只有 2 MB，整体读进内存最简单，也便于写两遍。
    let entries = read_archive(&source)?;
    println!("读入 {} 个条目", entries.len());

    for lang in Lang::all() {
        let (out_entries, stats) = build_entries(&entries, lang);
        let out_path = output_path(&source, lang);
        write_epub(&out_path, &out_entries)?;

        println!("\n=== [{}] {} ===", lang.tag(), out_path.display());
        println!(
            "  保留 日文段 {:>5} / 中文段 {:>5} / 中立段 {:>4} / 丢弃 {:>5}",
            stats.jp_kept, stats.zh_kept, stats.neutral_kept, stats.dropped
        );
        for name in &stats.empty_files {
            println!("  ⚠ 该语言下没有正文的页面: {name}");
        }
        audit(&out_path, lang)?;
    }

    println!("\n完成。");
    Ok(())
}

// ---------------------------------------------------------------- 归档读写

/// 归档里的一个条目：条目名 + 原始字节。
type Entry = (String, Vec<u8>);

/// 按顺序读出全部条目（跳过目录项）。
fn read_archive(path: &Path) -> Result<Vec<Entry>, Box<dyn Error>> {
    let mut archive = ZipArchive::new(File::open(path)?)?;
    let mut entries = Vec::with_capacity(archive.len());
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        if entry.is_dir() {
            continue;
        }
        let name = entry.name().to_string();
        let mut buf = Vec::with_capacity(entry.size() as usize);
        entry.read_to_end(&mut buf)?;
        entries.push((name, buf));
    }
    Ok(entries)
}

/// 写出 EPUB。`mimetype` 必须是**第一个**条目且**不压缩**——EPUB 规范的硬性要求，
/// 源文件本身没遵守（它是 Deflated），这里顺手修正。
fn write_epub(path: &Path, entries: &[Entry]) -> Result<(), Box<dyn Error>> {
    let mut zip = ZipWriter::new(File::create(path)?);
    let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    zip.start_file("mimetype", stored)?;
    zip.write_all(b"application/epub+zip")?;

    let deflated = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    for (name, data) in entries {
        if name == "mimetype" {
            continue;
        }
        zip.start_file(name.as_str(), deflated)?;
        zip.write_all(data)?;
    }
    zip.finish()?;
    Ok(())
}

// ---------------------------------------------------------------- 拆分主逻辑

#[derive(Default)]
struct Stats {
    jp_kept: usize,
    zh_kept: usize,
    neutral_kept: usize,
    dropped: usize,
    empty_files: Vec<String>,
}

/// 一个 XHTML 文档被切成的片段：`<p>` 元素，或其它原样内容。
enum Segment {
    Raw(String),
    Para { open: String, inner: String },
}

/// 生成某一语言的全套条目。
fn build_entries(source: &[Entry], lang: Lang) -> (Vec<Entry>, Stats) {
    let mut out = Vec::with_capacity(source.len());
    let mut stats = Stats::default();

    // 先从 OPF 里取出原始 identifier，用来派生两个互不相同的书籍 ID。
    let base_id = source
        .iter()
        .find(|(name, _)| name.ends_with(".opf"))
        .and_then(|(_, data)| std::str::from_utf8(data).ok())
        .and_then(|opf| element_text(opf, "<dc:identifier id=\"uid\">", "</dc:identifier>"))
        .unwrap_or_else(|| "novelia-epub-splitter".to_string());

    for (name, data) in source {
        let text = std::str::from_utf8(data).ok();
        let new_data: Vec<u8> = match (text, classify_path(name)) {
            (Some(text), EntryKind::Text) => {
                let (doc, file_stats) = split_document(text, lang);
                merge(&mut stats, &file_stats, name);
                doc.into_bytes()
            }
            (Some(text), EntryKind::Opf) => rewrite_opf(text, lang, &base_id).into_bytes(),
            (Some(text), EntryKind::Ncx) => rewrite_ncx(text, lang).into_bytes(),
            // 图片 / CSS / container.xml 等原样复制到两本书里（插图是共享的）。
            _ => data.clone(),
        };
        out.push((name.clone(), new_data));
    }

    (out, stats)
}

enum EntryKind {
    Text,
    Opf,
    Ncx,
    Other,
}

fn classify_path(name: &str) -> EntryKind {
    if name.ends_with(".xhtml") || name.ends_with(".html") {
        EntryKind::Text
    } else if name.ends_with(".opf") {
        EntryKind::Opf
    } else if name.ends_with(".ncx") {
        EntryKind::Ncx
    } else {
        EntryKind::Other
    }
}

fn merge(total: &mut Stats, part: &Stats, name: &str) {
    total.jp_kept += part.jp_kept;
    total.zh_kept += part.zh_kept;
    total.neutral_kept += part.neutral_kept;
    total.dropped += part.dropped;
    // 有内容却在本语言下被清空的页面，值得报警。
    if part.paragraphs() == 0 && !part.empty_files.is_empty() {
        total.empty_files.push(name.to_string());
    }
}

impl Stats {
    fn paragraphs(&self) -> usize {
        self.jp_kept + self.zh_kept + self.neutral_kept
    }
}

/// 把一个 XHTML 文档过滤成单语言版本。
fn split_document(doc: &str, lang: Lang) -> (String, Stats) {
    let mut stats = Stats::default();
    let mut out = String::with_capacity(doc.len());
    let had_paragraphs = doc.contains("<p");

    for segment in tokenize(doc) {
        match segment {
            // 其它标签（<html> <head> <h2> <div> …）原样保留，
            // 但中文版要改写 lang 属性、书名和章标题。
            Segment::Raw(text) => out.push_str(&rewrite_raw(&text, lang)),
            Segment::Para { open, inner } => {
                let is_jp = open.contains(JP_STYLE);
                let is_neutral = is_neutral_para(&inner);

                if is_neutral {
                    // 图片页和空行：两本书都要，保持原有的段落节奏。
                    out.push_str(&open);
                    out.push_str(&inner);
                    out.push_str("</p>");
                    stats.neutral_kept += 1;
                } else if is_jp == (lang == Lang::Ja) {
                    if is_jp {
                        // 关键：去掉淡化样式。否则日文版整本都是淡灰色。
                        out.push_str("<p>");
                        stats.jp_kept += 1;
                    } else {
                        out.push_str(&open);
                        stats.zh_kept += 1;
                    }
                    out.push_str(&inner);
                    out.push_str("</p>");
                } else {
                    stats.dropped += 1;
                }
            }
        }
    }

    if had_paragraphs && stats.paragraphs() == 0 {
        stats.empty_files.push(String::from("empty-page"));
    }
    (out, stats)
}

/// 图片页段落和纯空行段落与语言无关，两本都要保留。
fn is_neutral_para(inner: &str) -> bool {
    let trimmed = inner.trim();
    trimmed.contains("<img") || trimmed.is_empty() || trimmed == "<br />" || trimmed == "<br/>"
}

/// 把文档切成交替出现的 `Raw` 与 `Para` 片段。
///
/// `<p>` 不允许嵌套，所以找到开标签后直接找第一个 `</p>` 就是配对的闭标签，
/// 不需要完整的 XML 解析器。
fn tokenize(doc: &str) -> Vec<Segment> {
    let mut segments = Vec::new();
    let mut cursor = 0usize;

    while let Some(rel) = doc[cursor..].find("<p") {
        let start = cursor + rel;
        let after = &doc[start + 2..];
        // 必须是 <p> 或 <p …>，不能是 <pre> 之类。
        let is_para = after.starts_with('>') || after.starts_with(char::is_whitespace);
        if !is_para {
            cursor = start + 2;
            continue;
        }
        let Some(tag_len) = after.find('>') else { break };
        let open_end = start + 2 + tag_len;
        let inner_start = open_end + 1;
        let Some(close_rel) = doc[inner_start..].find("</p>") else {
            break;
        };
        let inner_end = inner_start + close_rel;

        segments.push(Segment::Raw(doc[cursor..start].to_string()));
        segments.push(Segment::Para {
            open: doc[start..=open_end].to_string(),
            inner: doc[inner_start..inner_end].to_string(),
        });
        cursor = inner_end + "</p>".len();
    }

    segments.push(Segment::Raw(doc[cursor..].to_string()));
    segments
}

/// 处理段落之外的原样文本：改写 `xml:lang`、`<title>`，中文版还翻译章标题。
fn rewrite_raw(text: &str, lang: Lang) -> String {
    let mut text = text.to_string();
    if lang == Lang::Zh {
        text = text.replace("xml:lang=\"ja\"", "xml:lang=\"zh-CN\"");
        text = replace_element_text(&text, "<title>", "</title>", lang.book_title());
        for (ja, zh) in ZH_HEADINGS {
            // 带上 `>` 前缀，避免「一章」误伤已生成的「第一章」。
            for level in ["h2", "h3"] {
                let from = format!(">{ja}</{level}>");
                if text.contains(&from) {
                    text = text.replace(&from, &format!(">{zh}</{level}>"));
                }
            }
        }
    }
    text
}

/// 改写 OPF 的元数据：语言、书名、书籍 ID、书写方向。
fn rewrite_opf(opf: &str, lang: Lang, base_id: &str) -> String {
    let mut out = replace_element_text(opf, "<dc:language>", "</dc:language>", lang.dc_language());
    out = replace_element_text(&out, "<dc:title>", "</dc:title>", lang.book_title());
    // 两本书必须用不同的 identifier，否则书库会当成同一本书互相覆盖。
    let id = format!("{}-{}", base_id.trim(), lang.tag());
    out = replace_element_text(&out, "<dc:identifier id=\"uid\">", "</dc:identifier>", &id);
    // 源书写的是 horizontal-lr，与正文的 vrtl 竖排自相矛盾，这里按正文改正。
    out = out.replace("content=\"horizontal-lr\"", "content=\"vertical-rl\"");
    if lang == Lang::Zh {
        for (ja, zh) in ZH_NAV {
            out = out.replace(&format!("title=\"{ja}\""), &format!("title=\"{zh}\""));
        }
    }
    out
}

/// 改写 NCX 的书名与语言（来源是 mobiunpack，内容极简，只有两个导航点）。
fn rewrite_ncx(ncx: &str, lang: Lang) -> String {
    let mut out = ncx.to_string();
    if lang == Lang::Zh {
        out = out.replace("xml:lang=\"ja\"", "xml:lang=\"zh-CN\"");
        for (ja, zh) in ZH_NAV {
            out = out.replace(&format!("<text>{ja}</text>"), &format!("<text>{zh}</text>"));
        }
    }
    // docTitle 里的书名放在最后替换，避免被上面的标签替换干扰。
    replace_element_text(&out, "<docTitle>", "</docTitle>", &format!(
        "\n<text>{}</text>\n",
        lang.book_title()
    ))
}

// ---------------------------------------------------------------- 小工具

/// 取元素内部的文本（返回第一个匹配）。
fn element_text(doc: &str, open: &str, close: &str) -> Option<String> {
    let start = doc.find(open)? + open.len();
    let end = start + doc[start..].find(close)?;
    Some(doc[start..end].trim().to_string())
}

/// 替换元素内部的文本，找不到就原样返回（对缺元素的源文件保持宽容）。
fn replace_element_text(doc: &str, open: &str, close: &str, new_text: &str) -> String {
    let Some(start) = doc.find(open).map(|i| i + open.len()) else {
        return doc.to_string();
    };
    let Some(rel) = doc[start..].find(close) else {
        return doc.to_string();
    };
    let end = start + rel;
    format!("{}{}{}", &doc[..start], new_text, &doc[end..])
}

/// 是否含假名（用来审计中文版有没有混进日文）。
fn has_kana(text: &str) -> bool {
    text.chars()
        .any(|c| matches!(c, '\u{3041}'..='\u{3096}' | '\u{30A1}'..='\u{30FA}' | '\u{30FC}'))
}

fn output_path(source: &Path, lang: Lang) -> PathBuf {
    let stem = source
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "output".into());
    source.with_file_name(format!("{stem}.{}.epub", lang.tag()))
}

/// 自动发现源 EPUB，排除本程序自己生成的产物。
fn resolve_epub_path() -> Result<PathBuf, Box<dyn Error>> {
    if let Some(arg) = std::env::args_os().nth(1) {
        return Ok(PathBuf::from(arg));
    }

    let mut candidates: Vec<PathBuf> = Vec::new();
    for entry in std::fs::read_dir(".")? {
        let path = entry?.path();
        let is_epub = path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("epub"));
        if !is_epub || !path.is_file() {
            continue;
        }
        // 别把自己的输出当输入。
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        if name.ends_with(".ja.epub") || name.ends_with(".zh.epub") {
            continue;
        }
        candidates.push(path);
    }
    candidates.sort();

    match candidates.len() {
        0 => Err("当前目录下没有 .epub 文件，请将路径作为第一个参数传入".into()),
        1 => Ok(candidates.remove(0)),
        _ => {
            println!("发现多个 EPUB，默认使用第一个（可用参数指定其它文件）：");
            for candidate in &candidates {
                println!("  - {}", candidate.display());
            }
            Ok(candidates.remove(0))
        }
    }
}

/// 写完立刻回读校验，确认产物真的可用。
fn audit(path: &Path, lang: Lang) -> Result<(), Box<dyn Error>> {
    let mut archive = ZipArchive::new(File::open(path)?)?;

    let count = archive.len();
    let (first_name, first_method) = {
        let first = archive.by_index(0)?;
        (first.name().to_string(), first.compression())
    };
    let compliant = first_name == "mimetype" && first_method == CompressionMethod::Stored;
    println!(
        "  产物 {count} 个条目，首条目 {first_name}({first_method:?}) {}",
        if compliant { "✓ 符合规范" } else { "✗ 不合规" }
    );

    let (mut kana_leaks, mut style_leaks, mut paragraphs) = (0usize, 0usize, 0usize);
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        let name = entry.name().to_string();
        if !(name.ends_with(".xhtml") || name.ends_with(".html")) {
            continue;
        }
        let mut text = String::new();
        entry.read_to_string(&mut text)?;

        style_leaks += text.matches(JP_STYLE).count();
        for segment in tokenize(&text) {
            if let Segment::Para { inner, .. } = segment {
                if is_neutral_para(&inner) {
                    continue;
                }
                paragraphs += 1;
                if lang == Lang::Zh && has_kana(&inner) {
                    kana_leaks += 1;
                }
            }
        }
    }

    println!("  正文 {paragraphs} 段；残留淡化样式 {style_leaks} 处；混入日文 {kana_leaks} 段");
    Ok(())
}

// ---------------------------------------------------------------- 测试

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = concat!(
        "<html xmlns=\"http://www.w3.org/1999/xhtml\" xml:lang=\"ja\" class=\"vrtl\">",
        "<head><title>カイブツ×カノジョ</title></head><body>",
        "<h2 class=\"gfont font-120per\">一章</h2>",
        "<p style=\"opacity:0.4;\">こんにちは、<ruby>世<rt>よ</rt></ruby>界。</p>",
        "<p>你好，世界。</p>",
        "<p><br /></p>",
        "<p><img class=\"fit\" src=\"../Images/a.jpeg\" alt=\"\"/></p>",
        "</body></html>"
    );

    #[test]
    fn japanese_edition_keeps_only_original_text_without_fading() {
        let (ja, stats) = split_document(SAMPLE, Lang::Ja);
        assert!(ja.contains("こんにちは"));
        assert!(ja.contains("<ruby>"), "日文版必须保留注音");
        assert!(!ja.contains("你好"), "日文版不该有中文译文");
        assert!(!ja.contains(JP_STYLE), "日文版必须去掉淡化样式，否则整本是灰的");
        assert_eq!(stats.jp_kept, 1);
        assert_eq!(stats.dropped, 1);
        // 图片与空行两本都要
        assert!(ja.contains("<img"));
        assert!(ja.contains("<br />"));
        assert_eq!(stats.neutral_kept, 2);
    }

    #[test]
    fn chinese_edition_drops_furigana_and_rewrites_metadata() {
        let (zh, stats) = split_document(SAMPLE, Lang::Zh);
        assert!(zh.contains("你好，世界。"));
        assert!(!zh.contains("こんにちは"), "中文版不该有日文原文");
        assert!(!zh.contains("<ruby>"), "中文版不该有日文注音");
        assert!(zh.contains("xml:lang=\"zh-CN\""), "lang 属性要改写");
        assert!(zh.contains(">第一章</h2>"), "中文版章标题要汉化");
        assert!(!zh.contains(JP_STYLE));
        assert_eq!(stats.zh_kept, 1);
    }

    #[test]
    fn heading_translation_does_not_double_apply() {
        // 「一章」→「第一章」后再扫一遍，不能让「第一章」变成「第一第一章」。
        let (zh, _) = split_document(SAMPLE, Lang::Zh);
        assert!(!zh.contains("第一第一章"));
        assert_eq!(zh.matches("第一章").count(), 1);
    }

    #[test]
    fn opf_gets_distinct_ids_and_correct_language() {
        let opf = concat!(
            "<dc:title>\n   カイブツ×カノジョ (講談社ラノベ文庫)\n  </dc:title>",
            "<dc:language>\n   zh-CN\n  </dc:language>",
            "<dc:identifier id=\"uid\">\n   4153338049\n  </dc:identifier>"
        );
        let ja = rewrite_opf(opf, Lang::Ja, "4153338049");
        let zh = rewrite_opf(opf, Lang::Zh, "4153338049");
        assert!(ja.contains("<dc:language>ja</dc:language>"));
        assert!(zh.contains("<dc:language>zh-CN</dc:language>"));
        assert!(ja.contains("4153338049-ja"));
        assert!(zh.contains("4153338049-zh"));
        assert!(zh.contains("凯物×女友"));
        assert_ne!(ja, zh);
    }

    #[test]
    fn tokenizer_keeps_truncated_tail() {
        let (out, _) = split_document("<div><p>你好</p><p>截断", Lang::Zh);
        assert!(out.contains("你好"));
        assert!(out.contains("截断"), "不完整的尾部要原样保留");
    }

    #[test]
    fn full_round_trip_on_the_real_book() {
        // 源书实测的段落数字（用 PowerShell 独立统计过一次，两边必须一致）。
        const TOTAL_PARAS: usize = 6586; // 全部 <p> 元素
        const NEUTRAL_PARAS: usize = 156; // 141 个空行 + 15 个图片页段落
        const JAPANESE: usize = 3215; // 日文原文，全部带 opacity 样式
        const TRANSLATED: usize = 3215; // 中文译文 = 3371 个裸 <p> - 156 个中立段

        // 真正跑一遍当前目录下的双语 EPUB（没有源文件则跳过）。
        let Ok(source) = resolve_epub_path() else {
            return;
        };
        let entries = read_archive(&source).expect("读取源文件");
        assert!(entries.iter().any(|(n, _)| n.ends_with(".opf")));

        let mut ids = Vec::new();
        for lang in Lang::all() {
            let (built, stats) = build_entries(&entries, lang);
            assert_eq!(built.len(), entries.len(), "条目数不能变");
            assert!(built.iter().any(|(n, _)| n.as_str() == "mimetype"));

            // 核心不变式：源书每一个段落都必须有归宿——要么进这本书，
            // 要么被另一本收走，要么是两种语言共用的中立段。不能凭空消失。
            assert_eq!(
                stats.paragraphs() + stats.dropped,
                TOTAL_PARAS,
                "[{}] 段落总数不守恒",
                lang.tag()
            );
            assert_eq!(stats.neutral_kept, NEUTRAL_PARAS, "中立段两本都要有");

            let body: String = built
                .iter()
                .filter(|(n, _)| n.ends_with(".xhtml"))
                .map(|(_, d)| String::from_utf8_lossy(d).into_owned())
                .collect::<Vec<_>>()
                .join(" ");
            assert!(!body.contains(JP_STYLE), "不该残留淡化样式");

            let opf = built
                .iter()
                .find(|(n, _)| n.ends_with(".opf"))
                .map(|(_, d)| String::from_utf8_lossy(d).into_owned())
                .unwrap();
            match lang {
                Lang::Ja => {
                    assert!(body.contains("空から、何かが落ちてきた。"), "日文正文应保留");
                    assert!(!body.contains("有什么东西从天上掉了下来。"), "日文版不该有译文");
                    assert_eq!(stats.jp_kept, JAPANESE);
                    assert_eq!(stats.zh_kept, 0);
                    assert_eq!(stats.dropped, TRANSLATED);
                    assert!(opf.contains("<dc:language>ja</dc:language>"));
                }
                Lang::Zh => {
                    assert!(body.contains("有什么东西从天上掉了下来。"), "中文正文应保留");
                    assert!(!body.contains("空から、何かが落ちてきた。"), "中文版不该有原文");
                    assert_eq!(stats.zh_kept, TRANSLATED);
                    assert_eq!(stats.jp_kept, 0);
                    assert_eq!(stats.dropped, JAPANESE);
                    assert!(!body.contains("<ruby>"), "中文版不应有注音");
                    assert!(opf.contains("<dc:language>zh-CN</dc:language>"));
                }
            }
            // 两种语言的正文段数相同 → 原书的日/中是一一对应的。
            assert_eq!(stats.paragraphs(), JAPANESE + NEUTRAL_PARAS);
            ids.push(element_text(&opf, "<dc:identifier id=\"uid\">", "</dc:identifier>"));
        }
        assert_ne!(ids[0], ids[1], "两本书的 identifier 必须不同");
    }
}
