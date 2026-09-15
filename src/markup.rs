//! XHTML 片段层：把文档切成 `<p>` 元素和其余原文，外加几处纯文本替换工具。
//!
//! 这一层**刻意不认识「语言」**，只认识标记结构。判断哪一段是日文原文、
//! 哪一段是中文译文是 [`crate::split`] 的职责；这里只回答「这个 `<p>` 带不带
//! 淡化样式」「这段内容是不是只是一张图」。

/// 判断一个开标签是否带「淡化」样式。
///
/// 对照排版里原文被淡化成灰，而两本源书的 CSS 文件都是 0 字节（没有任何 class
/// 可供识别），全部信息就压在这一个内联属性上。
///
/// 判定标准是 **`opacity` 的值小于 1**，而不是某个固定字面量——淡化多少因书而异，
/// 写死 `opacity:0.4` 会在换一本书时失效。
pub fn has_fading_style(open_tag: &str) -> bool {
    find_style_attr(open_tag).is_some_and(|style| style.value.split(';').any(is_fading_declaration))
}

/// 去掉开标签里的淡化声明，**保留其它一切属性和声明**。
///
/// 不能简单地把整个开标签换成 `<p>`：实测另一本源书里有
/// `<p id="page_171" class="class_s2t" style="opacity:0.4;">` 这种写法，
/// 整段替换会把这些 `id` / `class` 一起抹掉。
///
/// 没有可删的声明时原样返回，不做无意义改写。
pub fn strip_fading_style(open_tag: &str) -> String {
    let Some(style) = find_style_attr(open_tag) else {
        return open_tag.to_string();
    };
    if !style.value.split(';').any(is_fading_declaration) {
        return open_tag.to_string();
    }

    let kept: Vec<&str> = style
        .value
        .split(';')
        .map(str::trim)
        .filter(|decl| !decl.is_empty() && !is_fading_declaration(decl))
        .collect();

    let mut out = String::with_capacity(open_tag.len());
    if kept.is_empty() {
        // style 属性整个都是用来淡化的 → 连属性一起删掉
        out.push_str(open_tag[..style.attr_start].trim_end());
        out.push_str(&open_tag[style.attr_end..]);
    } else {
        // 还留着别的声明 → 只换掉值
        out.push_str(&open_tag[..style.value_start]);
        out.push_str(&kept.join("; "));
        out.push_str(&open_tag[style.value_end..]);
    }
    out
}

/// `style` 属性在开标签里的位置。
struct StyleAttr<'a> {
    /// `style=` 的起点。
    attr_start: usize,
    /// 闭合引号之后。
    attr_end: usize,
    /// 值本身的起止（不含引号）。
    value_start: usize,
    value_end: usize,
    value: &'a str,
}

/// 找出 `style="…"`：单双引号都认，属性名大小写不敏感。
fn find_style_attr(open_tag: &str) -> Option<StyleAttr<'_>> {
    let bytes = open_tag.as_bytes();
    let mut i = 0usize;
    while i + 7 <= bytes.len() {
        if bytes[i..i + 6].eq_ignore_ascii_case(b"style=") {
            let quote = bytes[i + 6];
            if quote == b'"' || quote == b'\'' {
                let value_start = i + 7;
                let end = open_tag[value_start..].find(quote as char)?;
                let value_end = value_start + end;
                return Some(StyleAttr {
                    attr_start: i,
                    attr_end: value_end + 1,
                    value_start,
                    value_end,
                    value: &open_tag[value_start..value_end],
                });
            }
        }
        i += 1;
    }
    None
}

/// 一条 CSS 声明是不是「淡化」：属性是 `opacity` 且值小于 1。
fn is_fading_declaration(declaration: &str) -> bool {
    let Some((property, value)) = declaration.split_once(':') else {
        return false;
    };
    if !property.trim().eq_ignore_ascii_case("opacity") {
        return false;
    }
    // 容忍 `0.4 !important` 这类写法
    let number = value.split('!').next().unwrap_or("").trim();
    number.parse::<f32>().is_ok_and(|v| v < 1.0)
}

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
///
/// 开标签可以带任意属性，所以 `<dc:title>` 和 `<dc:title id="id">` 都能命中
/// ——实测两本源书在这点上不一样，按死板的开标签匹配会漏。
pub fn element_text(doc: &str, tag: &str) -> Option<String> {
    let close = format!("</{tag}>");
    let open_end = open_tag_end(doc, tag)?;
    let end = open_end + doc[open_end..].find(&close)?;
    Some(doc[open_end..end].trim().to_string())
}

/// 替换元素内部的文本。
///
/// 元素不存在时返回 `None`，让调用方自己决定是「放弃」还是「补一个」——
/// 实测有一本源书压根没有 `<dc:identifier>` 元素。
pub fn replace_element_text(doc: &str, tag: &str, new_text: &str) -> Option<String> {
    let close = format!("</{tag}>");
    let open_end = open_tag_end(doc, tag)?;
    let end = open_end + doc[open_end..].find(&close)?;
    let mut out = String::with_capacity(doc.len());
    out.push_str(&doc[..open_end]);
    out.push_str(new_text);
    out.push_str(&doc[end..]);
    Some(out)
}

/// 找到 `<tag …>` 里 `>` 之后的位置。
fn open_tag_end(doc: &str, tag: &str) -> Option<usize> {
    let needle = format!("<{tag}");
    let after_name = doc.find(&needle)? + needle.len();
    // 后面必须紧跟 `>` 或空白，否则 `<dc:titleX>` 之类会被误命中。
    let next = doc[after_name..].chars().next()?;
    if next != '>' && !next.is_whitespace() {
        return None;
    }
    Some(after_name + doc[after_name..].find('>')? + 1)
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
        assert!(has_fading_style(&open));
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
        assert_eq!(element_text(doc, "dc:language").unwrap(), "zh-CN");
        assert_eq!(element_text(doc, "dc:title"), None);
    }

    #[test]
    fn element_text_accepts_attributes_on_the_open_tag() {
        // 两本源书在这点上不同：一本写 <dc:title>，另一本写 <dc:title id="id">。
        assert_eq!(
            element_text("<dc:title id=\"id\">书名</dc:title>", "dc:title").unwrap(),
            "书名"
        );
        assert_eq!(
            element_text("<dc:identifier id=\"BookId\">42</dc:identifier>", "dc:identifier")
                .unwrap(),
            "42"
        );
    }

    #[test]
    fn element_text_does_not_match_a_longer_tag_name() {
        // `<dc:titleX>` 不该被当成 `<dc:title>`。
        assert_eq!(element_text("<dc:titleX>甲</dc:titleX>", "dc:title"), None);
    }

    #[test]
    fn replace_element_text_reports_a_missing_element() {
        let doc = "<dc:language>zh-CN</dc:language>";
        assert_eq!(
            replace_element_text(doc, "dc:language", "ja").unwrap(),
            "<dc:language>ja</dc:language>"
        );
        // 元素不存在时返回 None，交给调用方决定怎么办。
        assert_eq!(replace_element_text(doc, "dc:bogus", "x"), None);
    }

    #[test]
    fn replace_element_text_keeps_the_open_tag_attributes() {
        assert_eq!(
            replace_element_text("<dc:title id=\"id\">旧</dc:title>", "dc:title", "新").unwrap(),
            "<dc:title id=\"id\">新</dc:title>"
        );
    }

    #[test]
    fn fading_style_is_detected_by_the_opacity_value() {
        assert!(has_fading_style("<p style=\"opacity:0.4;\">"));
        assert!(has_fading_style("<p style=\"opacity: 0.5\">"), "淡化多少因书而异");
        assert!(has_fading_style("<p style=\"OPACITY:0.4\">"), "属性名大小写不敏感");
        assert!(has_fading_style("<p style=\"opacity:0.4 !important\">"));
        assert!(
            has_fading_style("<p id=\"page_171\" class=\"class_s2t\" style=\"opacity:0.4;\">"),
            "淡化声明可以和别的属性共存"
        );
    }

    #[test]
    fn non_fading_paragraphs_are_not_flagged() {
        assert!(!has_fading_style("<p>"));
        assert!(!has_fading_style("<p class=\"class_s2t\">"));
        assert!(!has_fading_style("<p style=\"opacity:1;\">"), "不透明不算淡化");
        assert!(!has_fading_style("<p style=\"opacity:1.0\">"));
        assert!(!has_fading_style("<p style=\"color:red;\">"), "别的属性不能被误判");
    }

    #[test]
    fn stripping_fading_style_keeps_every_other_attribute() {
        // 实测另一本源书的写法：整段替换会连 id / class 一起丢掉。
        assert_eq!(
            strip_fading_style("<p id=\"page_171\" class=\"class_s2t\" style=\"opacity:0.4;\">"),
            "<p id=\"page_171\" class=\"class_s2t\">"
        );
        assert_eq!(
            strip_fading_style("<p class=\"pius2\" style=\"opacity:0.4;\">"),
            "<p class=\"pius2\">"
        );
        assert_eq!(strip_fading_style("<p style=\"opacity:0.4;\">"), "<p>");
    }

    #[test]
    fn stripping_keeps_other_style_declarations() {
        assert_eq!(
            strip_fading_style("<p style=\"color:red; opacity:0.4;\">"),
            "<p style=\"color:red\">"
        );
        assert_eq!(
            strip_fading_style("<p style='opacity:0.4;text-align:center'>"),
            "<p style='text-align:center'>"
        );
    }

    #[test]
    fn stripping_a_paragraph_without_fading_changes_nothing() {
        // 没有可删的声明就必须字节不变，否则会给所有中文段做无意义改写。
        for open in ["<p>", "<p class=\"x\">", "<p style=\"opacity:1;\">", "<p>"] {
            assert_eq!(strip_fading_style(open), open);
        }
    }

    #[test]
    fn stripping_is_idempotent() {
        let once = strip_fading_style("<p id=\"a\" style=\"opacity:0.4;\">");
        assert_eq!(strip_fading_style(&once), once, "去一次和去两次结果必须相同");
    }
}
