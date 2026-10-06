//! Strict ODC CPIO / dvzip support. Stream to private temporary storage, enforce
//! advertised bounds and metadata before publishing. No upstream salvage parser.
use anyhow::{Context, Result, bail};
use luftlift_rs::plist_impl::AskFile;
use std::{
    collections::HashMap,
    fs::File,
    io::{Read, Seek, SeekFrom, Write},
    path::PathBuf,
};

pub fn inflate(
    mut input: File,
    content_type: &str,
    limit: u64,
    cancel: Option<&tokio_util::sync::CancellationToken>,
) -> Result<File> {
    input.rewind()?;
    let mut output = tempfile::tempfile()?;
    if content_type.starts_with("application/x-cpio") {
        let mut prefix = [0; 2];
        let n = input.read(&mut prefix)?;
        input.rewind()?;
        let bytes = if n == 2 && prefix == [0x1f, 0x8b] {
            checked_copy(
                &mut flate2::read::MultiGzDecoder::new(input).take(limit + 1),
                &mut output,
                cancel,
            )?
        } else {
            checked_copy(&mut input.take(limit + 1), &mut output, cancel)?
        };
        if bytes > limit {
            bail!("Archive exceeds consented size");
        }
    } else if content_type.starts_with("application/x-dvzip") {
        let mut written = 0u64;
        loop {
            let mut length = [0; 4];
            let first = input.read(&mut length[..1])?;
            if first == 0 {
                break;
            }
            input.read_exact(&mut length[1..])?;
            let length = u32::from_be_bytes(length) as u64;
            if length == 0
                || input
                    .stream_position()?
                    .checked_add(length)
                    .context("Block length overflow")?
                    > input.metadata()?.len()
            {
                bail!("Invalid dvzip block size");
            }
            let compressed = Read::by_ref(&mut input).take(length);
            let mut decoder = flate2::read::ZlibDecoder::new(compressed);
            let size = checked_copy(
                &mut decoder.by_ref().take(limit - written + 1),
                &mut output,
                cancel,
            )?;
            written = written
                .checked_add(size)
                .context("Archive length overflow")?;
            if written > limit {
                bail!("Decompressed archive exceeds consented size");
            }
            if decoder.total_in() != length {
                bail!("Trailing or malformed zlib bytes");
            }
        }
    } else {
        bail!("Unsupported AirDrop upload type");
    }
    output.rewind()?;
    Ok(output)
}

pub struct Entry {
    pub name: String,
    pub offset: u64,
    pub size: u64,
}
pub fn index(input: &mut File, expected: &[AskFile]) -> Result<Vec<Entry>> {
    input.rewind()?;
    let length = input.metadata()?.len();
    let mut wanted = expected
        .iter()
        .map(|f| (f.file_name.clone(), f.file_size))
        .collect::<HashMap<_, _>>();
    if wanted.len() != expected.len() {
        bail!("Duplicate advertised filename");
    }
    let mut entries = vec![];
    loop {
        let mut header = [0u8; 76];
        input
            .read_exact(&mut header)
            .context("Truncated CPIO archive")?;
        if &header[..6] != b"070707" {
            bail!("Unsupported archive format");
        }
        let octal = |slice: &[u8]| -> Result<u64> {
            Ok(u64::from_str_radix(std::str::from_utf8(slice)?, 8)?)
        };
        let mode = octal(&header[18..24])?;
        let namesize = octal(&header[59..65])?;
        let size = octal(&header[65..76])?;
        if namesize == 0 || namesize > 2048 {
            bail!("Invalid archive name length");
        }
        let mut name = vec![0; namesize as usize];
        input.read_exact(&mut name)?;
        if name.pop() != Some(0) {
            bail!("Archive name missing terminator");
        }
        let raw = std::str::from_utf8(&name)?;
        if raw == "TRAILER!!!" {
            if size != 0 || !wanted.is_empty() {
                bail!("Archive omitted advertised files");
            }
            break;
        }
        if raw == "." && mode & 0o170000 == 0o040000 && size == 0 {
            continue;
        }
        let name = raw.strip_prefix("./").unwrap_or(raw);
        linuxdrop_storage::validate_name(name)?;
        if mode & 0o170000 != 0o100000 {
            bail!("Only regular archive files are accepted");
        }
        let expected_size = wanted
            .remove(name)
            .context("Unadvertised or duplicate archive entry")?;
        // Some Apple versions omit FileSize in Ask. Zero then means unknown;
        // the total decompression policy still bounds the receive.
        if expected_size != 0 && expected_size != size {
            bail!("Archive file size differs from request");
        }
        let offset = input.stream_position()?;
        if offset
            .checked_add(size)
            .context("Archive length overflow")?
            > length
        {
            bail!("Truncated archive data");
        }
        input.seek(SeekFrom::Start(offset + size))?;
        entries.push(Entry {
            name: name.into(),
            offset,
            size,
        });
    }
    Ok(entries)
}

/// Create independently compressed 1 MiB frames, keeping memory bounded.
pub fn encode(
    files: &[PathBuf],
    cancel: Option<&tokio_util::sync::CancellationToken>,
) -> Result<(File, u64)> {
    let mut cpio = tempfile::tempfile()?;
    for (index, path) in files.iter().enumerate() {
        let mut file = File::open(path)?;
        let meta = file.metadata()?;
        if !meta.is_file() {
            bail!("Only regular files can be shared");
        }
        let name = path
            .file_name()
            .context("Missing filename")?
            .to_str()
            .context("Filename must be UTF-8")?;
        linuxdrop_storage::validate_name(name)?;
        write_header(&mut cpio, name, meta.len(), index as u32)?;
        let copied = checked_copy(
            &mut Read::by_ref(&mut file).take(meta.len() + 1),
            &mut cpio,
            cancel,
        )?;
        if copied != meta.len() {
            bail!("Source file changed during preparation");
        }
    }
    write_header(&mut cpio, "TRAILER!!!", 0, files.len() as u32)?;
    cpio.rewind()?;
    let mut out = tempfile::tempfile()?;
    let mut block = vec![0; 1024 * 1024];
    loop {
        if cancel.is_some_and(|c| c.is_cancelled()) {
            bail!("Transfer cancelled");
        }
        let count = cpio.read(&mut block)?;
        if count == 0 {
            break;
        }
        let mut compressor =
            flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
        compressor.write_all(&block[..count])?;
        let compressed = compressor.finish()?;
        out.write_all(&(compressed.len() as u32).to_be_bytes())?;
        out.write_all(&compressed)?;
    }
    let length = out.stream_position()?;
    out.rewind()?;
    Ok((out, length))
}
fn write_header(out: &mut File, name: &str, size: u64, index: u32) -> Result<()> {
    if size > 0o77777777777 {
        bail!("File too large for AirDrop ODC encoding");
    }
    write!(
        out,
        "070707000000{:06o}10060000000000000000000100000000000000000{:06o}{:011o}",
        index & 0o777777,
        name.len() + 1,
        size
    )?;
    out.write_all(name.as_bytes())?;
    out.write_all(&[0])?;
    Ok(())
}

fn checked_copy(
    reader: &mut impl Read,
    writer: &mut impl Write,
    cancel: Option<&tokio_util::sync::CancellationToken>,
) -> Result<u64> {
    let mut count = 0;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        if cancel.is_some_and(|c| c.is_cancelled()) {
            bail!("Transfer cancelled");
        }
        let n = reader.read(&mut buffer)?;
        if n == 0 {
            return Ok(count);
        }
        writer.write_all(&buffer[..n])?;
        count += n as u64;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn archive_roundtrip_and_consent_boundary() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hello.txt");
        std::fs::write(&path, b"hello").unwrap();
        let (encoded, _) = encode(&[path], None).unwrap();
        let mut decoded = inflate(encoded, "application/x-dvzip", 4096, None).unwrap();
        let offer = AskFile {
            file_name: "hello.txt".into(),
            file_type: "public.data".into(),
            file_size: 5,
            file_bom_path: "./hello.txt".into(),
        };
        assert_eq!(index(&mut decoded, &[offer]).unwrap()[0].size, 5);
        assert!(index(&mut decoded, &[]).is_err());
    }
    #[test]
    fn decompression_limit_is_enforced() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("large");
        std::fs::write(&path, vec![0; 1024 * 1024]).unwrap();
        let (encoded, _) = encode(&[path], None).unwrap();
        assert!(inflate(encoded, "application/x-dvzip", 1024, None).is_err());
    }
    #[test]
    fn archive_rejects_traversal_and_truncation() {
        let mut f = tempfile::tempfile().unwrap();
        write_header(&mut f, "../escape", 0, 0).unwrap();
        write_header(&mut f, "TRAILER!!!", 0, 1).unwrap();
        f.rewind().unwrap();
        assert!(index(&mut f, &[]).is_err());
        let mut f = tempfile::tempfile().unwrap();
        write_header(&mut f, "x", 20, 0).unwrap();
        f.rewind().unwrap();
        assert!(index(&mut f, &[]).is_err());
    }
}
