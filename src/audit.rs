//! 产物自检：写完之后回读一遍，确认拆出来的书真的能用，而且没混进另一种语言。
//!
//! 这一层只产出数据（[`Report`]），格式化与打印交给 `main`，方便被测试直接断言。
//!
//! # 为什么不能「见到假名就报错」
//!
//! 早期版本把「中文段落里出现假名」一律当成故障。换一本书就撞墙了：中文译本会
//! **合法地保留**一些日文专有名词（实测有 `ダンTV` 这个虚构直播平台、`とくまろ`
//! 这位插画师），全书 4765 段里有 3 段命中，工具因此拒绝产出。
//!
//! 真正该抓的故障是「判别式在这本书上失效」，它的表现是**整页整章大面积判错**，
//! 而不是零星几个专有名词。所以这里按页判断：一页里超过 1/3 的正文段疑似判错，
//! 才认定这页出了问题（见 [`Report::misclassified_pages`]）；零星命中只作为
//! 提示列出，交由人工过目。

use std::error::Error;
use std::fs::File;
use std::io::{Read, Seek};
use std::path::Path;

use zip::{CompressionMethod, ZipArchive};

use crate::lang::{contains_kana, Lang};
use crate::markup::{self, Segment};

/// 判定「整页判错」要求一页至少有这么多正文段，避免小页面被误判。
const MISCLASSIFY_MIN_PARAGRAPHS: usize = 5;

/// 收集多少条疑似段落作为样例供人工核对。
const SUSPECT_SAMPLES: usize = 10;

/// 样例文本截断长度。
const SAMPLE_CHARS: usize = 60;

/// 自检结果。
pub struct Report {
    pub entries: usize,
    /// 归档里第一个条目的名字。
    pub first_entry: String,
    /// 第一个条目是否未压缩。
    pub first_entry_is_stored: bool,
    /// 正文段落数（不含图片页和空行）。
    pub content_paragraphs: usize,
    /// 语言无关的段落数：图片页与空行。
    pub neutral_paragraphs: usize,
    /// 残留淡化样式的段落数。两本书都必须是 0，否则日文版整本是灰的。
    pub fading_style_leaks: usize,
    /// 疑似混进另一种语言的正文段总数。只有中文版才检查。
    pub suspect_count: usize,
    /// 疑似段落的样例（最多 [`SUSPECT_SAMPLES`] 条）。
    pub suspect_samples: Vec<String>,
    /// 疑似「整页判错」的页面：判别式很可能在这些页上失效了。
    pub misclassified_pages: Vec<String>,
}

impl Report {
    /// `mimetype` 是第一个条目且未压缩——EPUB 规范的硬性要求。
    pub fn is_spec_compliant(&self) -> bool {
        self.first_entry == "mimetype" && self.first_entry_is_stored
    }

    /// 严格意义上的干净：没有任何残留样式、疑似段落或整页误判。
    ///
    /// 注意这**不是**默认的失败判据——零星专有名词命中会让它返回 `false`，
    /// 但那属于正常情况，见 [`Report::fatal_issues`]。
    pub fn is_clean(&self) -> bool {
        self.fading_style_leaks == 0
            && self.suspect_count == 0
            && self.misclassified_pages.is_empty()
    }

    /// 致命问题：产物不合规，或拆分明显失败。
    pub fn fatal_issues(&self) -> Vec<String> {
        let mut issues = Vec::new();
        if !self.is_spec_compliant() {
            issues.push(format!(
                "mimetype 不是第一个条目或未压缩（实际首条目为 {}）",
                self.first_entry
            ));
        }
        if self.fading_style_leaks > 0 {
            issues.push(format!(
                "产物里还有 {} 段带淡化样式，日文版会整本显示为灰色",
                self.fading_style_leaks
            ));
        }
        if !self.misclassified_pages.is_empty() {
            issues.push(format!(
                "以下页面疑似整页判错（判别式在这些页上可能失效）：{}",
                self.misclassified_pages.join(", ")
            ));
        }
        issues
    }

    /// 提示：需要人工过目，但大概率是正常的。
    pub fn warnings(&self) -> Vec<String> {
        let mut warnings = Vec::new();
        if self.suspect_count > 0 {
            let mut text = format!(
                "{} 段正文含日文假名，可能是译本里保留的专有名词，也可能是漏翻",
                self.suspect_count
            );
            for sample in &self.suspect_samples {
                text.push_str(&format!("\n      · {sample}"));
            }
            if self.suspect_count > self.suspect_samples.len() {
                text.push_str(&format!(
                    "\n      … 另有 {} 段未列出",
                    self.suspect_count - self.suspect_samples.len()
                ));
            }
            warnings.push(text);
        }
        warnings
    }

    /// 归档里的段落总数（正文 + 图片页 + 空行）。
    ///
    /// 两本单语书这个数字应当相同：源书的日/中段落是一一对应的。
    pub fn paragraphs_total(&self) -> usize {
        self.content_paragraphs + self.neutral_paragraphs
    }
}

/// 回读一个归档并统计。
pub fn inspect<R: Read + Seek>(reader: R, lang: Lang) -> Result<Report, Box<dyn Error>> {
    let mut archive = ZipArchive::new(reader)?;
    let entries = archive.len();

    let (first_entry, first_entry_is_stored) = {
        let first = archive.by_index(0)?;
        (
            first.name().to_string(),
            first.compression() == CompressionMethod::Stored,
        )
    };

    let mut report = Report {
        entries,
        first_entry,
        first_entry_is_stored,
        content_paragraphs: 0,
        neutral_paragraphs: 0,
        fading_style_leaks: 0,
        suspect_count: 0,
        suspect_samples: Vec::new(),
        misclassified_pages: Vec::new(),
    };

    for i in 0..entries {
        let mut entry = archive.by_index(i)?;
        let name = entry.name().to_string();
        if !(name.ends_with(".xhtml") || name.ends_with(".html")) {
            continue;
        }
        let mut text = String::new();
        entry.read_to_string(&mut text)?;

        // 逐页统计，才能判断「整页判错」而不只是「零星命中」。
        let mut page_content = 0usize;
        let mut page_suspect = 0usize;

        for segment in markup::tokenize(&text) {
            let Segment::Para { open, inner } = segment else {
                continue;
            };
            if markup::has_fading_style(&open) {
                report.fading_style_leaks += 1;
            }
            // 图片页和空行与语言无关，不计入正文。
            if markup::is_neutral_para(&inner) {
                report.neutral_paragraphs += 1;
                continue;
            }
            report.content_paragraphs += 1;
            page_content += 1;

            // 只有中文版需要查有没有混进未翻译的日文。
            if lang == Lang::Zh && contains_kana(&inner) {
                report.suspect_count += 1;
                page_suspect += 1;
                if report.suspect_samples.len() < SUSPECT_SAMPLES {
                    report.suspect_samples.push(truncate(&inner, SAMPLE_CHARS));
                }
            }
        }

        // 一页里超过 1/3 的正文段疑似判错 —— 这不可能是「几个专有名词」，
        // 更像是判别式在这一页上失效了。
        let third = page_content.div_ceil(3);
        if page_content >= MISCLASSIFY_MIN_PARAGRAPHS && page_suspect >= third {
            report.misclassified_pages.push(name);
        }
    }

    Ok(report)
}

/// 按字符截断并压平空白，用于样例展示。
fn truncate(text: &str, max_chars: usize) -> String {
    let flattened = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut out: String = flattened.chars().take(max_chars).collect();
    if flattened.chars().count() > max_chars {
        out.push('…');
    }
    out
}

/// 从磁盘回读一个 EPUB。
pub fn inspect_path(path: &Path, lang: Lang) -> Result<Report, Box<dyn Error>> {
    inspect(File::open(path)?, lang)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::epub;
    use std::io::Cursor;

    /// 造一个归档并回读自检。
    fn audit(entries: Vec<epub::Entry>, lang: Lang) -> Report {
        let mut buf = Cursor::new(Vec::new());
        epub::write(&mut buf, &entries).unwrap();
        buf.set_position(0);
        inspect(buf, lang).unwrap()
    }

    fn book(body: &str) -> Vec<epub::Entry> {
        vec![("OEBPS/Text/a.xhtml".into(), body.as_bytes().to_vec())]
    }

    #[test]
    fn clean_output_passes_every_check() {
        let report = audit(book("<body><p>你好，世界。</p><p><br /></p></body>"), Lang::Zh);
        assert!(report.is_spec_compliant());
        assert!(report.is_clean());
        assert!(report.fatal_issues().is_empty());
        assert!(report.warnings().is_empty());
        assert_eq!(report.content_paragraphs, 1, "空行不算正文");
        assert_eq!(report.entries, 2, "mimetype + 一个正文");
    }

    #[test]
    fn detects_leftover_fading_style() {
        // 忘记去掉淡化样式是最容易犯的错，必须能被查出来。
        let report = audit(book("<p style=\"opacity:0.4;\">日本語</p>"), Lang::Ja);
        assert_eq!(report.fading_style_leaks, 1);
        assert!(!report.is_clean());
        assert!(!report.fatal_issues().is_empty(), "残留淡化样式是致命问题");
    }

    #[test]
    fn a_few_legitimate_japanese_names_are_only_a_warning() {
        // 实测：中文译本里合法保留了「ダンTV」「とくまろ」，全书 4765 段里只有 3 段。
        // 早期版本把这种零星命中当成故障，导致工具拒绝产出。
        let body = concat!(
            "<p>你竟然在ダンTV的在线人数排行榜上排名第一了！</p>",
            "<p>我慌忙看了看ダンTV的主页。</p>",
            "<p>负责插画的とくまろ老师。</p>",
            "<p>这里是一段普通的中文。</p>",
        );
        let report = audit(book(body), Lang::Zh);
        assert_eq!(report.suspect_count, 3);
        assert!(report.misclassified_pages.is_empty(), "零星命中不算整页判错");
        assert!(report.fatal_issues().is_empty(), "不该判为致命");
        assert_eq!(report.warnings().len(), 1, "但要有提示");
        assert!(report.suspect_samples.len() <= SUSPECT_SAMPLES);
        // 严格意义上的 is_clean 仍为 false —— 语义不同，别混淆。
        assert!(!report.is_clean());
    }

    #[test]
    fn a_page_that_went_to_the_wrong_language_is_fatal() {
        // 判别式失效的样子：整页的段落都判错了。
        let mut body = String::new();
        for i in 0..6 {
            body.push_str(&format!("<p>日本語の段落その{i}。</p>"));
        }
        let report = audit(book(&body), Lang::Zh);
        assert_eq!(report.misclassified_pages, vec!["OEBPS/Text/a.xhtml"]);
        assert!(!report.fatal_issues().is_empty(), "整页判错必须致命");
    }

    #[test]
    fn tiny_pages_are_not_reported_as_misclassified() {
        // 正文段太少时不下「整页判错」的结论，避免小页面误报。
        let report = audit(book("<p>日本語だけ</p>"), Lang::Zh);
        assert_eq!(report.suspect_count, 1);
        assert!(report.misclassified_pages.is_empty());
    }

    #[test]
    fn japanese_text_is_not_a_leak_in_the_japanese_edition() {
        let report = audit(book("<p>日本語</p>"), Lang::Ja);
        assert_eq!(report.suspect_count, 0);
        assert!(report.is_clean());
    }

    #[test]
    fn suspect_samples_are_flattened_and_truncated() {
        let long = format!("<p>ダンTV{}　带换行\n的文本</p>", "文".repeat(200));
        let report = audit(book(&long), Lang::Zh);
        let sample = &report.suspect_samples[0];
        assert!(sample.ends_with('…'), "过长要截断：{sample}");
        assert!(!sample.contains('\n'), "样例要压平空白");
        assert!(sample.chars().count() <= SAMPLE_CHARS + 1);
    }

    #[test]
    fn counts_only_real_paragraphs() {
        let report = audit(
            book("<p>甲</p><p><br /></p><p><img src=\"a.jpeg\"/></p>"),
            Lang::Zh,
        );
        assert_eq!(report.content_paragraphs, 1);
        assert_eq!(report.neutral_paragraphs, 2, "空行 + 图片页");
        assert_eq!(report.paragraphs_total(), 3);
    }
}
