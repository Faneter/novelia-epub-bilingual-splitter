//! 端到端集成测试：真正拆一遍、写进内存、再回读自检。
//!
//! 各模块内部的单元测试只覆盖自己那一层；这里覆盖整条链路
//! （读取 → 切分 → 改写元数据 → 写回 → 自检），确保拼起来仍然正确。

use std::io::Cursor;

use novelia_epub_bilingual_splitter::audit::{self, Report};
use novelia_epub_bilingual_splitter::epub::{self, Entry};
use novelia_epub_bilingual_splitter::lang::Lang;
use novelia_epub_bilingual_splitter::paths;
use novelia_epub_bilingual_splitter::pipeline::{self, BookStats};

/// 源书实测数字。曾用 PowerShell 独立统计过一次，实现必须与之一致。
const TOTAL_PARAS: usize = 6586; // 全部 <p> 元素
const NEUTRAL_PARAS: usize = 156; // 141 个空行 + 15 个图片页段落
const JAPANESE: usize = 3215; // 日文原文，全部带 opacity 样式
const TRANSLATED: usize = 3215; // 中文译文 = 3371 个裸 <p> - 156 个中立段

/// 跑完整链路：切分 → 写进内存 → 回读自检。
///
/// 写进内存而不是写文件，等于把「写出后重新打开」这条路径也覆盖了，
/// 同时不给测试留下临时文件。
fn split_and_audit(source: &[Entry], lang: Lang) -> (Vec<Entry>, BookStats, Report) {
    let (archive, stats) = pipeline::build(source, lang);

    let mut buf = Cursor::new(Vec::new());
    epub::write(&mut buf, &archive).expect("写出归档");
    let bytes = buf.into_inner();

    let report = audit::inspect(Cursor::new(bytes.clone()), lang).expect("回读自检");
    let reread = epub::read(Cursor::new(bytes)).expect("重新读取");
    assert_eq!(reread, archive, "写回再读出的内容必须与写入时一致");

    (archive, stats, report)
}

fn body_of(entries: &[Entry]) -> String {
    entries
        .iter()
        .filter(|(name, _)| name.ends_with(".xhtml"))
        .map(|(_, data)| String::from_utf8_lossy(data).into_owned())
        .collect::<Vec<_>>()
        .join(" ")
}

fn opf_of(entries: &[Entry]) -> String {
    entries
        .iter()
        .find(|(name, _)| name.ends_with(".opf"))
        .map(|(_, data)| String::from_utf8_lossy(data).into_owned())
        .expect("产物里应该有 OPF")
}

#[test]
fn splits_the_real_book_end_to_end() {
    let Ok(source) = paths::resolve_source(None) else {
        eprintln!("跳过：当前目录下没有源 EPUB");
        return;
    };
    let entries = epub::read_path(&source.path).expect("读取源文件");
    assert!(entries.iter().any(|(name, _)| name.ends_with(".opf")));

    let images = entries
        .iter()
        .filter(|(name, _)| name.contains("/Images/"))
        .count();
    assert!(images > 0, "源书应该有插图");

    let mut identifiers = Vec::new();
    for lang in Lang::ALL {
        let (archive, stats, report) = split_and_audit(&entries, lang);

        assert_eq!(archive.len(), entries.len(), "[{}] 条目数不能变", lang.tag());
        assert_eq!(
            archive.iter().filter(|(n, _)| n.contains("/Images/")).count(),
            images,
            "[{}] 插图是共享资源，两本都要带全",
            lang.tag()
        );

        // 核心不变式：源书每一个段落都必须有归宿——要么进这本书，要么被另一本
        // 收走，要么是两种语言共用的中立段。不能凭空消失，也不能重复计入。
        assert_eq!(
            stats.paragraphs.paragraphs() + stats.paragraphs.dropped,
            TOTAL_PARAS,
            "[{}] 段落总数不守恒",
            lang.tag()
        );
        assert_eq!(
            stats.paragraphs.neutral_kept, NEUTRAL_PARAS,
            "[{}] 中立段（空行 + 图片页）两本都要有",
            lang.tag()
        );
        assert!(
            stats.empty_pages.is_empty(),
            "[{}] 不该有正文被清空的页面: {:?}",
            lang.tag(),
            stats.empty_pages
        );

        // 两本书的正文段数相同 → 原书的日/中是一一对应的。
        assert_eq!(report.content_paragraphs, JAPANESE);
        assert_eq!(report.paragraphs_total(), JAPANESE + NEUTRAL_PARAS);

        assert!(report.is_spec_compliant(), "mimetype 必须是第一个且未压缩");
        assert!(report.is_clean(), "不该残留淡化样式或混进另一种语言");

        let body = body_of(&archive);
        let opf = opf_of(&archive);
        match lang {
            Lang::Ja => {
                assert!(body.contains("空から、何かが落ちてきた。"), "日文正文应保留");
                assert!(!body.contains("有什么东西从天上掉了下来。"), "日文版不该有译文");
                assert!(body.contains("<ruby>"), "日文版必须保留注音");
                assert_eq!(stats.paragraphs.jp_kept, JAPANESE);
                assert_eq!(stats.paragraphs.zh_kept, 0);
                assert_eq!(stats.paragraphs.dropped, TRANSLATED);
                assert!(opf.contains("<dc:language>ja</dc:language>"));
            }
            Lang::Zh => {
                assert!(body.contains("有什么东西从天上掉了下来。"), "中文正文应保留");
                assert!(!body.contains("空から、何かが落ちてきた。"), "中文版不该有原文");
                assert!(!body.contains("<ruby>"), "中文版不应有注音");
                assert_eq!(stats.paragraphs.zh_kept, TRANSLATED);
                assert_eq!(stats.paragraphs.jp_kept, 0);
                assert_eq!(stats.paragraphs.dropped, JAPANESE);
                assert!(opf.contains("<dc:language>zh-CN</dc:language>"));
                // 章标题只有日文，中文版必须靠映射表汉化。
                assert!(body.contains(">第一章</h2>"));
                assert!(!body.contains(">一章</h2>"));
            }
        }

        identifiers.push(
            novelia_epub_bilingual_splitter::markup::element_text(
                &opf,
                "<dc:identifier id=\"uid\">",
                "</dc:identifier>",
            )
            .expect("产物 OPF 必须有 uid"),
        );
    }

    // 两本书的 identifier 必须不同，否则书库会当成同一本书互相覆盖。
    assert_ne!(identifiers[0], identifiers[1]);
}

#[test]
fn splits_a_synthetic_book_end_to_end() {
    // 一本手工造的迷你双语书：封面图、二进制图片、双语正文、纯图片页。
    let source: Vec<Entry> = vec![
        ("mimetype".into(), b"application/epub+zip".to_vec()),
        (
            "META-INF/container.xml".into(),
            br#"<container><rootfiles><rootfile full-path="OEBPS/content.opf"/></rootfiles></container>"#
                .to_vec(),
        ),
        (
            "OEBPS/content.opf".into(),
            concat!(
                "<package><metadata>",
                "<dc:title>试验书</dc:title>",
                "<dc:language>zh-CN</dc:language>",
                "<dc:identifier id=\"uid\">12345</dc:identifier>",
                "</metadata>",
                "<manifest/><spine/></package>"
            )
            .as_bytes()
            .to_vec(),
        ),
        (
            "OEBPS/Text/chapter.xhtml".into(),
            concat!(
                "<html xml:lang=\"ja\" class=\"vrtl\"><head><title>试验书</title></head><body>",
                "<h2 class=\"gfont\">一章</h2>",
                "<p style=\"opacity:0.4;\">むかしむかし、<ruby>昔<rt>むかし</rt></ruby>々々。</p>",
                "<p>从前从前。</p>",
                "<p><br /></p>",
                "<p style=\"opacity:0.4;\">おわり。</p>",
                "<p>完。</p>",
                "</body></html>"
            )
            .as_bytes()
            .to_vec(),
        ),
        (
            "OEBPS/Text/images.xhtml".into(),
            b"<html xml:lang=\"ja\"><body class=\"image-page\"><p><img src=\"../Images/a.jpeg\"/></p></body></html>"
                .to_vec(),
        ),
        // 二进制条目：必须原样透传，不能被当成文本处理。
        ("OEBPS/Images/a.jpeg".into(), vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10]),
    ];

    let mut identifiers = Vec::new();
    for lang in Lang::ALL {
        let (archive, stats, report) = split_and_audit(&source, lang);

        assert_eq!(archive.len(), source.len());
        assert!(report.is_spec_compliant());
        assert!(report.is_clean(), "[{}] 自检应通过", lang.tag());
        assert!(stats.empty_pages.is_empty());

        // 每本书各留 2 段正文（两段日文或两段中文），外加 2 个中立段
        // （一个空行 + 一个图片页），两者相加就是归档里的全部 `<p>`。
        assert_eq!(report.content_paragraphs, 2);
        assert_eq!(report.neutral_paragraphs, 2, "空行 + 图片页");
        assert_eq!(stats.paragraphs.neutral_kept, 2);
        assert_eq!(report.paragraphs_total(), 4);

        let body = body_of(&archive);
        let opf = opf_of(&archive);
        let image = archive
            .iter()
            .find(|(n, _)| n.ends_with("a.jpeg"))
            .expect("图片必须被复制过来");
        assert_eq!(image.1, vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10], "二进制要原样保留");

        assert!(body.contains("<img"), "图片页两本都要有");
        match lang {
            Lang::Ja => {
                assert!(body.contains("むかしむかし"));
                assert!(body.contains("おわり。"));
                assert!(body.contains("<ruby>"), "注音要保留");
                assert!(!body.contains("从前从前。"));
                assert!(opf.contains("<dc:language>ja</dc:language>"));
                assert!(opf.contains("12345-ja"));
            }
            Lang::Zh => {
                assert!(body.contains("从前从前。"));
                assert!(body.contains("完。"));
                assert!(!body.contains("むかしむかし"));
                assert!(!body.contains("<ruby>"), "中文版不该有注音");
                assert!(body.contains("xml:lang=\"zh-CN\""));
                assert!(body.contains(">第一章</h2>"), "章标题要汉化");
                assert!(opf.contains("<dc:language>zh-CN</dc:language>"));
                assert!(opf.contains("12345-zh"));
            }
        }

        identifiers.push(
            novelia_epub_bilingual_splitter::markup::element_text(
                &opf,
                "<dc:identifier id=\"uid\">",
                "</dc:identifier>",
            )
            .unwrap(),
        );
    }
    assert_ne!(identifiers[0], identifiers[1], "两本书的 identifier 必须不同");
}
