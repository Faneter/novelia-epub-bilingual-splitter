//! 语言领域模型。
//!
//! 本工具只处理两种语言，凡是与语言绑定、且**与具体哪本书无关**的写法
//! （文件名后缀、`dc:language`、`xml:lang`）都集中在这里。
//!
//! 书名**不在这里**：它是每本书各不相同的，必须从源文件读出来
//! （见 [`crate::metadata::SourceMeta`]）。曾经把它写死在这张表里，
//! 结果换一本书就会把上一本书的书名盖到新书上。

/// 拆分目标语言。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lang {
    /// 日文原文。
    Ja,
    /// 中文译文。
    Zh,
}

impl Lang {
    /// 需要产出的全部语言。
    pub const ALL: [Lang; 2] = [Lang::Ja, Lang::Zh];

    /// 输出文件名后缀，如 `book.ja.epub`。
    pub fn tag(self) -> &'static str {
        match self {
            Lang::Ja => "ja",
            Lang::Zh => "zh",
        }
    }

    /// 该语言的标准标签：OPF 里的 `dc:language`，同时也是 XHTML 上的
    /// `xml:lang` 取值。
    ///
    /// 源书的 `dc:language` 未必对（实测两本源书都写死成 `zh-CN`），
    /// 所以两个方向都必须按目标语言改写，而不是只改一边。
    pub fn dc_language(self) -> &'static str {
        match self {
            Lang::Ja => "ja",
            Lang::Zh => "zh-CN",
        }
    }
}

/// 是否含日文假名。
///
/// 用来判断「中文段落里是不是混进了未翻译的日文」。注意这只是**启发式**：
/// 中文译本里会合法地保留一些日文专有名词（实测有 `ダンTV`、`とくまろ` 这类
/// 服务名和人名），所以 [`crate::audit`] 只在「整页大面积命中」时才当作故障，
/// 零星命中只提示。
pub fn contains_kana(text: &str) -> bool {
    text.chars()
        .any(|c| matches!(c, '\u{3041}'..='\u{3096}' | '\u{30A1}'..='\u{30FA}' | '\u{30FC}'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_kana() {
        assert!(contains_kana("こんにちは"), "平假名");
        assert!(contains_kana("カイブツ"), "片假名");
        assert!(contains_kana("コーヒー"), "长音符也算");
        assert!(contains_kana("「……っ、」"), "促音");
    }

    #[test]
    fn does_not_flag_chinese_as_japanese() {
        assert!(!contains_kana("你好，世界。"));
        assert!(!contains_kana("凱物×女友"), "汉字不是假名");
        assert!(!contains_kana("「……骗人的吧……？」"));
        assert!(!contains_kana("二〇一五年二月一日发行"));
    }

    #[test]
    fn chinese_translations_may_legitimately_keep_japanese_names() {
        // 这三句都是实测的中文正文，假名部分是保留的专有名词。
        // 它们会命中 contains_kana，所以调用方不能把「有假名」直接当成故障。
        assert!(contains_kana("你竟然在ダンTV的在线人数排行榜上排名第一了！"));
        assert!(contains_kana("负责插画的とくまろ老师。"));
    }

    #[test]
    fn every_language_has_a_distinct_tag() {
        let tags: Vec<_> = Lang::ALL.iter().map(|l| l.tag()).collect();
        let langs: Vec<_> = Lang::ALL.iter().map(|l| l.dc_language()).collect();
        assert_ne!(tags[0], tags[1], "两个产物文件名必须不同，否则会互相覆盖");
        assert_ne!(langs[0], langs[1]);
    }
}
