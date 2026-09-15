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
}
