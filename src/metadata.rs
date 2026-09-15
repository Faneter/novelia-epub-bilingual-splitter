//! 书目信息改写：OPF 元数据、NCX 导航，以及正文里的 `lang` 属性、书名和章标题。
//!
//! 这一层只管「把源书里的日文信息换成目标语言的对应写法」，不决定任何段落的去留
//! ——那是 [`crate::split`] 的事。

use crate::lang::Lang;
use crate::markup;

/// 源书的章标题只有日文，没有中文对照，所以中文版需要这张映射表。
///
/// 想让中文版保留原日文标题，把这张表清空即可（改成空数组）。
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

/// 改写正文里段落之外的内容：`xml:lang` 属性、`<title>`、章标题。
///
/// 日文版不需要任何改动，直接原样返回。
pub fn rewrite_raw(text: &str, lang: Lang) -> String {
    if lang == Lang::Ja {
        return text.to_string();
    }

    let mut text = text.replace("xml:lang=\"ja\"", "xml:lang=\"zh-CN\"");
    text = markup::replace_element_text(&text, "<title>", "</title>", lang.book_title());
    rewrite_headings(&text)
}

/// 把 `<h2>一章</h2>` 之类换成中文标题。
///
/// 匹配时带上 `>` 前缀，所以「一章」不会误伤已经替换出来的「第一章」——否则
/// 重复扫描会得到「第一第一章」。
fn rewrite_headings(text: &str) -> String {
    let mut text = text.to_string();
    for (ja, zh) in ZH_HEADINGS {
        for level in ["h2", "h3"] {
            let from = format!(">{ja}</{level}>");
            if text.contains(&from) {
                text = text.replace(&from, &format!(">{zh}</{level}>"));
            }
        }
    }
    text
}

/// 改写 OPF 元数据：语言、书名、书籍 ID、书写方向、guide 标题。
pub fn rewrite_opf(opf: &str, lang: Lang, base_id: &str) -> String {
    let mut out =
        markup::replace_element_text(opf, "<dc:language>", "</dc:language>", lang.dc_language());
    out = markup::replace_element_text(&out, "<dc:title>", "</dc:title>", lang.book_title());

    // 两本书必须用不同的 identifier，否则阅读器书库会把它们当成同一本书互相覆盖。
    let id = format!("{}-{}", base_id.trim(), lang.tag());
    out = markup::replace_element_text(&out, "<dc:identifier id=\"uid\">", "</dc:identifier>", &id);

    // 源书写的是 horizontal-lr，与正文的 vrtl 竖排自相矛盾，这里按正文改正。
    out = out.replace("content=\"horizontal-lr\"", "content=\"vertical-rl\"");

    if lang == Lang::Zh {
        for (ja, zh) in ZH_NAV {
            out = out.replace(&format!("title=\"{ja}\""), &format!("title=\"{zh}\""));
        }
    }
    out
}

/// 改写 NCX：语言、导航标签、书名。
///
/// 源书的 NCX 来自 mobiunpack，内容极简，只有「目次」和「奥付」两个导航点。
pub fn rewrite_ncx(ncx: &str, lang: Lang) -> String {
    let mut out = ncx.to_string();

    if lang == Lang::Zh {
        out = out.replace("xml:lang=\"ja\"", "xml:lang=\"zh-CN\"");
        for (ja, zh) in ZH_NAV {
            out = out.replace(&format!("<text>{ja}</text>"), &format!("<text>{zh}</text>"));
        }
    }

    // 书名放在最后替换：先换书名的话，`<text>` 里的内容会参与上面的标签替换。
    let doc_title = format!("\n<text>{}</text>\n", lang.book_title());
    markup::replace_element_text(&out, "<docTitle>", "</docTitle>", &doc_title)
}

#[cfg(test)]
mod tests {
    use super::*;

    const OPF: &str = concat!(
        "<dc:title>\n   カイブツ×カノジョ (講談社ラノベ文庫)\n  </dc:title>",
        "<dc:language>\n   zh-CN\n  </dc:language>",
        "<dc:identifier id=\"uid\">\n   4153338049\n  </dc:identifier>",
        "<meta name=\"primary-writing-mode\" content=\"horizontal-lr\" />",
        "<reference type=\"toc\" title=\"目次\" href=\"Text/part0005.xhtml\" />"
    );

    #[test]
    fn opf_gets_correct_language_title_and_distinct_ids() {
        let ja = rewrite_opf(OPF, Lang::Ja, "4153338049");
        let zh = rewrite_opf(OPF, Lang::Zh, "4153338049");

        assert!(ja.contains("<dc:language>ja</dc:language>"), "源书写死 zh-CN，日文版必须改回来");
        assert!(zh.contains("<dc:language>zh-CN</dc:language>"));
        assert!(zh.contains("凯物×女友"));
        // identifier 必须不同，否则两本书在书库里会互相覆盖。
        assert!(ja.contains("4153338049-ja"));
        assert!(zh.contains("4153338049-zh"));
        assert_ne!(ja, zh);
    }

    #[test]
    fn opf_fixes_the_contradictory_writing_mode() {
        for lang in Lang::ALL {
            let out = rewrite_opf(OPF, lang, "id");
            assert!(out.contains("content=\"vertical-rl\""));
            assert!(!out.contains("horizontal-lr"));
        }
    }

    #[test]
    fn opf_translates_the_guide_title_only_for_chinese() {
        assert!(rewrite_opf(OPF, Lang::Zh, "id").contains("title=\"目录\""));
        assert!(rewrite_opf(OPF, Lang::Ja, "id").contains("title=\"目次\""));
    }

    #[test]
    fn japanese_edition_raw_text_is_untouched() {
        let raw = "<html xml:lang=\"ja\"><h2 class=\"g\">一章</h2>";
        assert_eq!(rewrite_raw(raw, Lang::Ja), raw);
    }

    #[test]
    fn heading_translation_does_not_double_apply() {
        // 「一章」→「第一章」之后再扫一遍，不能让「第一章」变成「第一第一章」。
        let out = rewrite_raw("<h2 class=\"g\">一章</h2>", Lang::Zh);
        assert_eq!(out, "<h2 class=\"g\">第一章</h2>");
        assert_eq!(out.matches("第一章").count(), 1);
        assert_eq!(out.matches("第一第一章").count(), 0);
    }

    #[test]
    fn headings_are_rewritten_at_every_level() {
        let out = rewrite_raw("<h2>幕間</h2><h3>あとがき</h3>", Lang::Zh);
        assert!(out.contains(">间章</h2>"));
        assert!(out.contains(">后记</h3>"));
    }

    #[test]
    fn full_width_numbers_in_headings_are_left_alone() {
        // 数字章节号（１ ２ ３）本身与语言无关，不该被动。
        let out = rewrite_raw("<h3 class=\"font-100per\">１</h3>", Lang::Zh);
        assert!(out.contains(">１</h3>"));
    }

    #[test]
    fn ncx_gets_chinese_labels_and_titles() {
        let ncx = concat!(
            "<ncx xml:lang=\"ja\"><docTitle>\n<text>カイブツ×カノジョ</text>\n</docTitle>",
            "<navPoint><navLabel><text>目次</text></navLabel></navPoint>",
            "<navPoint><navLabel><text>奥付</text></navLabel></navPoint></ncx>"
        );
        let zh = rewrite_ncx(ncx, Lang::Zh);
        assert!(zh.contains("xml:lang=\"zh-CN\""));
        assert!(zh.contains("<text>目录</text>"));
        assert!(zh.contains("<text>版权页</text>"));
        assert!(zh.contains("凯物×女友"));
        assert!(!zh.contains("目次"));

        // 日文版只换书名，其余保持原样。
        let ja = rewrite_ncx(ncx, Lang::Ja);
        assert!(ja.contains("<text>目次</text>"));
        assert!(ja.contains("カイブツ×カノジョ (講談社ラノベ文庫)"));
    }
}
