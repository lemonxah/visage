use serde::{Deserialize, Serialize};

use super::decode::Image;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Roi {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Default for Roi {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            width: 1.0,
            height: 1.0,
        }
    }
}

impl Roi {
    pub fn sanitized(self) -> Self {
        let finite = |v: f32, fallback: f32| if v.is_finite() { v } else { fallback };
        let x = finite(self.x, 0.0).clamp(0.0, 0.98);
        let y = finite(self.y, 0.0).clamp(0.0, 0.98);
        let width = finite(self.width, 1.0).min(1.0 - x).max(0.02);
        let height = finite(self.height, 1.0).min(1.0 - y).max(0.02);
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub fn from_corners(a: (f32, f32), b: (f32, f32)) -> Self {
        let (x0, x1) = (a.0.min(b.0).clamp(0.0, 1.0), a.0.max(b.0).clamp(0.0, 1.0));
        let (y0, y1) = (a.1.min(b.1).clamp(0.0, 1.0), a.1.max(b.1).clamp(0.0, 1.0));
        Self {
            x: x0,
            y: y0,
            width: x1 - x0,
            height: y1 - y0,
        }
        .sanitized()
    }

    fn pixels(&self, width: usize, height: usize) -> PixelRect {
        let roi = self.sanitized();
        let x0 = ((roi.x * width as f32).floor() as usize).min(width - 1);
        let y0 = ((roi.y * height as f32).floor() as usize).min(height - 1);
        let x1 = (((roi.x + roi.width) * width as f32).ceil() as usize).clamp(x0 + 1, width);
        let y1 = (((roi.y + roi.height) * height as f32).ceil() as usize).clamp(y0 + 1, height);
        PixelRect {
            x: x0,
            y: y0,
            width: x1 - x0,
            height: y1 - y0,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct View {
    pub roi: Roi,
    pub rotation: f32,
    pub flip_x: bool,
    pub flip_y: bool,
}

impl View {
    pub fn sanitized(mut self) -> Self {
        self.roi = self.roi.sanitized();
        self.rotation = if self.rotation.is_finite() {
            (self.rotation + 180.0).rem_euclid(360.0) - 180.0
        } else {
            0.0
        };
        self
    }
}

#[derive(Clone, Copy)]
struct PixelRect {
    x: usize,
    y: usize,
    width: usize,
    height: usize,
}

#[derive(Clone, Copy)]
enum Channel {
    Luma,
    Index(usize),
}

struct Tap {
    start: usize,
    weights: Vec<f32>,
}

fn taps(source: usize, target: usize) -> Vec<Tap> {
    let scale = source as f64 / target as f64;
    (0..target)
        .map(|index| {
            let begin = index as f64 * scale;
            let end = ((index + 1) as f64 * scale).min(source as f64);
            let start = (begin.floor() as usize).min(source - 1);
            let stop = (end.ceil() as usize).clamp(start + 1, source);
            let span = (end - begin).max(1e-9);
            let weights = (start..stop)
                .map(|pixel| {
                    let overlap = (end.min(pixel as f64 + 1.0) - begin.max(pixel as f64)).max(0.0);
                    (overlap / span) as f32
                })
                .collect();
            Tap { start, weights }
        })
        .collect()
}

fn resize_plane(
    image: &Image,
    rect: PixelRect,
    channel: Channel,
    width: usize,
    height: usize,
) -> Vec<f32> {
    let columns = taps(rect.width, width);
    let rows = taps(rect.height, height);
    let mut horizontal = vec![0f32; rect.height * width];
    for (row, out) in horizontal.chunks_exact_mut(width).enumerate() {
        let y = rect.y + row;
        for (value, tap) in out.iter_mut().zip(&columns) {
            *value = tap
                .weights
                .iter()
                .enumerate()
                .map(|(offset, weight)| {
                    let x = rect.x + tap.start + offset;
                    let pixel = match channel {
                        Channel::Luma => image.luma(x, y),
                        Channel::Index(c) => {
                            image.data[(y * image.width + x) * image.channels + c] as f32
                        }
                    };
                    weight * pixel
                })
                .sum();
        }
    }
    let mut output = vec![0f32; width * height];
    for (out_row, tap) in output.chunks_exact_mut(width).zip(&rows) {
        for (offset, weight) in tap.weights.iter().enumerate() {
            let source = &horizontal[(tap.start + offset) * width..][..width];
            for (value, pixel) in out_row.iter_mut().zip(source) {
                *value += weight * pixel;
            }
        }
    }
    output
}

fn bilinear(plane: &[f32], size: usize, x: f32, y: f32) -> f32 {
    let max = (size - 1) as f32;
    if !(x >= -0.5 && y >= -0.5 && x <= max + 0.5 && y <= max + 0.5) {
        return 0.0;
    }
    let x = x.clamp(0.0, max);
    let y = y.clamp(0.0, max);
    let x0 = x.floor() as usize;
    let y0 = y.floor() as usize;
    let x1 = (x0 + 1).min(size - 1);
    let y1 = (y0 + 1).min(size - 1);
    let fx = x - x0 as f32;
    let fy = y - y0 as f32;
    let top = plane[y0 * size + x0] * (1.0 - fx) + plane[y0 * size + x1] * fx;
    let bottom = plane[y1 * size + x0] * (1.0 - fx) + plane[y1 * size + x1] * fx;
    top * (1.0 - fy) + bottom * fy
}

fn orient(plane: Vec<f32>, size: usize, view: &View) -> Vec<f32> {
    let rotation = view.rotation.to_radians();
    if rotation.abs() < 1e-4 && !view.flip_x && !view.flip_y {
        return plane;
    }
    let (sin, cos) = rotation.sin_cos();
    let center = (size as f32 - 1.0) / 2.0;
    let mut output = vec![0f32; size * size];
    for y in 0..size {
        for x in 0..size {
            let mut u = x as f32 - center;
            let mut v = y as f32 - center;
            if view.flip_x {
                u = -u;
            }
            if view.flip_y {
                v = -v;
            }
            let sx = cos * u + sin * v + center;
            let sy = -sin * u + cos * v + center;
            output[y * size + x] = bilinear(&plane, size, sx, sy);
        }
    }
    output
}

pub fn prepare(image: &Image, view: &View, size: usize, channels: usize, out: &mut Vec<f32>) {
    let rect = view.roi.pixels(image.width, image.height);
    for channel in 0..channels {
        let source = if channels == 1 || image.channels == 1 {
            Channel::Luma
        } else {
            Channel::Index(channel.min(image.channels - 1))
        };
        let plane = resize_plane(image, rect, source, size, size);
        out.extend(orient(plane, size, view));
    }
}

pub fn thumbnail(image: &Image, max_width: usize) -> (usize, usize, Vec<u8>) {
    let width = image.width.min(max_width).max(1);
    let height = ((image.height * width) as f32 / image.width as f32)
        .round()
        .max(1.0) as usize;
    let rect = PixelRect {
        x: 0,
        y: 0,
        width: image.width,
        height: image.height,
    };
    let plane = resize_plane(image, rect, Channel::Luma, width, height);
    (
        width,
        height,
        plane
            .iter()
            .map(|v| v.round().clamp(0.0, 255.0) as u8)
            .collect(),
    )
}

pub fn plane_to_gray(planes: &[f32], size: usize, scale: f32) -> Vec<u8> {
    let count = (planes.len() / (size * size)).max(1);
    (0..size * size)
        .map(|index| {
            let sum: f32 = (0..count)
                .filter_map(|c| planes.get(c * size * size + index))
                .sum();
            (sum / count as f32 * scale).round().clamp(0.0, 255.0) as u8
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gradient(width: usize, height: usize) -> Image {
        let data = (0..height)
            .flat_map(|y| (0..width).map(move |x| (x * 10 + y) as u8))
            .collect();
        Image::new(width, height, 1, data).unwrap()
    }

    #[test]
    fn flat_images_keep_their_level() {
        let image = Image::new(7, 5, 1, vec![90; 35]).unwrap();
        let mut out = Vec::new();
        prepare(&image, &View::default(), 4, 1, &mut out);
        assert!(out.iter().all(|v| (v - 90.0).abs() < 1e-3));
    }

    #[test]
    fn roi_crops_before_resizing() {
        let image = gradient(10, 10);
        let view = View {
            roi: Roi {
                x: 0.5,
                y: 0.0,
                width: 0.1,
                height: 0.1,
            },
            ..Default::default()
        };
        let mut out = Vec::new();
        prepare(&image, &view, 1, 1, &mut out);
        assert_eq!(out, vec![50.0]);
    }

    #[test]
    fn flips_and_half_turns_agree() {
        let image = gradient(6, 6);
        let mut flipped = Vec::new();
        let flip = View {
            flip_x: true,
            flip_y: true,
            ..Default::default()
        };
        prepare(&image, &flip, 6, 1, &mut flipped);
        let mut rotated = Vec::new();
        let turn = View {
            rotation: 180.0,
            ..Default::default()
        };
        prepare(&image, &turn, 6, 1, &mut rotated);
        for (a, b) in flipped.iter().zip(&rotated) {
            assert!((a - b).abs() < 1e-3);
        }
        assert_eq!(flipped[0], image.data[35] as f32);
    }

    #[test]
    fn color_channels_stay_planar_and_gray_is_replicated() {
        let data = [10u8, 20, 30].repeat(4);
        let image = Image::new(2, 2, 3, data).unwrap();
        let mut out = Vec::new();
        prepare(&image, &View::default(), 2, 3, &mut out);
        assert_eq!(out.len(), 12);
        assert_eq!(&out[..4], &[10.0; 4]);
        assert_eq!(&out[8..], &[30.0; 4]);

        let gray = Image::new(2, 2, 1, vec![60; 4]).unwrap();
        out.clear();
        prepare(&gray, &View::default(), 2, 3, &mut out);
        assert!(out.iter().all(|v| *v == 60.0));
    }

    #[test]
    fn roi_is_sanitized() {
        let roi = Roi {
            x: -1.0,
            y: 0.9,
            width: 5.0,
            height: f32::NAN,
        }
        .sanitized();
        assert_eq!(roi.x, 0.0);
        assert_eq!(roi.width, 1.0);
        assert!((roi.height - 0.1).abs() < 1e-6);
        let from = Roi::from_corners((0.8, 0.6), (0.2, 0.1));
        assert!((from.x - 0.2).abs() < 1e-6 && (from.width - 0.6).abs() < 1e-6);
    }
}
