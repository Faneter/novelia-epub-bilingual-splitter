//! 产物自检：写完之后回读一遍，确认拆出来的书真的能用，而且没混进另一种语言。
//!
//! 这一层只产出数据（[`Report`]），格式化与打印交给 `main`，方便被测试直接断言。

use std::error::Error;
use std::fs::File;
use std::io::{Read, Seek};
use std::path::Path;

use zip::{CompressionMethod, ZipArchive};

use crate::lang::{contains_kana, Lang};
use crate::markup::{self, FADING_STYLE, Segment};

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
    /// 残留的淡化样式处数。两本书都必须是 0。
    pub fading_style_leaks: usize,
    /// 混进日文的段数。只有中文版才检查。
    pub kana_leaks: usize,
}

impl Report {
    /// `mimetype` 是第一个条目且未压缩——EPUB 规范的硬性要求。
    pub fn is_spec_compliant(&self) -> bool {
        self.first_entry == "mimetype" && self.first_entry_is_stored
    }

    /// 没有残留淡化样式，也没有混进另一种语言。
    pub fn is_clean(&self) -> bool {
        self.fading_style_leaks == 0 && self.kana_leaks == 0
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
        kana_leaks: 0,
    };

    for i in 0..entries {
        let mut entry = archive.by_index(i)?;
        let name = entry.name().to_string();
        if !(name.ends_with(".xhtml") || name.ends_with(".html")) {
            continue;
        }
        let mut text = String::new();
        entry.read_to_string(&mut text)?;

        report.fading_style_leaks += text.matches(FADING_STYLE).count();

        for segment in markup::tokenize(&text) {
            if let Segment::Para { inner, .. } = segment {
                // 图片页和空行与语言无关，不计入正文。
                if markup::is_neutral_para(&inner) {
                    report.neutral_paragraphs += 1;
                    continue;
                }
                report.content_paragraphs += 1;
                // 只有中文版需要查有没有混进未翻译的日文。
                if lang == Lang::Zh && contains_kana(&inner) {
                    report.kana_leaks += 1;
                }
            }
        }
    }

    Ok(report)
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
        assert_eq!(report.content_paragraphs, 1, "空行不算正文");
        assert_eq!(report.entries, 2, "mimetype + 一个正文");
    }

    #[test]
    fn detects_leftover_fading_style() {
        // 忘记去掉 opacity 是这套源文件最容易犯的错，必须能被查出来。
        let report = audit(
            book("<p style=\"opacity:0.4;\">日本語</p>"),
            Lang::Ja,
        );
        assert_eq!(report.fading_style_leaks, 1);
        assert!(!report.is_clean());
    }

    #[test]
    fn detects_untranslated_japanese_in_the_chinese_edition() {
        let report = audit(book("<p>日本語が残っている</p>"), Lang::Zh);
        assert_eq!(report.kana_leaks, 1);
        assert!(!report.is_clean());
    }

    #[test]
    fn japanese_text_is_not_a_leak_in_the_japanese_edition() {
        let report = audit(book("<p>日本語</p>"), Lang::Ja);
        assert_eq!(report.kana_leaks, 0);
        assert!(report.is_clean());
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
