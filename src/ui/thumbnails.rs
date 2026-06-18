//! Sixel rendering of per-device thumbnails for the approve-transfer dialog.
//!
//! Compiled only under the `device-thumbnails` feature. JPEG/PNG images are decoded with the
//! `image` crate and encoded to sixel with `ratatui-image`; transparent pixels (e.g. in PNGs) are
//! left unpainted so the cell background shows through. Both the decoded images and the encoded
//! sixel protocols are cached so the work is done once rather than on every frame.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use ratatui::Frame;
use ratatui::layout::{Rect, Size};
use ratatui_image::picker::{Capability, Picker, ProtocolType};
use ratatui_image::protocol::Protocol;
use ratatui_image::{FilterType, Image, Resize};

/// A protocol is encoded for one image fitted to one cell area, so re-encoding is only needed when
/// that area changes size (e.g. the terminal is resized). The cache is therefore keyed on the
/// source path together with the area's width and height in cells.
type ProtocolCacheKey = (PathBuf, u16, u16);

pub struct ThumbnailRenderer {
    picker: Picker,
    /// Whether the terminal reported sixel support during the capability query. When false we never
    /// emit sixel (which an unsupporting terminal would print as garbage) and let the caller fall
    /// back to a placeholder icon instead.
    sixel_supported: bool,
    /// Decoded source images, keyed by absolute path. A `None` value marks a path that failed to
    /// load so a broken or missing file is not re-read on every frame.
    decoded_images: RefCell<HashMap<PathBuf, Option<Rc<image::DynamicImage>>>>,
    /// Encoded sixel protocols, keyed by path and target area. A `None` value marks a combination
    /// that failed to encode.
    encoded_protocols: RefCell<HashMap<ProtocolCacheKey, Option<Protocol>>>,
}

impl ThumbnailRenderer {
    pub fn new() -> Self {
        // Querying the terminal both detects the font cell size (so images scale correctly) and
        // reports which graphics protocols it supports. If the query fails we keep a sensible
        // fallback font size but treat the terminal as having no graphics support.
        let mut picker = Picker::from_query_stdio()
            .unwrap_or_else(|_| Picker::halfblocks());
        let sixel_supported = picker.capabilities().contains(&Capability::Sixel);
        // The thumbnails were requested specifically as sixel, so force that protocol (the query
        // prefers kitty when both are available). It is only ever used when `sixel_supported`.
        picker.set_protocol_type(ProtocolType::Sixel);
        Self {
            picker,
            sixel_supported,
            decoded_images: RefCell::new(HashMap::new()),
            encoded_protocols: RefCell::new(HashMap::new()),
        }
    }

    /// Decodes (and caches) the image at `path`, returning `None` if it cannot be read or decoded.
    fn decode(&self, path: &Path) -> Option<Rc<image::DynamicImage>> {
        if let Some(cached) = self.decoded_images.borrow().get(path) {
            return cached.clone();
        }
        let decoded = image::ImageReader::open(path)
            .ok()
            .and_then(|reader| reader.with_guessed_format().ok())
            .and_then(|reader| reader.decode().ok())
            .map(Rc::new);
        self.decoded_images
            .borrow_mut()
            .insert(path.to_path_buf(), decoded.clone());
        decoded
    }

    /// Draws the thumbnail at `path` into `area` as sixel, scaled to fit the area while preserving
    /// proportions. Returns `false` without drawing anything when the image cannot be loaded or
    /// encoded, so the caller can fall back to drawing a placeholder icon.
    pub fn render_thumbnail(&self, frame: &mut Frame, area: Rect, path: &Path) -> bool {
        if !self.sixel_supported || area.width == 0 || area.height == 0 {
            return false;
        }

        let cache_key = (path.to_path_buf(), area.width, area.height);
        if !self.encoded_protocols.borrow().contains_key(&cache_key) {
            let protocol = self.decode(path).and_then(|image| {
                self.picker
                    .new_protocol(
                        image.as_ref().clone(),
                        Size::new(area.width, area.height),
                        // Lanczos3 resampling, rather than the default nearest-neighbour, to avoid
                        // aliasing when the (typically larger) source image is scaled to fit.
                        Resize::Fit(Some(FilterType::Lanczos3)),
                    )
                    .ok()
            });
            self.encoded_protocols
                .borrow_mut()
                .insert(cache_key.clone(), protocol);
        }

        if let Some(Some(protocol)) = self.encoded_protocols.borrow().get(&cache_key) {
            frame.render_widget(Image::new(protocol), area);
            true
        } else {
            false
        }
    }
}
