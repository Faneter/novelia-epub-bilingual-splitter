//! 书目信息改写：OPF 元数据、NCX 导航，以及正文里的 `lang` 属性和章标题。
//!
//! 这一层只管「把源书里的信息换成目标语言的对应写法」，不决定任何段落的去留
//! ——那是 [`crate::split`] 的事。
//!
//! **书名一律来自源文件**（见 [`SourceMeta`]）。早期版本把书名写死在代码里，
//! 换一本书就会把上一本书的书名盖到新书上。

use crate::epub::Entry;
use crate::lang::Lang;
use crate::markup;

/// 源书的章标题往往只有日文、没有中文对照，所以中文版需要一张映射表。
///
/// 只在**整段完全相等**时才替换（见 [`rewrite_headings`]），所以
/// 「プロローグ 魔王様、うっかり爆誕してしまう」这种带副标题的标题不会被
/// 翻译成半中半日。想让中文版保留原日文标题，把这张表清空即可。
const ZH_HEADINGS: [(&str, &str); 12] = [
    ("プロローグ", "序章"),
    ("エピローグ", "终章"),
    ("あとがき", "后记"),
    ("幕間", "间章"),
    ("番外編", "番外篇"),
    ("序章", "序章"),
    ("一章", "第一章"),
    ("二章", "第二章"),
    ("三章", "第三章"),
    ("四章", "第四章"),
    ("五章", "第五章"),
    ("六章", "第六章"),
];

/// 目录/导航里的固定日文标签。
const ZH_NAV: [(&str, &str); 2] = [("目次", "目录"), ("奥付", "版权页")];

/// 从源书里读出来的元数据。
///
/// 两个字段都可能为空——实测有一本源书的 `dc:title` 是空元素、而且压根没有
/// `dc:identifier` 元素。调用方需要为空的情况准备兜底值。
#[derive(Default, Clone, Debug, PartialEq, Eq)]
pub struct SourceMeta {
    /// 源 OPF 的 `dc:title`。
    pub title: String,
    /// 源 OPF 的 `dc:identifier`。
    pub identifier: String,
}

impl SourceMeta {
    /// 从源归档里读出元数据（取第一个 `.opf`）。
    pub fn extract(source: &[Entry]) -> Self {
        let Some(opf) = source
            .iter()
            .find(|(name, _)| name.ends_with(".opf"))
            .and_then(|(_, data)| std::str::from_utf8(data).ok())
        else {
            return Self::default();
        };
        Self {
            title: markup::element_text(opf, "dc:title").unwrap_or_default(),
            identifier: markup::element_text(opf, "dc:identifier").unwrap_or_default(),
        }
    }
}

/// 一次拆分要用到的书名与书籍 ID。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    ja_title: String,
    zh_title: String,
    identifier: String,
}

impl Plan {
    /// 默认计划：两本书都用同一个书名。
    ///
    /// 源书里通常只有一个书名（往往是日文原名），中文名未必存在，所以默认
    /// 两本同名；需要区分时用 [`Plan::with_zh_title`] 显式指定。
    pub fn new(title: &str, identifier: &str) -> Self {
        Self {
            ja_title: title.trim().to_string(),
            zh_title: title.trim().to_string(),
            identifier: identifier.trim().to_string(),
        }
    }

    /// 指定中文版书名（例如源书版权页里给出的中文译名）。
    pub fn with_zh_title(mut self, title: impl Into<String>) -> Self {
        self.zh_title = title.into();
        self
    }

    /// 指定日文版书名。
    pub fn with_ja_title(mut self, title: impl Into<String>) -> Self {
        self.ja_title = title.into();
        self
    }

    /// 该语言版本要写入 `dc:title` 的书名。
    pub fn title(&self, lang: Lang) -> &str {
        match lang {
            Lang::Ja => &self.ja_title,
            Lang::Zh => &self.zh_title,
        }
    }

    /// 该语言版本的书籍 ID。
    ///
    /// 两本书必须不同，否则阅读器书库会把它们当成同一本书互相覆盖。源书没有
    /// identifier 时用书名派生一个稳定值——**不能退回常量**，否则同一个书库里
    /// 任意两本无 ID 的书也会撞在一起。
    pub fn book_id(&self, lang: Lang) -> String {
        if self.identifier.is_empty() {
            format!("novelia-{:016x}-{}", stable_hash(&self.ja_title), lang.tag())
        } else {
            format!("{}-{}", self.identifier, lang.tag())
        }
    }
}

/// FNV-1a。只为得到一个稳定的短标识，不是密码学用途。
fn stable_hash(text: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// 改写正文里段落之外的内容：`xml:lang` 属性与章标题。
///
/// 两件事都**必须按目标语言做**，不能只做中文方向：实测一本源书的根元素写的是
/// `xml:lang="zh-CN"`，如果只往中文方向改，日文版就会声明自己是中文书。
///
/// 注意这里**不动 `<title>`**。早期版本把每个文件的 `<title>` 都换成书名，
/// 结果撞上一本源书——它的 `<title>` 是各自的章节名（「あとがき」「表紙」，
/// 甚至有论坛帖标题），全被书名覆盖毁掉了。现在只在内容**恰好等于**映射表里
/// 某个标题时才改写。
pub fn rewrite_raw(text: &str, lang: Lang) -> String {
    let mut text = set_lang_attribute(text, lang);
    if lang == Lang::Zh {
        text = rewrite_headings(&text);
    }
    text
}

/// 把根元素上的 `xml:lang="…"` / `lang="…"` 改成目标语言。
fn set_lang_attribute(text: &str, lang: Lang) -> String {
    let mut out = text.to_string();
    for attribute in ["xml:lang=\"", "lang=\""] {
        // `xml:lang="` 里不含 ` lang="`（中间是冒号不是空格），所以两个模式
        // 不会互相误伤。
        let mut search = 0usize;
        while let Some(rel) = out[search..].find(attribute) {
            let value_start = search + rel + attribute.len();
            let Some(quote_rel) = out[value_start..].find('"') else {
                break;
            };
            let value_end = value_start + quote_rel;
            if &out[value_start..value_end] != lang.dc_language() {
                out.replace_range(value_start..value_end, lang.dc_language());
            }
            search = value_start + lang.dc_language().len();
        }
    }
    out
}

/// 把 `<h4>あとがき</h4>`、`<title>あとがき</title>` 之类换成中文标题。
///
/// 匹配时带上 `>` 前缀并**要求整段相等**，所以：
/// * 「一章」不会误伤已经替换出来的「第一章」（否则会得到「第一第一章」）；
/// * 带副标题的「プロローグ 魔王様、…」不会被翻成半中半日。
fn rewrite_headings(text: &str) -> String {
    let mut text = text.to_string();
    // 源书的正文标题实测用过 h2/h3/h4，`<title>` 也要一起处理。
    let levels = ["title", "h1", "h2", "h3", "h4", "h5", "h6"];
    for (ja, zh) in ZH_HEADINGS {
        for level in levels {
            let from = format!(">{ja}</{level}>");
            if text.contains(&from) {
                text = text.replace(&from, &format!(">{zh}</{level}>"));
            }
        }
    }
    text
}

/// 改写 OPF 元数据：语言、书名、书籍 ID、书写方向、guide 标题。
pub fn rewrite_opf(opf: &str, lang: Lang, plan: &Plan) -> String {
    let mut out = set(opf, "dc:language", lang.dc_language());
    out = set(&out, "dc:title", plan.title(lang));
    out = set_identifier(&out, &plan.book_id(lang));

    // 源书写的是 horizontal-lr，却与正文的 vrtl 竖排自相矛盾，这里按正文改正。
    // 源书没写这一项时保持不动（不是所有书都是竖排）。
    out = out.replace("content=\"horizontal-lr\"", "content=\"vertical-rl\"");

    if lang == Lang::Zh {
        for (ja, zh) in ZH_NAV {
            out = out.replace(&format!("title=\"{ja}\""), &format!("title=\"{zh}\""));
        }
    }
    out
}

/// 改写 NCX：语言、导航标签、书名。
pub fn rewrite_ncx(ncx: &str, lang: Lang, plan: &Plan) -> String {
    let mut out = set_lang_attribute(ncx, lang);

    if lang == Lang::Zh {
        for (ja, zh) in ZH_NAV {
            out = out.replace(&format!("<text>{ja}</text>"), &format!("<text>{zh}</text>"));
        }
    }

    // 书名放在最后替换：先换书名的话，`<text>` 里的内容会参与上面的标签替换。
    let doc_title = format!("\n<text>{}</text>\n", plan.title(lang));
    set(&out, "docTitle", &doc_title)
}

/// 替换元素文本；元素不存在就原样返回（源书缺标签不该让整个流程失败）。
fn set(doc: &str, tag: &str, value: &str) -> String {
    markup::replace_element_text(doc, tag, value).unwrap_or_else(|| doc.to_string())
}

/// 设置 `dc:identifier`；元素不存在时补一个。
///
/// 实测有一本源书 `unique-identifier="BookId"` 却**没有对应的元素**，
/// 直接跳过会让产物的 `unique-identifier` 指向不存在的东西。
fn set_identifier(opf: &str, value: &str) -> String {
    if let Some(out) = markup::replace_element_text(opf, "dc:identifier", value) {
        return out;
    }
    let Some(metadata_end) = opf.find("</metadata>") else {
        return opf.to_string();
    };
    let id = attribute_value(opf, "package", "unique-identifier").unwrap_or_else(|| "BookId".into());
    let mut out = String::with_capacity(opf.len() + value.len() + 40);
    out.push_str(&opf[..metadata_end]);
    out.push_str(&format!("  <dc:identifier id=\"{id}\">{value}</dc:identifier>\n "));
    out.push_str(&opf[metadata_end..]);
    out
}

/// 读某个元素开标签上的属性值。
fn attribute_value(doc: &str, tag: &str, attribute: &str) -> Option<String> {
    let start = doc.find(&format!("<{tag}"))?;
    let end = start + doc[start..].find('>')?;
    let tag_text = &doc[start..end];
    let needle = format!("{attribute}=\"");
    let value_start = tag_text.find(&needle)? + needle.len();
    let value_end = value_start + tag_text[value_start..].find('"')?;
    Some(tag_text[value_start..value_end].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 第一本源书的 OPF 片段（dc:title 无属性、identifier 带 id="uid"）。
    const OPF: &str = concat!(
        "<package version=\"2.0\" unique-identifier=\"uid\">",
        "<metadata>",
        "<dc:title>\n   カイブツ×カノジョ (講談社ラノベ文庫)\n  </dc:title>",
        "<dc:language>\n   zh-CN\n  </dc:language>",
        "<dc:identifier id=\"uid\">\n   4153338049\n  </dc:identifier>",
        "<meta name=\"primary-writing-mode\" content=\"horizontal-lr\" />",
        "</metadata>",
        "<reference type=\"toc\" title=\"目次\" href=\"Text/part0005.xhtml\" />",
        "</package>"
    );

    /// 第二本源书的 OPF 片段（EPUB 3，dc:title 带属性，没有 identifier）。
    const EPUB3_OPF: &str = concat!(
        "<package version=\"3.0\" unique-identifier=\"BookId\">",
        "<metadata>",
        "<dc:title id=\"id\">\n   </dc:title>",
        "<dc:creator id=\"cre\">\n   邑上主水\n  </dc:creator>",
        "<dc:language>\n   zh-CN\n  </dc:language>",
        "</metadata>",
        "</package>"
    );

    fn plan() -> Plan {
        Plan::new("试验书", "999")
    }

    #[test]
    fn extract_reads_metadata_from_the_opf() {
        assert_eq!(
            SourceMeta::extract(&[("OEBPS/content.opf".into(), OPF.as_bytes().to_vec())]),
            SourceMeta {
                title: "カイブツ×カノジョ (講談社ラノベ文庫)".into(),
                identifier: "4153338049".into(),
            }
        );
    }

    #[test]
    fn extract_handles_empty_and_missing_fields() {
        // 实测第二本源书：dc:title 是空的，而且没有 dc:identifier。
        let meta =
            SourceMeta::extract(&[("OEBPS/content.opf".into(), EPUB3_OPF.as_bytes().to_vec())]);
        assert_eq!(meta.title, "");
        assert_eq!(meta.identifier, "");
        // 连 OPF 都没有时也不能崩。
        assert_eq!(SourceMeta::extract(&[]), SourceMeta::default());
    }

    #[test]
    fn opf_gets_correct_language_and_per_language_titles() {
        let plan = Plan::new("原名", "42").with_zh_title("中文名");
        let ja = rewrite_opf(OPF, Lang::Ja, &plan);
        let zh = rewrite_opf(OPF, Lang::Zh, &plan);

        assert!(ja.contains("<dc:language>ja</dc:language>"), "源书写死 zh-CN，日文版必须改回来");
        assert!(zh.contains("<dc:language>zh-CN</dc:language>"));
        assert!(ja.contains("原名"));
        assert!(zh.contains("中文名"));
        assert_ne!(ja, zh);
    }

    #[test]
    fn opf_book_ids_are_distinct_even_without_a_source_identifier() {
        // 源书没有 identifier 时，两本书的 ID 仍必须不同，且不同书之间也要不同。
        let meta = SourceMeta {
            title: "甲".into(),
            identifier: String::new(),
        };
        let a = Plan::new(&meta.title, &meta.identifier);
        let b = Plan::new("乙", &meta.identifier);

        assert_ne!(
            a.book_id(Lang::Ja),
            a.book_id(Lang::Zh),
            "同一本书的两个语言版本不能撞 ID"
        );
        assert_ne!(
            a.book_id(Lang::Ja),
            b.book_id(Lang::Ja),
            "两本不同的书不能撞 ID（早期版本会一起退回同一个常量）"
        );
        // 稳定：同样的输入必须得到同样的 ID。
        assert_eq!(a.book_id(Lang::Ja), Plan::new("甲", "").book_id(Lang::Ja));
    }

    #[test]
    fn opf_inserts_a_missing_identifier() {
        let out = rewrite_opf(EPUB3_OPF, Lang::Ja, &Plan::new("试验书", ""));
        assert!(out.contains("<dc:identifier id=\"BookId\">"), "要按 unique-identifier 补元素");
        assert!(!out.contains("<dc:title id=\"id\">\n   </dc:title>"), "空书名要被填上");
        assert!(out.contains("试验书"));
    }

    #[test]
    fn opf_fixes_the_contradictory_writing_mode() {
        let out = rewrite_opf(OPF, Lang::Ja, &plan());
        assert!(out.contains("content=\"vertical-rl\""));
        assert!(!out.contains("horizontal-lr"));
        // 源书没写这一项时不该凭空加上。
        assert!(!rewrite_opf(EPUB3_OPF, Lang::Ja, &plan()).contains("writing-mode"));
    }

    #[test]
    fn opf_translates_the_guide_title_only_for_chinese() {
        assert!(rewrite_opf(OPF, Lang::Zh, &plan()).contains("title=\"目录\""));
        assert!(rewrite_opf(OPF, Lang::Ja, &plan()).contains("title=\"目次\""));
    }

    #[test]
    fn lang_attribute_is_rewritten_in_both_directions() {
        // 源书声明 zh-CN 时，日文版必须改成 ja（早期版本只做中文方向，日文版会
        // 声明自己是中文书）。
        let source = "<html xmlns=\"…\" xml:lang=\"zh-CN\" xmlns:epub=\"…\">";
        assert!(rewrite_raw(source, Lang::Ja).contains("xml:lang=\"ja\""));
        assert!(rewrite_raw(source, Lang::Zh).contains("xml:lang=\"zh-CN\""));

        // 源书声明 ja 时，中文版必须改成 zh-CN（第一本源书的情况）。
        let source = "<html xml:lang=\"ja\" class=\"vrtl\">";
        assert!(rewrite_raw(source, Lang::Zh).contains("xml:lang=\"zh-CN\""));
        assert!(rewrite_raw(source, Lang::Ja).contains("xml:lang=\"ja\""));
    }

    #[test]
    fn lang_rewrite_does_not_touch_other_attributes() {
        let source = "<html xml:lang=\"ja\" class=\"vrtl\" xmlns:xml=\"http://www.w3.org/XML/1998/namespace\">";
        let out = rewrite_raw(source, Lang::Zh);
        assert!(out.contains("class=\"vrtl\""), "别的属性不能被动");
        assert!(out.contains("xmlns:xml="), "命名空间声明不能被当成 lang 属性");
    }

    #[test]
    fn per_file_titles_are_never_replaced_by_the_book_title() {
        // 第二本源书每个文件的 <title> 是章节名之类，甚至有论坛帖标题。
        // 早期版本会用书名把它们全部覆盖掉。
        //
        // 注意这些标题都**不在**映射表里：整段等于表项的（比如「あとがき」）
        // 是会被有意汉化的，见
        // `headings_are_translated_at_every_level`。
        for title in [
            "表紙",
            "プロローグ 魔王様、うっかり爆誕してしまう",
            "【イケメン】トモ様　突撃となりのモンスター　２８９匹目【ポンコツ】",
        ] {
            let doc = format!("<head><title>{title}</title></head>");
            let out = rewrite_raw(&doc, Lang::Zh);
            assert!(out.contains(title), "「{title}」被改写了：{out}");
        }
    }

    #[test]
    fn japanese_edition_keeps_japanese_headings() {
        let doc = "<h4>あとがき</h4>";
        assert_eq!(rewrite_raw(doc, Lang::Ja), doc);
    }

    #[test]
    fn headings_are_translated_at_every_level() {
        // 第一本源书用 h2/h3，第二本用 h4，`<title>` 也要一起处理。
        assert!(rewrite_raw("<h2>幕間</h2>", Lang::Zh).contains(">间章</h2>"));
        assert!(rewrite_raw("<h3>あとがき</h3>", Lang::Zh).contains(">后记</h3>"));
        assert!(rewrite_raw("<h4>あとがき</h4>", Lang::Zh).contains(">后记</h4>"));
        assert!(rewrite_raw("<title>あとがき</title>", Lang::Zh).contains(">后记</title>"));
    }

    #[test]
    fn headings_with_subtitles_are_left_alone() {
        // 「プロローグ 魔王様、…」整段不等于「プロローグ」，不能翻成半中半日。
        let doc = "<h4>プロローグ 魔王様、うっかり爆誕してしまう</h4>";
        assert_eq!(rewrite_raw(doc, Lang::Zh), doc);
    }

    #[test]
    fn heading_translation_does_not_double_apply() {
        let out = rewrite_raw("<h2 class=\"g\">一章</h2>", Lang::Zh);
        assert_eq!(out, "<h2 class=\"g\">第一章</h2>");
        assert_eq!(out.matches("第一章").count(), 1);
        assert!(!out.contains("第一第一章"));
    }

    #[test]
    fn full_width_numbers_in_headings_are_left_alone() {
        let out = rewrite_raw("<h3 class=\"font-100per\">１</h3>", Lang::Zh);
        assert!(out.contains(">１</h3>"));
    }

    #[test]
    fn ncx_gets_chinese_labels_and_titles() {
        let ncx = concat!(
            "<ncx xml:lang=\"ja\"><docTitle>\n<text>原名</text>\n</docTitle>",
            "<navPoint><navLabel><text>目次</text></navLabel></navPoint>",
            "<navPoint><navLabel><text>奥付</text></navLabel></navPoint></ncx>"
        );
        let zh = rewrite_ncx(ncx, Lang::Zh, &Plan::new("原名", "1").with_zh_title("中文名"));
        assert!(zh.contains("xml:lang=\"zh-CN\""));
        assert!(zh.contains("<text>目录</text>"));
        assert!(zh.contains("<text>版权页</text>"));
        assert!(zh.contains("中文名"));
        assert!(!zh.contains("目次"));

        let ja = rewrite_ncx(ncx, Lang::Ja, &Plan::new("原名", "1"));
        assert!(ja.contains("<text>目次</text>"));
        assert!(ja.contains("原名"));
    }
}
