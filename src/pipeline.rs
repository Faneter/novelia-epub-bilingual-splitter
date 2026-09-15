//! 把源归档组装成一本书：按条目类型分派给 [`crate::split`] / [`crate::metadata`]，
//! 其余条目原样透传。
//!
//! 这一层只做编排，不含任何切分或改写逻辑。

use crate::epub::Entry;
use crate::lang::Lang;
use crate::markup;
use crate::metadata;
use crate::split::{self, Stats};

/// 整本书的拆分结果。
#[derive(Default)]
pub struct BookStats {
    /// 全书段落的累加去向。
    pub paragraphs: Stats,
    /// 在目标语言下没有任何正文的页面名（源书里不该出现，出现即报）。
    pub empty_pages: Vec<String>,
}

impl BookStats {
    fn absorb(&mut self, page: &str, part: &Stats) {
        self.paragraphs.add(part);
        if part.is_empty_page() {
            self.empty_pages.push(page.to_string());
        }
    }
}

/// 条目该怎么处理。
enum Kind {
    /// 正文，需要按语言过滤。
    Text,
    /// 书目元数据。
    Opf,
    /// 导航。
    Ncx,
    /// 图片、样式表、`container.xml` 等，原样复制。
    Passthrough,
}

fn classify(name: &str) -> Kind {
    if name.ends_with(".xhtml") || name.ends_with(".html") {
        Kind::Text
    } else if name.ends_with(".opf") {
        Kind::Opf
    } else if name.ends_with(".ncx") {
        Kind::Ncx
    } else {
        Kind::Passthrough
    }
}

/// 生成某一语言的整套条目，顺序与源归档保持一致。
pub fn build(source: &[Entry], lang: Lang) -> (Vec<Entry>, BookStats) {
    let base_id = original_identifier(source);
    let mut out = Vec::with_capacity(source.len());
    let mut stats = BookStats::default();

    for (name, data) in source {
        // 不是合法 UTF-8 的条目一律原样透传：图片、字体本来就不该当文本处理。
        // 这样即使源书里混进二进制文件也不会 panic。
        let new_data = match (std::str::from_utf8(data), classify(name)) {
            (Ok(text), Kind::Text) => {
                let (doc, page_stats) = split::split_document(text, lang);
                stats.absorb(name, &page_stats);
                doc.into_bytes()
            }
            (Ok(text), Kind::Opf) => metadata::rewrite_opf(text, lang, &base_id).into_bytes(),
            (Ok(text), Kind::Ncx) => metadata::rewrite_ncx(text, lang).into_bytes(),
            // 插图是两本书共享的资源，都要带上一份。
            _ => data.clone(),
        };
        out.push((name.clone(), new_data));
    }

    (out, stats)
}

/// 从源 OPF 里取出原始 identifier，用来派生两个互不相同的书籍 ID。
fn original_identifier(source: &[Entry]) -> String {
    source
        .iter()
        .find(|(name, _)| name.ends_with(".opf"))
        .and_then(|(_, data)| std::str::from_utf8(data).ok())
        .and_then(|opf| markup::element_text(opf, "<dc:identifier id=\"uid\">", "</dc:identifier>"))
        .unwrap_or_else(|| "novelia-epub-splitter".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source() -> Vec<Entry> {
        vec![
            ("mimetype".into(), b"application/epub+zip".to_vec()),
            (
                "OEBPS/content.opf".into(),
                concat!(
                    "<dc:title>原書</dc:title>",
                    "<dc:language>zh-CN</dc:language>",
                    "<dc:identifier id=\"uid\">999</dc:identifier>"
                )
                .as_bytes()
                .to_vec(),
            ),
            (
                "OEBPS/Text/a.xhtml".into(),
                concat!(
                    "<html xml:lang=\"ja\"><body>",
                    "<p style=\"opacity:0.4;\">日本語</p>",
                    "<p>中文</p>",
                    "</body></html>"
                )
                .as_bytes()
                .to_vec(),
            ),
            // 二进制条目：不该被当成文本处理。
            ("OEBPS/Images/a.jpeg".into(), vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00]),
        ]
    }

    fn find<'a>(entries: &'a [Entry], name: &str) -> &'a [u8] {
        &entries
            .iter()
            .find(|(n, _)| n == name)
            .unwrap_or_else(|| panic!("缺少条目 {name}"))
            .1
    }

    #[test]
    fn keeps_every_entry_in_order() {
        let src = source();
        for lang in Lang::ALL {
            let (out, _) = build(&src, lang);
            assert_eq!(out.len(), src.len(), "条目数不能变");
            let names: Vec<_> = out.iter().map(|(n, _)| n.as_str()).collect();
            let expected: Vec<_> = src.iter().map(|(n, _)| n.as_str()).collect();
            assert_eq!(names, expected, "条目顺序必须保持不变");
        }
    }

    #[test]
    fn passes_binary_entries_through_untouched() {
        let src = source();
        let (out, _) = build(&src, Lang::Ja);
        assert_eq!(find(&out, "OEBPS/Images/a.jpeg"), &[0xFF, 0xD8, 0xFF, 0xE0, 0x00]);
        assert_eq!(find(&out, "mimetype"), b"application/epub+zip");
    }

    #[test]
    fn splits_text_and_rewrites_metadata() {
        let src = source();

        let (ja, stats) = build(&src, Lang::Ja);
        let text = String::from_utf8_lossy(find(&ja, "OEBPS/Text/a.xhtml")).into_owned();
        assert!(text.contains("日本語"));
        assert!(!text.contains("中文"));
        let opf = String::from_utf8_lossy(find(&ja, "OEBPS/content.opf")).into_owned();
        assert!(opf.contains("<dc:language>ja</dc:language>"));
        assert_eq!(stats.paragraphs.jp_kept, 1);
        assert_eq!(stats.paragraphs.dropped, 1);

        let (zh, stats) = build(&src, Lang::Zh);
        let text = String::from_utf8_lossy(find(&zh, "OEBPS/Text/a.xhtml")).into_owned();
        assert!(text.contains("中文"));
        assert!(!text.contains("日本語"));
        assert_eq!(stats.paragraphs.zh_kept, 1);
    }

    #[test]
    fn reports_pages_without_the_target_language() {
        let src = vec![(
            "OEBPS/Text/jp_only.xhtml".into(),
            "<p style=\"opacity:0.4;\">日本語だけ</p>".as_bytes().to_vec(),
        )];
        let (_, stats) = build(&src, Lang::Zh);
        assert_eq!(stats.empty_pages, vec!["OEBPS/Text/jp_only.xhtml"]);

        let (_, stats) = build(&src, Lang::Ja);
        assert!(stats.empty_pages.is_empty());
    }

    #[test]
    fn works_without_an_opf() {
        // 源文件缺 OPF 时不该失败，只是退化成默认 identifier。
        let src = vec![("OEBPS/Text/a.xhtml".into(), b"<p>hi</p>".to_vec())];
        let (out, _) = build(&src, Lang::Ja);
        assert_eq!(out.len(), 1);
        assert_eq!(original_identifier(&src), "novelia-epub-splitter");
    }
}
