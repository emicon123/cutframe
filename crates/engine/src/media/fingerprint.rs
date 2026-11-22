//! Content fingerprint of a media file: the cache key and the relink match (design §3, §5).

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::time::UNIX_EPOCH;

use app_core::media::Fingerprint;

use super::error::MediaError;

/// Bytes hashed from each end of the file.
pub const FINGERPRINT_CHUNK: u64 = 1024 * 1024;

/// BLAKE3 over the size (LE u64) + the first 1 MiB + the last 1 MiB, truncated to 128 bits, plus
/// size and modification time. Reads at most 2 MiB; for files under 2 MiB the two ranges do not
/// overlap, so every byte is hashed once.
pub fn fingerprint(path: &Path) -> Result<Fingerprint, MediaError> {
    let io = |err: std::io::Error| MediaError::from_io(&err);
    let mut file = File::open(path).map_err(io)?;
    let meta = file.metadata().map_err(io)?;
    if !meta.is_file() {
        return Err(MediaError::new(
            super::MediaErrorKind::UnsupportedPath,
            "not a regular file",
        ));
    }
    let size = meta.len();
    let modified = modified_millis(&meta);

    let mut hasher = blake3::Hasher::new();
    hasher.update(&size.to_le_bytes());
    let head_len = size.min(FINGERPRINT_CHUNK);
    copy_range(&mut file, 0, head_len, &mut hasher).map_err(io)?;
    let tail_start = head_len.max(size.saturating_sub(FINGERPRINT_CHUNK));
    copy_range(&mut file, tail_start, size - tail_start, &mut hasher).map_err(io)?;

    let digest = hasher.finalize();
    let hash = digest.as_bytes()[..16]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    Ok(Fingerprint {
        size,
        modified,
        hash,
    })
}

/// The file's last-modified time as stored in [`Fingerprint::modified`]: milliseconds since the
/// Unix epoch, 0 when unknown. Compare it with the stored value for the cheap "unchanged" check.
pub fn modified_millis(meta: &std::fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

fn copy_range(
    file: &mut File,
    start: u64,
    len: u64,
    hasher: &mut blake3::Hasher,
) -> std::io::Result<()> {
    if len == 0 {
        return Ok(());
    }
    file.seek(SeekFrom::Start(start))?;
    let mut buf = vec![0; usize::try_from(len).unwrap_or(usize::MAX)];
    file.read_exact(&mut buf)?;
    hasher.update(&buf);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_file(name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("cutframe-fp-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }

    #[test]
    fn hash_is_128_bit_hex_over_size_head_and_tail() {
        let small = temp_file("small.bin", b"hello");
        let fp = fingerprint(&small).unwrap();
        assert_eq!(fp.size, 5);
        assert_eq!(fp.hash.len(), 32);
        assert!(
            fp.hash
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        );
        let mut expected = blake3::Hasher::new();
        expected.update(&5u64.to_le_bytes());
        expected.update(b"hello");
        assert_eq!(fp.hash, expected.finalize().to_hex()[..32]);
        assert!(fp.modified > 0);
    }

    #[test]
    fn only_the_ends_of_large_files_count() {
        let len = 3 * FINGERPRINT_CHUNK as usize;
        let mut bytes = vec![7u8; len];
        let a = fingerprint(&temp_file("a.bin", &bytes)).unwrap();
        // A change in the middle is not seen (by design: the size and mtime catch most edits).
        bytes[len / 2] = 8;
        let b = fingerprint(&temp_file("b.bin", &bytes)).unwrap();
        assert_eq!(a.hash, b.hash);
        // A change in the last MiB is.
        bytes[len - 1] = 8;
        let c = fingerprint(&temp_file("c.bin", &bytes)).unwrap();
        assert_ne!(a.hash, c.hash);
        // So is the size.
        bytes.push(7);
        let d = fingerprint(&temp_file("d.bin", &bytes)).unwrap();
        assert_ne!(c.hash, d.hash);
    }

    #[test]
    fn missing_files_and_directories_are_errors() {
        let dir = temp_file("x.bin", b"x").parent().unwrap().to_owned();
        assert_eq!(
            fingerprint(&dir.join("nope.bin")).unwrap_err().kind,
            super::super::MediaErrorKind::FileNotFound
        );
        assert!(fingerprint(&dir).is_err());
    }
}
