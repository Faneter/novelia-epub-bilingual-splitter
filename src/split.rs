//! 核心：按内联样式把双语 XHTML 过滤成单语言。
//!
//! 判别式只有一条——`<p>` 的开标签带不带 [`markup::FADING_STYLE`]。
//! 带的是日文原文（在对照排版里被淡化），不带的是中文译文。
//!
//! 之所以敢只靠这一条属性：源书 39 个 XHTML（37 个含段落）里只有这两种 `<p>`，
//! 没有第三种变体；而且中文段落里假名出现 0 次、414 个 `<ruby>` 注音 100% 落在日文段落内。
//! 所以切分零歧义，不需要任何语言识别。

use crate::lang::Lang;
use crate::markup::{self, FADING_STYLE, Segment};
use crate::metadata;

/// 一份文档（或整本书累加后）的段落去向统计。
#[derive(Default, Clone, Copy)]
pub struct Stats {
    /// 留下的日文原文段数。
    pub jp_kept: usize,
    /// 留下的中文译文段数。
    pub zh_kept: usize,
    /// 留下的语言无关段数（图片页、空行）。
    pub neutral_kept: usize,
    /// 因为属于另一种语言而丢弃的段数。
    pub dropped: usize,
    /// 这份文档原本有没有**正文段**（图片页和空行不算正文）。
    ///
    /// 用来区分「本来就是图片页」和「正文被清空了」——后者说明判别式在这个文件上
    /// 失效了，值得报警。
    pub had_content_paragraphs: bool,
}

impl Stats {
    /// 留在这本书里的段落总数（含语言无关段）。
    pub fn paragraphs(&self) -> usize {
        self.jp_kept + self.zh_kept + self.neutral_kept
    }

    /// 原本有正文，但在目标语言下一段正文都不剩。
    ///
    /// 注意只保留空行、没保留任何正文的页面也算被清空；而纯图片页不算，
    /// 因为它本来就没有正文。
    pub fn is_empty_page(&self) -> bool {
        self.had_content_paragraphs && self.jp_kept + self.zh_kept == 0
    }

    /// 累加另一份统计，用于整本书汇总。
    pub fn add(&mut self, other: &Stats) {
        self.jp_kept += other.jp_kept;
        self.zh_kept += other.zh_kept;
        self.neutral_kept += other.neutral_kept;
        self.dropped += other.dropped;
        self.had_content_paragraphs |= other.had_content_paragraphs;
    }
}

/// 把一个 XHTML 文档过滤成单语言版本。
///
/// 段落由本模块决定去留；段落之外的内容（`<html>`、`<head>`、`<h2>`、`<div>`）
/// 原样保留，但会交给 [`metadata`] 改写书名、`lang` 属性和章标题。
pub fn split_document(doc: &str, lang: Lang) -> (String, Stats) {
    let segments = markup::tokenize(doc);
    let mut stats = Stats {
        // 只有「正文段」才算数：纯图片页不该被误判成正文被清空。
        had_content_paragraphs: segments.iter().any(|s| match s {
            Segment::Para { inner, .. } => !markup::is_neutral_para(inner),
            Segment::Raw(_) => false,
        }),
        ..Default::default()
    };
    let mut out = String::with_capacity(doc.len());

    for segment in segments {
        match segment {
            Segment::Raw(text) => out.push_str(&metadata::rewrite_raw(&text, lang)),
            Segment::Para { open, inner } => {
                let is_japanese = open.contains(FADING_STYLE);

                if markup::is_neutral_para(&inner) {
                    // 图片页与空行：两种语言都要，砍掉会破坏段落节奏。
                    write_para(&mut out, &open, &inner);
                    stats.neutral_kept += 1;
                } else if is_japanese == (lang == Lang::Ja) {
                    if is_japanese {
                        // 关键：淡化样式必须去掉。留着的话日文版整本都是灰的。
                        write_para(&mut out, "<p>", &inner);
                        stats.jp_kept += 1;
                    } else {
                        write_para(&mut out, &open, &inner);
                        stats.zh_kept += 1;
                    }
                } else {
                    stats.dropped += 1;
                }
            }
        }
    }

    (out, stats)
}

fn write_para(out: &mut String, open: &str, inner: &str) {
    out.push_str(open);
    out.push_str(inner);
    out.push_str("</p>");
}

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
    fn japanese_edition_keeps_original_text_without_fading() {
        let (ja, stats) = split_document(SAMPLE, Lang::Ja);
        assert!(ja.contains("こんにちは"));
        assert!(ja.contains("<ruby>"), "日文版必须保留注音");
        assert!(!ja.contains("你好"), "日文版不该有中文译文");
        assert!(
            !ja.contains(FADING_STYLE),
            "日文版必须去掉淡化样式，否则整本是灰的"
        );
        assert_eq!(stats.jp_kept, 1);
        assert_eq!(stats.zh_kept, 0);
        assert_eq!(stats.dropped, 1);
        // 图片与空行两本都要
        assert!(ja.contains("<img"));
        assert!(ja.contains("<br />"));
        assert_eq!(stats.neutral_kept, 2);
        assert!(stats.had_content_paragraphs);
        assert!(!stats.is_empty_page());
    }

    #[test]
    fn chinese_edition_drops_furigana_and_rewrites_metadata() {
        let (zh, stats) = split_document(SAMPLE, Lang::Zh);
        assert!(zh.contains("你好，世界。"));
        assert!(!zh.contains("こんにちは"), "中文版不该有日文原文");
        assert!(!zh.contains("<ruby>"), "中文版不该有日文注音");
        assert!(zh.contains("xml:lang=\"zh-CN\""), "lang 属性要改写");
        assert!(zh.contains(">第一章</h2>"), "中文版章标题要汉化");
        assert!(!zh.contains(FADING_STYLE));
        assert_eq!(stats.zh_kept, 1);
        assert_eq!(stats.jp_kept, 0);
        assert_eq!(stats.dropped, 1);
    }

    #[test]
    fn paragraph_count_is_conserved() {
        // 每个段落都必须有归宿：进这本书、被另一本收走、或当成中立段两边都要。
        let total = 4; // SAMPLE 里共 4 个 <p>
        for lang in Lang::ALL {
            let (_, stats) = split_document(SAMPLE, lang);
            assert_eq!(
                stats.paragraphs() + stats.dropped,
                total,
                "[{}] 段落总数不守恒",
                lang.tag()
            );
            // 中文段落共 1 段 + 2 个中立段，两本书应该一样多。
            assert_eq!(stats.neutral_kept, 2);
        }
    }

    #[test]
    fn image_only_page_is_not_an_empty_page() {
        let page = "<body><p><img src=\"a.jpeg\" alt=\"\"/></p></body>";
        for lang in Lang::ALL {
            let (out, stats) = split_document(page, lang);
            assert!(out.contains("<img"), "图片页两本都要有");
            assert!(
                !stats.had_content_paragraphs,
                "图片页本来就没有正文"
            );
            assert!(!stats.is_empty_page(), "原本就没有正文，不算被清空");
        }
    }

    #[test]
    fn page_keeping_only_blank_lines_counts_as_emptied() {
        // 正文全被丢掉、只剩空行的页面，仍应算被清空，否则会漏报判别式失效。
        let page = "<p style=\"opacity:0.4;\">日本語だけ</p><p><br /></p>";
        let (_, stats) = split_document(page, Lang::Zh);
        assert_eq!(stats.neutral_kept, 1);
        assert_eq!(stats.jp_kept, 0);
        assert!(stats.is_empty_page());
    }

    #[test]
    fn page_without_the_target_language_is_reported() {
        // 只有日文的页面，对中文版来说就是被清空的页面。
        let page = "<p style=\"opacity:0.4;\">日本語だけ</p>";
        let (_, stats) = split_document(page, Lang::Zh);
        assert!(stats.is_empty_page());
        assert_eq!(stats.dropped, 1);

        let (_, stats) = split_document(page, Lang::Ja);
        assert!(!stats.is_empty_page());
        assert_eq!(stats.jp_kept, 1);
    }

    #[test]
    fn stats_accumulate_across_pages() {
        let (_, a) = split_document(SAMPLE, Lang::Ja);
        let mut total = Stats::default();
        total.add(&a);
        total.add(&a);
        assert_eq!(total.jp_kept, 2);
        assert_eq!(total.neutral_kept, 4);
        assert_eq!(total.dropped, 2);
        assert!(total.had_content_paragraphs);
        // 每份 SAMPLE 留下 1 段正文 + 2 个中立段，两份共 6 段（dropped 不计入）。
        assert_eq!(total.paragraphs(), 6);
    }

    #[test]
    fn empty_document_is_still_valid() {
        let (out, stats) = split_document("", Lang::Ja);
        assert!(out.is_empty());
        assert!(!stats.had_content_paragraphs);
        assert!(!stats.is_empty_page());
    }
}
