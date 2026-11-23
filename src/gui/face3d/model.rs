use std::f32::consts::{FRAC_PI_2, PI, TAU};

use glam::{vec3, Affine3A, Mat3, Vec3};

pub const STRIDE: usize = 11;
pub const EYE_RADIUS: f32 = 0.135;
pub const VIEW_TARGET: Vec3 = Vec3::new(0.0, -0.2, 0.12);

const CENTER: Vec3 = Vec3::new(0.0, -0.1, 0.0);
const COLS: usize = 168;
const ROWS: usize = 140;
const SLIT_Y: f32 = -0.52;
const MOUTH_HALF: f32 = 0.25;
const EYE_X: f32 = 0.27;
const EYE_Y: f32 = 0.04;
const EYE_Z: f32 = 0.645;
const JAW_PIVOT: Vec3 = Vec3::new(0.0, -0.06, -0.08);
const JAW_MAX: f32 = 0.384;
const LID_COLS: usize = 33;
const LID_ROWS: usize = 16;
const LID_RADIUS: f32 = 1.05;
const LID_SPAN: f32 = 2.5;
const TONGUE_SEG: usize = 28;
const TONGUE_RING: usize = 16;
const TONGUE_ROOT: Vec3 = Vec3::new(0.0, -0.655, 0.34);
const EYE_SEG: usize = 48;
const EYE_RINGS: usize = 32;
pub const IRIS_ANGLE: f32 = 0.42;
pub const NECK_FADE: (f32, f32) = (-1.38, -0.86);

const SKIN: Vec3 = Vec3::new(0.68, 0.42, 0.32);
const BLUSH: Vec3 = Vec3::new(0.82, 0.38, 0.34);
const LIP: Vec3 = Vec3::new(0.56, 0.19, 0.21);
const LIP_INNER: Vec3 = Vec3::new(0.48, 0.15, 0.17);
const BROW: Vec3 = Vec3::new(0.06, 0.04, 0.032);
const LASH: Vec3 = Vec3::new(0.03, 0.022, 0.02);
const TEETH: Vec3 = Vec3::new(0.86, 0.84, 0.78);
const MOUTH: Vec3 = Vec3::new(0.16, 0.035, 0.045);
const TONGUE: Vec3 = Vec3::new(0.74, 0.30, 0.33);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Morph {
    BrowInnerUp,
    BrowOuterUp,
    BrowLowerer,
    BrowPinch,
    EyeSquint,
    EyeWide,
    NasalDilation,
    NasalConstrict,
    NoseSneer,
    CheekSquint,
    CheekPuff,
    CheekSuck,
    JawClench,
    LipSuckUpper,
    LipSuckLower,
    LipSuckCorner,
    LipFunnelUpper,
    LipFunnelLower,
    LipPuckerUpper,
    LipPuckerLower,
    MouthUpperUp,
    MouthUpperDeepen,
    MouthLowerDown,
    MouthUpperShift,
    MouthLowerShift,
    MouthCornerPull,
    MouthCornerSlant,
    MouthFrown,
    MouthStretch,
    MouthDimple,
    MouthPress,
    MouthTightener,
    MouthRaiserUpper,
    MouthRaiserLower,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
    Center,
}

const SIDED: [Morph; 32] = [
    Morph::BrowInnerUp,
    Morph::BrowOuterUp,
    Morph::BrowLowerer,
    Morph::BrowPinch,
    Morph::EyeSquint,
    Morph::EyeWide,
    Morph::NasalDilation,
    Morph::NasalConstrict,
    Morph::NoseSneer,
    Morph::CheekSquint,
    Morph::CheekPuff,
    Morph::CheekSuck,
    Morph::JawClench,
    Morph::LipSuckUpper,
    Morph::LipSuckLower,
    Morph::LipSuckCorner,
    Morph::LipFunnelUpper,
    Morph::LipFunnelLower,
    Morph::LipPuckerUpper,
    Morph::LipPuckerLower,
    Morph::MouthUpperUp,
    Morph::MouthUpperDeepen,
    Morph::MouthLowerDown,
    Morph::MouthUpperShift,
    Morph::MouthLowerShift,
    Morph::MouthCornerPull,
    Morph::MouthCornerSlant,
    Morph::MouthFrown,
    Morph::MouthStretch,
    Morph::MouthDimple,
    Morph::MouthPress,
    Morph::MouthTightener,
];

const CENTERED: [Morph; 2] = [Morph::MouthRaiserUpper, Morph::MouthRaiserLower];

pub const TARGET_COUNT: usize = SIDED.len() * 2 + CENTERED.len();

pub fn target_index(morph: Morph, side: Side) -> Option<usize> {
    if let Some(i) = SIDED.iter().position(|m| *m == morph) {
        return match side {
            Side::Left => Some(i * 2),
            Side::Right => Some(i * 2 + 1),
            Side::Center => None,
        };
    }
    let i = CENTERED.iter().position(|m| *m == morph)?;
    (side == Side::Center).then_some(SIDED.len() * 2 + i)
}

fn target_at(k: usize) -> (Morph, f32) {
    if k < SIDED.len() * 2 {
        (SIDED[k / 2], if k % 2 == 0 { 1.0 } else { -1.0 })
    } else {
        (CENTERED[k - SIDED.len() * 2], 0.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lid {
    pub upper: f32,
    pub lower: f32,
}

impl Default for Lid {
    fn default() -> Self {
        Self {
            upper: 0.384,
            lower: -0.436,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Jaw {
    pub open: f32,
    pub side: f32,
    pub forward: f32,
    pub raise: f32,
    pub seal: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Tongue {
    pub out: f32,
    pub up: f32,
    pub down: f32,
    pub left: f32,
    pub right: f32,
    pub roll: f32,
    pub bend_down: f32,
    pub curl_up: f32,
    pub squish: f32,
    pub flat: f32,
    pub twist: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pose {
    pub weights: [f32; TARGET_COUNT],
    pub jaw: Jaw,
    pub lids: [Lid; 2],
    pub tongue: Tongue,
}

impl Default for Pose {
    fn default() -> Self {
        Self {
            weights: [0.0; TARGET_COUNT],
            jaw: Jaw::default(),
            lids: [Lid::default(); 2],
            tongue: Tongue::default(),
        }
    }
}

pub fn unit(v: f32) -> f32 {
    if v.is_finite() {
        v.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

pub fn signed(v: f32) -> f32 {
    if v.is_finite() {
        v.clamp(-1.0, 1.0)
    } else {
        0.0
    }
}

fn clamp_or(v: f32, lo: f32, hi: f32, fallback: f32) -> f32 {
    if v.is_finite() {
        v.clamp(lo, hi)
    } else {
        fallback
    }
}

impl Pose {
    pub fn set(&mut self, morph: Morph, side: Side, value: f32) {
        if let Some(i) = target_index(morph, side) {
            self.weights[i] = unit(value);
        }
    }

    pub fn weight(&self, morph: Morph, side: Side) -> f32 {
        target_index(morph, side).map_or(0.0, |i| self.weights[i])
    }

    pub fn sanitize(&mut self) {
        for w in &mut self.weights {
            *w = unit(*w);
        }
        self.jaw = Jaw {
            open: unit(self.jaw.open),
            side: signed(self.jaw.side),
            forward: signed(self.jaw.forward),
            raise: unit(self.jaw.raise),
            seal: unit(self.jaw.seal),
        };
        let rest = Lid::default();
        for lid in &mut self.lids {
            lid.upper = clamp_or(lid.upper, -0.6, 1.2, rest.upper);
            lid.lower = clamp_or(lid.lower, -1.2, 0.6, rest.lower);
            if lid.upper < lid.lower {
                let mid = 0.5 * (lid.upper + lid.lower);
                lid.upper = mid;
                lid.lower = mid;
            }
        }
        let t = &mut self.tongue;
        for v in [
            &mut t.out,
            &mut t.up,
            &mut t.down,
            &mut t.left,
            &mut t.right,
            &mut t.roll,
            &mut t.bend_down,
            &mut t.curl_up,
            &mut t.squish,
            &mut t.flat,
        ] {
            *v = unit(*v);
        }
        t.twist = signed(t.twist);
    }

    pub fn difference(&self, other: &Pose) -> f32 {
        let mut d = 0.0f32;
        let pairs = [
            (self.jaw.open, other.jaw.open),
            (self.jaw.side, other.jaw.side),
            (self.jaw.forward, other.jaw.forward),
            (self.jaw.raise, other.jaw.raise),
            (self.jaw.seal, other.jaw.seal),
            (self.lids[0].upper, other.lids[0].upper),
            (self.lids[0].lower, other.lids[0].lower),
            (self.lids[1].upper, other.lids[1].upper),
            (self.lids[1].lower, other.lids[1].lower),
            (self.tongue.out, other.tongue.out),
            (self.tongue.up, other.tongue.up),
            (self.tongue.down, other.tongue.down),
            (self.tongue.left, other.tongue.left),
            (self.tongue.right, other.tongue.right),
            (self.tongue.roll, other.tongue.roll),
            (self.tongue.bend_down, other.tongue.bend_down),
            (self.tongue.curl_up, other.tongue.curl_up),
            (self.tongue.squish, other.tongue.squish),
            (self.tongue.flat, other.tongue.flat),
            (self.tongue.twist, other.tongue.twist),
        ];
        for (a, b) in self
            .weights
            .iter()
            .zip(other.weights.iter())
            .map(|(a, b)| (*a, *b))
            .chain(pairs)
        {
            let x = (a - b).abs();
            if !x.is_finite() {
                return f32::INFINITY;
            }
            d = d.max(x);
        }
        d
    }

    pub fn jaw_transform(&self) -> Affine3A {
        let angle = self.jaw.open * JAW_MAX - self.jaw.raise * 0.05;
        let rot = Mat3::from_rotation_y(self.jaw.side * 0.06) * Mat3::from_rotation_x(angle);
        let shift = vec3(self.jaw.side * 0.05, 0.0, self.jaw.forward * 0.045);
        Affine3A::from_translation(JAW_PIVOT + shift)
            * Affine3A::from_mat3(rot)
            * Affine3A::from_translation(-JAW_PIVOT)
    }
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn smin(a: f32, b: f32, k: f32) -> f32 {
    let h = (0.5 + 0.5 * (b - a) / k).clamp(0.0, 1.0);
    b + (a - b) * h - k * h * (1.0 - h)
}

fn ellipsoid(p: Vec3, c: Vec3, r: Vec3) -> f32 {
    let q = p - c;
    let k1 = (q / (r * r)).length();
    if k1 < 1e-9 {
        return -r.min_element();
    }
    let k0 = (q / r).length();
    k0 * (k0 - 1.0) / k1
}

fn capsule(p: Vec3, a: Vec3, b: Vec3, r: f32) -> f32 {
    let pa = p - a;
    let ba = b - a;
    let h = (pa.dot(ba) / ba.dot(ba)).clamp(0.0, 1.0);
    (pa - ba * h).length() - r
}

fn sphere(p: Vec3, c: Vec3, r: f32) -> f32 {
    (p - c).length() - r
}

fn smax(a: f32, b: f32, k: f32) -> f32 {
    -smin(-a, -b, k)
}

pub fn head_sdf(p: Vec3) -> f32 {
    let m = vec3(p.x.abs(), p.y, p.z);
    let mut d = ellipsoid(p, vec3(0.0, 0.20, -0.05), vec3(0.72, 0.75, 0.84));
    d = smin(
        d,
        ellipsoid(p, vec3(0.0, -0.18, 0.10), vec3(0.66, 0.68, 0.64)),
        0.25,
    );
    d = smin(
        d,
        ellipsoid(p, vec3(0.0, -0.58, 0.18), vec3(0.54, 0.30, 0.52)),
        0.2,
    );
    d = smin(
        d,
        ellipsoid(p, vec3(0.0, -0.76, 0.48), vec3(0.21, 0.16, 0.18)),
        0.16,
    );
    d = smin(
        d,
        ellipsoid(m, vec3(0.40, -0.10, 0.40), vec3(0.24, 0.19, 0.24)),
        0.15,
    );
    d = smin(
        d,
        ellipsoid(m, vec3(0.24, 0.25, 0.575), vec3(0.27, 0.10, 0.18)),
        0.12,
    );
    d = smin(
        d,
        ellipsoid(m, vec3(EYE_X, -0.095, 0.65), vec3(0.13, 0.06, 0.1)),
        0.08,
    );
    d = smin(
        d,
        ellipsoid(p, vec3(0.0, -0.48, 0.56), vec3(0.30, 0.22, 0.27)),
        0.15,
    );
    d = smin(
        d,
        capsule(p, vec3(0.0, 0.08, 0.77), vec3(0.0, -0.17, 0.89), 0.055),
        0.08,
    );
    d = smin(d, sphere(p, vec3(0.0, -0.20, 0.885), 0.08), 0.06);
    d = smin(
        d,
        ellipsoid(m, vec3(0.062, -0.25, 0.80), vec3(0.045, 0.036, 0.045)),
        0.07,
    );
    d = smin(
        d,
        ellipsoid(p, vec3(0.0, -0.475, 0.78), vec3(0.20, 0.052, 0.07)),
        0.05,
    );
    d = smin(
        d,
        ellipsoid(p, vec3(0.0, -0.568, 0.77), vec3(0.17, 0.058, 0.07)),
        0.05,
    );
    let ear = smax(
        ellipsoid(m, vec3(0.74, 0.0, -0.10), vec3(0.08, 0.21, 0.14)),
        -ellipsoid(m, vec3(0.83, -0.02, -0.08), vec3(0.045, 0.12, 0.08)),
        0.03,
    );
    d = smin(d, ear, 0.07);
    let neck = capsule(p, vec3(0.0, -0.5, -0.16), vec3(0.0, -2.3, -0.12), 0.40);
    smin(d, neck, 0.22)
}

fn sdf_normal(p: Vec3) -> Vec3 {
    let e = 0.002;
    vec3(
        head_sdf(p + Vec3::X * e) - head_sdf(p - Vec3::X * e),
        head_sdf(p + Vec3::Y * e) - head_sdf(p - Vec3::Y * e),
        head_sdf(p + Vec3::Z * e) - head_sdf(p - Vec3::Z * e),
    )
    .normalize_or(Vec3::Z)
}

fn cast(origin: Vec3, dir: Vec3, t_max: f32) -> Option<Vec3> {
    let mut t = t_max;
    let mut outside = t;
    let mut hit = false;
    for _ in 0..600 {
        let d = head_sdf(origin + dir * t);
        if d < 0.0 {
            hit = true;
            break;
        }
        outside = t;
        t -= (d * 0.6).max(0.003);
        if t <= 0.0 {
            t = 0.0;
            hit = head_sdf(origin) < 0.0;
            break;
        }
    }
    if !hit {
        return None;
    }
    let (mut lo, mut hi) = (t, outside);
    for _ in 0..20 {
        let mid = 0.5 * (lo + hi);
        if head_sdf(origin + dir * mid) < 0.0 {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    Some(origin + dir * (0.5 * (lo + hi)))
}

fn radial_surface(dir: Vec3) -> Vec3 {
    cast(CENTER, dir, 2.6).unwrap_or(CENTER + dir * 0.5)
}

fn front_surface(x: f32, y: f32) -> Vec3 {
    cast(vec3(x, y, 0.0), Vec3::Z, 1.6).unwrap_or(vec3(x, y, 0.7))
}

fn inverse_cdf(density: impl Fn(f32) -> f32, lo: f32, hi: f32, fractions: &[f32]) -> Vec<f32> {
    const N: usize = 4096;
    let step = (hi - lo) / N as f32;
    let mut cdf = Vec::with_capacity(N + 1);
    let mut acc = 0.0;
    cdf.push(0.0);
    for i in 0..N {
        let x = lo + (i as f32 + 0.5) * step;
        acc += density(x) * step;
        cdf.push(acc);
    }
    fractions
        .iter()
        .map(|f| {
            let target = f.clamp(0.0, 1.0) * acc;
            let i = cdf.partition_point(|c| *c < target).clamp(1, N);
            let (c0, c1) = (cdf[i - 1], cdf[i]);
            let t = if c1 > c0 {
                (target - c0) / (c1 - c0)
            } else {
                0.0
            };
            lo + (i as f32 - 1.0 + t) * step
        })
        .collect()
}

fn column_angles() -> Vec<f32> {
    let density = |t: f32| 1.0 + 2.2 * (-(t / 0.8).powi(2)).exp();
    let fractions: Vec<f32> = (0..COLS)
        .map(|c| (c as f32 / COLS as f32 - 0.5).abs() * 2.0)
        .collect();
    let half = inverse_cdf(density, 0.0, PI, &fractions);
    (0..COLS)
        .map(|c| if c < COLS / 2 { -half[c] } else { half[c] })
        .collect()
}

fn row_angles(mouth: f32, eye: f32) -> (Vec<f32>, usize) {
    let density = |p: f32| {
        1.0 + 2.6 * (-((p - mouth) / 0.24).powi(2)).exp()
            + 1.6 * (-((p - eye) / 0.25).powi(2)).exp()
    };
    let fractions: Vec<f32> = (0..ROWS)
        .map(|r| (r as f32 + 1.0) / (ROWS as f32 + 1.0))
        .collect();
    let mut rows = inverse_cdf(density, -FRAC_PI_2, FRAC_PI_2, &fractions);
    let slit = rows
        .iter()
        .enumerate()
        .min_by(|a, b| (a.1 - mouth).abs().total_cmp(&(b.1 - mouth).abs()))
        .map_or(ROWS / 2, |(i, _)| i);
    rows[slit] = mouth;
    (rows, slit)
}

fn ray(theta: f32, phi: f32) -> Vec3 {
    vec3(theta.sin() * phi.cos(), phi.sin(), theta.cos() * phi.cos())
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Part {
    Head,
    Flap,
    Brow,
    Teeth,
    Cavity,
}

#[derive(Clone, Copy, Debug)]
struct Info {
    part: Part,
    lower: f32,
    dy: f32,
}

struct Builder {
    pos: Vec<Vec3>,
    col: Vec<Vec3>,
    ao: Vec<f32>,
    gloss: Vec<f32>,
    info: Vec<Info>,
    tris: Vec<u32>,
}

impl Builder {
    fn new() -> Self {
        Self {
            pos: Vec::new(),
            col: Vec::new(),
            ao: Vec::new(),
            gloss: Vec::new(),
            info: Vec::new(),
            tris: Vec::new(),
        }
    }

    fn vertex(&mut self, p: Vec3, info: Info, col: Vec3, ao: f32, gloss: f32) -> u32 {
        self.pos.push(p);
        self.col.push(col);
        self.ao.push(ao);
        self.gloss.push(gloss);
        self.info.push(info);
        (self.pos.len() - 1) as u32
    }

    fn tri(&mut self, a: u32, b: u32, c: u32) {
        if a != b && b != c && a != c {
            self.tris.extend_from_slice(&[a, b, c]);
        }
    }

    fn orient_closed(&mut self, from: usize, outward: bool) {
        let vol = signed_volume(&self.pos, &self.tris[from..]);
        if (vol > 0.0) != outward {
            flip(&mut self.tris[from..]);
        }
    }

    fn orient_towards(&mut self, from: usize, dir: Vec3) {
        let n = face_sum(&self.pos, &self.tris[from..]);
        if n.dot(dir) < 0.0 {
            flip(&mut self.tris[from..]);
        }
    }
}

fn signed_volume(pos: &[Vec3], tris: &[u32]) -> f32 {
    tris.chunks_exact(3)
        .map(|t| {
            let (a, b, c) = (pos[t[0] as usize], pos[t[1] as usize], pos[t[2] as usize]);
            a.dot(b.cross(c))
        })
        .sum::<f32>()
        / 6.0
}

fn face_sum(pos: &[Vec3], tris: &[u32]) -> Vec3 {
    tris.chunks_exact(3)
        .map(|t| {
            let (a, b, c) = (pos[t[0] as usize], pos[t[1] as usize], pos[t[2] as usize]);
            (b - a).cross(c - a)
        })
        .sum()
}

fn flip(tris: &mut [u32]) {
    for t in tris.chunks_exact_mut(3) {
        t.swap(1, 2);
    }
}

pub fn vertex_normals(pos: &[Vec3], tris: &[u32]) -> Vec<Vec3> {
    let mut n = vec![Vec3::ZERO; pos.len()];
    accumulate_normals(pos, tris, &mut n);
    n
}

fn accumulate_normals(pos: &[Vec3], tris: &[u32], n: &mut [Vec3]) {
    for v in n.iter_mut() {
        *v = Vec3::ZERO;
    }
    for t in tris.chunks_exact(3) {
        let (a, b, c) = (t[0] as usize, t[1] as usize, t[2] as usize);
        let f = (pos[b] - pos[a]).cross(pos[c] - pos[a]);
        n[a] += f;
        n[b] += f;
        n[c] += f;
    }
    for v in n.iter_mut() {
        *v = v.normalize_or(Vec3::Y);
    }
}

struct Anchors {
    mouth: Vec3,
    corner: Vec3,
    brow: [Vec3; 3],
    under_eye: Vec3,
    above_eye: Vec3,
    ala: Vec3,
    sneer: Vec3,
    sneer_lip: Vec3,
    cheek: Vec3,
    cheek_low: Vec3,
    puff: Vec3,
    masseter: Vec3,
    chin: Vec3,
}

#[derive(Clone, Copy)]
struct Delta {
    index: u32,
    d: [f32; 6],
}

pub struct Model {
    base: Vec<f32>,
    jaw_weight: Vec<f32>,
    seal_weight: Vec<f32>,
    targets: Vec<Vec<Delta>>,
    skin_count: usize,
    tongue_tris: Vec<u32>,
    tongue_attr: Vec<[f32; 5]>,
    lid_tris: Vec<u32>,
    lid_attr: Vec<[f32; 5]>,
    pub indices: Vec<u32>,
    pub lines: Vec<u32>,
    pub vertex_count: usize,
    pub eye_vertices: Vec<f32>,
    pub eye_indices: Vec<u32>,
    pub eyes: [Vec3; 2],
}

fn lip_paint(p: Vec3, dy: f32, corner_x: f32) -> f32 {
    let xn = p.x / (corner_x * 1.04);
    if xn.abs() >= 1.0 || p.z < 0.45 {
        return 0.0;
    }
    let base = 1.0 - xn * xn;
    let bow = 1.0 - 0.2 * (-(xn / 0.13).powi(2)).exp()
        + 0.08 * (-((xn.abs() - 0.22) / 0.12).powi(2)).exp();
    let hu = (0.058 * base.powf(0.55) * bow).max(1e-4);
    let hl = (0.074 * base.powf(0.5)).max(1e-4);
    let v = if dy >= 0.0 { dy / hu } else { -dy / hl };
    1.0 - smoothstep(0.75, 1.1, v)
}

fn jaw_region(p: Vec3) -> f32 {
    let q = vec3(p.x / 0.78, (p.y + 0.62) / 0.55, (p.z - 0.30) / 0.70);
    1.0 - smoothstep(0.75, 1.1, q.length())
}

fn ambient_occlusion(p: Vec3, n: Vec3, eyes: &[Vec3; 2]) -> f32 {
    let scene = |q: Vec3| {
        let mut d = head_sdf(q);
        for e in eyes {
            d = d.min(sphere(q, *e, EYE_RADIUS));
        }
        d
    };
    let mut occ = 0.0;
    let mut w = 1.0;
    for i in 1..=5 {
        let h = 0.035 * i as f32;
        occ += (h - scene(p + n * h)).max(0.0) * w;
        w *= 0.6;
    }
    (1.0 - occ * 1.8).clamp(0.35, 1.0)
}

fn field(morph: Morph, s: f32, info: &Info, p: Vec3, n: Vec3, a: &Anchors) -> Vec3 {
    if matches!(info.part, Part::Teeth | Part::Cavity) {
        return Vec3::ZERO;
    }
    let mir = |q: Vec3| vec3(q.x * s, q.y, q.z);
    let g = |c: Vec3, r: f32| (-(p - c).length_squared() / (r * r)).exp();
    let dy = info.dy;
    let lower = info.lower;
    let front = smoothstep(0.3, 0.6, p.z);
    let lipmask = (-((p.x / 0.25).powi(2) + (dy / 0.10).powi(2))).exp() * front;
    let up = lipmask * (1.0 - lower);
    let lo = lipmask * lower;
    let edge = (-(dy / 0.05).powi(2)).exp();
    let half = smoothstep(-0.03, 0.03, s * p.x);
    let corner = mir(a.corner);
    let narrow = (1.0 - (p.x / (a.corner.x * 1.05)).powi(2)).max(0.0);
    let lateral = |x0: f32, rx: f32, ry: f32| {
        (-((p.x - s * x0) / rx).powi(2)).exp() * (-(dy / ry).powi(2)).exp() * front
    };
    match morph {
        Morph::BrowInnerUp => vec3(0.0, 0.07, 0.006) * g(mir(a.brow[0]), 0.15),
        Morph::BrowOuterUp => vec3(0.0, 0.06, 0.0) * g(mir(a.brow[2]), 0.15),
        Morph::BrowLowerer => vec3(-s * 0.008, -0.05, 0.014) * g(mir(a.brow[1]), 0.2),
        Morph::BrowPinch => vec3(-s * 0.035, -0.02, 0.01) * g(mir(a.brow[0]), 0.13),
        Morph::EyeSquint => vec3(0.0, 0.022, 0.01) * g(mir(a.under_eye), 0.11),
        Morph::EyeWide => vec3(0.0, 0.014, 0.0) * g(mir(a.above_eye), 0.13),
        Morph::NasalDilation => vec3(s * 0.022, 0.0, 0.004) * g(mir(a.ala), 0.06),
        Morph::NasalConstrict => vec3(-s * 0.014, 0.004, 0.0) * g(mir(a.ala), 0.06),
        Morph::NoseSneer => {
            vec3(0.0, 0.035, -0.006) * g(mir(a.sneer), 0.11)
                + vec3(0.0, 0.02, 0.0) * up * g(mir(a.sneer_lip), 0.09)
        }
        Morph::CheekSquint => vec3(0.0, 0.035, 0.016) * g(mir(a.cheek), 0.16),
        Morph::CheekPuff => n * 0.09 * g(mir(a.puff), 0.22),
        Morph::CheekSuck => n * -0.05 * g(mir(a.puff), 0.17),
        Morph::JawClench => n * 0.025 * g(mir(a.masseter), 0.16),
        Morph::LipSuckUpper => vec3(0.0, -0.012, -0.05) * up * edge * half,
        Morph::LipSuckLower => vec3(0.0, 0.012, -0.05) * lo * edge * half,
        Morph::LipSuckCorner => vec3(-s * 0.03, 0.0, -0.03) * g(corner, 0.07),
        Morph::LipFunnelUpper => vec3(-p.x * 0.15, 0.03 * narrow, 0.045) * up * edge * half,
        Morph::LipFunnelLower => vec3(-p.x * 0.15, -0.035 * narrow, 0.045) * lo * edge * half,
        Morph::LipPuckerUpper => vec3(-p.x * 0.35, -0.004, 0.06) * up * half,
        Morph::LipPuckerLower => vec3(-p.x * 0.35, 0.004, 0.06) * lo * half,
        Morph::MouthUpperUp => vec3(0.0, 0.045, 0.004) * (1.0 - lower) * lateral(0.09, 0.12, 0.13),
        Morph::MouthUpperDeepen => {
            vec3(s * 0.015, 0.03, -0.015) * (1.0 - lower) * lateral(0.16, 0.09, 0.12)
        }
        Morph::MouthLowerDown => vec3(0.0, -0.045, 0.004) * lower * lateral(0.09, 0.12, 0.13),
        Morph::MouthUpperShift => vec3(s * 0.05, 0.0, 0.0) * up,
        Morph::MouthLowerShift => vec3(s * 0.05, 0.0, 0.0) * lo,
        Morph::MouthCornerPull => {
            vec3(s * 0.035, 0.06, -0.028) * g(corner, 0.13)
                + vec3(0.0, 0.02, 0.015) * g(mir(a.cheek_low), 0.15)
        }
        Morph::MouthCornerSlant => vec3(s * 0.01, 0.035, -0.01) * g(corner, 0.1),
        Morph::MouthFrown => vec3(s * 0.01, -0.05, -0.005) * g(corner, 0.12),
        Morph::MouthStretch => vec3(s * 0.05, -0.012, -0.02) * g(corner, 0.13),
        Morph::MouthDimple => vec3(s * 0.012, 0.004, -0.035) * g(corner, 0.09),
        Morph::MouthPress => {
            (vec3(0.0, -0.012, -0.012) * up + vec3(0.0, 0.012, -0.012) * lo) * edge * half
        }
        Morph::MouthTightener => vec3(-p.x * 0.12, 0.0, 0.012) * lipmask * half,
        Morph::MouthRaiserUpper => vec3(0.0, 0.02, 0.006) * up,
        Morph::MouthRaiserLower => {
            vec3(0.0, 0.04, 0.02) * g(a.chin, 0.2) * lower + vec3(0.0, 0.025, 0.02) * lo
        }
    }
}

fn eye_centers() -> [Vec3; 2] {
    [vec3(EYE_X, EYE_Y, EYE_Z), vec3(-EYE_X, EYE_Y, EYE_Z)]
}

pub fn tongue_points(t: &Tongue, out: &mut Vec<Vec3>) {
    out.clear();
    let length = 0.35 + 0.40 * t.out;
    let width = 0.12 * (1.0 - 0.3 * t.squish + 0.25 * t.flat) * (1.0 - 0.15 * t.roll);
    let thick = 0.045 * (1.0 + 0.3 * t.squish - 0.35 * t.flat);
    let step = length / TONGUE_SEG as f32;
    let mut c = TONGUE_ROOT;
    for i in 0..=TONGUE_SEG {
        let s = i as f32 / TONGUE_SEG as f32;
        let bend = s.powf(1.6);
        let pitch = 0.07 - 0.05 * t.out
            + (t.up - t.down) * 0.96 * bend
            + t.curl_up * 1.75 * smoothstep(0.5, 1.0, s)
            - t.bend_down * 1.4 * smoothstep(0.45, 1.0, s);
        let yaw = (t.left - t.right) * 0.8 * bend;
        let roll = t.twist * 0.6 * s;
        let rot = Mat3::from_rotation_y(yaw)
            * Mat3::from_rotation_x(-pitch)
            * Mat3::from_rotation_z(roll);
        let side = rot * Vec3::X;
        let upv = rot * Vec3::Y;
        let fwd = rot * Vec3::Z;
        let tip = if s > 0.78 {
            (1.0 - ((s - 0.78) / 0.22).powi(2)).max(0.0).sqrt()
        } else {
            1.0
        };
        let root = (s / 0.06).min(1.0).sqrt();
        let w = width * tip * root * (0.85 + 0.15 * smoothstep(0.0, 0.4, s));
        let h = thick * tip * root * (1.0 - 0.35 * s);
        let lift = Vec3::Y * (t.out * 0.14 * smoothstep(0.1, 0.6, s));
        for j in 0..TONGUE_RING {
            let a = TAU * j as f32 / TONGUE_RING as f32;
            let x = a.cos();
            let mut y = a.sin() * h;
            if y > 0.0 {
                y -= 0.35 * h * (-(x / 0.3).powi(2)).exp() * a.sin();
            }
            y += t.roll * 0.06 * x * x * tip * root;
            out.push(c + lift + side * (x * w) + upv * y);
        }
        c += fwd * step;
    }
}

pub fn lid_points(center: Vec3, lid: Lid, out: &mut Vec<Vec3>) {
    let r = EYE_RADIUS;
    for (dir, edge) in [(1.0f32, lid.upper), (-1.0, lid.lower)] {
        for row in 0..LID_ROWS {
            let (alpha, rad) = match row {
                0 => (edge - dir * 0.045, 1.035),
                1 => (edge - dir * 0.02, 1.047),
                _ => {
                    let f = (row - 2) as f32 / (LID_ROWS - 3) as f32;
                    (edge + dir * LID_SPAN * f.powf(1.6), LID_RADIUS + 0.012 * f)
                }
            };
            for col in 0..LID_COLS {
                let beta = -FRAC_PI_2 + PI * col as f32 / (LID_COLS - 1) as f32;
                let local = vec3(
                    beta.sin(),
                    beta.cos() * alpha.sin(),
                    beta.cos() * alpha.cos(),
                );
                out.push(center + local * (rad * r));
            }
        }
    }
}

fn grid_tris(rows: usize, cols: usize, base: u32, tris: &mut Vec<u32>) {
    for r in 0..rows - 1 {
        for c in 0..cols - 1 {
            let v00 = base + (r * cols + c) as u32;
            let v01 = v00 + 1;
            let v10 = v00 + cols as u32;
            let v11 = v10 + 1;
            tris.extend_from_slice(&[v00, v01, v11, v00, v11, v10]);
        }
    }
}

fn ring_tris(rings: usize, ring: usize, base: u32, tris: &mut Vec<u32>) {
    for i in 0..rings - 1 {
        for j in 0..ring {
            let j1 = (j + 1) % ring;
            let a = base + (i * ring + j) as u32;
            let b = base + (i * ring + j1) as u32;
            let c = base + ((i + 1) * ring + j1) as u32;
            let d = base + ((i + 1) * ring + j) as u32;
            tris.extend_from_slice(&[a, b, c, a, c, d]);
        }
    }
}

fn push_vertex(out: &mut Vec<f32>, p: Vec3, n: Vec3, attr: &[f32; 5]) {
    out.extend_from_slice(&[p.x, p.y, p.z, n.x, n.y, n.z]);
    out.extend_from_slice(attr);
}

fn build_eye() -> (Vec<f32>, Vec<u32>) {
    let mut pos = Vec::new();
    pos.push(vec3(0.0, 0.0, EYE_RADIUS * 1.03));
    for k in 1..EYE_RINGS {
        let theta = PI * (k as f32 / EYE_RINGS as f32).powf(1.3);
        let bulge = (1.0 - (theta / IRIS_ANGLE).min(1.0).powi(2)).powi(2);
        let r = EYE_RADIUS * (1.0 + 0.03 * bulge);
        for j in 0..EYE_SEG {
            let a = TAU * j as f32 / EYE_SEG as f32;
            pos.push(vec3(theta.sin() * a.cos(), theta.sin() * a.sin(), theta.cos()) * r);
        }
    }
    pos.push(vec3(0.0, 0.0, -EYE_RADIUS));
    let back = (pos.len() - 1) as u32;
    let mut tris = Vec::new();
    for j in 0..EYE_SEG {
        let j1 = (j + 1) % EYE_SEG;
        tris.extend_from_slice(&[0, 1 + j as u32, 1 + j1 as u32]);
    }
    ring_tris(EYE_RINGS - 1, EYE_SEG, 1, &mut tris);
    let last = 1 + ((EYE_RINGS - 2) * EYE_SEG) as u32;
    for j in 0..EYE_SEG {
        let j1 = (j + 1) % EYE_SEG;
        tris.extend_from_slice(&[back, last + j1 as u32, last + j as u32]);
    }
    if signed_volume(&pos, &tris) < 0.0 {
        flip(&mut tris);
    }
    let normals = vertex_normals(&pos, &tris);
    let mut verts = Vec::with_capacity(pos.len() * STRIDE);
    for (p, n) in pos.iter().zip(normals.iter()) {
        push_vertex(&mut verts, *p, *n, &[0.9, 0.9, 0.9, 1.0, 1.0]);
    }
    (verts, tris)
}

fn edges(tris: &[u32]) -> Vec<u32> {
    let mut list: Vec<u64> = tris
        .chunks_exact(3)
        .flat_map(|t| [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])])
        .map(|(a, b)| ((a.min(b) as u64) << 32) | a.max(b) as u64)
        .collect();
    list.sort_unstable();
    list.dedup();
    list.iter()
        .flat_map(|e| [(e >> 32) as u32, (e & 0xffff_ffff) as u32])
        .collect()
}

impl Model {
    pub fn build() -> Model {
        let eyes = eye_centers();
        let mut b = Builder::new();

        let mouth_surface = front_surface(0.0, SLIT_Y);
        let phi_mouth = (SLIT_Y - CENTER.y).atan2(mouth_surface.z - CENTER.z);
        let phi_eye = (EYE_Y - CENTER.y).atan2(0.7);
        let thetas = column_angles();
        let (phis, slit) = row_angles(phi_mouth, phi_eye);

        let grid_pos: Vec<Vec3> = (0..ROWS)
            .flat_map(|r| {
                let phi = phis[r];
                thetas.iter().map(move |th| (r, *th, phi))
            })
            .map(|(_, th, phi)| radial_surface(ray(th, phi)))
            .collect();
        let slit_y: Vec<f32> = (0..COLS).map(|c| grid_pos[slit * COLS + c].y).collect();
        let split: Vec<bool> = thetas.iter().map(|t| t.abs() < MOUTH_HALF).collect();
        let c_lo = split.iter().position(|s| *s).unwrap_or(COLS / 2);
        let c_hi = split.iter().rposition(|s| *s).unwrap_or(COLS / 2);
        let corner_theta = thetas[(c_hi + 1) % COLS].abs();

        let head_lower = |r: usize, c: usize, p: Vec3| {
            let th = thetas[c].abs();
            let dy = p.y - slit_y[c];
            if split[c] {
                if r < slit {
                    1.0
                } else {
                    0.0
                }
            } else {
                let lat = smoothstep(0.0, 0.5, th - MOUTH_HALF);
                let band = 0.012 + 0.3 * lat;
                1.0 - smoothstep(-band, band, dy)
            }
        };

        let head_start = b.tris.len();
        let mut grid = vec![0u32; ROWS * COLS];
        let mut lower_copy = vec![u32::MAX; COLS];
        for r in 0..ROWS {
            for c in 0..COLS {
                let p = grid_pos[r * COLS + c];
                let dy = p.y - slit_y[c];
                let info = Info {
                    part: Part::Head,
                    lower: head_lower(r, c, p),
                    dy,
                };
                grid[r * COLS + c] = b.vertex(p, info, SKIN, 1.0, 0.25);
                if r == slit && split[c] {
                    lower_copy[c] = b.vertex(p, Info { lower: 1.0, ..info }, SKIN, 1.0, 0.25);
                }
            }
        }
        let bottom_p = radial_surface(-Vec3::Y);
        let top_p = radial_surface(Vec3::Y);
        let bottom = b.vertex(
            bottom_p,
            Info {
                part: Part::Head,
                lower: 1.0,
                dy: bottom_p.y - SLIT_Y,
            },
            SKIN,
            1.0,
            0.25,
        );
        let top = b.vertex(
            top_p,
            Info {
                part: Part::Head,
                lower: 0.0,
                dy: top_p.y - SLIT_Y,
            },
            SKIN,
            1.0,
            0.25,
        );
        let below_id = |r: usize, c: usize| grid[r * COLS + c];
        let above_id = |r: usize, c: usize| {
            if r == slit && lower_copy[c] != u32::MAX {
                lower_copy[c]
            } else {
                grid[r * COLS + c]
            }
        };
        for r in 0..ROWS - 1 {
            for c in 0..COLS {
                let c1 = (c + 1) % COLS;
                let v00 = below_id(r, c);
                let v01 = below_id(r, c1);
                let v10 = above_id(r + 1, c);
                let v11 = above_id(r + 1, c1);
                if c >= COLS / 2 {
                    b.tri(v00, v01, v11);
                    b.tri(v00, v11, v10);
                } else {
                    b.tri(v00, v01, v10);
                    b.tri(v01, v11, v10);
                }
            }
        }
        for c in 0..COLS {
            let c1 = (c + 1) % COLS;
            b.tri(bottom, grid[c1], grid[c]);
            b.tri(
                top,
                grid[(ROWS - 1) * COLS + c],
                grid[(ROWS - 1) * COLS + c1],
            );
        }
        b.orient_closed(head_start, true);
        let head_end = b.pos.len();

        let mouth = grid_pos[slit * COLS + COLS / 2];
        let corner = grid_pos[slit * COLS + (c_hi + 1) % COLS];

        let flap_cols: Vec<usize> = (c_lo.saturating_sub(1)..=(c_hi + 1).min(COLS - 1)).collect();
        for (dir, lower) in [(1.0f32, 0.0f32), (-1.0, 1.0)] {
            let start = b.tris.len();
            let offsets = [
                vec3(0.0, 0.010 * dir, -0.020),
                vec3(0.0, 0.017 * dir, -0.045),
                vec3(0.0, 0.014 * dir, -0.075),
            ];
            let cols_ids: Vec<[u32; 4]> = flap_cols
                .iter()
                .map(|&c| {
                    let base = if lower > 0.5 {
                        above_id(slit, c)
                    } else {
                        below_id(slit, c)
                    };
                    if !split[c] {
                        return [base; 4];
                    }
                    let p0 = grid_pos[slit * COLS + c];
                    let depth = (1.0 - (thetas[c] / corner_theta).powi(2)).max(0.0).sqrt();
                    let mut ids = [base; 4];
                    for (k, off) in offsets.iter().enumerate() {
                        let p = p0 + *off * depth;
                        let shade = [0.6, 1.0, 1.0][k];
                        let col = LIP.lerp(LIP_INNER, shade);
                        let ao = [0.85, 0.65, 0.45][k];
                        let info = Info {
                            part: Part::Flap,
                            lower,
                            dy: p.y - slit_y[c],
                        };
                        ids[k + 1] = b.vertex(p, info, col, ao, 0.6);
                    }
                    ids
                })
                .collect();
            for w in cols_ids.windows(2) {
                for k in 0..3 {
                    b.tri(w[0][k], w[1][k], w[1][k + 1]);
                    b.tri(w[0][k], w[1][k + 1], w[0][k + 1]);
                }
            }
            b.orient_towards(start, vec3(0.0, -dir, 0.3));
        }

        let mut brow_anchor = [Vec3::ZERO; 3];
        for s in [1.0f32, -1.0] {
            let start_v = b.pos.len() as u32;
            let start_t = b.tris.len();
            const N: usize = 24;
            const RING: usize = 10;
            let pts: Vec<(Vec3, Vec3)> = (0..N)
                .map(|i| {
                    let t = i as f32 / (N - 1) as f32;
                    let x = s * (0.075 + 0.40 * t);
                    let y = 0.215 + 0.075 * (PI * (0.2 + 0.8 * t)).sin();
                    let p = front_surface(x, y);
                    let n = sdf_normal(p);
                    (p + n * 0.004, n)
                })
                .collect();
            if s > 0.0 {
                brow_anchor = [pts[2].0, pts[N / 2].0, pts[N - 3].0];
            }
            for i in 0..N {
                let t = i as f32 / (N - 1) as f32;
                let (c, n) = pts[i];
                let prev = pts[i.saturating_sub(1)].0;
                let next = pts[(i + 1).min(N - 1)].0;
                let tan = (next - prev)
                    .reject_from_normalized(n)
                    .normalize_or(Vec3::X);
                let mut bin = n.cross(tan).normalize_or(Vec3::Y);
                if bin.y < 0.0 {
                    bin = -bin;
                }
                let taper = smoothstep(0.0, 0.12, t).sqrt() * (1.0 - t.powi(4)).max(0.0).sqrt();
                let h = 0.034 * (1.0 - 0.55 * t) * taper;
                let k = 0.014 * taper;
                for j in 0..RING {
                    let a = TAU * j as f32 / RING as f32;
                    let p = c + bin * (a.cos() * h) + n * (a.sin() * k);
                    let info = Info {
                        part: Part::Brow,
                        lower: 0.0,
                        dy: p.y - SLIT_Y,
                    };
                    b.vertex(p, info, BROW, 1.0, 0.12);
                }
            }
            let mut tris = Vec::new();
            ring_tris(N, RING, start_v, &mut tris);
            for t in tris.chunks_exact(3) {
                b.tri(t[0], t[1], t[2]);
            }
            b.orient_closed(start_t, true);
        }

        for upper in [true, false] {
            let (front_z, ax, az, y_top, y_bot, lower) = if upper {
                (0.735, 0.20, 0.24, -0.445, -0.528, 0.0)
            } else {
                (0.715, 0.185, 0.23, -0.534, -0.60, 1.0)
            };
            const SEG: usize = 64;
            const RING: usize = 16;
            let start_v = b.pos.len() as u32;
            let start_t = b.tris.len();
            let half_y = 0.5 * (y_top - y_bot);
            let cy = 0.5 * (y_top + y_bot);
            for i in 0..=SEG {
                let a = -1.0 + 2.0 * i as f32 / SEG as f32;
                let psi = a * 1.25;
                let c = vec3(ax * psi.sin(), cy, front_z - az * (1.0 - psi.cos()));
                let out = vec3(az * psi.sin(), 0.0, ax * psi.cos()).normalize_or(Vec3::Z);
                let e = (1.0 - a.abs().powi(8)).max(0.0).sqrt();
                let gap = (PI * a * 6.0).sin().abs();
                let shade = 0.74 + 0.26 * smoothstep(0.0, 0.35, gap);
                for j in 0..RING {
                    let ang = TAU * j as f32 / RING as f32;
                    let (cu, sv) = (ang.cos(), ang.sin());
                    let u = cu.signum() * cu.abs().sqrt();
                    let v = sv.signum() * sv.abs().sqrt();
                    let p = c
                        + out * (u * 0.017 * e)
                        + Vec3::Y * (v * half_y * (1.0 - 0.3 * a.abs()) * e);
                    let info = Info {
                        part: Part::Teeth,
                        lower,
                        dy: p.y - SLIT_Y,
                    };
                    b.vertex(p, info, TEETH * shade, 0.62 * (1.0 - 0.4 * a.abs()), 0.55);
                }
            }
            let mut tris = Vec::new();
            ring_tris(SEG + 1, RING, start_v, &mut tris);
            for t in tris.chunks_exact(3) {
                b.tri(t[0], t[1], t[2]);
            }
            b.orient_closed(start_t, true);
        }

        {
            const SEG: usize = 24;
            const RINGS: usize = 14;
            let c = vec3(0.0, -0.56, 0.42);
            let radii = vec3(0.27, 0.19, 0.34);
            let start_t = b.tris.len();
            let info_at = |p: Vec3| Info {
                part: Part::Cavity,
                lower: smoothstep(SLIT_Y + 0.04, SLIT_Y - 0.06, p.y),
                dy: p.y - SLIT_Y,
            };
            let top_p = c + Vec3::Y * radii.y;
            let first = b.vertex(top_p, info_at(top_p), MOUTH, 0.3, 0.3);
            for k in 1..RINGS {
                let th = PI * k as f32 / RINGS as f32;
                for j in 0..SEG {
                    let a = TAU * j as f32 / SEG as f32;
                    let p = c + vec3(th.sin() * a.cos(), th.cos(), th.sin() * a.sin()) * radii;
                    b.vertex(p, info_at(p), MOUTH, 0.3, 0.3);
                }
            }
            let bot_p = c - Vec3::Y * radii.y;
            let last = b.vertex(bot_p, info_at(bot_p), MOUTH, 0.3, 0.3);
            let ring0 = first + 1;
            for j in 0..SEG {
                let j1 = (j + 1) % SEG;
                b.tri(first, ring0 + j as u32, ring0 + j1 as u32);
            }
            let mut tris = Vec::new();
            ring_tris(RINGS - 1, SEG, ring0, &mut tris);
            for t in tris.chunks_exact(3) {
                b.tri(t[0], t[1], t[2]);
            }
            let lastring = ring0 + ((RINGS - 2) * SEG) as u32;
            for j in 0..SEG {
                let j1 = (j + 1) % SEG;
                b.tri(last, lastring + j1 as u32, lastring + j as u32);
            }
            b.orient_closed(start_t, false);
        }

        let normals = vertex_normals(&b.pos, &b.tris);

        for i in 0..head_end {
            let p = b.pos[i];
            let info = b.info[i];
            let n = sdf_normal(p);
            let m = vec3(p.x.abs(), p.y, p.z);
            let gm = |c: Vec3, r: f32| (-(m - c).length_squared() / (r * r)).exp();
            let mut col = SKIN;
            col = col.lerp(BLUSH, 0.2 * gm(vec3(0.36, -0.24, 0.55), 0.16));
            col = col.lerp(BLUSH, 0.18 * gm(vec3(0.0, -0.22, 0.98), 0.09));
            col = col.lerp(BLUSH, 0.1 * gm(vec3(0.78, 0.0, -0.1), 0.12));
            col *= 1.0 - 0.08 * gm(vec3(EYE_X, EYE_Y, eyes[0].z + 0.1), 0.16);
            let lp = lip_paint(p, info.dy, corner.x);
            col = col.lerp(LIP, lp);
            let mut ao = ambient_occlusion(p, n, &eyes);
            let near_slit = info.dy.abs() < 0.004 && p.x.abs() < corner.x && p.z > 0.5;
            if near_slit {
                ao *= 0.6;
                col = col.lerp(LIP_INNER, 0.5);
            }
            b.col[i] = col;
            b.ao[i] = ao;
            b.gloss[i] = 0.25 + 0.4 * lp;
        }

        let anchors = Anchors {
            mouth,
            corner,
            brow: brow_anchor,
            under_eye: front_surface(EYE_X, -0.10),
            above_eye: front_surface(EYE_X, 0.2),
            ala: vec3(0.065, -0.25, 0.81),
            sneer: front_surface(0.09, -0.2),
            sneer_lip: front_surface(0.08, -0.45),
            cheek: front_surface(0.32, -0.16),
            cheek_low: front_surface(0.3, -0.32),
            puff: front_surface(0.36, -0.42),
            masseter: front_surface(0.56, -0.48),
            chin: front_surface(0.0, -0.78),
        };

        let skin_count = b.pos.len();
        let mut targets = Vec::with_capacity(TARGET_COUNT);
        let mut moved = b.pos.clone();
        let mut moved_n = vec![Vec3::ZERO; skin_count];
        for k in 0..TARGET_COUNT {
            let (morph, s) = target_at(k);
            let disp: Vec<Vec3> = (0..skin_count)
                .map(|i| field(morph, s, &b.info[i], b.pos[i], normals[i], &anchors))
                .collect();
            for (m, (p, d)) in moved.iter_mut().zip(b.pos.iter().zip(disp.iter())) {
                *m = *p + *d;
            }
            accumulate_normals(&moved, &b.tris, &mut moved_n);
            let deltas = (0..skin_count)
                .filter_map(|i| {
                    let dp = disp[i];
                    let dn = moved_n[i] - normals[i];
                    (dp.length_squared() > 1e-10 || dn.length_squared() > 1e-8).then_some(Delta {
                        index: i as u32,
                        d: [dp.x, dp.y, dp.z, dn.x, dn.y, dn.z],
                    })
                })
                .collect();
            targets.push(deltas);
        }

        let jaw_weight: Vec<f32> = (0..skin_count)
            .map(|i| {
                let info = &b.info[i];
                match info.part {
                    Part::Teeth | Part::Cavity => info.lower,
                    Part::Brow => 0.0,
                    Part::Head | Part::Flap => info.lower * jaw_region(b.pos[i]),
                }
            })
            .collect();

        let seal_weight: Vec<f32> = (0..skin_count)
            .map(|i| {
                let info = &b.info[i];
                let p = b.pos[i];
                match info.part {
                    Part::Head | Part::Flap => {
                        let wx = (1.0 - (p.x / (corner.x * 1.15)).powi(2)).max(0.0).sqrt();
                        let zone =
                            (-(info.dy / 0.11).powi(2)).exp() * wx * smoothstep(0.3, 0.6, p.z);
                        let w = jaw_weight[i];
                        w + (0.5 * jaw_region(p) - w) * zone
                    }
                    _ => jaw_weight[i],
                }
            })
            .collect();

        let mut base = Vec::with_capacity(skin_count * STRIDE);
        for ((p, n), (c, (ao, gloss))) in b
            .pos
            .iter()
            .zip(normals.iter())
            .zip(b.col.iter().zip(b.ao.iter().zip(b.gloss.iter())))
        {
            push_vertex(&mut base, *p, *n, &[c.x, c.y, c.z, *ao, *gloss]);
        }

        let mut tongue_pos = Vec::new();
        tongue_points(&Tongue::default(), &mut tongue_pos);
        let mut tongue_tris = Vec::new();
        ring_tris(TONGUE_SEG + 1, TONGUE_RING, 0, &mut tongue_tris);
        if signed_volume(&tongue_pos, &tongue_tris) < 0.0 {
            flip(&mut tongue_tris);
        }
        let tongue_attr: Vec<[f32; 5]> = (0..tongue_pos.len())
            .map(|i| {
                let s = (i / TONGUE_RING) as f32 / TONGUE_SEG as f32;
                let a = TAU * (i % TONGUE_RING) as f32 / TONGUE_RING as f32;
                let c = if a.sin() < -0.2 {
                    TONGUE * 0.78
                } else {
                    TONGUE
                };
                [c.x, c.y, c.z, 0.45 + 0.55 * smoothstep(0.35, 0.95, s), 0.65]
            })
            .collect();

        let per_lid = LID_ROWS * LID_COLS;
        let mut lid_pos = Vec::new();
        let mut lid_tris = Vec::new();
        let mut lid_attr = Vec::new();
        for (e, center) in eyes.iter().enumerate() {
            let first = lid_pos.len();
            lid_points(*center, Lid::default(), &mut lid_pos);
            for half in 0..2 {
                let base_v = first + half * per_lid;
                let mut tris = Vec::new();
                grid_tris(LID_ROWS, LID_COLS, base_v as u32, &mut tris);
                let mid = base_v + 4 * LID_COLS + LID_COLS / 2;
                let n = face_sum(&lid_pos, &tris);
                let outward = lid_pos[mid] - eyes[e];
                if n.dot(outward) < 0.0 {
                    flip(&mut tris);
                }
                lid_tris.extend(tris);
                for row in 0..LID_ROWS {
                    let (col, ao) = if half == 0 {
                        match row {
                            0 | 1 => (LASH, 0.8),
                            2 => (LASH.lerp(SKIN, 0.55), 0.85),
                            _ => (SKIN, 1.0),
                        }
                    } else {
                        match row {
                            0 => (SKIN.lerp(LIP_INNER, 0.45), 0.8),
                            1 => (SKIN.lerp(LASH, 0.35), 0.85),
                            _ => (SKIN, 1.0),
                        }
                    };
                    for _ in 0..LID_COLS {
                        lid_attr.push([col.x, col.y, col.z, ao, 0.3]);
                    }
                }
            }
        }

        let tongue_start = skin_count;
        let lid_start = tongue_start + tongue_pos.len();
        let vertex_count = lid_start + lid_pos.len();
        let mut indices = b.tris.clone();
        indices.extend(tongue_tris.iter().map(|i| i + tongue_start as u32));
        indices.extend(lid_tris.iter().map(|i| i + lid_start as u32));
        let lines = edges(&indices);
        let (eye_vertices, eye_indices) = build_eye();

        Model {
            base,
            jaw_weight,
            seal_weight,
            targets,
            skin_count,
            tongue_tris,
            tongue_attr,
            lid_tris,
            lid_attr,
            indices,
            lines,
            vertex_count,
            eye_vertices,
            eye_indices,
            eyes,
        }
    }

    pub fn pose_vertices(&self, pose: &Pose, out: &mut Vec<f32>) {
        out.clear();
        out.extend_from_slice(&self.base);
        for (deltas, w) in self.targets.iter().zip(pose.weights.iter()) {
            let w = unit(*w);
            if w < 1e-3 {
                continue;
            }
            for d in deltas {
                let o = d.index as usize * STRIDE;
                for (dst, src) in out[o..o + 6].iter_mut().zip(d.d.iter()) {
                    *dst += w * src;
                }
            }
        }
        let jaw = pose.jaw_transform();
        let seal = unit(pose.jaw.seal);
        for (v, (w, ws)) in out
            .chunks_exact_mut(STRIDE)
            .zip(self.jaw_weight.iter().zip(self.seal_weight.iter()))
        {
            let w = &(w + (ws - w) * seal);
            if *w <= 0.0 {
                continue;
            }
            let p = vec3(v[0], v[1], v[2]);
            let n = vec3(v[3], v[4], v[5]);
            let p = p.lerp(jaw.transform_point3(p), *w);
            let n = n.lerp(jaw.transform_vector3(n), *w);
            v[..6].copy_from_slice(&[p.x, p.y, p.z, n.x, n.y, n.z]);
        }

        let mut pos = Vec::with_capacity(self.tongue_attr.len());
        tongue_points(&pose.tongue, &mut pos);
        for p in &mut pos {
            *p = jaw.transform_point3(*p);
        }
        let normals = vertex_normals(&pos, &self.tongue_tris);
        for ((p, n), a) in pos.iter().zip(normals.iter()).zip(self.tongue_attr.iter()) {
            push_vertex(out, *p, *n, a);
        }

        pos.clear();
        for (center, lid) in self.eyes.iter().zip(pose.lids.iter()) {
            lid_points(*center, *lid, &mut pos);
        }
        let normals = vertex_normals(&pos, &self.lid_tris);
        for ((p, n), a) in pos.iter().zip(normals.iter()).zip(self.lid_attr.iter()) {
            push_vertex(out, *p, *n, a);
        }
        debug_assert_eq!(out.len(), self.vertex_count * STRIDE);
        for v in out.iter_mut() {
            if !v.is_finite() {
                *v = 0.0;
            }
        }
    }

    pub fn skin_vertex_count(&self) -> usize {
        self.skin_count
    }

    pub fn target_len(&self, k: usize) -> usize {
        self.targets.get(k).map_or(0, |t| t.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::OnceLock;

    pub fn model() -> &'static Model {
        static MODEL: OnceLock<Model> = OnceLock::new();
        MODEL.get_or_init(Model::build)
    }

    fn positions(data: &[f32]) -> Vec<Vec3> {
        data.chunks_exact(STRIDE)
            .map(|v| vec3(v[0], v[1], v[2]))
            .collect()
    }

    #[test]
    fn mesh_is_finite_and_indexed() {
        let m = model();
        assert_eq!(m.base.len(), m.skin_count * STRIDE);
        assert!(m.base.iter().all(|v| v.is_finite()));
        assert_eq!(m.indices.len() % 3, 0);
        assert_eq!(m.lines.len() % 2, 0);
        assert!(m.indices.iter().all(|i| (*i as usize) < m.vertex_count));
        assert!(m.lines.iter().all(|i| (*i as usize) < m.vertex_count));
        assert!(m
            .eye_indices
            .iter()
            .all(|i| (*i as usize) * STRIDE < m.eye_vertices.len()));
        assert!(m.eye_vertices.iter().all(|v| v.is_finite()));
        for v in m.base.chunks_exact(STRIDE) {
            let n = vec3(v[3], v[4], v[5]);
            assert!((n.length() - 1.0).abs() < 1e-3);
        }
    }

    #[test]
    fn rest_pose_vertices_are_complete() {
        let m = model();
        let mut out = Vec::new();
        m.pose_vertices(&Pose::default(), &mut out);
        assert_eq!(out.len(), m.vertex_count * STRIDE);
        assert!(out.iter().all(|v| v.is_finite()));
    }

    #[test]
    fn head_normals_point_outward() {
        let m = model();
        let pos = positions(&m.base);
        let normals: Vec<Vec3> = m
            .base
            .chunks_exact(STRIDE)
            .map(|v| vec3(v[3], v[4], v[5]))
            .collect();
        let head = ROWS * COLS;
        let outward = (0..head)
            .filter(|i| normals[*i].dot(pos[*i] - CENTER) > 0.0)
            .count();
        assert!(outward as f32 > head as f32 * 0.97, "{outward} of {head}");
    }

    #[test]
    fn teeth_rest_inside_the_head() {
        let m = model();
        let pos = positions(&m.base);
        for (i, p) in pos.iter().enumerate().skip(ROWS * COLS) {
            if m.jaw_weight[i] > 0.99 && p.z > 0.6 && p.y > -0.62 && p.y < -0.44 && p.x.abs() < 0.15
            {
                assert!(head_sdf(*p) < 0.02, "vertex {i} at {p} pokes out");
            }
        }
    }

    #[test]
    fn every_target_moves_something() {
        let m = model();
        for k in 0..TARGET_COUNT {
            assert!(m.target_len(k) > 0, "target {:?} is empty", target_at(k));
        }
    }

    #[test]
    fn target_indices_are_unique() {
        let mut seen = [false; TARGET_COUNT];
        for m in SIDED {
            for side in [Side::Left, Side::Right] {
                let i = target_index(m, side).unwrap();
                assert!(!seen[i]);
                seen[i] = true;
            }
            assert!(target_index(m, Side::Center).is_none());
        }
        for m in CENTERED {
            let i = target_index(m, Side::Center).unwrap();
            assert!(!seen[i]);
            seen[i] = true;
        }
        assert!(seen.iter().all(|s| *s));
    }

    #[test]
    fn extreme_pose_stays_finite() {
        let m = model();
        let mut pose = Pose {
            weights: [1.0; TARGET_COUNT],
            ..Pose::default()
        };
        pose.jaw = Jaw {
            open: 1.0,
            side: 1.0,
            forward: -1.0,
            raise: 1.0,
            seal: 0.5,
        };
        pose.tongue = Tongue {
            out: 1.0,
            up: 1.0,
            down: 0.0,
            left: 1.0,
            right: 0.0,
            roll: 1.0,
            bend_down: 1.0,
            curl_up: 1.0,
            squish: 1.0,
            flat: 1.0,
            twist: -1.0,
        };
        pose.lids = [
            Lid {
                upper: 1.2,
                lower: -1.2,
            },
            Lid {
                upper: -0.1,
                lower: -0.1,
            },
        ];
        let mut out = Vec::new();
        m.pose_vertices(&pose, &mut out);
        assert_eq!(out.len(), m.vertex_count * STRIDE);
        assert!(out.iter().all(|v| v.is_finite()));
    }

    #[test]
    fn sanitize_handles_garbage() {
        let mut pose = Pose::default();
        pose.weights[0] = f32::NAN;
        pose.weights[1] = 7.0;
        pose.weights[2] = -3.0;
        pose.jaw.open = f32::INFINITY;
        pose.jaw.side = -9.0;
        pose.lids[0].upper = f32::NAN;
        pose.lids[1] = Lid {
            upper: -0.5,
            lower: 0.5,
        };
        pose.tongue.twist = f32::NEG_INFINITY;
        pose.tongue.out = 2.0;
        pose.sanitize();
        assert_eq!(pose.weights[0], 0.0);
        assert_eq!(pose.weights[1], 1.0);
        assert_eq!(pose.weights[2], 0.0);
        assert_eq!(pose.jaw.open, 0.0);
        assert_eq!(pose.jaw.side, -1.0);
        assert_eq!(pose.lids[0].upper, Lid::default().upper);
        assert!(pose.lids[1].upper >= pose.lids[1].lower);
        assert_eq!(pose.tongue.twist, 0.0);
        assert_eq!(pose.tongue.out, 1.0);
        let mut out = Vec::new();
        model().pose_vertices(&pose, &mut out);
        assert!(out.iter().all(|v| v.is_finite()));
    }

    #[test]
    fn jaw_open_lowers_the_chin() {
        let m = model();
        let mut rest = Vec::new();
        m.pose_vertices(&Pose::default(), &mut rest);
        let mut pose = Pose::default();
        pose.jaw.open = 1.0;
        let mut open = Vec::new();
        m.pose_vertices(&pose, &mut open);
        let target = front_surface(0.0, -0.8);
        let chin = positions(&m.base)
            .iter()
            .enumerate()
            .take(ROWS * COLS)
            .min_by(|a, b| a.1.distance(target).total_cmp(&b.1.distance(target)))
            .map(|(i, _)| i)
            .unwrap();
        assert!(open[chin * STRIDE + 1] < rest[chin * STRIDE + 1] - 0.1);
    }

    #[test]
    fn pose_difference_detects_changes() {
        let a = Pose::default();
        let mut b = a;
        assert_eq!(a.difference(&b), 0.0);
        b.tongue.out = 0.5;
        assert!(a.difference(&b) >= 0.5);
        b.tongue.out = f32::NAN;
        assert!(a.difference(&b).is_infinite());
    }
}
