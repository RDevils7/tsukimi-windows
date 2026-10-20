#[cfg(not(target_os = "linux"))]
use std::time::SystemTime;
use std::{
    rc::Rc,
    time::Duration,
};

use anyhow::{
    Result,
    bail,
};
#[cfg(not(target_os = "linux"))]
use gdk_pixbuf::prelude::*;
use gtk::{
    gdk,
    gio,
    glib,
    graphene,
    prelude::*,
    subclass::prelude::*,
};
#[cfg(target_os = "linux")]
use tracing::warn;

#[cfg(target_os = "linux")]
use crate::utils::spawn;

const DEFAULT_ANIMATION_FRAME_DELAY: Duration = Duration::from_millis(100);

/// Where subsequent animation frames come from.
///
/// Linux decodes images through glycin (GNOME's sandboxed image loader),
/// which yields `glycin::Frame`s. Everywhere else gdk-pixbuf is used, whose
/// `PixbufAnimationIter` walks the frames of an animation.
pub enum FrameSource {
    #[cfg(target_os = "linux")]
    Glycin(glycin::Image),
    #[allow(deprecated)]
    #[cfg(not(target_os = "linux"))]
    Pixbuf {
        animation: gdk_pixbuf::PixbufAnimation,
        iter: gdk_pixbuf::PixbufAnimationIter,
    },
}

mod imp {
    use std::cell::RefCell;

    use super::*;

    #[derive(Default)]
    pub struct ImagePaintable {
        /// Frame source used to asynchronously request subsequent animation frames.
        pub image: RefCell<Option<Rc<FrameSource>>>,
        /// The currently displayed frame.
        pub frame: RefCell<Option<gdk::Texture>>,
        /// The source ID of the timeout to load the next frame, if any.
        pub timeout_source_id: RefCell<Option<glib::SourceId>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ImagePaintable {
        const NAME: &'static str = "ImagePaintable";
        type Type = super::ImagePaintable;
        type Interfaces = (gdk::Paintable,);
    }

    impl ObjectImpl for ImagePaintable {
        fn dispose(&self) {
            if let Some(source_id) = self.timeout_source_id.borrow_mut().take() {
                source_id.remove();
            }
            self.image.borrow_mut().take();
            self.frame.borrow_mut().take();
        }
    }

    impl PaintableImpl for ImagePaintable {
        fn intrinsic_height(&self) -> i32 {
            self.frame
                .borrow()
                .as_ref()
                .map(|texture| texture.height())
                .unwrap_or(-1)
        }

        fn intrinsic_width(&self) -> i32 {
            self.frame
                .borrow()
                .as_ref()
                .map(|texture| texture.width())
                .unwrap_or(-1)
        }

        fn snapshot(&self, snapshot: &gdk::Snapshot, width: f64, height: f64) {
            if let Some(texture) = &*self.frame.borrow() {
                texture.snapshot(snapshot, width, height);
            } else {
                let snapshot = snapshot.downcast_ref::<gtk::Snapshot>().unwrap();
                snapshot.append_color(
                    &gdk::RGBA::BLACK,
                    &graphene::Rect::new(0f32, 0f32, width as f32, height as f32),
                );
            }
        }

        fn flags(&self) -> gdk::PaintableFlags {
            gdk::PaintableFlags::STATIC_SIZE
        }

        fn current_image(&self) -> gdk::Paintable {
            self.frame
                .borrow()
                .as_ref()
                .cloned()
                .map(|frame| frame.upcast())
                .or_else(|| {
                    let snapshot = gtk::Snapshot::new();
                    self.obj().snapshot(&snapshot, 1.0, 1.0);

                    snapshot.to_paintable(None)
                })
                .expect("there should be a fallback paintable")
        }
    }
}

glib::wrapper! {
    /// A paintable that displays an animated image.
    ///
    /// `gtk::Picture` displays paintables but does not drive animation itself;
    /// this object owns the animation timer, requests the next frame from its
    /// [`FrameSource`], and invalidates its contents when the current frame
    /// changes.
    pub struct ImagePaintable(ObjectSubclass<imp::ImagePaintable>)
        @implements gdk::Paintable;
}

#[cfg(target_os = "linux")]
pub async fn paintable_from_file(
    file: gio::File, cancellable: Option<gio::Cancellable>,
) -> Result<gdk::Paintable> {
    if cancellable.as_ref().is_some_and(|c| c.is_cancelled()) {
        bail!("image load cancelled");
    }

    let mut loader = glycin::Loader::new(file);
    if let Some(cancellable) = cancellable {
        loader.cancellable(cancellable);
    }

    let image = loader.load().await?;
    let frame = image.next_frame().await?;

    if frame.delay().is_some() {
        Ok(ImagePaintable::new(image, frame).upcast())
    } else {
        Ok(frame.texture().upcast())
    }
}

/// gdk-pixbuf based loader for platforms without glycin (Windows, macOS).
///
/// gdk-pixbuf has no async API, so still images are decoded on the shared
/// tokio blocking pool and travel back as raw pixels, because `Pixbuf` is
/// `!Send`. Animated images are decoded on the main thread instead, because
/// `PixbufAnimation` is `!Send` as well. `http(s)` URIs are fetched with
/// reqwest because GVfs, which provides GIO's network backends on Linux
/// desktops, does not exist on Windows.
#[cfg(not(target_os = "linux"))]
pub async fn paintable_from_file(
    file: gio::File, cancellable: Option<gio::Cancellable>,
) -> Result<gdk::Paintable> {
    if cancellable.as_ref().is_some_and(|c| c.is_cancelled()) {
        bail!("image load cancelled");
    }

    let bytes = read_image_bytes(&file).await?;

    if cancellable.as_ref().is_some_and(|c| c.is_cancelled()) {
        bail!("image load cancelled");
    }

    let (decoded, bytes) = crate::utils::spawn_tokio_blocking(move || decode_image(bytes)).await?;

    let Some(image) = decoded else {
        // The image is animated: decode it on the main thread, the only
        // place the gdk-pixbuf animation objects may be used.
        let (animation, iter) = decode_animation(&bytes)?;
        let pixbuf = iter.pixbuf();
        let delay = frame_delay(iter.delay_time());

        return Ok(ImagePaintable::new_pixbuf(animation, iter, &pixbuf, delay).upcast());
    };

    if cancellable.as_ref().is_some_and(|c| c.is_cancelled()) {
        bail!("image load cancelled");
    }

    Ok(texture_from_pixbuf(&image.into_pixbuf()).upcast())
}

#[cfg(not(target_os = "linux"))]
async fn read_image_bytes(file: &gio::File) -> Result<Vec<u8>> {
    match file.uri_scheme().as_deref() {
        Some("http") | Some("https") => {
            let url = file.uri().to_string();

            let response = crate::utils::spawn_tokio(async move {
                crate::client::proxy::ReqClient::build()
                    .get(&url)
                    .send()
                    .await?
                    .error_for_status()?
                    .bytes()
                    .await
            })
            .await?;

            Ok(response.to_vec())
        }
        _ => {
            let path = file
                .path()
                .ok_or_else(|| anyhow::anyhow!("no local path for {}", file.uri()))?;

            Ok(crate::utils::spawn_tokio_blocking(move || std::fs::read(path)).await?)
        }
    }
}

/// Raw pixel data of a decoded still image.
///
/// `gdk_pixbuf::Pixbuf` is `!Send`, so a decoded image cannot travel back
/// from the blocking pool; only its pixels can. The `Pixbuf` is rebuilt from
/// them on the main thread, where GTK objects have to be created anyway.
#[cfg(not(target_os = "linux"))]
struct PixbufData {
    colorspace: gdk_pixbuf::Colorspace,
    has_alpha: bool,
    bits_per_sample: i32,
    width: i32,
    height: i32,
    rowstride: i32,
    pixels: Vec<u8>,
}

#[cfg(not(target_os = "linux"))]
impl PixbufData {
    fn from_pixbuf(pixbuf: &gdk_pixbuf::Pixbuf) -> Self {
        Self {
            colorspace: pixbuf.colorspace(),
            has_alpha: pixbuf.has_alpha(),
            bits_per_sample: pixbuf.bits_per_sample(),
            width: pixbuf.width(),
            height: pixbuf.height(),
            rowstride: pixbuf.rowstride(),
            pixels: pixbuf.read_pixel_bytes().to_vec(),
        }
    }

    fn into_pixbuf(self) -> gdk_pixbuf::Pixbuf {
        // Every parameter comes from a pixbuf gdk-pixbuf decoded itself, so
        // `from_mut_slice`'s assertions always hold.
        gdk_pixbuf::Pixbuf::from_mut_slice(
            self.pixels,
            self.colorspace,
            self.has_alpha,
            self.bits_per_sample,
            self.width,
            self.height,
            self.rowstride,
        )
    }
}

/// Decode a still image on the blocking pool, returning its raw pixels.
///
/// Returns `(None, bytes)` for animated images; those are decoded on the main
/// thread by [`decode_animation`], because the gdk-pixbuf animation objects
/// are `!Send`. The bytes are handed back so that second parse is possible;
/// animated images are rare and the first parse happens off the main thread.
#[cfg(not(target_os = "linux"))]
fn decode_image(bytes: Vec<u8>) -> Result<(Option<PixbufData>, Vec<u8>)> {
    let loader = gdk_pixbuf::PixbufLoader::new();
    loader.write(&bytes)?;
    loader.close()?;

    if loader.animation().is_some() {
        return Ok((None, bytes));
    }

    let Some(pixbuf) = loader.pixbuf() else {
        bail!("gdk-pixbuf could not decode the image");
    };

    Ok((Some(PixbufData::from_pixbuf(&pixbuf)), bytes))
}

/// Decode an animated image on the main thread.
#[cfg(not(target_os = "linux"))]
#[allow(deprecated)]
fn decode_animation(
    bytes: &[u8],
) -> Result<(gdk_pixbuf::PixbufAnimation, gdk_pixbuf::PixbufAnimationIter)> {
    let loader = gdk_pixbuf::PixbufLoader::new();
    loader.write(bytes)?;
    loader.close()?;

    let Some(animation) = loader.animation() else {
        bail!("gdk-pixbuf could not decode the animation");
    };

    let iter = animation.iter(None);

    Ok((animation, iter))
}

#[cfg(not(target_os = "linux"))]
fn texture_from_pixbuf(pixbuf: &gdk_pixbuf::Pixbuf) -> gdk::Texture {
    // `gdk::Texture::for_pixbuf` is deprecated since GTK 4.20 but is still
    // the only zero-copy Pixbuf -> Texture bridge; going through
    // `MemoryTexture` would require repacking every row to strip the pixbuf
    // rowstride padding and expanding RGB to RGBA by hand.
    #[allow(deprecated)]
    gdk::Texture::for_pixbuf(pixbuf)
}

fn frame_delay(delay: Option<Duration>) -> Duration {
    delay
        .filter(|d| !d.is_zero())
        .unwrap_or(DEFAULT_ANIMATION_FRAME_DELAY)
}

impl ImagePaintable {
    #[cfg(target_os = "linux")]
    fn new(image: glycin::Image, frame: glycin::Frame) -> Self {
        let obj = glib::Object::new::<Self>();
        obj.imp()
            .image
            .replace(Some(Rc::new(FrameSource::Glycin(image))));
        obj.set_frame(frame);
        obj
    }

    #[cfg(not(target_os = "linux"))]
    #[allow(deprecated)]
    fn new_pixbuf(
        animation: gdk_pixbuf::PixbufAnimation, iter: gdk_pixbuf::PixbufAnimationIter,
        first_frame: &gdk_pixbuf::Pixbuf, delay: Duration,
    ) -> Self {
        let obj = glib::Object::new::<Self>();
        obj.imp()
            .image
            .replace(Some(Rc::new(FrameSource::Pixbuf { animation, iter })));
        obj.show_frame(texture_from_pixbuf(first_frame), delay);
        obj
    }

    #[cfg(target_os = "linux")]
    fn set_frame(&self, frame: glycin::Frame) {
        let delay = frame_delay(frame.delay());
        self.show_frame(frame.texture(), delay);
    }

    fn show_frame(&self, texture: gdk::Texture, delay: Duration) {
        self.imp().frame.replace(Some(texture));
        self.invalidate_contents();
        self.schedule_next_frame(delay);
    }

    fn schedule_next_frame(&self, delay: Duration) {
        let imp = self.imp();
        if let Some(source_id) = imp.timeout_source_id.borrow_mut().take() {
            source_id.remove();
        }

        let source_id = glib::timeout_add_local_once(
            delay,
            glib::clone!(
                #[weak(rename_to = obj)]
                self,
                move || {
                    obj.imp().timeout_source_id.borrow_mut().take();
                    obj.load_next_frame();
                }
            ),
        );
        imp.timeout_source_id.replace(Some(source_id));
    }

    fn load_next_frame(&self) {
        #[cfg(target_os = "linux")]
        {
            // `glycin::Image` is not `Clone`, so the `Rc` holding the frame
            // source is moved into the spawned future and the borrow is taken
            // inside it; borrowing the `RefCell` guard in the caller could
            // not outlive the `'static` bound of the future.
            let Some(source) = self.imp().image.borrow().as_ref().map(Rc::clone) else {
                return;
            };

            spawn(glib::clone!(
                #[weak(rename_to = obj)]
                self,
                async move {
                    match &*source {
                        FrameSource::Glycin(image) => match image.next_frame().await {
                            Ok(frame) => obj.set_frame(frame),
                            Err(error) => {
                                warn!("Failed to load animated image frame with glycin: {error}")
                            }
                        },
                        #[cfg(not(target_os = "linux"))]
                        FrameSource::Pixbuf { .. } => {}
                    }
                }
            ));
        }

        #[cfg(not(target_os = "linux"))]
        {
            let Some(source) = self.imp().image.borrow().as_ref().map(Rc::clone) else {
                return;
            };

            // Without glycin this is the only variant of `FrameSource`, so
            // the pattern is irrefutable; a `let ... else` would warn.
            let FrameSource::Pixbuf { iter, .. } = &*source;

            if !iter.advance(SystemTime::now()) {
                // The animation is over; keep showing the last frame.
                return;
            }

            let pixbuf = iter.pixbuf();
            let delay = frame_delay(iter.delay_time());

            self.show_frame(texture_from_pixbuf(&pixbuf), delay);
        }
    }
}
