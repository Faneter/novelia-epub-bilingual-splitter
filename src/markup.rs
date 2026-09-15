//! XHTML 片段层：把文档切成 `<p>` 元素和其余原文，外加几处纯文本替换工具。
//!
//! 这一层**刻意不认识「语言」**，只认识标记结构。判断哪一段是日文原文、
//! 哪一段是中文译文是 [`crate::split`] 的职责；这里只回答「这个 `<p>` 带不带
//! 淡化样式」「这段内容是不是只是一张图」。

/// 源书用来标记「日文原文」的内联样式。
///
/// 对照排版里原文被淡化成灰，所以整套样式表其实是空的（5 个 CSS 文件全是
/// 0 字节），全部信息都压在这一个内联属性上——这也正是本工具能靠它做切分的
/// 原因：判别式稳定、且不依赖字符集猜测。
pub const FADING_STYLE: &str = "opacity:0.4;";

/// 一个 XHTML 文档被切成的片段。
pub enum Segment {
    /// `<p>` 以外的原样内容：`<html>`、`<h2>`、`<div>`、闭合标签之间的空白等。
    Raw(String),
    /// 一个完整的 `<p>` 元素，拆成开标签与内容两部分，便于按需重写开标签。
    Para { open: String, inner: String },
}

/// 把文档切成交替出现的 `Raw` 与 `Para` 片段。
///
/// `<p>` 不允许嵌套，所以找到开标签后，第一个 `</p>` 就是配对的闭标签，
/// 不需要引入完整的 XML 解析器（也就不需要额外依赖）。
/// 遇到没有闭合的 `<p>`（源文件损坏）会停止切分，把剩余内容原样留在 `Raw` 里。
pub fn tokenize(doc: &str) -> Vec<Segment> {
    let mut segments = Vec::new();
    // `cursor` 是尚未输出的原文起点，`search` 是下一个查找位置。两者必须分开：
    // 跳到非段落的 `<p…`（比如 `<pre>`）时只推进查找位置，不能推进 cursor，
    // 否则那几个字符会被静默丢掉。
    let mut cursor = 0usize;
    let mut search = 0usize;

    while let Some(rel) = doc[search..].find("<p") {
        let start = search + rel;
        let after = &doc[start + 2..];

        // 必须是 `<p>` 或 `<p …>`，不能把 `<pre>` 之类的标签误当成段落。
        let is_para = after.starts_with('>') || after.starts_with(char::is_whitespace);
        if !is_para {
            search = start + 2;
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
        search = cursor;
    }

    segments.push(Segment::Raw(doc[cursor..].to_string()));
    segments
}

/// 这个段落的内容是否与语言无关。
///
/// 图片页的 `<p><img …/></p>` 和空行 `<p><br /></p>` 在两种语言里都该保留，
/// 若当成正文过滤掉，两本书的段落节奏会被破坏。
pub fn is_neutral_para(inner: &str) -> bool {
    let trimmed = inner.trim();
    trimmed.contains("<img") || trimmed.is_empty() || trimmed == "<br />" || trimmed == "<br/>"
}

/// 取元素内部的文本（只返回第一个匹配）。
pub fn element_text(doc: &str, open: &str, close: &str) -> Option<String> {
    let start = doc.find(open)? + open.len();
    let end = start + doc[start..].find(close)?;
    Some(doc[start..end].trim().to_string())
}

/// 替换元素内部的文本。
///
/// 找不到元素就原样返回——源文件缺某个标签时不应该让整个流程失败。
pub fn replace_element_text(doc: &str, open: &str, close: &str, new_text: &str) -> String {
    let Some(start) = doc.find(open).map(|i| i + open.len()) else {
        return doc.to_string();
    };
    let Some(rel) = doc[start..].find(close) else {
        return doc.to_string();
    };
    let end = start + rel;
    format!("{}{}{}", &doc[..start], new_text, &doc[end..])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paras(doc: &str) -> Vec<(String, String)> {
        tokenize(doc)
            .into_iter()
            .filter_map(|s| match s {
                Segment::Para { open, inner } => Some((open, inner)),
                Segment::Raw(_) => None,
            })
            .collect()
    }

    #[test]
    fn tokenize_splits_paragraphs_and_keeps_the_rest_raw() {
        let doc = "<body><h2>一章</h2><p>甲</p><p>乙</p></body>";
        let segments = tokenize(doc);
        assert!(matches!(&segments[0], Segment::Raw(t) if t == "<body><h2>一章</h2>"));
        assert!(matches!(&segments[1], Segment::Para { inner, .. } if inner == "甲"));
        assert!(matches!(&segments[2], Segment::Raw(t) if t.is_empty()));
        assert!(matches!(&segments[3], Segment::Para { inner, .. } if inner == "乙"));
        assert!(matches!(&segments[4], Segment::Raw(t) if t == "</body>"));
    }

    #[test]
    fn tokenize_preserves_the_open_tag_verbatim() {
        let doc = "<p style=\"opacity:0.4;\">原文</p>";
        let (open, inner) = paras(doc).remove(0);
        assert_eq!(open, "<p style=\"opacity:0.4;\">");
        assert!(open.contains(FADING_STYLE));
        assert_eq!(inner, "原文");
    }

    #[test]
    fn tokenize_does_not_treat_pre_as_a_paragraph() {
        let segments = tokenize("<pre>代码</pre>");
        assert_eq!(segments.len(), 1);
        assert!(matches!(&segments[0], Segment::Raw(t) if t == "<pre>代码</pre>"));
    }

    #[test]
    fn tokenize_keeps_an_unclosed_tail() {
        let segments = tokenize("<p>完整</p><p>被截断");
        let last = segments.last().unwrap();
        assert!(matches!(last, Segment::Raw(t) if t == "<p>被截断"));
    }

    #[test]
    fn tokenize_handles_paragraphs_inside_containers() {
        // 源书里正文常被 <div class="start-1em"> 之类包住，容器不能被吃掉。
        let doc = "<div class=\"main\"><div class=\"start-4em\"><p>甲</p></div></div>";
        assert_eq!(paras(doc).len(), 1);
        let raw: String = tokenize(doc)
            .into_iter()
            .filter_map(|s| match s {
                Segment::Raw(t) => Some(t),
                Segment::Para { .. } => None,
            })
            .collect();
        assert!(raw.contains("start-4em"));
        assert!(raw.contains("</div></div>"));
    }

    #[test]
    fn neutral_paras_are_images_and_blank_lines() {
        assert!(is_neutral_para("<img class=\"fit\" src=\"a.jpeg\" alt=\"\"/>"));
        assert!(is_neutral_para("<br />"));
        assert!(is_neutral_para("  "));
        assert!(!is_neutral_para("你好"));
        assert!(!is_neutral_para("　空から、何かが落ちてきた。"));
    }

    #[test]
    fn element_text_reads_and_trims() {
        let doc = "<dc:language>\n   zh-CN\n  </dc:language>";
        assert_eq!(
            element_text(doc, "<dc:language>", "</dc:language>").unwrap(),
            "zh-CN"
        );
        assert_eq!(element_text(doc, "<dc:title>", "</dc:title>"), None);
    }

    #[test]
    fn replace_element_text_is_lenient_about_missing_elements() {
        let doc = "<dc:language>zh-CN</dc:language>";
        assert_eq!(
            replace_element_text(doc, "<dc:language>", "</dc:language>", "ja"),
            "<dc:language>ja</dc:language>"
        );
        // 元素不存在时原样返回，而不是留下半截内容。
        assert_eq!(replace_element_text(doc, "<dc:bogus>", "</dc:bogus>", "x"), doc);
    }
}
