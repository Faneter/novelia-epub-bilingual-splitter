//! 语言领域模型。
//!
//! 本工具只处理两种语言。凡是会出现在元数据里的写法（`ja`、`zh-CN`、书名）
//! 都集中在这里，避免这些字面量散落到各个模块里去。

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

    /// OPF 里的 `dc:language`。
    ///
    /// 注意源书写死成 `zh-CN`，对日文版是错的，所以必须按目标语言改写。
    pub fn dc_language(self) -> &'static str {
        match self {
            Lang::Ja => "ja",
            Lang::Zh => "zh-CN",
        }
    }

    /// 书名。中文名取自源书奥付里的「凯物×女友」。
    pub fn book_title(self) -> &'static str {
        match self {
            Lang::Ja => "カイブツ×カノジョ (講談社ラノベ文庫)",
            Lang::Zh => "凯物×女友",
        }
    }
}

/// 是否含日文假名。
///
/// 源书的中文段落里假名出现 0 次，所以拿它当「混进了未翻译原文」的探针，
/// 供 [`crate::audit`] 做产物自检。注意汉字不算假名——中文里也有汉字。
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
    fn every_language_has_distinct_metadata() {
        let tags: Vec<_> = Lang::ALL.iter().map(|l| l.tag()).collect();
        let langs: Vec<_> = Lang::ALL.iter().map(|l| l.dc_language()).collect();
        let titles: Vec<_> = Lang::ALL.iter().map(|l| l.book_title()).collect();
        // 两本书的这三项都必须不同，否则阅读器会当成同一本书。
        assert_ne!(tags[0], tags[1]);
        assert_ne!(langs[0], langs[1]);
        assert_ne!(titles[0], titles[1]);
    }
}
