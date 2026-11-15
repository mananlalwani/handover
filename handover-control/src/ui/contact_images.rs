//! Bounded asynchronous contact photo decoding for the GTK contact list.

use gtk::{gdk, gdk_pixbuf, gio, glib, prelude::*};
use std::{
    cell::RefCell,
    collections::{HashMap, VecDeque},
    io::{BufRead, BufReader, Read},
    rc::Rc,
};

const PHOTO_BYTES: usize = 48 * 1024;
const EDGE: i32 = 64;
const MAX_DIMENSION: i32 = 4096;
const MAX_PIXELS: i64 = 16_777_216;
const MAX_QUEUE: usize = 100;
const MAX_CACHE: usize = 128;
const WORKERS: usize = 2;

/// Create the initials avatar immediately. Call `load_photo` after inserting it.
pub fn avatar(name: &str, size: i32) -> adw::Avatar {
    let avatar = adw::Avatar::new(size, Some(name), true);
    avatar.add_css_class("contact-avatar");
    avatar.set_halign(gtk::Align::Center);
    avatar.set_valign(gtk::Align::Center);
    avatar
}

/// Asynchronously load inline base64 contact-photo data into an existing avatar.
///
/// `cache_key` should be the stable device id and local contact id. Rebuilding
/// rows reuses cached textures; duplicate in-flight requests share one decode.
/// The queue holds up to 100 requests with two decoders active, enough for the
/// contacts page's current display limit. Overflow requests leave initials.
pub fn load_photo(avatar: &adw::Avatar, cache_key: &str, photo: Option<&str>) {
    let Some(photo) = photo.map(str::trim).filter(|s| !s.is_empty()) else {
        return;
    };
    if photo.len() > encoded_limit() {
        return;
    }
    let key = format!("{cache_key}:{photo}");
    STATE.with(|cell| {
        let mut state = cell.borrow_mut();
        if let Some(texture) = state.cached(&key) {
            install(avatar, &texture);
            return;
        }
        let weak = avatar.downgrade();
        if let Some(targets) = state.in_flight.get_mut(&key) {
            if targets.len() < MAX_QUEUE {
                targets.push(weak);
            }
            return;
        }
        if state.queue.len() >= MAX_QUEUE {
            return;
        }
        state.in_flight.insert(key.clone(), vec![weak]);
        state.queue.push_back(Job {
            key,
            photo: photo.to_owned(),
        });
        pump(&mut state);
    });
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

#[derive(Default)]
struct State {
    active: usize,
    queue: VecDeque<Job>,
    in_flight: HashMap<String, Vec<glib::WeakRef<adw::Avatar>>>,
    cache: HashMap<String, gdk::Texture>,
    lru: VecDeque<String>,
}

struct Job {
    key: String,
    photo: String,
}

impl State {
    fn cached(&mut self, key: &str) -> Option<gdk::Texture> {
        let texture = self.cache.get(key)?.clone();
        self.lru.retain(|entry| entry != key);
        self.lru.push_back(key.to_owned());
        Some(texture)
    }

    fn put(&mut self, key: String, texture: gdk::Texture) {
        self.cache.insert(key.clone(), texture);
        self.lru.retain(|entry| entry != &key);
        self.lru.push_back(key);
        while self.lru.len() > MAX_CACHE {
            if let Some(old) = self.lru.pop_front() {
                self.cache.remove(&old);
            }
        }
    }
}

fn pump(state: &mut State) {
    while state.active < WORKERS {
        let Some(job) = state.queue.pop_front() else {
            break;
        };
        state.active += 1;
        glib::spawn_future_local(async move {
            let photo = job.photo;
            let decoded = gio::spawn_blocking(move || decode_photo(&photo)).await;
            let texture = decoded.ok().and_then(Result::ok).map(|image| {
                gdk::MemoryTexture::new(
                    image.width,
                    image.height,
                    if image.alpha {
                        gdk::MemoryFormat::R8g8b8a8
                    } else {
                        gdk::MemoryFormat::R8g8b8
                    },
                    &glib::Bytes::from_owned(image.pixels),
                    image.stride,
                )
                .upcast::<gdk::Texture>()
            });
            STATE.with(|cell| {
                let mut state = cell.borrow_mut();
                state.active = state.active.saturating_sub(1);
                let targets = state.in_flight.remove(&job.key).unwrap_or_default();
                if let Some(texture) = texture {
                    state.put(job.key, texture.clone());
                    for target in targets {
                        if let Some(widget) = target.upgrade() {
                            install(&widget, &texture);
                        }
                    }
                }
                pump(&mut state);
            });
        });
    }
}

struct Decoded {
    pixels: Vec<u8>,
    width: i32,
    height: i32,
    stride: usize,
    alpha: bool,
}

fn decode_photo(encoded: &str) -> Result<Decoded, ()> {
    if encoded.len() > encoded_limit() {
        return Err(());
    }
    let bytes = glib::base64_decode(encoded);
    if bytes.is_empty() || bytes.len() > PHOTO_BYTES {
        return Err(());
    }
    let mut input = BufReader::new(std::io::Cursor::new(bytes));
    let prefix = input.fill_buf().map_err(|_| ())?;
    let content_type = gio::content_type_guess(None::<&str>, Some(prefix)).0;
    let detected = gio::content_type_get_mime_type(&content_type);
    let format = raster_format(detected.as_deref()).ok_or(())?;
    let loader = gdk_pixbuf::PixbufLoader::with_type(&format).map_err(|_| ())?;
    let invalid = Rc::new(std::cell::Cell::new(false));
    let bad_dimensions = invalid.clone();
    loader.connect_size_prepared(move |loader, width, height| {
        if !valid_dimensions(width, height) {
            bad_dimensions.set(true);
            loader.set_size(1, 1);
            return;
        }
        let scale = (f64::from(EDGE) / f64::from(width.max(height))).min(1.0);
        loader.set_size(
            (f64::from(width) * scale).max(1.0) as i32,
            (f64::from(height) * scale).max(1.0) as i32,
        );
    });
    let mut buffer = [0u8; 8192];
    loop {
        let count = input.read(&mut buffer).map_err(|_| ())?;
        if count == 0 {
            break;
        }
        if loader.write(&buffer[..count]).is_err() || invalid.get() {
            let _ = loader.close();
            return Err(());
        }
    }
    loader.close().map_err(|_| ())?;
    let original = loader.pixbuf().ok_or(())?;
    if invalid.get() || !valid_dimensions(original.width(), original.height()) {
        return Err(());
    }
    let image = original.apply_embedded_orientation().unwrap_or(original);
    let image = if image.width() > EDGE || image.height() > EDGE {
        let scale = f64::from(EDGE) / f64::from(image.width().max(image.height()));
        image
            .scale_simple(
                (f64::from(image.width()) * scale).max(1.0) as i32,
                (f64::from(image.height()) * scale).max(1.0) as i32,
                gdk_pixbuf::InterpType::Bilinear,
            )
            .ok_or(())?
    } else {
        image
    };
    let side = image.width().min(image.height());
    let x = (image.width() - side) / 2;
    let y = (image.height() - side) / 2;
    let cropped = image.new_subpixbuf(x, y, side, side);
    Ok(Decoded {
        pixels: cropped.read_pixel_bytes().as_ref().to_vec(),
        width: side,
        height: side,
        stride: cropped.rowstride() as usize,
        alpha: cropped.has_alpha(),
    })
}

fn valid_dimensions(width: i32, height: i32) -> bool {
    width > 0
        && height > 0
        && width <= MAX_DIMENSION
        && height <= MAX_DIMENSION
        && i64::from(width) * i64::from(height) <= MAX_PIXELS
}

fn raster_format(detected: Option<&str>) -> Option<String> {
    let normalize = |mime: &str| match mime
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "image/jpg" | "image/pjpeg" => "image/jpeg".to_owned(),
        "image/heif-sequence" => "image/heif".to_owned(),
        "image/heic-sequence" => "image/heic".to_owned(),
        other => other.to_owned(),
    };
    let mime = normalize(detected?);
    if !mime.starts_with("image/") || mime == "image/svg+xml" {
        return None;
    }
    gdk_pixbuf::Pixbuf::formats()
        .into_iter()
        .find(|format| {
            !format.is_disabled()
                && !format.is_scalable()
                && format
                    .mime_types()
                    .iter()
                    .any(|supported| normalize(supported) == mime)
        })
        .and_then(|format| format.name().map(|name| name.to_string()))
}

fn install(avatar: &adw::Avatar, texture: &gdk::Texture) {
    avatar.set_custom_image(Some(texture));
}

fn encoded_limit() -> usize {
    PHOTO_BYTES.div_ceil(3) * 4 + 4
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn malformed_and_oversized_photos_are_rejected() {
        assert!(decode_photo("not base64!").is_err());
        assert!(decode_photo(&"A".repeat(encoded_limit() + 1)).is_err());
        let image =
            gdk_pixbuf::Pixbuf::new(gdk_pixbuf::Colorspace::Rgb, false, 8, 128, 128).unwrap();
        image.fill(0x112233ff);
        let bytes = image.save_to_bufferv("bmp", &[]).unwrap();
        assert!(bytes.len() > PHOTO_BYTES);
        assert!(decode_photo(&glib::base64_encode(&bytes)).is_err());
    }
    #[test]
    fn raster_photo_is_cropped_and_keeps_its_pixel_channels() {
        let image = gdk_pixbuf::Pixbuf::new(gdk_pixbuf::Colorspace::Rgb, true, 8, 8, 4).unwrap();
        image.fill(0x112233ff);
        let bytes = image.save_to_bufferv("png", &[]).unwrap();
        let decoded = decode_photo(&glib::base64_encode(&bytes)).unwrap();
        assert_eq!((decoded.width, decoded.height), (4, 4));
        assert!(decoded.alpha);
        assert_eq!(&decoded.pixels[..4], &[0x11, 0x22, 0x33, 0xff]);
    }
}
