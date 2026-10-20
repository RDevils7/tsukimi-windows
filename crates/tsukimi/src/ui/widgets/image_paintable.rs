use std::{
    rc::Rc,
    time::Duration,
};
#[cfg(not(target_os = "linux"))]
use std::time::SystemTime;

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
/// gdk-pixbuf has no async API, so the (potentially slow) decode runs on the
/// shared tokio blocking pool. `http(s)` URIs are fetched with reqwest
/// because GVfs, which provides GIO's network backends on Linux desktops,
/// does not exist on Windows.
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

    let decoded = crate::utils::spawn_tokio_blocking(move || decode_image(&bytes)).await?;

    match decoded {
        DecodedImage::Static(pixbuf) => {
            if cancellable.as_ref().is_some_and(|c| c.is_cancelled()) {
                bail!("image load cancelled");
            }

            Ok(texture_from_pixbuf(&pixbuf).upcast())
        }
        DecodedImage::Animated { animation, iter } => {
            let pixbuf = iter.pixbuf();
            let delay = frame_delay(iter.delay_time());

            Ok(ImagePaintable::new_pixbuf(animation, iter, &pixbuf, delay).upcast())
        }
    }
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

            crate::utils::spawn_tokio_blocking(move || std::fs::read(path)).await?
        }
    }
}

#[cfg(not(target_os = "linux"))]
enum DecodedImage {
    Static(gdk_pixbuf::Pixbuf),
    #[allow(deprecated)]
    Animated {
        animation: gdk_pixbuf::PixbufAnimation,
        iter: gdk_pixbuf::PixbufAnimationIter,
    },
}

#[cfg(not(target_os = "linux"))]
fn decode_image(bytes: &[u8]) -> Result<DecodedImage> {
    let loader = gdk_pixbuf::PixbufLoader::new();
    loader.write(bytes)?;
    loader.close()?;

    if let Some(animation) = loader.animation() {
        let iter = animation.iter(None);
        Ok(DecodedImage::Animated { animation, iter })
    } else if let Some(pixbuf) = loader.pixbuf() {
        Ok(DecodedImage::Static(pixbuf))
    } else {
        bail!("gdk-pixbuf could not decode the image")
    }
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
        obj.imp().image.replace(Some(Rc::new(FrameSource::Pixbuf {
            animation,
            iter,
        })));
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
            let Some(image) = self.imp().image.borrow().as_ref().and_then(|source| match &**source {
                FrameSource::Glycin(image) => Some(image.clone()),
                #[cfg(not(target_os = "linux"))]
                FrameSource::Pixbuf { .. } => None,
            }) else {
                return;
            };

            spawn(glib::clone!(
                #[weak(rename_to = obj)]
                self,
                async move {
                    match image.next_frame().await {
                        Ok(frame) => obj.set_frame(frame),
                        Err(error) => warn!("Failed to load animated image frame with glycin: {error}"),
                    }
                }
            ));
        }

        #[cfg(not(target_os = "linux"))]
        {
            let Some(source) = self.imp().image.borrow().as_ref().map(Rc::clone) else {
                return;
            };

            let FrameSource::Pixbuf { iter, .. } = &*source else {
                return;
            };

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
