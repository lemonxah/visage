use std::panic::{self, AssertUnwindSafe};

use anyhow::{anyhow, bail};
use zune_jpeg::{
    zune_core::{bytestream::ZCursor, colorspace::ColorSpace, options::DecoderOptions},
    JpegDecoder,
};

use super::source::RawFrame;

pub const MAX_DIMENSION: usize = 2048;

#[derive(Clone, Debug, PartialEq)]
pub struct Image {
    pub width: usize,
    pub height: usize,
    pub channels: usize,
    pub data: Vec<u8>,
}

impl Image {
    pub fn new(
        width: usize,
        height: usize,
        channels: usize,
        data: Vec<u8>,
    ) -> anyhow::Result<Self> {
        if width == 0 || height == 0 || width > MAX_DIMENSION || height > MAX_DIMENSION {
            bail!("unsupported image size {width}x{height}");
        }
        if !matches!(channels, 1 | 3) || data.len() != width * height * channels {
            bail!("image buffer does not match {width}x{height}x{channels}");
        }
        Ok(Self {
            width,
            height,
            channels,
            data,
        })
    }

    pub fn luma(&self, x: usize, y: usize) -> f32 {
        let index = (y * self.width + x) * self.channels;
        if self.channels == 1 {
            self.data[index] as f32
        } else {
            0.299 * self.data[index] as f32
                + 0.587 * self.data[index + 1] as f32
                + 0.114 * self.data[index + 2] as f32
        }
    }
}

pub fn decode(frame: &RawFrame, color: bool) -> anyhow::Result<Image> {
    match frame {
        RawFrame::Jpeg(bytes) => decode_jpeg(bytes, color),
        RawFrame::Yuyv {
            width,
            height,
            stride,
            data,
        } => yuyv(data, *width, *height, *stride, color),
    }
}

pub fn decode_jpeg(bytes: &[u8], color: bool) -> anyhow::Result<Image> {
    if !bytes.starts_with(&[0xFF, 0xD8]) {
        bail!("not a JPEG image");
    }
    let colorspace = if color {
        ColorSpace::RGB
    } else {
        ColorSpace::Luma
    };
    let options = DecoderOptions::default()
        .jpeg_set_out_colorspace(colorspace)
        .set_max_width(MAX_DIMENSION)
        .set_max_height(MAX_DIMENSION);
    let decoded = panic::catch_unwind(AssertUnwindSafe(|| {
        let mut decoder = JpegDecoder::new_with_options(ZCursor::new(bytes), options);
        let pixels = decoder.decode().map_err(|e| anyhow!("bad JPEG: {e:?}"))?;
        let info = decoder
            .info()
            .ok_or_else(|| anyhow!("bad JPEG: no image header"))?;
        Ok::<_, anyhow::Error>((info.width as usize, info.height as usize, pixels))
    }))
    .map_err(|_| anyhow!("bad JPEG: decoder crashed"))??;
    let (width, height, pixels) = decoded;
    let channels = if color { 3 } else { 1 };
    Image::new(width, height, channels, pixels)
}

pub fn yuyv(
    data: &[u8],
    width: usize,
    height: usize,
    stride: usize,
    color: bool,
) -> anyhow::Result<Image> {
    if width == 0 || height == 0 || width > MAX_DIMENSION || height > MAX_DIMENSION {
        bail!("unsupported YUYV size {width}x{height}");
    }
    let stride = stride.max(2 * width);
    if data.len() < stride * (height - 1) + 2 * width {
        bail!(
            "short YUYV frame ({} bytes for {width}x{height})",
            data.len()
        );
    }
    let channels = if color { 3 } else { 1 };
    let mut out = Vec::with_capacity(width * height * channels);
    for row in 0..height {
        let line = &data[row * stride..row * stride + 2 * width];
        for x in 0..width {
            let y = line[2 * x] as f32;
            if !color {
                out.push(y as u8);
                continue;
            }
            let pair = x & !1;
            let u = line[2 * pair + 1] as f32 - 128.0;
            let v = line.get(2 * pair + 3).map_or(0.0, |v| *v as f32 - 128.0);
            let clamp = |value: f32| value.round().clamp(0.0, 255.0) as u8;
            out.push(clamp(y + 1.402 * v));
            out.push(clamp(y - 0.344_136 * u - 0.714_136 * v));
            out.push(clamp(y + 1.772 * u));
        }
    }
    Image::new(width, height, channels, out)
}

#[cfg(test)]
pub const TEST_JPEG: [u8; 187] = [
    0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, 0x4A, 0x46, 0x49, 0x46, 0x00, 0x01, 0x01, 0x00, 0x00, 0x01,
    0x00, 0x01, 0x00, 0x00, 0xFF, 0xDB, 0x00, 0x43, 0x00, 0x03, 0x02, 0x02, 0x03, 0x02, 0x02, 0x03,
    0x03, 0x03, 0x03, 0x04, 0x03, 0x03, 0x04, 0x05, 0x08, 0x05, 0x05, 0x04, 0x04, 0x05, 0x0A, 0x07,
    0x07, 0x06, 0x08, 0x0C, 0x0A, 0x0C, 0x0C, 0x0B, 0x0A, 0x0B, 0x0B, 0x0D, 0x0E, 0x12, 0x10, 0x0D,
    0x0E, 0x11, 0x0E, 0x0B, 0x0B, 0x10, 0x16, 0x10, 0x11, 0x13, 0x14, 0x15, 0x15, 0x15, 0x0C, 0x0F,
    0x17, 0x18, 0x16, 0x14, 0x18, 0x12, 0x14, 0x15, 0x14, 0xFF, 0xC0, 0x00, 0x0B, 0x08, 0x00, 0x08,
    0x00, 0x08, 0x01, 0x01, 0x11, 0x00, 0xFF, 0xC4, 0x00, 0x14, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x04, 0xFF, 0xC4, 0x00, 0x21,
    0x10, 0x00, 0x00, 0x02, 0x0B, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x12, 0x52, 0x00, 0x01, 0x07, 0x09, 0x11, 0x13, 0x14, 0x16, 0x27, 0x45, 0x64, 0xA1, 0xFF,
    0xDA, 0x00, 0x08, 0x01, 0x01, 0x00, 0x00, 0x3F, 0x00, 0x23, 0x39, 0x77, 0x5D, 0x80, 0x0C, 0x81,
    0x5E, 0x18, 0xE9, 0x65, 0x9B, 0xA1, 0x66, 0xF1, 0x3F, 0xFF, 0xD9,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn yuyv_extracts_luma_and_color() {
        let data = [10u8, 128, 20, 128, 30, 128, 40, 128];
        let gray = yuyv(&data, 2, 2, 4, false).unwrap();
        assert_eq!(gray.data, vec![10, 20, 30, 40]);
        let color = yuyv(&data, 2, 2, 4, true).unwrap();
        assert_eq!(&color.data[..3], &[10, 10, 10]);
        assert!(yuyv(&data[..5], 2, 2, 4, false).is_err());
    }

    #[test]
    fn decodes_gray_and_color_jpegs() {
        let gray = decode_jpeg(&TEST_JPEG, false).unwrap();
        assert_eq!((gray.width, gray.height, gray.channels), (8, 8, 1));
        assert!(gray.data[7 * 8..]
            .iter()
            .all(|v| (*v as i32 - 200).abs() < 12));
        assert!(gray.data[0] < 20);
        let color = decode_jpeg(&TEST_JPEG, true).unwrap();
        assert_eq!((color.width, color.height, color.channels), (8, 8, 3));
        assert_eq!(color.data.len(), 8 * 8 * 3);
    }

    #[test]
    fn garbage_is_not_a_jpeg() {
        assert!(decode_jpeg(&[1, 2, 3], false).is_err());
        assert!(decode_jpeg(&[0xFF, 0xD8, 0xFF, 0xD9], false).is_err());
        let mut noise = vec![0xFF, 0xD8, 0xFF, 0xDB, 0x00, 0x43];
        noise.extend((0..500u32).map(|i| (i * 37 % 251) as u8));
        assert!(decode_jpeg(&noise, true).is_err());
    }
}
