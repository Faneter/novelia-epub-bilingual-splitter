//! 把「日文原文 + 中文译文」逐段交织排版的 EPUB，拆成两本各自单语的 EPUB。
//!
//! # 判别式：读样式，而不是识别语言
//!
//! 这类对照 EPUB 把两种语言**靠内联样式**区分，而不是靠标记或目录结构：
//! 原文被淡化（`opacity` 小于 1），译文正常显示。整套样式表通常是空的，全部
//! 信息就压在这一个内联属性上。
//!
//! | | 源文件里的写法 | 含义 |
//! |---|---|---|
//! | 日文原文 | `<p style="opacity:0.4;">…</p>` | 淡化显示 |
//! | 中文译文 | `<p>…</p>` | 正常显示 |
//! | 语言无关 | `<p><br /></p>` / `<p><img …/></p>` | 两本书都要保留 |
//!
//! 判定标准是「`opacity` 的值小于 1」而不是固定字面量——淡化多少因书而异。
//!
//! 之所以敢只靠这一条：实测两本对照源书都满足「段落只有淡化/不淡化两类」
//! 「中文段落里假名几乎不出现」「注音 `<ruby>` 只出现在日文段落里」。
//! 所以切分零歧义、不需要语言识别模型，中文版还自动不含注音。
//! 零星出现在译文里的日文专有名词由 [`audit`] 提示，不当故障。
//!
//! # 两本源书的差异（都验证过）
//!
//! | | 第一本 | 第二本 |
//! |---|---|---|
//! | 段落顺序 | 日文在前 | **中文在前** |
//! | 段落属性 | 只有 `style` | 还带 `id` / `class` |
//! | 章标题 | `<h2>` / `<h3>`，只有标签名 | `<h4>`，带副标题 |
//! | 每文件 `<title>` | 都是书名 | **各自的章节名** |
//! | 容器 | EPUB 2.0 + NCX | **EPUB 3 + `nav.xhtml`** |
//! | `dc:identifier` | 有 | **没有（悬空引用）** |
//! | `xml:lang` | `ja` | `zh-CN` |
//!
//! 因此凡是与具体书绑定的信息（书名、章节标题）都必须从源文件读出来或显式传入，
//! 不能写死在代码里。
//!
//! # 容易踩的坑
//!
//! 1. **去掉淡化样式时要保留其它属性**。整段替换成 `<p>` 会丢掉 `id` / `class`。
//! 2. **两个方向都要改写 `xml:lang`**。源书声明 `zh-CN` 时，只做中文方向会让
//!    日文版声明自己是中文书。
//! 3. **不要用书名覆盖每个文件的 `<title>`**。有的书里那是各自的章节名。
//! 4. **两本书的 `dc:identifier` 必须不同**，否则阅读器书库会互相覆盖；
//!    源书没有 identifier 时要派生，不能退回常量。
//!
//! # 模块分工
//!
//! | 模块 | 职责 |
//! |---|---|
//! | [`lang`] | 语言领域模型（`ja` / `zh-CN`） |
//! | [`paths`] | 决定读哪个文件、产物叫什么、书名兜底 |
//! | [`epub`] | ZIP 容器层：整包读入 / 写出 |
//! | [`markup`] | XHTML 片段层：切出 `<p>`、识别/剥离淡化样式 |
//! | [`split`] | 核心：按样式决定段落去留 |
//! | [`metadata`] | 源书元数据提取、OPF / NCX / 章标题改写 |
//! | [`pipeline`] | 编排：组装出一本书 |
//! | [`audit`] | 产物回读自检 |
//!
//! 依赖方向是单向的：`markup` 不认识语言，`split` / `metadata` 都只依赖
//! `lang` + `markup`，`pipeline` 把它们串起来，`main` 只负责打印。
//!
//! # 作为库使用
//!
//! 各模块都是公开的，可以嵌进自己的流程。下面的例子（与 `README.md` 一致）
//! 由 `cargo test` 编译校验，不会过时：
//!
//! ```no_run
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! use novelia_epub_bilingual_splitter::metadata::{Plan, SourceMeta};
//! use novelia_epub_bilingual_splitter::{audit, epub, lang::Lang, pipeline};
//!
//! let entries = epub::read_path("book.epub".as_ref())?;
//!
//! // 书名等每本书各不相同的信息由 Plan 提供；缺失时用文件名兜底。
//! let meta = SourceMeta::extract(&entries);
//! let plan = Plan::new(&meta.title, &meta.identifier);
//!
//! for lang in Lang::ALL {
//!     let (archive, stats) = pipeline::build(&entries, lang, &plan);
//!     println!(
//!         "[{}] 留下 {} 段，丢弃 {} 段",
//!         lang.tag(),
//!         stats.paragraphs.paragraphs(),
//!         stats.paragraphs.dropped
//!     );
//!
//!     let mut buf = std::io::Cursor::new(Vec::new());
//!     epub::write(&mut buf, &archive)?;
//!     buf.set_position(0);
//!
//!     let report = audit::inspect(buf, lang)?;
//!     // fatal_issues 只报「产物不合规 / 整页判错」这类真故障；
//!     // 零星的疑似段落走 warnings，需要人工过目。
//!     assert!(report.fatal_issues().is_empty());
//! }
//! # Ok(())
//! # }
//! ```
//!
//! `epub` 的读写都是泛型的，所以整条链路可以在内存里跑完（如上），
//! 不必落任何临时文件。

pub mod audit;
pub mod epub;
pub mod lang;
pub mod markup;
pub mod metadata;
pub mod paths;
pub mod pipeline;
pub mod split;

pub use epub::Entry;
pub use lang::Lang;
