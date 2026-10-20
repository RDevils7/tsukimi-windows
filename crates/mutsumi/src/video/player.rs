use glib::Object;
use gtk::{
    gdk::ModifierType,
    glib,
    subclass::prelude::*,
};

use crate::{
    ContextedMPV,
    MpvValue,
    PlayParams,
};

use super::backend::{
    BoxedFuture,
    TrackKind,
    TrackSelection,
};

#[cfg(not(target_os = "linux"))]
use crate::MPVGLArea;
#[cfg(target_os = "linux")]
use crate::MutsumiVideoSink;

mod imp {
    #[cfg(target_os = "linux")]
    use std::cell::Cell;

    use adw::{
        prelude::*,
        subclass::prelude::*,
    };
    use gtk::{
        CompositeTemplate,
        CssProvider,
    };

    #[cfg(not(target_os = "linux"))]
    use crate::MPVGLArea;
    #[cfg(target_os = "linux")]
    use crate::{
        MutsumiVideoSink,
        VIEWPORT_CHANNEL,
        Viewport,
    };

    use super::*;
    use crate::video::layout::MutsumiVideoLayout;

    #[derive(Default, CompositeTemplate, glib::Properties)]
    #[template(resource = "/io/github/mutsumiuniverse/mutsumi/ui/video_player.ui")]
    #[properties(wrapper_type = super::MutsumiVideoPlayer)]
    pub struct MutsumiVideoPlayer {
        /// On Linux this is the dmabuf paint source feeding the
        /// `GraphicsOffload` child; elsewhere it is the GLArea that mpv
        /// renders into through the mpv render API.
        #[cfg(target_os = "linux")]
        pub backend: MutsumiVideoSink,
        #[cfg(not(target_os = "linux"))]
        pub backend: MPVGLArea,
        #[cfg(target_os = "linux")]
        last_viewport: Cell<Viewport>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for MutsumiVideoPlayer {
        const NAME: &'static str = "MutsumiVideoPlayer";
        type Type = super::MutsumiVideoPlayer;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            MutsumiVideoLayout::ensure_type();
            klass.bind_template();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for MutsumiVideoPlayer {
        fn constructed(&self) {
            self.parent_constructed();

            let obj = self.obj();

            obj.add_css_class("mutsumi-video-player");

            let provider = CssProvider::new();
            provider.load_from_string(
                "
                .mutsumi-video-player {
                    background: black;
                }",
            );

            gtk::style_context_add_provider_for_display(
                &gtk::gdk::Display::default().expect("Could not connect to display"),
                &provider,
                gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );

            // Build the platform specific video child.
            //
            // Linux (Wayland): mpv renders into a dmabuf which the wl-proxy
            // embedder hands to the compositor; the resulting gdk::Texture
            // is displayed through a `GraphicsOffload` + `Picture` pair so
            // the compositor can keep presenting the last frame efficiently.
            //
            // Everywhere else (Windows, macOS): mpv renders straight into
            // this widget's own GL context through the cross-platform mpv
            // render API (see `MPVGLArea`), which becomes the Bin child.
            #[cfg(target_os = "linux")]
            {
                let picture = gtk::Picture::new();
                picture.set_paintable(Some(&self.backend));
                let offload = gtk::GraphicsOffload::new(Some(&picture));
                obj.set_child(Some(&offload));
            }

            #[cfg(not(target_os = "linux"))]
            obj.set_child(Some(&self.backend));
        }
    }

    impl MutsumiVideoPlayer {
        #[cfg(target_os = "linux")]
        pub fn update_viewport(&self, width: i32, height: i32) {
            if width <= 0 || height <= 0 {
                return;
            }

            let obj = self.obj();

            let Some(native) = obj.native() else {
                tracing::warn!(
                    "Failed to get native from widget. Video may not display correctly."
                );
                return;
            };

            let Some(surface) = native.surface() else {
                tracing::warn!(
                    "Failed to get surface from native. Video may not display correctly."
                );
                return;
            };

            let viewport = Viewport::new(width, height, surface.scale());
            if self.last_viewport.replace(viewport) != viewport {
                VIEWPORT_CHANNEL.send(viewport);
            }
        }

        /// No-op outside Linux: the render API backend derives its viewport
        /// from the widget allocation itself, there is no external embedder
        /// that needs to be told about resizes.
        #[cfg(not(target_os = "linux"))]
        pub fn update_viewport(&self, _width: i32, _height: i32) {}
    }

    impl WidgetImpl for MutsumiVideoPlayer {}
    impl BinImpl for MutsumiVideoPlayer {}
}

glib::wrapper! {
    pub struct MutsumiVideoPlayer(ObjectSubclass<imp::MutsumiVideoPlayer>)
        @extends gtk::Widget, adw::Bin,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Default for MutsumiVideoPlayer {
    fn default() -> Self {
        Self::new()
    }
}

impl MutsumiVideoPlayer {
    pub fn new() -> Self {
        Object::new()
    }

    pub fn update_viewport(&self, width: i32, height: i32) {
        self.imp().update_viewport(width, height);
    }

    #[cfg(target_os = "linux")]
    pub fn backend_ref(&self) -> &MutsumiVideoSink {
        &self.imp().backend
    }

    #[cfg(not(target_os = "linux"))]
    pub fn backend_ref(&self) -> &MPVGLArea {
        &self.imp().backend
    }

    pub fn mpv(&self) -> &ContextedMPV {
        self.imp().backend.mpv()
    }
}

impl MutsumiVideoPlayer {
    pub fn play(&self, source: &PlayParams) {
        let url = source.url().into_owned();
        let start_time = source.start_time;

        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = obj)]
            self,
            async move {
                let mpv = obj.mpv();

                tracing::info!(target: "mutsumi::mpv", source = url, "loading media");
                mpv.load_video(&url);

                if let Some(start_time) = start_time {
                    mpv.set_start_time(start_time);
                }

                mpv.pause(false);
            }
        ));
    }

    pub fn shutdown(&self) {
        self.mpv().shutdown();
    }

    pub fn stop(&self) {
        self.mpv().stop();
    }

    pub fn load_video(&self, url: &str) {
        self.mpv().load_video(url);
    }

    /// Clear the video area so the last decoded frame is not left on screen.
    ///
    /// Only meaningful for the texture based Linux backend; the render API
    /// backend simply keeps presenting the last frame until a new one
    /// arrives, which is the behaviour of every other mpv embedder.
    #[cfg(target_os = "linux")]
    pub fn push_an_empty_texture(&self) {
        self.backend_ref().push_an_empty_texture();
    }

    #[cfg(not(target_os = "linux"))]
    pub fn push_an_empty_texture(&self) {}

    pub fn set_loop_playlist(&self, loop_: &str) {
        self.mpv().set_loop_playlist(loop_);
    }

    pub fn set_loop_file(&self, loop_: &str) {
        self.mpv().set_loop_file(loop_);
    }

    pub fn playlist_shuffle(&self) {
        self.mpv().playlist_shuffle();
    }

    pub fn playlist_unshuffle(&self) {
        self.mpv().playlist_unshuffle();
    }

    pub fn set_playlist(&self, urls: &[String]) {
        self.mpv().set_playlist(urls);
    }

    pub fn set_playlist_pos(&self, pos: i64) {
        self.mpv().set_playlist_pos(pos);
    }

    pub fn playlist_add(&self, url: &str, index: i64) {
        self.mpv().playlist_add(url, index);
    }

    pub fn playlist_remove(&self, index: i64) {
        self.mpv().playlist_remove(index);
    }

    pub fn playlist_move(&self, from: i64, to: i64) {
        self.mpv().playlist_move(from, to);
    }

    pub fn add_sub(&self, url: &str) {
        self.mpv().add_sub(url);
    }

    pub fn pause(&self, pause: bool) {
        self.mpv().pause(pause);
    }

    pub fn command_pause(&self) {
        self.mpv().command_pause();
    }

    pub fn set_position(&self, value: f64) {
        self.mpv().set_position(value);
    }

    pub fn set_percent_position(&self, value: f64) {
        self.mpv().set_percent_position(value);
    }

    pub fn set_start_time(&self, second: u64) {
        self.mpv().set_start_time(second);
    }

    pub fn set_start(&self, second: f64) {
        self.mpv().set_start(second);
    }

    pub fn set_volume(&self, value: i64) {
        self.mpv().set_volume(value);
    }

    pub fn volume_scroll(&self, value: i64) {
        self.mpv().volume_scroll(value);
    }

    pub fn set_speed(&self, value: f64) {
        self.mpv().set_speed(value);
    }

    pub fn seek_forward(&self, value: i64) {
        self.mpv().seek_forward(value);
    }

    pub fn seek_backward(&self, value: i64) {
        self.mpv().seek_backward(value);
    }

    pub async fn position(&self) -> f64 {
        self.mpv().position().await
    }

    pub async fn paused(&self) -> bool {
        self.mpv().paused().await
    }

    pub async fn duration(&self) -> f64 {
        self.mpv().duration().await
    }

    pub fn set_aid(&self, value: TrackSelection) {
        self.mpv().set_aid(value);
    }

    pub fn set_sid(&self, value: TrackSelection) {
        self.mpv().set_sid(value);
    }

    pub fn disable_aid(&self) {
        self.mpv().set_aid(TrackSelection::None);
    }

    pub fn disable_sid(&self) {
        self.mpv().set_sid(TrackSelection::None);
    }

    pub fn set_keep_aspect_ratio(&self, keep: bool) {
        self.mpv().mpv.set_property("keepaspect", keep);
    }

    pub fn set_slang(&self, value: String) {
        self.mpv().set_slang(value);
    }

    pub fn get_track_id(&self, kind: TrackKind) -> BoxedFuture<'_, i64> {
        let type_ = match kind {
            TrackKind::Video => "vid",
            TrackKind::Audio => "aid",
            TrackKind::Subtitle => "sid",
        };

        // ContextedMPV is a Copy handle to the process wide mpv actor, so it
        // can cross the async boundary without borrowing the widget.
        let mpv = *self.mpv();
        Box::pin(async move { mpv.get_track_id(type_).await })
    }

    pub fn press_key(&self, key: u32, state: ModifierType) {
        self.mpv().press_key(key, state);
    }

    pub fn release_key(&self, key: u32, state: ModifierType) {
        self.mpv().release_key(key, state);
    }

    pub fn display_stats_toggle(&self) {
        self.mpv().display_stats_toggle();
    }

    pub fn set_brightness(&self, value: f64) {
        self.mpv().mpv.set_property("brightness", value);
    }

    pub fn set_contrast(&self, value: f64) {
        self.mpv().mpv.set_property("contrast", value);
    }

    pub fn set_gamma(&self, value: f64) {
        self.mpv().mpv.set_property("gamma", value);
    }

    pub fn set_hue(&self, value: f64) {
        self.mpv().mpv.set_property("hue", value);
    }

    pub fn set_saturation(&self, value: f64) {
        self.mpv().mpv.set_property("saturation", value);
    }

    pub fn set_sub_pos(&self, value: f64) {
        self.mpv().mpv.set_property("sub-pos", value);
    }

    pub fn set_sub_font_size(&self, value: f64) {
        self.mpv().mpv.set_property("sub-font-size", value);
    }

    pub fn set_sub_scale(&self, value: f64) {
        self.mpv().mpv.set_property("sub-scale", value);
    }

    pub fn set_sub_speed(&self, value: f64) {
        self.mpv().mpv.set_property("sub-speed", value);
    }

    pub fn set_sub_delay(&self, value: f64) {
        self.mpv().mpv.set_property("sub-delay", value);
    }

    pub fn set_sub_justify(&self, value: &str) {
        self.mpv().mpv.set_property("sub-justify", value.to_owned());
    }

    pub fn set_sub_bold(&self, value: bool) {
        self.mpv().mpv.set_property("sub-bold", value);
    }

    pub fn set_sub_italic(&self, value: bool) {
        self.mpv().mpv.set_property("sub-italic", value);
    }

    pub fn set_sub_font(&self, value: &str) {
        self.mpv().mpv.set_property("sub-font", value.to_owned());
    }

    pub fn set_sub_color(&self, value: &str) {
        self.mpv().mpv.set_property("sub-color", value.to_owned());
    }

    pub fn set_sub_border_color(&self, value: &str) {
        self.mpv()
            .mpv
            .set_property("sub-border-color", value.to_owned());
    }

    pub fn set_sub_back_color(&self, value: &str) {
        self.mpv()
            .mpv
            .set_property("sub-back-color", value.to_owned());
    }

    pub fn set_sub_border_style(&self, value: &str) {
        self.mpv()
            .mpv
            .set_property("sub-border-style", value.to_owned());
    }

    pub fn set_sub_border_size(&self, value: f64) {
        self.mpv().mpv.set_property("sub-border-size", value);
    }

    pub fn set_sub_shadow_offset(&self, value: f64) {
        self.mpv().mpv.set_property("sub-shadow-offset", value);
    }

    pub fn set_audio_delay(&self, value: f64) {
        self.mpv().mpv.set_property("audio-delay", value);
    }

    pub fn set_audio_channels(&self, value: &str) {
        self.mpv()
            .mpv
            .set_property("audio-channels", value.to_owned());
    }

    pub fn set_audio_pan(&self, value: &str) {
        self.mpv().mpv.set_property("af", value.to_owned());
    }

    pub fn clear_audio_pan(&self) {
        self.mpv().mpv.set_property("af", String::new());
    }

    pub fn set_scale(&self, value: &str) {
        self.mpv().mpv.set_property("scale", value.to_owned());
    }

    pub fn set_deband(&self, value: bool) {
        self.mpv().mpv.set_property("deband", value);
    }

    pub fn set_deband_iterations(&self, value: i64) {
        self.mpv().mpv.set_property("deband-iterations", value);
    }

    pub fn set_deband_threshold(&self, value: i64) {
        self.mpv().mpv.set_property("deband-threshold", value);
    }

    pub fn set_deband_range(&self, value: i64) {
        self.mpv().mpv.set_property("deband-range", value);
    }

    pub fn set_deband_grain(&self, value: i64) {
        self.mpv().mpv.set_property("deband-grain", value);
    }

    pub fn set_deinterlace(&self, value: bool) {
        self.mpv().mpv.set_property("deinterlace", value);
    }

    pub fn set_hwdec(&self, value: &str) {
        self.mpv().mpv.set_property("hwdec", value.to_owned());
    }

    pub fn set_panscan(&self, value: f64) {
        self.mpv().mpv.set_property("panscan", value);
    }

    pub fn set_stretch_image_subs_to_screen(&self, value: bool) {
        self.mpv()
            .mpv
            .set_property("stretch-image-subs-to-screen", value);
    }

    pub fn set_demuxer_max_bytes(&self, value: &str) {
        self.mpv()
            .mpv
            .set_property("demuxer-max-bytes", value.to_owned());
    }

    pub fn set_cache_secs(&self, value: f64) {
        self.mpv().mpv.set_property("cache-secs", value);
    }

    pub fn set_property<V>(&self, property: &str, value: V)
    where
        V: Into<MpvValue>,
    {
        self.mpv().mpv.set_property(property, value);
    }
}
