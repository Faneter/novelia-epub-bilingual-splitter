//! 文件系统定位：决定读哪个 EPUB，以及产物叫什么名字。
//!
//! 这一层只跟路径打交道，不打开文件——真正的读写在 [`crate::epub`]。

use std::error::Error;
use std::path::{Path, PathBuf};

use crate::lang::Lang;

/// 选定的输入，以及被放弃的其它候选。
pub struct Source {
    pub path: PathBuf,
    /// 同时发现但未选用的 EPUB，调用方可以提示用户用命令行参数指定。
    pub alternatives: Vec<PathBuf>,
}

/// 决定读哪个 EPUB：给了参数就用参数，否则在当前目录里自动发现。
pub fn resolve_source(explicit: Option<PathBuf>) -> Result<Source, Box<dyn Error>> {
    if let Some(path) = explicit {
        return Ok(Source {
            path,
            alternatives: Vec::new(),
        });
    }

    let mut candidates = Vec::new();
    for entry in std::fs::read_dir(".")? {
        let path = entry?.path();
        if !is_epub(&path) || !path.is_file() || is_generated(&path) {
            continue;
        }
        candidates.push(path);
    }
    candidates.sort();

    if candidates.is_empty() {
        return Err("当前目录下没有 .epub 文件，请将路径作为第一个参数传入".into());
    }
    let path = candidates.remove(0);
    Ok(Source {
        path,
        alternatives: candidates,
    })
}

/// 产物路径：`book.epub` → `book.zh.epub`。
pub fn output_path(source: &Path, lang: Lang) -> PathBuf {
    let stem = source
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "output".into());
    source.with_file_name(format!("{stem}.{}.epub", lang.tag()))
}

/// 从文件名粗略推导书名，作为源 OPF 里没有书名时的兜底。
///
/// 实测源文件名形如 `jp-zh.Ys.カイブツ×カノジョ (講談社ラノベ文庫).epub`：
/// 开头的 `jp-zh.Ys.` 是语言对与压制者标记，不属于书名。规则是从头开始逐段
/// 剥掉「只由 ASCII 字母数字和连字符组成」的点分段，遇到第一段含非 ASCII 字符
/// 就停下。
///
/// 只在**剩下部分含非 ASCII 字符**时才剥，这样纯英文书名里的点（`My.Book`）
/// 不会被误当成标记分隔符。
pub fn title_from_filename(path: &Path) -> String {
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();

    let mut rest = stem.as_str();
    while let Some((head, tail)) = rest.split_once('.') {
        let looks_like_tag = !head.is_empty()
            && head
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
        if !looks_like_tag || tail.is_ascii() {
            break;
        }
        rest = tail;
    }

    let title = rest.trim();
    if title.is_empty() {
        stem
    } else {
        title.to_string()
    }
}

fn is_epub(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("epub"))
}

/// 本程序自己的产物。
///
/// 必须排除掉，否则把输出重新当输入，第二次运行就会去拆自己的成品。
fn is_generated(path: &Path) -> bool {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    Lang::ALL
        .iter()
        .any(|lang| name.ends_with(&format!(".{}.epub", lang.tag())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_path_appends_language_tag() {
        let source = Path::new("books/カイブツ×カノジョ.epub");
        assert_eq!(
            output_path(source, Lang::Ja),
            Path::new("books/カイブツ×カノジョ.ja.epub")
        );
        assert_eq!(
            output_path(source, Lang::Zh),
            Path::new("books/カイブツ×カノジョ.zh.epub")
        );
    }

    #[test]
    fn own_outputs_are_not_treated_as_sources() {
        assert!(is_generated(Path::new("book.ja.epub")));
        assert!(is_generated(Path::new("book.zh.epub")));
        assert!(!is_generated(Path::new("book.epub")));
        assert!(!is_generated(Path::new("jp-zh.カイブツ×カノジョ.epub")));
    }

    #[test]
    fn explicit_path_wins_over_discovery() {
        let source = resolve_source(Some(PathBuf::from("whatever.epub"))).unwrap();
        assert_eq!(source.path, PathBuf::from("whatever.epub"));
        assert!(source.alternatives.is_empty());
    }

    #[test]
    fn title_is_derived_from_the_filename() {
        // 实测的两本源书文件名：前导的 `xx-yy.` 与压制者标记 `Ys.` 都要剥掉。
        assert_eq!(
            title_from_filename(Path::new("jp-zh.Ys.カイブツ×カノジョ (講談社ラノベ文庫).epub")),
            "カイブツ×カノジョ (講談社ラノベ文庫)"
        );
        assert_eq!(
            title_from_filename(Path::new(
                "zh-jp.Ys.幼女系底辺ダンジョン配信者、配信切り忘れてS級モンスターを愛でてたら魔王と勘違いされてバズってしまう_1.epub"
            )),
            "幼女系底辺ダンジョン配信者、配信切り忘れてS級モンスターを愛でてたら魔王と勘違いされてバズってしまう_1"
        );
    }

    #[test]
    fn title_derivation_does_not_eat_dots_in_ascii_titles() {
        // 纯 ASCII 书名里的点不能被当成标记分隔符。
        assert_eq!(title_from_filename(Path::new("My.Book.epub")), "My.Book");
        assert_eq!(title_from_filename(Path::new("book.epub")), "book");
    }
}
