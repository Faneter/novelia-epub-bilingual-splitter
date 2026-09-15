//! EPUB 容器层。
//!
//! EPUB 就是一个 ZIP，这一层只负责整包读入 / 写出，完全不关心里面装了什么内容。
//!
//! 读写都写成泛型 `Read + Seek` / `Write + Seek`，所以单元测试用内存里的
//! `Cursor` 就能跑完整链路，不必碰磁盘。

use std::error::Error;
use std::fs::File;
use std::io::{Read, Seek, Write};
use std::path::Path;

use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

/// 归档里的一个条目：条目名 + 原始字节。
pub type Entry = (String, Vec<u8>);

/// 读出全部条目（跳过目录项），保持归档中原有的顺序。
pub fn read<R: Read + Seek>(reader: R) -> Result<Vec<Entry>, Box<dyn Error>> {
    let mut archive = ZipArchive::new(reader)?;
    let mut entries = Vec::with_capacity(archive.len());
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        if entry.is_dir() {
            continue;
        }
        let name = entry.name().to_string();
        let mut buf = Vec::with_capacity(entry.size() as usize);
        entry.read_to_end(&mut buf)?;
        entries.push((name, buf));
    }
    Ok(entries)
}

/// 从磁盘读一个 EPUB。
pub fn read_path(path: &Path) -> Result<Vec<Entry>, Box<dyn Error>> {
    read(File::open(path)?)
}

/// 写出一个 EPUB。
///
/// `mimetype` 必须是**第一个**条目且**不压缩**——这是 EPUB 规范的硬性要求，
/// 否则严格的阅读器会拒收。源文件本身其实不合规（它是 Deflated），这里顺手修正。
pub fn write<W: Write + Seek>(writer: W, entries: &[Entry]) -> Result<(), Box<dyn Error>> {
    let mut zip = ZipWriter::new(writer);

    let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    zip.start_file("mimetype", stored)?;
    zip.write_all(b"application/epub+zip")?;

    let deflated = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    for (name, data) in entries {
        if name == "mimetype" {
            continue; // 上面已经单独写过，不能重复
        }
        zip.start_file(name.as_str(), deflated)?;
        zip.write_all(data)?;
    }

    zip.finish()?;
    Ok(())
}

/// 把归档写到磁盘。
pub fn write_path(path: &Path, entries: &[Entry]) -> Result<(), Box<dyn Error>> {
    write(File::create(path)?, entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn sample() -> Vec<Entry> {
        vec![
            ("mimetype".into(), b"application/epub+zip".to_vec()),
            ("META-INF/container.xml".into(), b"<container/>".to_vec()),
            ("OEBPS/Text/a.xhtml".into(), "<p>你好</p>".as_bytes().to_vec()),
        ]
    }

    #[test]
    fn round_trips_entries_in_order() {
        let mut out = Cursor::new(Vec::new());
        write(&mut out, &sample()).unwrap();
        out.set_position(0);

        let entries = read(out).unwrap();
        assert_eq!(entries, sample(), "读写一圈后条目与顺序都必须不变");
    }

    #[test]
    fn mimetype_is_first_and_uncompressed() {
        let mut out = Cursor::new(Vec::new());
        write(&mut out, &sample()).unwrap();
        out.set_position(0);

        let mut archive = ZipArchive::new(out).unwrap();
        let first = archive.by_index(0).unwrap();
        assert_eq!(first.name(), "mimetype");
        assert_eq!(
            first.compression(),
            CompressionMethod::Stored,
            "mimetype 必须不压缩，否则不符合 EPUB 规范"
        );
    }

    #[test]
    fn mimetype_is_not_written_twice() {
        // 即使调用者传进来的条目里已经带了 mimetype，也只能有一个。
        let mut out = Cursor::new(Vec::new());
        write(&mut out, &sample()).unwrap();
        out.set_position(0);

        let mut archive = ZipArchive::new(out).unwrap();
        let count = (0..archive.len())
            .filter(|i| archive.by_index(*i).unwrap().name() == "mimetype")
            .count();
        assert_eq!(count, 1);
    }

    #[test]
    fn directory_entries_are_skipped() {
        // 造一个带目录项的归档，读出来时目录项应被过滤掉。
        let mut out = Cursor::new(Vec::new());
        {
            let mut zip = ZipWriter::new(&mut out);
            let opts = SimpleFileOptions::default();
            zip.add_directory("OEBPS/", opts).unwrap();
            zip.start_file("OEBPS/a.xhtml", opts).unwrap();
            zip.write_all(b"x").unwrap();
            zip.finish().unwrap();
        }
        out.set_position(0);

        let entries = read(out).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].0, "OEBPS/a.xhtml");
    }
}
