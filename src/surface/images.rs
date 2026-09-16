use crate::app::{Pane, Workspace};
use crate::image_assets::load_preview;
use gpui::*;
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet, VecDeque},
    path::{Path, PathBuf},
    rc::Rc,
    sync::Arc,
    time::SystemTime,
};

const MAX_CACHED_IMAGES: usize = 24;
/// Limit simultaneous full-resolution decodes and their temporary buffers.
const MAX_IMAGE_WORKERS: usize = 2;

type ImageStamp = Option<(Option<SystemTime>, u64)>;
type LoadedImage = Result<Arc<RenderImage>, &'static str>;

struct CachedImage {
    stamp: ImageStamp,
    image: LoadedImage,
}

#[derive(Default)]
struct ImageCache {
    entries: HashMap<PathBuf, CachedImage>,
    lru: VecDeque<PathBuf>,
    visible: HashMap<EntityId, (WeakEntity<Workspace>, HashSet<PathBuf>)>,
    pending: HashSet<PathBuf>,
    queue: VecDeque<PathBuf>,
    workers: usize,
}

impl ImageCache {
    fn get(&mut self, path: &Path) -> Option<LoadedImage> {
        if let Some(pos) = self.lru.iter().position(|p| p == path) {
            self.lru.remove(pos);
        }
        self.lru.push_back(path.to_path_buf());
        self.entries.get(path).map(|entry| entry.image.clone())
    }

    fn insert(&mut self, path: PathBuf, stamp: ImageStamp, image: LoadedImage) {
        self.entries
            .insert(path.clone(), CachedImage { stamp, image });
        // Completion order is not recency. A queued, now-offscreen image must
        // not evict previews that the latest layout is still displaying.
        while self.entries.len() > MAX_CACHED_IMAGES {
            // Visible previews are working memory, not idle cache entries.
            // Evicting them would trigger a decode/redraw loop in tall windows.
            let Some(pos) = self.lru.iter().position(|candidate| {
                candidate != &path
                    && !self
                        .visible
                        .values()
                        .any(|(_, paths)| paths.contains(candidate))
                    && self.entries.contains_key(candidate)
            }) else {
                break;
            };
            if let Some(oldest) = self.lru.remove(pos) {
                self.entries.remove(&oldest);
            }
        }
    }
}

// Per-application state stays on the UI executor; workers only own paths and
// decoded results. In particular, no cache lock spans disk access or decoding.
#[derive(Default)]
struct PreviewImages(Rc<RefCell<ImageCache>>);
impl Global for PreviewImages {}

pub(super) fn load_image(path: &Path, cx: &App) -> LoadedImage {
    let shared = cx.global::<PreviewImages>().0.clone();
    let mut cache = shared.borrow_mut();
    let image = cache.get(path).unwrap_or(Err("Loading local image"));
    if cache.pending.insert(path.to_path_buf()) {
        cache.queue.push_back(path.to_path_buf());
    }
    if cache.workers >= MAX_IMAGE_WORKERS || cache.queue.is_empty() {
        return image;
    }
    cache.workers += 1;
    drop(cache);
    cx.spawn(async move |cx| {
        loop {
            let next = {
                let mut cache = shared.borrow_mut();
                match cache.queue.pop_front() {
                    Some(path) => {
                        let stamp = cache.entries.get(&path).map(|entry| entry.stamp);
                        Some((path, stamp))
                    }
                    None => {
                        cache.workers -= 1;
                        None
                    }
                }
            };
            let Some((path, previous_stamp)) = next else {
                break;
            };
            let (path, stamp, decoded) = cx
                .background_executor()
                .spawn(async move {
                    let stamp = std::fs::metadata(&path)
                        .ok()
                        .map(|metadata| (metadata.modified().ok(), metadata.len()));
                    let decoded = (previous_stamp != Some(stamp)).then(|| decode_image(&path));
                    (path, stamp, decoded)
                })
                .await;
            let changed = decoded.is_some();
            {
                let mut cache = shared.borrow_mut();
                cache.pending.remove(&path);
                if let Some(image) = decoded {
                    cache.insert(path, stamp, image);
                }
            }
            // An unchanged metadata check must not create a redraw/check loop.
            // Refresh the entire layout: aspect ratio changes move following rows.
            if changed {
                let _ = cx.update(|cx| cx.refresh_windows());
            }
        }
    })
    .detach();
    image
}

fn decode_image(path: &Path) -> LoadedImage {
    let mut rgba = load_preview(path)?;
    // The sprite atlas expects BGRA bytes.
    for pixel in rgba.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
    }
    Ok(Arc::new(RenderImage::new(vec![image::Frame::new(rgba)])))
}

pub(super) fn begin_layout(workspace: &Entity<Workspace>, pane: Pane, cx: &mut App) {
    if !cx.has_global::<PreviewImages>() {
        cx.set_global(PreviewImages::default());
    }
    if pane == Pane::Editor {
        let mut cache = cx.global::<PreviewImages>().0.borrow_mut();
        cache
            .visible
            .retain(|_, (workspace, _)| workspace.upgrade().is_some());
        cache.visible.insert(
            workspace.entity_id(),
            (workspace.downgrade(), HashSet::new()),
        );
    }
}

pub(super) fn track_visible(workspace: EntityId, path: &Path, cx: &App) {
    if let Some((_, paths)) = cx
        .global::<PreviewImages>()
        .0
        .borrow_mut()
        .visible
        .get_mut(&workspace)
    {
        paths.insert(path.to_path_buf());
    }
}

#[cfg(test)]
mod tests {
    use crate::image_assets::local_image_path;
    use gpui::TestAppContext;

    fn oriented_jpeg(width: u32, height: u32, orientation: u16) -> Vec<u8> {
        let colors = [[240, 20, 20], [20, 240, 20], [20, 20, 240], [240, 240, 20]];
        let image = image::RgbImage::from_fn(width, height, |x, y| {
            image::Rgb(colors[(usize::from(y >= height / 2) * 2) + usize::from(x >= width / 2)])
        });
        let mut jpeg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 100)
            .encode_image(&image::DynamicImage::ImageRgb8(image))
            .unwrap();
        let mut exif = b"Exif\0\0II\x2a\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0".to_vec();
        exif.extend_from_slice(&orientation.to_le_bytes());
        exif.extend_from_slice(&[0; 6]);
        let mut result = jpeg[..2].to_vec();
        result.extend_from_slice(&[0xff, 0xe1]);
        result.extend_from_slice(&((exif.len() + 2) as u16).to_be_bytes());
        result.extend(exif);
        result.extend_from_slice(&jpeg[2..]);
        result
    }

    #[test]
    fn preview_honors_rotated_and_mirrored_exif_without_changing_source() {
        use crate::image_assets::{ImageInput, import_images};
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("camera.jpg");
        let colors = [[240, 20, 20], [20, 240, 20], [20, 20, 240], [240, 240, 20]];
        for (orientation, corners) in [
            (1, [0, 1, 2, 3]),
            (2, [1, 0, 3, 2]),
            (3, [3, 2, 1, 0]),
            (4, [2, 3, 0, 1]),
            (5, [0, 2, 1, 3]),
            (6, [2, 0, 3, 1]),
            (7, [3, 1, 2, 0]),
            (8, [1, 3, 0, 2]),
        ] {
            let source = oriented_jpeg(64, 32, orientation);
            std::fs::write(&path, &source).unwrap();
            let preview = super::load_preview(&path).unwrap();
            let markdown = import_images(
                &directory.path().join("note.md"),
                "assets",
                &[ImageInput::File(path.clone())],
            )
            .unwrap();
            let url = markdown
                .rsplit_once("](")
                .unwrap()
                .1
                .strip_suffix(')')
                .unwrap();
            let imported = local_image_path(directory.path(), url).unwrap();
            assert_eq!(super::load_preview(&imported).unwrap(), preview);
            assert_eq!(std::fs::read(imported).unwrap(), source);
            let dimensions = if orientation >= 5 { (32, 64) } else { (64, 32) };
            assert_eq!(
                preview.dimensions(),
                dimensions,
                "orientation {orientation}"
            );
            for (index, (x, y)) in [(1, 1), (3, 1), (1, 3), (3, 3)].into_iter().enumerate() {
                let pixel = preview.get_pixel(preview.width() * x / 4, preview.height() * y / 4);
                for channel in 0..3 {
                    assert!(
                        pixel[channel].abs_diff(colors[corners[index]][channel]) < 10,
                        "orientation {orientation}, corner {index}, pixel {pixel:?}"
                    );
                }
            }
            assert_eq!(std::fs::read(&path).unwrap(), source);
        }
    }

    #[test]
    fn portrait_preview_bounds_upload_size_and_preserves_aspect_ratio() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("large.jpg");
        std::fs::write(&path, oriented_jpeg(2000, 1000, 6)).unwrap();
        assert_eq!(
            super::load_preview(&path).unwrap().dimensions(),
            (800, 1600)
        );
    }

    #[gpui::test]
    fn preview_loads_asynchronously_reuses_bitmap_and_reloads_changed_file(
        cx: &mut TestAppContext,
    ) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("image.png");
        image::RgbaImage::new(40, 20).save(&path).unwrap();
        cx.update(|cx| {
            cx.set_global(super::PreviewImages::default());
            for _ in 0..10 {
                assert!(
                    super::load_image(&path, cx).is_err(),
                    "request must not decode inline"
                );
            }
        });
        cx.run_until_parked();
        let first = cx.update(|cx| super::load_image(&path, cx).unwrap());
        cx.run_until_parked();
        let cached = cx.update(|cx| super::load_image(&path, cx).unwrap());
        assert!(std::sync::Arc::ptr_eq(&first, &cached));
        cx.run_until_parked();
        image::RgbaImage::new(20, 60).save(&path).unwrap();
        cx.update(|cx| {
            assert!(std::sync::Arc::ptr_eq(
                &first,
                &super::load_image(&path, cx).unwrap()
            ));
        });
        cx.run_until_parked();
        cx.update(|cx| {
            let changed = super::load_image(&path, cx).unwrap();
            assert_eq!(
                (changed.size(0).width.0, changed.size(0).height.0),
                (20, 60)
            );
            assert!(!std::sync::Arc::ptr_eq(&first, &changed));
        });
        cx.run_until_parked();
    }
}
