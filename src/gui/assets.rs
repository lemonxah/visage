use eframe::egui;
use std::sync::{Arc, OnceLock};

use egui::{
    load::SizedTexture, vec2, Color32, ColorImage, Context, Image, TextureFilter, TextureHandle,
    TextureOptions, TextureWrapMode,
};

const APP_ICON: &[u8] = include_bytes!("../../assets/icons/visage-256.png");

pub struct Glyphs {
    pub logo: TextureHandle,
    pub camera: TextureHandle,
    pub tongue: TextureHandle,
    pub face: TextureHandle,
    pub log: TextureHandle,
    pub save: TextureHandle,
    pub revert: TextureHandle,
    pub trash: TextureHandle,
    pub reset: TextureHandle,
    pub headset: TextureHandle,
}

static GLYPHS: OnceLock<Glyphs> = OnceLock::new();

pub fn window_icon() -> Option<Arc<egui::IconData>> {
    eframe::icon_data::from_png_bytes(APP_ICON)
        .ok()
        .map(Arc::new)
}

fn texture(ctx: &Context, name: &str, bytes: &[u8]) -> TextureHandle {
    let icon = eframe::icon_data::from_png_bytes(bytes).expect("bundled png");
    let image =
        ColorImage::from_rgba_unmultiplied([icon.width as usize, icon.height as usize], &icon.rgba);
    ctx.load_texture(
        name,
        image,
        TextureOptions {
            magnification: TextureFilter::Linear,
            minification: TextureFilter::Linear,
            wrap_mode: TextureWrapMode::ClampToEdge,
            mipmap_mode: Some(TextureFilter::Linear),
        },
    )
}

macro_rules! glyph_texture {
    ($ctx:expr, $name:literal) => {
        texture(
            $ctx,
            $name,
            include_bytes!(concat!("../../assets/glyphs/", $name, ".png")),
        )
    };
}

pub fn load(ctx: &Context) {
    GLYPHS.get_or_init(|| Glyphs {
        logo: texture(ctx, "logo", APP_ICON),
        camera: glyph_texture!(ctx, "camera"),
        tongue: glyph_texture!(ctx, "tongue"),
        face: glyph_texture!(ctx, "face"),
        log: glyph_texture!(ctx, "log"),
        save: glyph_texture!(ctx, "save"),
        revert: glyph_texture!(ctx, "revert"),
        trash: glyph_texture!(ctx, "trash"),
        reset: glyph_texture!(ctx, "reset"),
        headset: glyph_texture!(ctx, "headset"),
    });
}

pub fn glyphs() -> &'static Glyphs {
    GLYPHS.get().expect("glyphs are loaded at startup")
}

pub fn glyph(texture: &TextureHandle, size: f32, tint: Color32) -> Image<'static> {
    Image::new(SizedTexture::new(texture.id(), vec2(size, size)))
        .fit_to_exact_size(vec2(size, size))
        .tint(tint)
}
