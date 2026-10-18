//! Old and new versions of image files, so diffs can show them as pictures.

use super::{ChangedFile, FileStatus, Repo, StatusEntry};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Once;
use std::time::{Duration, SystemTime};

/// Larger images are not previewed.
const MAX_IMAGE_BYTES: usize = 20 * 1024 * 1024;

const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "svg"];

/// One side of an image diff.
#[derive(Clone, Debug, PartialEq)]
pub struct ImageVersion {
    /// A content-addressed copy in the temp dir. GPUI caches decoded images by
    /// path, so giving every content its own path keeps edits from showing stale pixels.
    pub path: PathBuf,
    pub len: usize,
    /// Pixel dimensions, when the header could be read (not for SVG).
    pub size: Option<(u32, u32)>,
    pub format: &'static str,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ImageVersions {
    pub old: Option<ImageVersion>,
    pub new: Option<ImageVersion>,
}

pub fn is_image_path(path: &str) -> bool {
    Path::new(path).extension().and_then(|e| e.to_str()).map(|e| IMAGE_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str())).unwrap_or(false)
}

impl ImageVersions {
    fn from_bytes(path: &str, old: Option<Vec<u8>>, new: Option<Vec<u8>>) -> Option<ImageVersions> {
        let old = old.and_then(|b| ImageVersion::store(path, &b));
        let new = new.and_then(|b| ImageVersion::store(path, &b));
        (old.is_some() || new.is_some()).then_some(ImageVersions { old, new })
    }
}

impl ImageVersion {
    fn store(path: &str, bytes: &[u8]) -> Option<ImageVersion> {
        if bytes.is_empty() || bytes.len() > MAX_IMAGE_BYTES {
            return None;
        }
        let ext = Path::new(path).extension()?.to_str()?.to_ascii_lowercase();
        let dir = std::env::temp_dir().join("gitter-images");
        static CLEANUP: Once = Once::new();
        CLEANUP.call_once(|| remove_stale_files(&dir));
        let mut hasher = DefaultHasher::new();
        bytes.hash(&mut hasher);
        let file = dir.join(format!("{:016x}-{}.{ext}", hasher.finish(), bytes.len()));
        if !file.exists() {
            std::fs::create_dir_all(&dir).ok()?;
            // Write under a temporary name so the image loader never sees a partial file.
            let tmp = dir.join(format!("{:016x}.{}.tmp", hasher.finish(), std::process::id()));
            std::fs::write(&tmp, bytes).ok()?;
            if std::fs::rename(&tmp, &file).is_err() {
                let _ = std::fs::remove_file(&tmp);
                if !file.exists() {
                    return None;
                }
            }
        }
        let format = format_name(bytes, &ext);
        let size = if format == "SVG" { svg_size(bytes) } else { image_size(bytes) };
        Some(ImageVersion { path: file, len: bytes.len(), size, format })
    }
}

/// Removes previews left over from earlier runs.
fn remove_stale_files(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let cutoff = SystemTime::now() - Duration::from_secs(3 * 24 * 3600);
    for entry in entries.flatten() {
        if entry.metadata().and_then(|m| m.modified()).map(|t| t < cutoff).unwrap_or(false) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

fn format_name(bytes: &[u8], ext: &str) -> &'static str {
    if bytes.starts_with(b"\x89PNG") {
        "PNG"
    } else if bytes.starts_with(&[0xFF, 0xD8]) {
        "JPEG"
    } else if bytes.starts_with(b"GIF8") {
        "GIF"
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        "WebP"
    } else if ext == "svg" {
        "SVG"
    } else {
        "Image"
    }
}

/// Reads the pixel dimensions from a PNG, GIF, JPEG or WebP header.
pub fn image_size(b: &[u8]) -> Option<(u32, u32)> {
    let be16 = |i: usize| -> Option<u32> { Some(u16::from_be_bytes([*b.get(i)?, *b.get(i + 1)?]) as u32) };
    let le16 = |i: usize| -> Option<u32> { Some(u16::from_le_bytes([*b.get(i)?, *b.get(i + 1)?]) as u32) };
    let be32 = |i: usize| -> Option<u32> { Some(u32::from_be_bytes(b.get(i..i + 4)?.try_into().ok()?)) };
    let le24 = |i: usize| -> Option<u32> { Some(u32::from_le_bytes([*b.get(i)?, *b.get(i + 1)?, *b.get(i + 2)?, 0])) };

    if b.starts_with(b"\x89PNG\r\n\x1a\n") && b.get(12..16) == Some(b"IHDR") {
        return Some((be32(16)?, be32(20)?));
    }
    if b.starts_with(b"GIF87a") || b.starts_with(b"GIF89a") {
        return Some((le16(6)?, le16(8)?));
    }
    if b.starts_with(&[0xFF, 0xD8]) {
        let mut i = 2;
        while i + 3 < b.len() {
            if b[i] != 0xFF {
                return None;
            }
            let marker = b[i + 1];
            if marker == 0xFF {
                i += 1;
                continue;
            }
            if marker == 0xD8 || marker == 0x01 || (0xD0..=0xD7).contains(&marker) {
                i += 2;
                continue;
            }
            // Start-of-frame markers carry the size; C4, C8 and CC are other tables.
            if (0xC0..=0xCF).contains(&marker) && ![0xC4, 0xC8, 0xCC].contains(&marker) {
                return Some((be16(i + 7)?, be16(i + 5)?));
            }
            i += 2 + be16(i + 2)? as usize;
        }
        return None;
    }
    if b.len() >= 30 && &b[..4] == b"RIFF" && &b[8..12] == b"WEBP" {
        return match &b[12..16] {
            b"VP8 " => Some((le16(26)? & 0x3FFF, le16(28)? & 0x3FFF)),
            b"VP8L" => {
                let bits = u32::from_le_bytes(b[21..25].try_into().ok()?);
                Some(((bits & 0x3FFF) + 1, ((bits >> 14) & 0x3FFF) + 1))
            }
            b"VP8X" => Some((le24(24)? + 1, le24(27)? + 1)),
            _ => None,
        };
    }
    None
}

/// Reads the size an SVG declares on its root element: `width`/`height` in
/// pixels, or else the `viewBox`.
pub fn svg_size(b: &[u8]) -> Option<(u32, u32)> {
    let text = String::from_utf8_lossy(&b[..b.len().min(8192)]);
    let start = text.find("<svg")?;
    let tag = &text[start..start + text[start..].find('>')?];
    let number = |s: &str| s.trim().trim_end_matches("px").parse::<f32>().ok().filter(|v| *v > 0.);
    if let (Some(w), Some(h)) = (svg_attr(tag, "width").and_then(number), svg_attr(tag, "height").and_then(number)) {
        return Some((w.round() as u32, h.round() as u32));
    }
    let view_box: Vec<f32> = svg_attr(tag, "viewBox")?.split(|c: char| c == ',' || c.is_whitespace()).filter_map(|s| s.parse().ok()).collect();
    match view_box[..] {
        [_, _, w, h] if w > 0. && h > 0. => Some((w.round() as u32, h.round() as u32)),
        _ => None,
    }
}

/// The quoted value of attribute `name` in an XML start tag.
fn svg_attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let mut from = 0;
    while let Some(i) = tag[from..].find(name) {
        let at = from + i;
        from = at + name.len();
        // Skip matches inside longer names such as "stroke-width".
        if !tag[..at].ends_with(char::is_whitespace) {
            continue;
        }
        let Some(rest) = tag[from..].trim_start().strip_prefix('=') else { continue };
        let rest = rest.trim_start();
        let quote = rest.chars().next().filter(|q| *q == '"' || *q == '\'')?;
        let rest = &rest[1..];
        return rest.find(quote).map(|end| &rest[..end]);
    }
    None
}

impl Repo {
    /// Raw contents of `path` at `rev` (an empty rev means the index).
    fn blob(&self, rev: &str, path: &str) -> Option<Vec<u8>> {
        self.read_bytes(&["cat-file", "blob", &format!("{rev}:{path}")]).ok()
    }

    /// Image versions of a working copy file: index vs. disk, or HEAD vs. index when `staged`.
    pub(super) fn working_images(&self, entry: &StatusEntry, staged: bool) -> Option<ImageVersions> {
        if !is_image_path(&entry.path) || entry.status == FileStatus::Conflicted {
            return None;
        }
        let (old, new) = if staged {
            let old_path = entry.old_path.as_deref().unwrap_or(&entry.path);
            let old = if entry.status == FileStatus::Added { None } else { self.blob("HEAD", old_path) };
            let new = if entry.status == FileStatus::Deleted { None } else { self.blob("", &entry.path) };
            (old, new)
        } else {
            let old = if entry.status == FileStatus::Untracked { None } else { self.blob("", &entry.path) };
            (old, std::fs::read(self.workdir.join(&entry.path)).ok())
        };
        ImageVersions::from_bytes(&entry.path, old, new)
    }

    /// Image versions of a file changed by commit `oid` (relative to `parent`).
    pub(super) fn commit_images(&self, oid: &str, parent: Option<&str>, file: &ChangedFile) -> Option<ImageVersions> {
        if !is_image_path(&file.path) {
            return None;
        }
        let old_path = file.old_path.as_deref().unwrap_or(&file.path);
        let old = match parent {
            Some(p) if !matches!(file.status, FileStatus::Added | FileStatus::Untracked) => self.blob(p, old_path),
            _ => None,
        };
        let new = if file.status == FileStatus::Deleted { None } else { self.blob(oid, &file.path) };
        ImageVersions::from_bytes(&file.path, old, new)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_paths() {
        assert!(is_image_path("assets/logo.PNG"));
        assert!(is_image_path("a/b.svg"));
        assert!(!is_image_path("src/main.rs"));
        assert!(!is_image_path("png"));
    }

    #[test]
    fn png_and_gif_sizes() {
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
        png.extend_from_slice(&640u32.to_be_bytes());
        png.extend_from_slice(&480u32.to_be_bytes());
        assert_eq!(image_size(&png), Some((640, 480)));

        let mut gif = b"GIF89a".to_vec();
        gif.extend_from_slice(&32u16.to_le_bytes());
        gif.extend_from_slice(&16u16.to_le_bytes());
        assert_eq!(image_size(&gif), Some((32, 16)));
    }

    #[test]
    fn jpeg_size_skips_segments() {
        // SOI, an APP0 segment of length 4, then SOF0 with height 20 and width 30.
        let jpg = [0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x04, 0x00, 0x00, 0xFF, 0xC0, 0x00, 0x11, 0x08, 0x00, 0x14, 0x00, 0x1E, 0x03];
        assert_eq!(image_size(&jpg), Some((30, 20)));
    }

    #[test]
    fn svg_sizes() {
        assert_eq!(svg_size(br#"<?xml version="1.0"?><svg xmlns="http://www.w3.org/2000/svg" width="120px" height='80'><rect stroke-width="2"/></svg>"#), Some((120, 80)));
        assert_eq!(svg_size(b"<svg\n  viewBox=\"0 0 24 16\"\n  stroke-width=\"2\">"), Some((24, 16)));
        assert_eq!(svg_size(br#"<svg width="100%" height="100%" viewBox="0,0,48,32">"#), Some((48, 32)));
        assert_eq!(svg_size(br#"<svg stroke-width="3">"#), None);
    }

    #[test]
    fn versions_from_repo() {
        let dir = std::env::temp_dir().join(format!("gitter-test-images-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let repo = Repo::init(&dir).unwrap();
        repo.run(&["config", "user.name", "Test"]).unwrap();
        repo.run(&["config", "user.email", "test@example.com"]).unwrap();
        // Just a PNG header, which is all the size probe reads.
        let png = |width: u32| {
            let mut b = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
            b.extend_from_slice(&width.to_be_bytes());
            b.extend_from_slice(&8u32.to_be_bytes());
            b
        };
        let sizes = |v: &ImageVersions| (v.old.as_ref().and_then(|i| i.size), v.new.as_ref().and_then(|i| i.size));

        std::fs::write(dir.join("a.png"), png(1)).unwrap();
        repo.stage_all().unwrap();
        repo.commit("one", false, false).unwrap();
        std::fs::write(dir.join("a.png"), png(2)).unwrap();
        let (_, status) = repo.status().unwrap();
        let unstaged = repo.working_diff(&status.unstaged[0], false, 3).unwrap();
        assert!(unstaged.binary);
        assert_eq!(sizes(unstaged.images.as_ref().unwrap()), (Some((1, 8)), Some((2, 8))));

        repo.stage_all().unwrap();
        std::fs::write(dir.join("a.png"), png(3)).unwrap();
        let (_, status) = repo.status().unwrap();
        let staged = repo.working_diff(&status.staged[0], true, 3).unwrap();
        assert_eq!(sizes(staged.images.as_ref().unwrap()), (Some((1, 8)), Some((2, 8))));

        repo.commit("two", false, false).unwrap();
        let log = repo.log(&super::super::LogScope::Head, 0, 10).unwrap();
        let details = repo.commit_details(&log[0].oid).unwrap();
        let diff = repo.commit_file_diff(&details.oid, details.parents.first().map(|s| s.as_str()), &details.files[0]).unwrap();
        assert_eq!(sizes(diff.images.as_ref().unwrap()), (Some((1, 8)), Some((2, 8))));
        let root = repo.commit_details(&log[1].oid).unwrap();
        let diff = repo.commit_file_diff(&root.oid, None, &root.files[0]).unwrap();
        assert_eq!(sizes(diff.images.as_ref().unwrap()), (None, Some((1, 8))));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn webp_lossless_size() {
        let mut webp = b"RIFF\0\0\0\0WEBPVP8L\0\0\0\0\x2f".to_vec();
        // 14 bits width-1, 14 bits height-1.
        let bits: u32 = (99) | (49 << 14);
        webp.extend_from_slice(&bits.to_le_bytes());
        webp.resize(30, 0);
        assert_eq!(image_size(&webp), Some((100, 50)));
    }
}
