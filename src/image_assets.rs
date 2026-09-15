use anyhow::{Context, Result, bail};
use std::{
    borrow::Cow,
    collections::VecDeque,
    fs::{self, OpenOptions},
    io::{BufRead, BufReader, Cursor, Read, Seek, Write},
    path::{Component, Path, PathBuf},
    sync::Mutex,
};

pub enum ImageInput {
    File(PathBuf),
    Bytes(Vec<u8>),
}

/// Asset directories are portable child paths, never document-relative traversal.
pub fn validate_assets_dir(value: &str) -> Result<()> {
    if value.trim().is_empty()
        || value.contains(['\\', ':'])
        || !Path::new(value)
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
    {
        bail!(
            "image_assets_dir must be a nonempty relative child path without parent traversal, roots, or prefixes"
        );
    }
    Ok(())
}

/// Files and directories owned by this import only. Existing assets are never removed.
#[derive(Default)]
struct ImportBatch {
    files: Vec<PathBuf>,
    directories: Vec<PathBuf>,
    committed: bool,
}

impl Drop for ImportBatch {
    fn drop(&mut self) {
        if !self.committed {
            for path in self.files.iter().rev() {
                let _ = fs::remove_file(path);
            }
            for path in self.directories.iter().rev() {
                let _ = fs::remove_dir(path);
            }
        }
    }
}

fn asset_directory(base: &Path, relative: &str, batch: &mut ImportBatch) -> Result<PathBuf> {
    let base = base.canonicalize().context("Opening document directory")?;
    let mut directory = base.clone();
    for part in Path::new(relative).components() {
        directory.push(part.as_os_str());
        match fs::create_dir(&directory) {
            Ok(()) => batch.directories.push(directory.clone()),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error).context("Creating image assets directory"),
        }
        let metadata =
            fs::symlink_metadata(&directory).context("Checking image assets directory")?;
        if metadata.file_type().is_symlink()
            || !metadata.is_dir()
            || !directory.canonicalize()?.starts_with(&base)
        {
            bail!(
                "Image assets directory must stay inside the document directory and cannot contain symlinks"
            );
        }
    }
    Ok(directory)
}

fn validated_image(
    bytes: &[u8],
) -> Result<(
    &'static str,
    image::DynamicImage,
    image::metadata::Orientation,
)> {
    let format = image::guess_format(bytes)
        .context("Unsupported image format; use PNG, JPEG, GIF, or WebP")?;
    let extension = match format {
        image::ImageFormat::Png => "png",
        image::ImageFormat::Jpeg => "jpg",
        image::ImageFormat::Gif => "gif",
        image::ImageFormat::WebP => "webp",
        _ => bail!("Unsupported image format; use PNG, JPEG, GIF, or WebP"),
    };
    let (image, orientation) =
        decode_pixels(image::ImageReader::with_format(Cursor::new(bytes), format))
            .context("Invalid image data; could not decode the image")?;
    Ok((extension, image, orientation))
}

/// Bound uploads independently of the original photograph's resolution.
const MAX_PREVIEW_DIMENSION: u32 = 1600;
/// A bounded, one-use handoff from import validation to the UI's own bitmap cache.
const MAX_IMPORTED_PREVIEWS: usize = 4;
static IMPORTED_PREVIEWS: Mutex<VecDeque<PreparedPreview>> = Mutex::new(VecDeque::new());

struct PreparedPreview {
    path: PathBuf,
    metadata: fs::Metadata,
    pixels: image::RgbaImage,
}

fn decode_pixels(
    reader: image::ImageReader<impl BufRead + Seek>,
) -> image::ImageResult<(image::DynamicImage, image::metadata::Orientation)> {
    use image::ImageDecoder;

    let mut decoder = reader.into_decoder()?;
    // Keep ImageReader::decode's allocation guard when reading EXIF through
    // the decoder directly; format sniffing alone must not authorize huge buffers.
    let mut limits = image::Limits::default();
    limits.reserve(decoder.total_bytes())?;
    decoder.set_limits(limits)?;
    let orientation = decoder.orientation()?;
    let image = image::DynamicImage::from_decoder(decoder)?;
    Ok((image, orientation))
}

fn preview_pixels(
    mut image: image::DynamicImage,
    orientation: image::metadata::Orientation,
) -> image::RgbaImage {
    // Resize before rotating/mirroring so EXIF transforms touch preview pixels,
    // not every pixel of a full-resolution camera image. The bound is square.
    if image.width() > MAX_PREVIEW_DIMENSION || image.height() > MAX_PREVIEW_DIMENSION {
        image = image.thumbnail(MAX_PREVIEW_DIMENSION, MAX_PREVIEW_DIMENSION);
    }
    image.apply_orientation(orientation);
    image.into_rgba8()
}

fn same_revision(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if left.dev() != right.dev() || left.ino() != right.ino() {
            return false;
        }
    }
    left.len() == right.len() && left.modified().ok() == right.modified().ok()
}

pub(crate) fn load_preview(path: &Path) -> Result<image::RgbaImage, &'static str> {
    let file = fs::File::open(path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            "Missing local image; restore the file or update its path"
        } else {
            "Cannot read local image; check its path and permissions"
        }
    })?;
    if let (Ok(path), Ok(metadata)) = (path.canonicalize(), file.metadata()) {
        let mut prepared = IMPORTED_PREVIEWS
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(index) = prepared.iter().position(|preview| preview.path == path) {
            let preview = prepared.remove(index).unwrap();
            if same_revision(&preview.metadata, &metadata) {
                return Ok(preview.pixels);
            }
        }
    }
    let reader = image::ImageReader::new(BufReader::new(file))
        .with_guessed_format()
        .map_err(|_| "Cannot read local image; check its permissions")?;
    let (image, orientation) = decode_pixels(reader)
        .map_err(|_| "Unsupported or damaged local image; use PNG, JPEG, GIF, or WebP")?;
    Ok(preview_pixels(image, orientation))
}

fn escaped_alt(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for ch in text.chars() {
        if ch.is_ascii_punctuation() {
            escaped.push('\\');
        }
        escaped.push(if ch.is_control() { ' ' } else { ch });
    }
    escaped
}

fn encoded_path(path: &Path) -> String {
    const HEX: &[u8] = b"0123456789ABCDEF";
    let mut encoded = String::new();
    for part in path.components() {
        if !encoded.is_empty() {
            encoded.push('/');
        }
        for byte in part.as_os_str().to_string_lossy().bytes() {
            if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
                encoded.push(byte as char);
            } else {
                encoded.push('%');
                encoded.push(HEX[(byte >> 4) as usize] as char);
                encoded.push(HEX[(byte & 15) as usize] as char);
            }
        }
    }
    encoded
}

/// Copy validated image bytes beside a Markdown document and return one insertion.
/// Names use the decoded format, not the supplied extension. A failed batch rolls
/// back only its newly created files; original files and reused assets are untouched.
pub fn import_images(
    document_path: &Path,
    assets_dir: &str,
    inputs: &[ImageInput],
) -> Result<String> {
    validate_assets_dir(assets_dir)?;
    if inputs.is_empty() {
        return Ok(String::new());
    }
    let base = document_path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut batch = ImportBatch::default();
    let directory = asset_directory(base, assets_dir, &mut batch)?;
    let mut markdown = String::new();
    let mut previews = Vec::with_capacity(inputs.len().min(MAX_IMPORTED_PREVIEWS));
    for input in inputs {
        let (bytes, original): (Cow<'_, [u8]>, Cow<'_, str>) = match input {
            ImageInput::File(path) => (
                Cow::Owned(
                    fs::read(path).with_context(|| format!("Reading image {}", path.display()))?,
                ),
                path.file_stem()
                    .map(|stem| stem.to_string_lossy())
                    .unwrap_or(Cow::Borrowed("image")),
            ),
            ImageInput::Bytes(bytes) => (Cow::Borrowed(bytes), Cow::Borrowed("image")),
        };
        let (extension, image, orientation) = validated_image(&bytes)?;
        let stem: String = original
            .chars()
            .map(|ch| {
                if ch.is_control() || "<>:\"/\\|?*".contains(ch) {
                    '_'
                } else {
                    ch
                }
            })
            .collect();
        let stem = stem.trim_matches([' ', '.']);
        let stem = if stem.is_empty() { "image" } else { stem };
        let mut index = 0_u64;
        let (filename, metadata) = loop {
            let filename = if index == 0 {
                format!("{stem}.{extension}")
            } else {
                format!("{stem}-{index}.{extension}")
            };
            let target = directory.join(&filename);
            match OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&target)
            {
                Ok(mut file) => {
                    batch.files.push(target);
                    file.write_all(&bytes).context("Writing image asset")?;
                    file.sync_all().context("Saving image asset")?;
                    break (filename, file.metadata().context("Checking image asset")?);
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    // Do not follow an asset symlink, even when its bytes happen to match.
                    if fs::symlink_metadata(&target)
                        .is_ok_and(|metadata| metadata.file_type().is_file())
                        && let Ok(mut file) = fs::File::open(&target)
                        && let Ok(metadata) = file.metadata()
                    {
                        let mut existing = Vec::new();
                        if file.read_to_end(&mut existing).is_ok() && existing == bytes.as_ref() {
                            break (filename, metadata);
                        }
                    }
                    index = index
                        .checked_add(1)
                        .context("Too many image name collisions")?;
                }
                Err(error) => return Err(error).context("Creating image asset"),
            }
        };
        if previews.len() < MAX_IMPORTED_PREVIEWS {
            previews.push(PreparedPreview {
                path: directory.join(&filename),
                metadata,
                pixels: preview_pixels(image, orientation),
            });
        }
        if !markdown.is_empty() {
            markdown.push('\n');
        }
        markdown.push_str(&format!(
            "![{}]({})",
            escaped_alt(&original),
            encoded_path(&Path::new(assets_dir).join(filename))
        ));
    }
    batch.committed = true;
    let mut prepared = IMPORTED_PREVIEWS
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    for preview in previews {
        prepared.retain(|existing| existing.path != preview.path);
        prepared.push_back(preview);
    }
    while prepared.len() > MAX_IMPORTED_PREVIEWS {
        prepared.pop_front();
    }
    Ok(markdown)
}

/// Resolve local Markdown image URLs without ever treating a URI or network path
/// as a local image. Decode once so a literal percent escape in a filename survives.
pub fn local_image_path(base: &Path, url: &str) -> Option<PathBuf> {
    fn local(value: &str) -> bool {
        #[cfg(windows)]
        if value
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphabetic)
            && value.as_bytes().get(1) == Some(&b':')
            && matches!(value.as_bytes().get(2), Some(b'/' | b'\\'))
        {
            return true;
        }
        !value.starts_with("//")
            && !value.starts_with("\\\\")
            && !value.split(['/', '\\']).next().unwrap_or("").contains(':')
    }
    if !local(url) {
        return None;
    }
    let url = url.split(['?', '#']).next()?;
    let mut decoded = Vec::with_capacity(url.len());
    let mut bytes = url.bytes();
    while let Some(byte) = bytes.next() {
        decoded.push(if byte == b'%' {
            let hi = (bytes.next()? as char).to_digit(16)?;
            let lo = (bytes.next()? as char).to_digit(16)?;
            (hi * 16 + lo) as u8
        } else {
            byte
        });
    }
    let decoded = String::from_utf8(decoded).ok()?;
    if decoded.is_empty() || decoded.contains('\0') || !local(&decoded) {
        return None;
    }
    Some(base.join(decoded))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(value: u8) -> Vec<u8> {
        let mut bytes = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            1,
            1,
            image::Rgba([value, 0, 0, 255]),
        ))
        .write_to(&mut bytes, image::ImageFormat::Png)
        .unwrap();
        bytes.into_inner()
    }

    fn destination(base: &Path, markdown: &str) -> PathBuf {
        let url = markdown
            .rsplit_once("](")
            .unwrap()
            .1
            .strip_suffix(')')
            .unwrap();
        local_image_path(base, url).unwrap()
    }

    #[test]
    fn supported_formats_keep_their_original_encoded_bytes() {
        let temp = tempfile::tempdir().unwrap();
        for (format, extension) in [
            (image::ImageFormat::Png, "png"),
            (image::ImageFormat::Jpeg, "jpg"),
            (image::ImageFormat::Gif, "gif"),
            (image::ImageFormat::WebP, "webp"),
        ] {
            let mut bytes = Cursor::new(Vec::new());
            image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
                1,
                1,
                image::Rgb([1, 2, 3]),
            ))
            .write_to(&mut bytes, format)
            .unwrap();
            let bytes = bytes.into_inner();
            let source = temp.path().join("misleading.txt");
            fs::write(&source, &bytes).unwrap();
            let markdown = import_images(
                &temp.path().join("note.md"),
                "assets",
                &[ImageInput::File(source)],
            )
            .unwrap();
            let target = destination(temp.path(), &markdown);
            assert_eq!(target.extension().unwrap(), extension);
            assert_eq!(fs::read(target).unwrap(), bytes);
        }
    }

    #[test]
    fn reserved_unicode_names_roundtrip_through_markdown() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("中文 #50% (图)[1].png");
        let bytes = png(1);
        fs::write(&source, &bytes).unwrap();
        let markdown = import_images(
            &temp.path().join("note.md"),
            "assets/图片",
            &[ImageInput::File(source.clone())],
        )
        .unwrap();
        let path = destination(temp.path(), &markdown);
        assert_eq!(path.file_name().unwrap(), "中文 #50% (图)[1].png");
        assert_eq!(fs::read(path).unwrap(), bytes);
        assert_eq!(fs::read(source).unwrap(), bytes);
        let events: Vec<_> = pulldown_cmark::Parser::new(&markdown).collect();
        let url = events
            .iter()
            .find_map(|event| match event {
                pulldown_cmark::Event::Start(pulldown_cmark::Tag::Image { dest_url, .. }) => {
                    Some(dest_url)
                }
                _ => None,
            })
            .unwrap();
        assert_eq!(
            destination(temp.path(), &markdown),
            local_image_path(temp.path(), url).unwrap()
        );
        let alt: String = events
            .iter()
            .filter_map(|event| match event {
                pulldown_cmark::Event::Text(text) => Some(text.as_ref()),
                _ => None,
            })
            .collect();
        assert_eq!(alt, "中文 #50% (图)[1]");
    }

    #[test]
    fn collisions_preserve_originals_and_reuse_identical_assets() {
        let temp = tempfile::tempdir().unwrap();
        let document = temp.path().join("note.md");
        fs::write(&document, "document").unwrap();
        let first = import_images(&document, "assets", &[ImageInput::Bytes(png(1))]).unwrap();
        let second = import_images(&document, "assets", &[ImageInput::Bytes(png(2))]).unwrap();
        assert_ne!(
            destination(temp.path(), &first),
            destination(temp.path(), &second)
        );
        assert_eq!(fs::read(destination(temp.path(), &first)).unwrap(), png(1));
        assert_eq!(fs::read(destination(temp.path(), &second)).unwrap(), png(2));
        assert_eq!(
            import_images(&document, "assets", &[ImageInput::Bytes(png(1))]).unwrap(),
            first
        );
        let source = destination(temp.path(), &first);
        import_images(&document, "assets", &[ImageInput::File(source.clone())]).unwrap();
        assert_eq!(fs::read(source).unwrap(), png(1));
        assert_eq!(fs::read_to_string(document).unwrap(), "document");
    }

    #[test]
    fn invalid_batch_removes_new_assets_but_keeps_reused_assets() {
        let temp = tempfile::tempdir().unwrap();
        let document = temp.path().join("note.md");
        let saved = import_images(&document, "assets", &[ImageInput::Bytes(png(1))]).unwrap();
        assert!(
            import_images(
                &document,
                "assets",
                &[
                    ImageInput::Bytes(png(1)),
                    ImageInput::Bytes(png(2)),
                    ImageInput::Bytes(b"not an image".to_vec()),
                ]
            )
            .is_err()
        );
        assert_eq!(fs::read(destination(temp.path(), &saved)).unwrap(), png(1));
        assert_eq!(fs::read_dir(temp.path().join("assets")).unwrap().count(), 1);
        assert!(import_images(&document, "new/nested", &[ImageInput::Bytes(vec![])]).is_err());
        assert!(!temp.path().join("new").exists());
    }

    #[test]
    fn damaged_pixels_with_valid_headers_roll_back_the_entire_import() {
        let temp = tempfile::tempdir().unwrap();
        let mut damaged = png(7);
        let data = damaged
            .windows(4)
            .position(|bytes| bytes == b"IDAT")
            .unwrap()
            + 4;
        damaged[data] ^= 0xff;
        // A header-only validator would accept this image, but the pixel stream
        // cannot be decoded and must never commit a document insertion.
        assert_eq!(
            image::ImageReader::with_format(Cursor::new(&damaged), image::ImageFormat::Png)
                .into_dimensions()
                .unwrap(),
            (1, 1)
        );
        assert!(
            import_images(
                &temp.path().join("note.md"),
                "assets",
                &[ImageInput::Bytes(png(1)), ImageInput::Bytes(damaged)],
            )
            .is_err()
        );
        assert!(!temp.path().join("assets").exists());
    }

    #[test]
    fn imported_preview_observes_removal_and_replacement_before_first_display() {
        let temp = tempfile::tempdir().unwrap();
        let markdown = import_images(
            &temp.path().join("note.md"),
            "assets",
            &[ImageInput::Bytes(png(1))],
        )
        .unwrap();
        let target = destination(temp.path(), &markdown);
        fs::remove_file(&target).unwrap();
        assert!(load_preview(&target).is_err());
        fs::write(&target, png(99)).unwrap();
        fs::File::options()
            .write(true)
            .open(&target)
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(std::time::SystemTime::UNIX_EPOCH))
            .unwrap();
        assert_eq!(
            load_preview(&target).unwrap().get_pixel(0, 0).0,
            [99, 0, 0, 255]
        );
    }

    #[test]
    fn traversal_and_network_urls_are_rejected() {
        for directory in [
            "",
            " ",
            ".",
            "../assets",
            "assets/../out",
            "/assets",
            "C:/assets",
            "assets\\out",
        ] {
            assert!(validate_assets_dir(directory).is_err(), "{directory}");
        }
        for url in [
            "https://host/image.png",
            "//host/image.png",
            "data:image/png,test",
            "file:///image.png",
            "%2F%2Fhost/image.png",
            "\\\\host\\image.png",
        ] {
            assert!(local_image_path(Path::new("."), url).is_none(), "{url}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn symlink_directories_cannot_escape_and_asset_symlinks_are_not_reused() {
        let temp = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), temp.path().join("assets")).unwrap();
        let document = temp.path().join("note.md");
        assert!(import_images(&document, "assets/nested", &[ImageInput::Bytes(png(1))]).is_err());
        assert!(!outside.path().join("nested").exists());
        fs::remove_file(temp.path().join("assets")).unwrap();
        fs::create_dir(temp.path().join("assets")).unwrap();
        let original = outside.path().join("original.png");
        fs::write(&original, png(1)).unwrap();
        std::os::unix::fs::symlink(&original, temp.path().join("assets/image.png")).unwrap();
        let imported = import_images(&document, "assets", &[ImageInput::Bytes(png(1))]).unwrap();
        assert_eq!(
            destination(temp.path(), &imported).file_name().unwrap(),
            "image-1.png"
        );
        assert_eq!(fs::read(original).unwrap(), png(1));
    }
}
