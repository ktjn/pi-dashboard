use std::f32::consts::{PI, TAU};

use jiff::Timestamp;
use macroquad::camera::{Camera3D, Projection, set_camera, set_default_camera};
use macroquad::color::Color;
use macroquad::math::{Vec3, vec2, vec3};
use macroquad::models::{Mesh, Vertex, draw_mesh};
use macroquad::shapes::draw_rectangle;
use macroquad::time::get_time;
use macroquad::window::{screen_dpi_scale, screen_height, screen_width};
use time::OffsetDateTime;

use crate::daylight::DaylightWindow;
use crate::theme::palette;

pub const FLIP_DURATION_SECONDS: f32 = 0.34;

const SOLAR_ARC_HALF_SPAN_X: f32 = 3.2;
const SOLAR_ARC_PEAK_Y: f32 = 1.7;
const SOLAR_ARC_BASE_Z: f32 = -1.2;
const SOLAR_ARC_CURVE_Z: f32 = 0.4;
const SOLAR_ARC_SAMPLES: usize = 25;
const GOLDEN_HOUR_FRACTION: f32 = 0.10;
const MAX_LOWER_SHADOW_ALPHA: f32 = 0.55;
const DIORAMA_Z_SHIFT: f32 = 1.0;
const DIORAMA_PEDESTAL_BASE_RX: f32 = 3.8;
const DIORAMA_PEDESTAL_RZ: f32 = 2.15;
const DIORAMA_PLINTH_BASE_RX: f32 = 3.94;
const DIORAMA_PLINTH_RZ: f32 = 2.26;
const CLOUD_SHADOW_RADIUS: f32 = 0.52;
const PRISM_SIDES: usize = 6;

fn clamp_logical_rect(
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    screen_w: f32,
    screen_h: f32,
) -> Option<(f32, f32, f32, f32)> {
    if !x.is_finite()
        || !y.is_finite()
        || !width.is_finite()
        || !height.is_finite()
        || !screen_w.is_finite()
        || !screen_h.is_finite()
    {
        return None;
    }
    if width <= 0.0 || height <= 0.0 || screen_w <= 0.0 || screen_h <= 0.0 {
        return None;
    }

    let left = x.clamp(0.0, screen_w);
    let right = (x + width).clamp(0.0, screen_w);
    let top = y.clamp(0.0, screen_h);
    let bottom = (y + height).clamp(0.0, screen_h);
    let clamped_w = right - left;
    let clamped_h = bottom - top;

    if clamped_w <= 0.0 || clamped_h <= 0.0 {
        return None;
    }

    Some((left, top, clamped_w, clamped_h))
}

pub fn viewport_rect_to_gl(
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    screen_h: f32,
    dpi_scale: f32,
) -> Option<(i32, i32, i32, i32)> {
    if !x.is_finite()
        || !y.is_finite()
        || !width.is_finite()
        || !height.is_finite()
        || !screen_h.is_finite()
        || !dpi_scale.is_finite()
    {
        return None;
    }
    if width <= 0.0 || height <= 0.0 || screen_h <= 0.0 || dpi_scale <= 0.0 {
        return None;
    }

    let left = x.max(0.0);
    let right = (x + width).max(0.0);
    let clamped_w = right - left;

    let top = y.clamp(0.0, screen_h);
    let bottom = (y + height).clamp(0.0, screen_h);
    let clamped_h = bottom - top;

    if clamped_w <= 0.0 || clamped_h <= 0.0 {
        return None;
    }

    let vx = (left * dpi_scale).round() as i32;
    let vy = ((screen_h - bottom) * dpi_scale).round().max(0.0) as i32;
    let vw = (clamped_w * dpi_scale).round() as i32;
    let vh = (clamped_h * dpi_scale).round() as i32;

    if vw <= 0 || vh <= 0 {
        return None;
    }

    Some((vx, vy, vw, vh))
}

struct MeshBuilder {
    vertices: Vec<Vertex>,
    indices: Vec<u16>,
}

impl MeshBuilder {
    fn with_capacity(vertices: usize, indices: usize) -> Self {
        Self {
            vertices: Vec::with_capacity(vertices.min(usize::from(u16::MAX))),
            indices: Vec::with_capacity(indices),
        }
    }

    fn push_triangle(&mut self, p0: Vec3, p1: Vec3, p2: Vec3, color: Color) {
        if self.vertices.len() + 3 > usize::from(u16::MAX) {
            return;
        }
        let base = self.vertices.len() as u16;
        self.vertices.push(Vertex::new2(p0, vec2(0.0, 0.0), color));
        self.vertices.push(Vertex::new2(p1, vec2(1.0, 0.0), color));
        self.vertices.push(Vertex::new2(p2, vec2(0.5, 1.0), color));
        self.indices.extend_from_slice(&[base, base + 1, base + 2]);
    }

    fn push_quad(&mut self, p0: Vec3, p1: Vec3, p2: Vec3, p3: Vec3, color: Color) {
        if self.vertices.len() + 4 > usize::from(u16::MAX) {
            return;
        }
        let base = self.vertices.len() as u16;
        self.vertices.push(Vertex::new2(p0, vec2(0.0, 0.0), color));
        self.vertices.push(Vertex::new2(p1, vec2(1.0, 0.0), color));
        self.vertices.push(Vertex::new2(p2, vec2(1.0, 1.0), color));
        self.vertices.push(Vertex::new2(p3, vec2(0.0, 1.0), color));
        self.indices
            .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }

    fn build(self) -> Mesh {
        Mesh {
            vertices: self.vertices,
            indices: self.indices,
            texture: None,
        }
    }
}

fn empty_mesh() -> Mesh {
    Mesh {
        vertices: Vec::new(),
        indices: Vec::new(),
        texture: None,
    }
}

fn rotate_vec3_x(v: Vec3, angle_rad: f32) -> Vec3 {
    let (sin_a, cos_a) = angle_rad.sin_cos();
    vec3(v.x, v.y * cos_a - v.z * sin_a, v.y * sin_a + v.z * cos_a)
}

fn rotate_vec3_y(v: Vec3, angle_rad: f32) -> Vec3 {
    let (sin_a, cos_a) = angle_rad.sin_cos();
    vec3(v.x * cos_a + v.z * sin_a, v.y, -v.x * sin_a + v.z * cos_a)
}

fn blend_color(a: Color, b: Color, t: f32) -> Color {
    let k = t.clamp(0.0, 1.0);
    Color::new(
        a.r + (b.r - a.r) * k,
        a.g + (b.g - a.g) * k,
        a.b + (b.b - a.b) * k,
        a.a + (b.a - a.a) * k,
    )
}

fn shade_color(base: Color, factor: f32) -> Color {
    Color::new(
        (base.r * factor).clamp(0.0, 1.0),
        (base.g * factor).clamp(0.0, 1.0),
        (base.b * factor).clamp(0.0, 1.0),
        base.a,
    )
}

pub fn shade_triangle_color(
    base: Color,
    normal: Vec3,
    light_dir: Vec3,
    view_dir: Vec3,
    ambient: f32,
    spec_strength: f32,
) -> Color {
    let n = normal.normalize_or_zero();
    let l = light_dir.normalize_or_zero();
    let v = view_dir.normalize_or_zero();

    let ndotl = n.dot(l);
    let diffuse = ndotl.max(0.0);
    let ambient_clamped = ambient.clamp(0.0, 1.0);
    let hemisphere = ambient_clamped * (0.65 + 0.35 * (ndotl * 0.5 + 0.5));
    let lighting = hemisphere + (1.0 - ambient_clamped) * diffuse;

    let spec = if diffuse > 0.0 && spec_strength > 0.0 {
        let half_vec = (l + v).normalize_or_zero();
        half_vec.dot(n).max(0.0).powf(20.0) * spec_strength
    } else {
        0.0
    };

    Color::new(
        (base.r * lighting + spec).clamp(0.0, 1.0),
        (base.g * lighting + spec).clamp(0.0, 1.0),
        (base.b * lighting + spec).clamp(0.0, 1.0),
        base.a,
    )
}

#[allow(clippy::too_many_arguments)]
fn build_lit_box_mesh_styled(
    origin: Vec3,
    half_extents: Vec3,
    rotation_x: f32,
    pivot_offset: Vec3,
    base_color: Color,
    light_dir: Vec3,
    view_dir: Vec3,
    ambient: f32,
    spec_strength: f32,
) -> Mesh {
    let hx = half_extents.x.abs();
    let hy = half_extents.y.abs();
    let hz = half_extents.z.abs();

    let faces: [([Vec3; 4], Vec3); 6] = [
        (
            [
                vec3(-hx, -hy, hz),
                vec3(hx, -hy, hz),
                vec3(hx, hy, hz),
                vec3(-hx, hy, hz),
            ],
            Vec3::Z,
        ),
        (
            [
                vec3(hx, -hy, -hz),
                vec3(-hx, -hy, -hz),
                vec3(-hx, hy, -hz),
                vec3(hx, hy, -hz),
            ],
            -Vec3::Z,
        ),
        (
            [
                vec3(-hx, hy, hz),
                vec3(hx, hy, hz),
                vec3(hx, hy, -hz),
                vec3(-hx, hy, -hz),
            ],
            Vec3::Y,
        ),
        (
            [
                vec3(-hx, -hy, -hz),
                vec3(hx, -hy, -hz),
                vec3(hx, -hy, hz),
                vec3(-hx, -hy, hz),
            ],
            -Vec3::Y,
        ),
        (
            [
                vec3(hx, -hy, hz),
                vec3(hx, -hy, -hz),
                vec3(hx, hy, -hz),
                vec3(hx, hy, hz),
            ],
            Vec3::X,
        ),
        (
            [
                vec3(-hx, -hy, -hz),
                vec3(-hx, -hy, hz),
                vec3(-hx, hy, hz),
                vec3(-hx, hy, -hz),
            ],
            -Vec3::X,
        ),
    ];

    let mut builder = MeshBuilder::with_capacity(24, 36);
    for (corners, local_normal) in faces {
        let world_normal = rotate_vec3_x(local_normal, rotation_x);
        let color = shade_triangle_color(
            base_color,
            world_normal,
            light_dir,
            view_dir,
            ambient,
            spec_strength,
        );
        let p0 = origin + rotate_vec3_x(pivot_offset + corners[0], rotation_x);
        let p1 = origin + rotate_vec3_x(pivot_offset + corners[1], rotation_x);
        let p2 = origin + rotate_vec3_x(pivot_offset + corners[2], rotation_x);
        let p3 = origin + rotate_vec3_x(pivot_offset + corners[3], rotation_x);
        builder.push_quad(p0, p1, p2, p3, color);
    }
    builder.build()
}

pub fn build_lit_box_mesh(
    center: Vec3,
    half_extents: Vec3,
    base_color: Color,
    light_dir: Vec3,
    view_dir: Vec3,
) -> Mesh {
    build_lit_box_mesh_styled(
        center,
        half_extents,
        0.0,
        Vec3::ZERO,
        base_color,
        light_dir,
        view_dir,
        0.24,
        0.30,
    )
}

pub fn build_lit_transformed_box_mesh(
    origin: Vec3,
    half_extents: Vec3,
    rotation_x: f32,
    pivot_offset: Vec3,
    base_color: Color,
    light_dir: Vec3,
    view_dir: Vec3,
) -> Mesh {
    build_lit_box_mesh_styled(
        origin,
        half_extents,
        rotation_x,
        pivot_offset,
        base_color,
        light_dir,
        view_dir,
        0.24,
        0.36,
    )
}

pub fn build_lit_icosphere_mesh(
    center: Vec3,
    radius: f32,
    subdivisions: u8,
    base_color: Color,
    light_dir: Vec3,
    view_dir: Vec3,
    emissive: f32,
) -> Mesh {
    if !radius.is_finite() || radius <= 0.0 {
        return empty_mesh();
    }

    let phi = (1.0 + 5.0_f32.sqrt()) * 0.5;
    let base_verts: [Vec3; 12] = [
        vec3(-1.0, phi, 0.0).normalize(),
        vec3(1.0, phi, 0.0).normalize(),
        vec3(-1.0, -phi, 0.0).normalize(),
        vec3(1.0, -phi, 0.0).normalize(),
        vec3(0.0, -1.0, phi).normalize(),
        vec3(0.0, 1.0, phi).normalize(),
        vec3(0.0, -1.0, -phi).normalize(),
        vec3(0.0, 1.0, -phi).normalize(),
        vec3(phi, 0.0, -1.0).normalize(),
        vec3(phi, 0.0, 1.0).normalize(),
        vec3(-phi, 0.0, -1.0).normalize(),
        vec3(-phi, 0.0, 1.0).normalize(),
    ];

    let base_faces: [[usize; 3]; 20] = [
        [0, 11, 5],
        [0, 5, 1],
        [0, 1, 7],
        [0, 7, 10],
        [0, 10, 11],
        [1, 5, 9],
        [5, 11, 4],
        [11, 10, 2],
        [10, 7, 6],
        [7, 1, 8],
        [3, 9, 4],
        [3, 4, 2],
        [3, 2, 6],
        [3, 6, 8],
        [3, 8, 9],
        [4, 9, 5],
        [2, 4, 11],
        [6, 2, 10],
        [8, 6, 7],
        [9, 8, 1],
    ];

    let mut triangles: Vec<[Vec3; 3]> = base_faces
        .iter()
        .map(|f| [base_verts[f[0]], base_verts[f[1]], base_verts[f[2]]])
        .collect();

    for _ in 0..subdivisions.min(3) {
        let mut next = Vec::with_capacity(triangles.len() * 4);
        for [a, b, c] in triangles {
            let ab = (a + b).normalize();
            let bc = (b + c).normalize();
            let ca = (c + a).normalize();
            next.push([a, ab, ca]);
            next.push([b, bc, ab]);
            next.push([c, ca, bc]);
            next.push([ab, bc, ca]);
        }
        triangles = next;
    }

    let e = emissive.clamp(0.0, 1.0);
    let ambient = (0.24 + e * 0.44).clamp(0.0, 0.76);
    let mut builder = MeshBuilder::with_capacity(triangles.len() * 3, triangles.len() * 3);

    for [a, mut b, mut c] in triangles {
        let mut normal = (b - a).cross(c - a).normalize_or_zero();
        if normal.dot(a + b + c) < 0.0 {
            std::mem::swap(&mut b, &mut c);
            normal = -normal;
        }
        let lit = shade_triangle_color(base_color, normal, light_dir, view_dir, ambient, 0.38);
        let color = Color::new(
            (lit.r + base_color.r * e * 0.20).clamp(0.0, 1.0),
            (lit.g + base_color.g * e * 0.20).clamp(0.0, 1.0),
            (lit.b + base_color.b * e * 0.20).clamp(0.0, 1.0),
            base_color.a,
        );
        builder.push_triangle(
            center + a * radius,
            center + b * radius,
            center + c * radius,
            color,
        );
    }

    builder.build()
}

pub fn build_lit_prism_segment_mesh(
    start: Vec3,
    end: Vec3,
    radius: f32,
    base_color: Color,
    light_dir: Vec3,
    view_dir: Vec3,
) -> Mesh {
    let delta = end - start;
    if !radius.is_finite() || radius <= 0.0 || delta.length_squared() <= 1e-10 {
        return empty_mesh();
    }

    let axis = delta.normalize();
    let ref_up = if axis.y.abs() < 0.9 { Vec3::Y } else { Vec3::X };
    let right = axis.cross(ref_up).normalize();
    let forward = right.cross(axis).normalize();

    let mut ring_offsets = [Vec3::ZERO; PRISM_SIDES];
    for (idx, slot) in ring_offsets.iter_mut().enumerate() {
        let angle = (idx as f32 / PRISM_SIDES as f32) * TAU;
        let (sin_a, cos_a) = angle.sin_cos();
        *slot = (right * cos_a + forward * sin_a) * radius;
    }

    let mut builder = MeshBuilder::with_capacity(PRISM_SIDES * 10, PRISM_SIDES * 12);
    let start_cap_color = shade_triangle_color(base_color, -axis, light_dir, view_dir, 0.24, 0.28);
    let end_cap_color = shade_triangle_color(base_color, axis, light_dir, view_dir, 0.24, 0.28);

    for i in 0..PRISM_SIDES {
        let next = (i + 1) % PRISM_SIDES;
        let o0 = ring_offsets[i];
        let o1 = ring_offsets[next];
        let s0 = start + o0;
        let s1 = start + o1;
        let e0 = end + o0;
        let e1 = end + o1;

        let outward = (o0 + o1).normalize_or_zero();
        let side_color = shade_triangle_color(base_color, outward, light_dir, view_dir, 0.24, 0.34);

        let face_n = (s1 - s0).cross(e1 - s0);
        if face_n.dot(outward) >= 0.0 {
            builder.push_quad(s0, s1, e1, e0, side_color);
        } else {
            builder.push_quad(s0, e0, e1, s1, side_color);
        }

        let start_n = (s1 - start).cross(s0 - start);
        if start_n.dot(-axis) >= 0.0 {
            builder.push_triangle(start, s1, s0, start_cap_color);
        } else {
            builder.push_triangle(start, s0, s1, start_cap_color);
        }

        let end_n = (e0 - end).cross(e1 - end);
        if end_n.dot(axis) >= 0.0 {
            builder.push_triangle(end, e0, e1, end_cap_color);
        } else {
            builder.push_triangle(end, e1, e0, end_cap_color);
        }
    }

    builder.build()
}

#[allow(clippy::too_many_arguments)]
fn build_lit_elliptical_cylinder_mesh(
    center: Vec3,
    radius_x: f32,
    radius_z: f32,
    height: f32,
    segments: usize,
    top_color: Color,
    side_color: Color,
    light_dir: Vec3,
    view_dir: Vec3,
) -> Mesh {
    if !radius_x.is_finite()
        || !radius_z.is_finite()
        || !height.is_finite()
        || radius_x <= 0.0
        || radius_z <= 0.0
        || height <= 0.0
    {
        return empty_mesh();
    }

    let sides = segments.max(3);
    let half_h = height * 0.5;
    let y_top = center.y + half_h;
    let y_bot = center.y - half_h;
    let top_center = vec3(center.x, y_top, center.z);
    let bot_center = vec3(center.x, y_bot, center.z);

    let shaded_top = shade_triangle_color(top_color, Vec3::Y, light_dir, view_dir, 0.28, 0.22);
    let shaded_bot = shade_triangle_color(side_color, -Vec3::Y, light_dir, view_dir, 0.22, 0.10);

    let mut builder = MeshBuilder::with_capacity(sides * 10, sides * 12);

    for i in 0..sides {
        let a0 = (i as f32 / sides as f32) * TAU;
        let a1 = ((i + 1) as f32 / sides as f32) * TAU;
        let (s0, c0) = a0.sin_cos();
        let (s1, c1) = a1.sin_cos();

        let t0 = vec3(center.x + radius_x * c0, y_top, center.z + radius_z * s0);
        let t1 = vec3(center.x + radius_x * c1, y_top, center.z + radius_z * s1);
        let b0 = vec3(center.x + radius_x * c0, y_bot, center.z + radius_z * s0);
        let b1 = vec3(center.x + radius_x * c1, y_bot, center.z + radius_z * s1);

        let mid_angle = (a0 + a1) * 0.5;
        let outward =
            vec3(mid_angle.cos() / radius_x, 0.0, mid_angle.sin() / radius_z).normalize_or_zero();
        let shaded_side =
            shade_triangle_color(side_color, outward, light_dir, view_dir, 0.22, 0.30);

        builder.push_triangle(top_center, t1, t0, shaded_top);
        builder.push_triangle(bot_center, b0, b1, shaded_bot);
        builder.push_quad(b0, t0, t1, b1, shaded_side);
    }

    builder.build()
}

#[allow(clippy::too_many_arguments)]
pub fn build_lit_cylinder_mesh(
    center: Vec3,
    radius: f32,
    height: f32,
    segments: usize,
    top_color: Color,
    side_color: Color,
    light_dir: Vec3,
    view_dir: Vec3,
) -> Mesh {
    build_lit_elliptical_cylinder_mesh(
        center, radius, radius, height, segments, top_color, side_color, light_dir, view_dir,
    )
}

struct ArchipelagoHill {
    center: Vec3,
    radius_x: f32,
    radius_z: f32,
    x_stretch: f32,
    height: f32,
    peak_offset_x: f32,
    peak_offset_z: f32,
    sides: usize,
    rotation_y: f32,
    color: Color,
}

fn hill_base_ring(hill: &ArchipelagoHill) -> Vec<Vec3> {
    let side_count = hill.sides.max(4);
    let mut ring = Vec::with_capacity(side_count);

    for i in 0..side_count {
        let angle = (i as f32 / side_count as f32) * TAU;
        let jitter = 0.88 + 0.22 * ((i as f32 * 2.3 + hill.rotation_y).sin());
        let local = vec3(
            angle.cos() * hill.radius_x * jitter,
            0.0,
            angle.sin() * hill.radius_z * jitter,
        );
        let rotated = rotate_vec3_y(local, hill.rotation_y);
        ring.push(hill.center + vec3(rotated.x * hill.x_stretch, 0.0, rotated.z));
    }

    ring
}

fn build_lit_faceted_hill_mesh(hill: &ArchipelagoHill, light_dir: Vec3, view_dir: Vec3) -> Mesh {
    let ring = hill_base_ring(hill);
    let side_count = ring.len();
    let apex = hill.center
        + vec3(
            hill.peak_offset_x * hill.x_stretch,
            hill.height,
            hill.peak_offset_z,
        );

    let mut builder = MeshBuilder::with_capacity(side_count * 3, side_count * 3);
    for i in 0..side_count {
        let p0 = ring[i];
        let p1 = ring[(i + 1) % side_count];
        let mut normal = (p1 - apex).cross(p0 - apex).normalize_or_zero();
        let face_center = (p0 + p1 + apex) / 3.0;
        if normal.dot(face_center - hill.center) < 0.0 {
            normal = -normal;
            let color = shade_triangle_color(hill.color, normal, light_dir, view_dir, 0.24, 0.18);
            builder.push_triangle(apex, p0, p1, color);
        } else {
            let color = shade_triangle_color(hill.color, normal, light_dir, view_dir, 0.24, 0.18);
            builder.push_triangle(apex, p1, p0, color);
        }
    }

    builder.build()
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SplitFlapSlot {
    pub current: char,
    pub previous: char,
    pub progress: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SplitFlapBoard {
    slots: Vec<SplitFlapSlot>,
}

impl SplitFlapBoard {
    pub fn new(initial: &str) -> Self {
        let slots = initial
            .chars()
            .map(|ch| SplitFlapSlot {
                current: ch,
                previous: ch,
                progress: 1.0,
            })
            .collect();
        Self { slots }
    }

    pub fn update(&mut self, text: &str, dt_seconds: f32) {
        let dt = if dt_seconds.is_finite() {
            dt_seconds.max(0.0)
        } else {
            0.0
        };
        let step = dt / FLIP_DURATION_SECONDS;
        let target_chars: Vec<char> = text.chars().collect();
        let shared_len = self.slots.len().min(target_chars.len());

        for (slot, &target_ch) in self.slots[..shared_len]
            .iter_mut()
            .zip(&target_chars[..shared_len])
        {
            if target_ch != slot.current {
                slot.previous = slot.current;
                slot.current = target_ch;
                slot.progress = 0.0;
            } else if slot.progress < 1.0 {
                slot.progress = (slot.progress + step).min(1.0);
            }
        }

        if target_chars.len() < self.slots.len() {
            self.slots.truncate(target_chars.len());
        } else if target_chars.len() > self.slots.len() {
            for &ch in &target_chars[shared_len..] {
                self.slots.push(SplitFlapSlot {
                    current: ch,
                    previous: ch,
                    progress: 1.0,
                });
            }
        }
    }

    pub fn slots(&self) -> &[SplitFlapSlot] {
        &self.slots
    }

    pub fn is_animating(&self) -> bool {
        self.slots.iter().any(|slot| slot.progress < 1.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FlapPose {
    pub angle_rad: f32,
    pub is_upper_falling: bool,
    pub lower_shadow_alpha: f32,
}

fn ease_in_out_cubic(t: f32) -> f32 {
    if t < 0.5 {
        4.0 * t * t * t
    } else {
        1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
    }
}

pub fn compute_flap_pose(slot: &SplitFlapSlot) -> FlapPose {
    let clamped = if slot.progress.is_finite() {
        slot.progress.clamp(0.0, 1.0)
    } else {
        1.0
    };
    let eased = ease_in_out_cubic(clamped);
    let angle_rad = eased * PI;
    let is_upper_falling = eased < 0.5;

    let lower_shadow_alpha = if clamped <= 0.0 || clamped >= 1.0 {
        0.0
    } else {
        MAX_LOWER_SHADOW_ALPHA * angle_rad.sin().max(0.0)
    };

    FlapPose {
        angle_rad,
        is_upper_falling,
        lower_shadow_alpha,
    }
}

pub fn digit_segments(ch: char) -> [bool; 7] {
    match ch {
        '0' => [true, true, true, false, true, true, true],
        '1' => [false, false, true, false, false, true, false],
        '2' => [true, false, true, true, true, false, true],
        '3' => [true, false, true, true, false, true, true],
        '4' => [false, true, true, true, false, true, false],
        '5' => [true, true, false, true, false, true, true],
        '6' => [true, true, false, true, true, true, true],
        '7' => [true, false, true, false, false, true, false],
        '8' => [true, true, true, true, true, true, true],
        '9' => [true, true, true, true, false, true, true],
        '-' => [false, false, false, true, false, false, false],
        _ => [false; 7],
    }
}

pub fn digit_half_segment_boxes(
    ch: char,
    upper: bool,
    half_w: f32,
    half_h: f32,
) -> Vec<(Vec3, Vec3)> {
    let seg_w = half_w * 0.68;
    let bar_t = (half_w * 0.16).min(half_h * 0.11).max(0.012);
    let bar_depth = (half_w * 0.042).clamp(0.0055, 0.0075);
    let top_y = half_h * 0.72;
    let gap_y = (half_h * 0.065).max(0.014);
    let mid_half_h = bar_t * 0.72;
    let vert_top = top_y - bar_t * 0.25;
    let vert_bot = gap_y + bar_t * 0.15;
    let vert_center_y = (vert_top + vert_bot) * 0.5;
    let vert_half_h = ((vert_top - vert_bot) * 0.5).max(0.01);

    if ch == ':' {
        let dot_y = if upper { half_h * 0.40 } else { -half_h * 0.40 };
        let dot_r = bar_t * 1.25;
        return vec![(vec3(0.0, dot_y, 0.0), vec3(dot_r, dot_r, bar_depth * 1.15))];
    }

    let segs = digit_segments(ch);
    let mut boxes = Vec::with_capacity(4);

    if upper {
        if segs[0] {
            boxes.push((vec3(0.0, top_y, 0.0), vec3(seg_w, bar_t, bar_depth)));
        }
        if segs[1] {
            boxes.push((
                vec3(-seg_w + bar_t, vert_center_y, 0.0),
                vec3(bar_t, vert_half_h, bar_depth),
            ));
        }
        if segs[2] {
            boxes.push((
                vec3(seg_w - bar_t, vert_center_y, 0.0),
                vec3(bar_t, vert_half_h, bar_depth),
            ));
        }
        if segs[3] {
            boxes.push((
                vec3(0.0, gap_y + mid_half_h, 0.0),
                vec3(seg_w - bar_t * 0.35, mid_half_h, bar_depth),
            ));
        }
    } else {
        if segs[3] {
            boxes.push((
                vec3(0.0, -(gap_y + mid_half_h), 0.0),
                vec3(seg_w - bar_t * 0.35, mid_half_h, bar_depth),
            ));
        }
        if segs[4] {
            boxes.push((
                vec3(-seg_w + bar_t, -vert_center_y, 0.0),
                vec3(bar_t, vert_half_h, bar_depth),
            ));
        }
        if segs[5] {
            boxes.push((
                vec3(seg_w - bar_t, -vert_center_y, 0.0),
                vec3(bar_t, vert_half_h, bar_depth),
            ));
        }
        if segs[6] {
            boxes.push((vec3(0.0, -top_y, 0.0), vec3(seg_w, bar_t, bar_depth)));
        }
    }

    boxes
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SplitFlapLayout {
    pub slot_pitch_x: f32,
    pub tile_half_w: f32,
    pub tile_half_h: f32,
    pub start_x: f32,
    pub wall_t: f32,
}

pub fn split_flap_layout(rect_w: f32, rect_h: f32, slot_count: usize) -> SplitFlapLayout {
    let safe_w = if rect_w.is_finite() {
        rect_w.max(1.0)
    } else {
        1.0
    };
    let safe_h = if rect_h.is_finite() {
        rect_h.max(1.0)
    } else {
        1.0
    };
    let aspect = (safe_w / safe_h).max(0.1);
    let visible_half_h = 0.68;
    let visible_half_w = visible_half_h * aspect;
    let tile_half_h = 0.42;
    let count = slot_count.max(1) as f32;
    let slot_pitch_x = (2.0 * visible_half_w * 0.75) / count;
    let tile_half_w = (slot_pitch_x * 0.41).min(tile_half_h * 0.76);
    let start_x = -((count - 1.0) * 0.5) * slot_pitch_x;
    let wall_t = (tile_half_w * 0.13).clamp(0.022, 0.044);

    SplitFlapLayout {
        slot_pitch_x,
        tile_half_w,
        tile_half_h,
        start_x,
        wall_t,
    }
}

pub fn split_flap_camera(rect_w: f32, rect_h: f32, orbit_time_s: f32) -> Camera3D {
    let safe_w = if rect_w.is_finite() {
        rect_w.max(1.0)
    } else {
        1.0
    };
    let safe_h = if rect_h.is_finite() {
        rect_h.max(1.0)
    } else {
        1.0
    };
    let aspect = (safe_w / safe_h).max(0.1);
    let fovy = 30.0_f32.to_radians();
    let visible_half_h = 0.68;
    let cam_z = visible_half_h / (fovy * 0.5).tan();
    let visible_half_w = visible_half_h * aspect;
    let safe_time = if orbit_time_s.is_finite() {
        orbit_time_s
    } else {
        0.0
    };
    let sway_x = (safe_time * 0.45).sin() * (visible_half_w * 0.08).clamp(0.12, 0.24);

    Camera3D {
        position: vec3(sway_x, 0.44, cam_z),
        target: vec3(0.0, -0.02, 0.0),
        up: Vec3::Y,
        fovy,
        aspect: Some(aspect),
        projection: Projection::Perspective,
        ..Default::default()
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_static_digit_half_3d(
    cx: f32,
    card_front_z: f32,
    ch: char,
    upper: bool,
    half_w: f32,
    half_h: f32,
    color: Color,
    light_dir: Vec3,
    view_dir: Vec3,
) {
    for (seg_offset, seg_half) in digit_half_segment_boxes(ch, upper, half_w, half_h) {
        let center = vec3(cx + seg_offset.x, seg_offset.y, card_front_z + seg_half.z);
        let mesh = build_lit_box_mesh_styled(
            center,
            seg_half,
            0.0,
            Vec3::ZERO,
            color,
            light_dir,
            view_dir,
            0.62,
            0.42,
        );
        draw_mesh(&mesh);
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_tile_mechanical_3d(
    cx: f32,
    layout: &SplitFlapLayout,
    slot: &SplitFlapSlot,
    pose: &FlapPose,
    light_dir: Vec3,
    view_dir: Vec3,
) {
    let half_w = layout.tile_half_w;
    let half_h = layout.tile_half_h;
    let wall_t = layout.wall_t;
    let is_animating = slot.progress < 1.0;
    let outer_w = half_w + wall_t;
    let outer_h = half_h + wall_t;
    let split_gap = 0.015;

    let casing_color = Color::new(0.16, 0.21, 0.30, 1.0);
    let rim_color = Color::new(0.24, 0.31, 0.42, 1.0);
    let card_upper_color = Color::new(0.11, 0.15, 0.22, 1.0);
    let card_lower_color = Color::new(0.095, 0.13, 0.195, 1.0);
    let hinge_axle_color = Color::new(0.08, 0.10, 0.14, 1.0);
    let hinge_pin_color = Color::new(0.62, 0.69, 0.80, 1.0);
    let digit_color = Color::new(0.96, 0.98, 1.0, 1.0);
    let colon_color = Color::new(0.98, 0.76, 0.36, 1.0);

    let active_glyph_color = |ch: char| {
        if ch == ':' { colon_color } else { digit_color }
    };

    let back_housing = build_lit_box_mesh(
        vec3(cx, 0.0, -0.12),
        vec3(outer_w, outer_h, 0.11),
        casing_color,
        light_dir,
        view_dir,
    );
    draw_mesh(&back_housing);

    let rim_z_center = 0.016;
    let rim_half_z = 0.036;
    let top_rim = build_lit_box_mesh(
        vec3(cx, half_h + wall_t * 0.5, rim_z_center),
        vec3(outer_w, wall_t * 0.5, rim_half_z),
        rim_color,
        light_dir,
        view_dir,
    );
    let bottom_rim = build_lit_box_mesh(
        vec3(cx, -(half_h + wall_t * 0.5), rim_z_center),
        vec3(outer_w, wall_t * 0.5, rim_half_z),
        rim_color,
        light_dir,
        view_dir,
    );
    let left_rim = build_lit_box_mesh(
        vec3(cx - (half_w + wall_t * 0.5), 0.0, rim_z_center),
        vec3(wall_t * 0.5, half_h, rim_half_z),
        rim_color,
        light_dir,
        view_dir,
    );
    let right_rim = build_lit_box_mesh(
        vec3(cx + (half_w + wall_t * 0.5), 0.0, rim_z_center),
        vec3(wall_t * 0.5, half_h, rim_half_z),
        rim_color,
        light_dir,
        view_dir,
    );
    draw_mesh(&top_rim);
    draw_mesh(&bottom_rim);
    draw_mesh(&left_rim);
    draw_mesh(&right_rim);

    let card_half_h = (half_h - split_gap) * 0.5;
    let card_center_y = split_gap + card_half_h;
    let static_card_z = -0.009;
    let static_card_half_z = 0.005;
    let static_front_z = static_card_z + static_card_half_z;

    let upper_card = build_lit_box_mesh(
        vec3(cx, card_center_y, static_card_z),
        vec3(half_w * 0.96, card_half_h, static_card_half_z),
        card_upper_color,
        light_dir,
        view_dir,
    );
    draw_mesh(&upper_card);
    draw_static_digit_half_3d(
        cx,
        static_front_z,
        slot.current,
        true,
        half_w,
        half_h,
        active_glyph_color(slot.current),
        light_dir,
        view_dir,
    );

    let lower_shade = if is_animating {
        (1.0 - pose.lower_shadow_alpha).clamp(0.30, 1.0)
    } else {
        1.0
    };
    let lower_ch = if is_animating {
        slot.previous
    } else {
        slot.current
    };
    let lower_card = build_lit_box_mesh(
        vec3(cx, -card_center_y, static_card_z),
        vec3(half_w * 0.96, card_half_h, static_card_half_z),
        shade_color(card_lower_color, lower_shade),
        light_dir,
        view_dir,
    );
    draw_mesh(&lower_card);
    draw_static_digit_half_3d(
        cx,
        static_front_z,
        lower_ch,
        false,
        half_w,
        half_h,
        shade_color(active_glyph_color(lower_ch), lower_shade),
        light_dir,
        view_dir,
    );

    let hinge_z = 0.018;
    let hinge_axle = build_lit_box_mesh(
        vec3(cx, 0.0, hinge_z),
        vec3(half_w * 0.98, split_gap * 0.85, 0.012),
        hinge_axle_color,
        light_dir,
        view_dir,
    );
    draw_mesh(&hinge_axle);

    let pin_half_w = (half_w * 0.08).clamp(0.016, 0.032);
    let pin_half_h = split_gap * 1.45;
    let left_pin = build_lit_box_mesh(
        vec3(cx - half_w + pin_half_w, 0.0, hinge_z + 0.006),
        vec3(pin_half_w, pin_half_h, 0.018),
        hinge_pin_color,
        light_dir,
        view_dir,
    );
    let right_pin = build_lit_box_mesh(
        vec3(cx + half_w - pin_half_w, 0.0, hinge_z + 0.006),
        vec3(pin_half_w, pin_half_h, 0.018),
        hinge_pin_color,
        light_dir,
        view_dir,
    );
    draw_mesh(&left_pin);
    draw_mesh(&right_pin);

    if is_animating {
        let flap_half_z = 0.0065;
        let flap_origin = vec3(cx, 0.0, hinge_z);
        let flap_base = if pose.is_upper_falling {
            card_upper_color
        } else {
            card_lower_color
        };
        let rotating_flap = build_lit_transformed_box_mesh(
            flap_origin,
            vec3(half_w * 0.96, card_half_h, flap_half_z),
            pose.angle_rad,
            vec3(0.0, card_center_y, 0.0),
            flap_base,
            light_dir,
            view_dir,
        );
        draw_mesh(&rotating_flap);

        let front_color = active_glyph_color(slot.previous);
        for (seg_offset, seg_half) in digit_half_segment_boxes(slot.previous, true, half_w, half_h)
        {
            let bar = build_lit_box_mesh_styled(
                flap_origin,
                seg_half,
                pose.angle_rad,
                vec3(seg_offset.x, seg_offset.y, flap_half_z + seg_half.z),
                front_color,
                light_dir,
                view_dir,
                0.58,
                0.46,
            );
            draw_mesh(&bar);
        }

        let back_color = active_glyph_color(slot.current);
        for (seg_offset, seg_half) in digit_half_segment_boxes(slot.current, false, half_w, half_h)
        {
            let bar = build_lit_box_mesh_styled(
                flap_origin,
                seg_half,
                pose.angle_rad,
                vec3(seg_offset.x, -seg_offset.y, -(flap_half_z + seg_half.z)),
                back_color,
                light_dir,
                view_dir,
                0.58,
                0.46,
            );
            draw_mesh(&bar);
        }
    }
}

pub fn render_split_flap_clock_3d_at_time(
    board: &SplitFlapBoard,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    orbit_time_s: f32,
) {
    let screen_w = screen_width();
    let screen_h = screen_height();
    let Some((rect_x, rect_y, rect_w, rect_h)) =
        clamp_logical_rect(x, y, width, height, screen_w, screen_h)
    else {
        return;
    };
    let Some(viewport) =
        viewport_rect_to_gl(rect_x, rect_y, rect_w, rect_h, screen_h, screen_dpi_scale())
    else {
        return;
    };

    let slots = board.slots();
    if slots.is_empty() {
        return;
    }

    let layout = split_flap_layout(rect_w, rect_h, slots.len());
    let mut camera = split_flap_camera(rect_w, rect_h, orbit_time_s);
    camera.viewport = Some(viewport);

    let light_dir = vec3(-0.38, 0.78, 0.56).normalize();
    let poses: Vec<FlapPose> = slots.iter().map(compute_flap_pose).collect();

    set_camera(&camera);

    let rail_half_w =
        ((slots.len() as f32 - 1.0) * 0.5 * layout.slot_pitch_x) + layout.tile_half_w + 0.04;
    let rail_view_dir = (camera.position - vec3(0.0, 0.0, -0.25)).normalize_or_zero();
    let back_rail = build_lit_box_mesh(
        vec3(0.0, 0.0, -0.25),
        vec3(rail_half_w, layout.tile_half_h * 0.78, 0.04),
        Color::new(0.09, 0.12, 0.18, 1.0),
        light_dir,
        rail_view_dir,
    );
    draw_mesh(&back_rail);

    for (idx, (slot, pose)) in slots.iter().zip(&poses).enumerate() {
        let cx = layout.start_x + (idx as f32) * layout.slot_pitch_x;
        let view_dir = (camera.position - vec3(cx, 0.0, 0.0)).normalize_or_zero();
        draw_tile_mechanical_3d(cx, &layout, slot, pose, light_dir, view_dir);
    }

    set_default_camera();
}

pub fn render_split_flap_clock_3d(board: &SplitFlapBoard, x: f32, y: f32, width: f32, height: f32) {
    render_split_flap_clock_3d_at_time(board, x, y, width, height, get_time() as f32);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SolarPhase {
    Night,
    CivilDawn,
    Day,
    GoldenHour,
    CivilDusk,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SolarSceneState {
    pub phase: SolarPhase,
    pub daylight_fraction: f32,
    pub sun_altitude_norm: f32,
    pub sun_position: [f32; 3],
    pub arc_points: Vec<[f32; 3]>,
    pub sun_color: Color,
    pub horizon_glow: Color,
    pub sky_tint: Color,
}

fn offset_datetime_to_jiff(now_utc: OffsetDateTime, fallback: Timestamp) -> Timestamp {
    let nanos = now_utc.unix_timestamp_nanos();
    let secs = nanos.div_euclid(1_000_000_000) as i64;
    let subsec = nanos.rem_euclid(1_000_000_000) as i32;
    Timestamp::new(secs, subsec).unwrap_or(fallback)
}

fn timestamp_seconds_f64(ts: Timestamp) -> f64 {
    ts.as_second() as f64 + f64::from(ts.subsec_nanosecond()) / 1e9
}

fn resolve_civil_boundary(
    civil_dt: jiff::civil::DateTime,
    anchor_civil: jiff::civil::DateTime,
    anchor_utc: Timestamp,
    time_zone: &str,
) -> Timestamp {
    if let Ok(zoned) = civil_dt.in_tz(time_zone) {
        return zoned.timestamp();
    }
    let duration = civil_dt.duration_since(anchor_civil);
    if let Ok(shifted) = anchor_utc.checked_add(duration) {
        return shifted;
    }
    anchor_utc
}

fn daytime_arc_point(t: f32) -> [f32; 3] {
    let theta = (t - 0.5) * PI;
    let x = SOLAR_ARC_HALF_SPAN_X * theta.sin();
    let elevation = theta.cos().max(0.0);
    let y = SOLAR_ARC_PEAK_Y * elevation;
    let z = SOLAR_ARC_BASE_Z + SOLAR_ARC_CURVE_Z * elevation;
    [x, y, z]
}

fn phase_palette(phase: SolarPhase) -> (Color, Color, Color) {
    match phase {
        SolarPhase::Night => (
            Color::new(0.36, 0.45, 0.62, 1.0),
            Color::new(0.11, 0.16, 0.26, 0.75),
            palette::INK,
        ),
        SolarPhase::CivilDawn => (
            Color::new(0.95, 0.56, 0.38, 1.0),
            Color::new(0.68, 0.36, 0.34, 0.85),
            Color::new(0.12, 0.15, 0.27, 1.0),
        ),
        SolarPhase::GoldenHour => (
            Color::new(0.98, 0.70, 0.28, 1.0),
            Color::new(palette::STALE.r, 0.55, 0.22, 0.90),
            Color::new(0.16, 0.18, 0.28, 1.0),
        ),
        SolarPhase::Day => (
            Color::new(1.0, 0.92, 0.66, 1.0),
            Color::new(0.38, 0.62, 0.84, 0.80),
            Color::new(0.12, 0.20, 0.33, 1.0),
        ),
        SolarPhase::CivilDusk => (
            Color::new(0.92, 0.42, 0.30, 1.0),
            Color::new(0.58, 0.28, 0.36, 0.85),
            Color::new(0.10, 0.12, 0.23, 1.0),
        ),
    }
}

pub fn compute_solar_scene(
    window: &DaylightWindow,
    now_utc: OffsetDateTime,
    time_zone: &str,
) -> SolarSceneState {
    let now_ts = offset_datetime_to_jiff(now_utc, window.sunrise_utc);
    let civil_dawn_ts = resolve_civil_boundary(
        window.civil_dawn,
        window.sunrise,
        window.sunrise_utc,
        time_zone,
    );
    let civil_dusk_ts = resolve_civil_boundary(
        window.civil_dusk,
        window.sunset,
        window.sunset_utc,
        time_zone,
    );

    let now_s = timestamp_seconds_f64(now_ts);
    let dawn_s = timestamp_seconds_f64(civil_dawn_ts);
    let sunrise_s = timestamp_seconds_f64(window.sunrise_utc);
    let sunset_s = timestamp_seconds_f64(window.sunset_utc);
    let dusk_s = timestamp_seconds_f64(civil_dusk_ts);

    let day_len = (sunset_s - sunrise_s).max(1.0);
    let daylight_fraction = ((now_s - sunrise_s) / day_len).clamp(0.0, 1.0) as f32;

    let phase = if now_s < dawn_s {
        SolarPhase::Night
    } else if now_s < sunrise_s {
        SolarPhase::CivilDawn
    } else if now_s <= sunset_s {
        if daylight_fraction <= GOLDEN_HOUR_FRACTION
            || daylight_fraction >= 1.0 - GOLDEN_HOUR_FRACTION
        {
            SolarPhase::GoldenHour
        } else {
            SolarPhase::Day
        }
    } else if now_s <= dusk_s {
        SolarPhase::CivilDusk
    } else {
        SolarPhase::Night
    };

    let (sun_altitude_norm, sun_position) = if now_s >= sunrise_s && now_s <= sunset_s {
        let theta = (daylight_fraction - 0.5) * PI;
        let altitude_norm = theta.cos().max(0.0);
        (altitude_norm, daytime_arc_point(daylight_fraction))
    } else if now_s >= dawn_s && now_s < sunrise_s {
        let twilight_len = (sunrise_s - dawn_s).max(1.0);
        let depth = ((sunrise_s - now_s) / twilight_len).clamp(0.0, 1.0) as f32;
        let altitude_norm = -0.22 * depth;
        (
            altitude_norm,
            [
                -SOLAR_ARC_HALF_SPAN_X,
                SOLAR_ARC_PEAK_Y * altitude_norm,
                SOLAR_ARC_BASE_Z,
            ],
        )
    } else if now_s > sunset_s && now_s <= dusk_s {
        let twilight_len = (dusk_s - sunset_s).max(1.0);
        let depth = ((now_s - sunset_s) / twilight_len).clamp(0.0, 1.0) as f32;
        let altitude_norm = -0.22 * depth;
        (
            altitude_norm,
            [
                SOLAR_ARC_HALF_SPAN_X,
                SOLAR_ARC_PEAK_Y * altitude_norm,
                SOLAR_ARC_BASE_Z,
            ],
        )
    } else if now_s < dawn_s {
        let night_depth = ((dawn_s - now_s) / 21_600.0).clamp(0.0, 1.0) as f32;
        let altitude_norm = -0.22 - 0.33 * night_depth;
        (
            altitude_norm,
            [
                -SOLAR_ARC_HALF_SPAN_X * (1.0 - night_depth),
                SOLAR_ARC_PEAK_Y * altitude_norm,
                SOLAR_ARC_BASE_Z - 0.25 * night_depth,
            ],
        )
    } else {
        let night_depth = ((now_s - dusk_s) / 21_600.0).clamp(0.0, 1.0) as f32;
        let altitude_norm = -0.22 - 0.33 * night_depth;
        (
            altitude_norm,
            [
                SOLAR_ARC_HALF_SPAN_X * (1.0 - night_depth),
                SOLAR_ARC_PEAK_Y * altitude_norm,
                SOLAR_ARC_BASE_Z - 0.25 * night_depth,
            ],
        )
    };

    let arc_points: Vec<[f32; 3]> = (0..SOLAR_ARC_SAMPLES)
        .map(|i| daytime_arc_point(i as f32 / (SOLAR_ARC_SAMPLES - 1) as f32))
        .collect();

    let (sun_color, horizon_glow, sky_tint) = phase_palette(phase);

    SolarSceneState {
        phase,
        daylight_fraction,
        sun_altitude_norm,
        sun_position,
        arc_points,
        sun_color,
        horizon_glow,
        sky_tint,
    }
}

pub fn solar_horizontal_scale(viewport_width: f32, viewport_height: f32) -> f32 {
    if !viewport_width.is_finite()
        || !viewport_height.is_finite()
        || viewport_width <= 0.0
        || viewport_height <= 0.0
    {
        return 1.0;
    }
    let aspect = (viewport_width / viewport_height).max(1.0);
    (aspect * 0.50).clamp(1.0, 10.0)
}

pub fn is_solar_arc_segment_lit(scene: &SolarSceneState, segment_fraction: f32) -> bool {
    scene.daylight_fraction > 0.0
        && matches!(scene.phase, SolarPhase::Day | SolarPhase::GoldenHour)
        && segment_fraction <= scene.daylight_fraction
}

pub fn diorama_cloud_count(precip_probability_pct: u8) -> usize {
    if precip_probability_pct < 10 {
        0
    } else if precip_probability_pct < 40 {
        2
    } else {
        3
    }
}

pub fn diorama_rain_streak_count(precip_probability_pct: u8) -> usize {
    if precip_probability_pct < 20 {
        0
    } else {
        let clamped = precip_probability_pct.min(100) as usize;
        4 + ((clamped - 20) * 14) / 80
    }
}

fn diorama_point_scaled(raw: [f32; 3], x_scale: f32) -> Vec3 {
    vec3(raw[0] * x_scale, raw[1], raw[2] + DIORAMA_Z_SHIFT)
}

pub fn compute_diorama_light_dir(scene: &SolarSceneState) -> Vec3 {
    let sun = diorama_point_scaled(scene.sun_position, 1.0);
    if scene.sun_altitude_norm > 0.05 {
        vec3(sun.x, sun.y.max(0.45), sun.z + 0.65).normalize()
    } else {
        match scene.phase {
            SolarPhase::CivilDawn => vec3(-0.72, 0.56, 0.40).normalize(),
            SolarPhase::CivilDusk => vec3(0.72, 0.56, 0.40).normalize(),
            _ => vec3(0.32, 0.80, 0.50).normalize(),
        }
    }
}

pub fn diorama_camera(rect_w: f32, rect_h: f32, orbit_time_s: f32) -> Camera3D {
    let safe_w = if rect_w.is_finite() {
        rect_w.max(1.0)
    } else {
        1.0
    };
    let safe_h = if rect_h.is_finite() {
        rect_h.max(1.0)
    } else {
        1.0
    };
    let aspect = (safe_w / safe_h).max(0.1);
    let x_scale = solar_horizontal_scale(safe_w, safe_h);
    let safe_time = if orbit_time_s.is_finite() {
        orbit_time_s
    } else {
        0.0
    };
    let max_azimuth = (0.22 / x_scale).clamp(0.035, 0.22);
    let azimuth = (safe_time * 0.25).sin() * max_azimuth;
    let distance_scale = (1.85 / aspect).max(1.0);
    let distance = 8.8 * distance_scale;
    let elevation_y = 2.30 * distance_scale.sqrt();
    let target = vec3(0.0, 0.74, 0.0);
    let cam_pos = vec3(
        distance * azimuth.sin(),
        elevation_y,
        distance * azimuth.cos(),
    );

    Camera3D {
        position: cam_pos,
        target,
        up: Vec3::Y,
        fovy: 31.0_f32.to_radians(),
        aspect: Some(aspect),
        projection: Projection::Perspective,
        ..Default::default()
    }
}

pub fn diorama_cloud_centers(
    precip_probability_pct: u8,
    x_scale: f32,
    orbit_time_s: f32,
) -> Vec<Vec3> {
    let cloud_count = diorama_cloud_count(precip_probability_pct);
    if cloud_count == 0 {
        return Vec::new();
    }

    let safe_scale = if x_scale.is_finite() {
        x_scale.max(1.0)
    } else {
        1.0
    };
    let safe_time = if orbit_time_s.is_finite() {
        orbit_time_s
    } else {
        0.0
    };
    let pedestal_rx = DIORAMA_PEDESTAL_BASE_RX * safe_scale;

    let cloud_specs: [(f32, f32, f32, f32); 3] = [
        (-1.45, 1.48, -1.18, 0.22),
        (1.15, 1.38, 1.28, 0.16),
        (-0.15, 1.66, -1.40, 0.26),
    ];

    cloud_specs
        .iter()
        .take(cloud_count)
        .map(|&(base_x, base_y, base_z, speed)| {
            let z_extreme = (base_z.abs() + CLOUD_SHADOW_RADIUS).min(DIORAMA_PEDESTAL_RZ * 0.99);
            let max_edge_x = pedestal_rx
                * (1.0 - (z_extreme / DIORAMA_PEDESTAL_RZ).powi(2))
                    .max(0.0)
                    .sqrt();
            let half_span = (max_edge_x - CLOUD_SHADOW_RADIUS - 0.05)
                .min(2.4 * safe_scale)
                .max(0.05);
            let span = half_span * 2.0;
            let drift = ((base_x * safe_scale + safe_time * speed * safe_scale + half_span)
                .rem_euclid(span))
                - half_span;
            let bob_y = base_y + (safe_time * 0.9 + base_z * 2.0).sin() * 0.04;
            vec3(drift, bob_y, base_z)
        })
        .collect()
}

fn archipelago_hills(x_scale: f32, hill_base: Color, ridge_base: Color) -> [ArchipelagoHill; 8] {
    let safe_scale = if x_scale.is_finite() {
        x_scale.max(1.0)
    } else {
        1.0
    };
    let x_stretch = (0.65 + 0.35 * safe_scale).min(2.6);

    [
        ArchipelagoHill {
            center: vec3(-1.35 * safe_scale, 0.0, -0.78),
            radius_x: 0.72,
            radius_z: 0.52,
            x_stretch,
            height: 0.68,
            peak_offset_x: 0.14,
            peak_offset_z: -0.06,
            sides: 6,
            rotation_y: 0.3,
            color: ridge_base,
        },
        ArchipelagoHill {
            center: vec3(0.15 * safe_scale, 0.0, -0.92),
            radius_x: 0.86,
            radius_z: 0.60,
            x_stretch,
            height: 0.82,
            peak_offset_x: -0.12,
            peak_offset_z: 0.08,
            sides: 6,
            rotation_y: 1.1,
            color: hill_base,
        },
        ArchipelagoHill {
            center: vec3(1.45 * safe_scale, 0.0, -0.75),
            radius_x: 0.72,
            radius_z: 0.52,
            x_stretch,
            height: 0.62,
            peak_offset_x: -0.10,
            peak_offset_z: -0.05,
            sides: 5,
            rotation_y: 2.0,
            color: ridge_base,
        },
        ArchipelagoHill {
            center: vec3(-2.05 * safe_scale, 0.0, 0.32),
            radius_x: 0.62,
            radius_z: 0.45,
            x_stretch,
            height: 0.42,
            peak_offset_x: 0.08,
            peak_offset_z: 0.04,
            sides: 5,
            rotation_y: 0.7,
            color: hill_base,
        },
        ArchipelagoHill {
            center: vec3(-0.65 * safe_scale, 0.0, 0.48),
            radius_x: 0.72,
            radius_z: 0.48,
            x_stretch,
            height: 0.48,
            peak_offset_x: -0.09,
            peak_offset_z: -0.04,
            sides: 6,
            rotation_y: 1.6,
            color: ridge_base,
        },
        ArchipelagoHill {
            center: vec3(0.95 * safe_scale, 0.0, 0.60),
            radius_x: 0.68,
            radius_z: 0.48,
            x_stretch,
            height: 0.44,
            peak_offset_x: 0.11,
            peak_offset_z: -0.04,
            sides: 5,
            rotation_y: 2.6,
            color: hill_base,
        },
        ArchipelagoHill {
            center: vec3(2.10 * safe_scale, 0.0, 0.38),
            radius_x: 0.58,
            radius_z: 0.44,
            x_stretch,
            height: 0.36,
            peak_offset_x: -0.06,
            peak_offset_z: 0.03,
            sides: 5,
            rotation_y: 0.4,
            color: ridge_base,
        },
        ArchipelagoHill {
            center: vec3(-0.10 * safe_scale, 0.0, 1.15),
            radius_x: 0.62,
            radius_z: 0.42,
            x_stretch,
            height: 0.30,
            peak_offset_x: 0.06,
            peak_offset_z: -0.03,
            sides: 5,
            rotation_y: 1.9,
            color: hill_base,
        },
    ]
}

pub fn diorama_hill_base_vertices(x_scale: f32) -> Vec<Vec3> {
    let neutral = Color::new(0.2, 0.25, 0.3, 1.0);
    archipelago_hills(x_scale, neutral, neutral)
        .iter()
        .flat_map(hill_base_ring)
        .collect()
}

fn draw_diorama_archipelago_hills(
    scene: &SolarSceneState,
    x_scale: f32,
    light_dir: Vec3,
    view_dir: Vec3,
) {
    let warm_blend = match scene.phase {
        SolarPhase::GoldenHour => 0.34,
        SolarPhase::Day => 0.18,
        SolarPhase::CivilDawn | SolarPhase::CivilDusk => 0.24,
        SolarPhase::Night => 0.0,
    };
    let hill_base = blend_color(
        Color::new(0.16, 0.24, 0.31, 1.0),
        scene.sun_color,
        warm_blend,
    );
    let ridge_base = blend_color(
        Color::new(0.20, 0.28, 0.36, 1.0),
        scene.horizon_glow,
        warm_blend * 0.85,
    );

    for hill in &archipelago_hills(x_scale, hill_base, ridge_base) {
        let mesh = build_lit_faceted_hill_mesh(hill, light_dir, view_dir);
        draw_mesh(&mesh);
    }
}

fn draw_diorama_pylons_and_ribs(
    scene: &SolarSceneState,
    x_scale: f32,
    light_dir: Vec3,
    view_dir: Vec3,
) {
    let pylon_side = Color::new(0.24, 0.30, 0.40, 1.0);
    let pylon_top = Color::new(0.42, 0.50, 0.62, 1.0);
    let east_beacon = Color::new(0.98, 0.72, 0.32, 1.0);
    let west_beacon = Color::new(0.94, 0.46, 0.32, 1.0);
    let rib_color = Color::new(0.26, 0.34, 0.46, 1.0);

    let east_base = scene
        .arc_points
        .first()
        .copied()
        .map(|p| diorama_point_scaled(p, x_scale))
        .unwrap_or_else(|| {
            vec3(
                -SOLAR_ARC_HALF_SPAN_X * x_scale,
                0.0,
                SOLAR_ARC_BASE_Z + DIORAMA_Z_SHIFT,
            )
        });
    let west_base = scene
        .arc_points
        .last()
        .copied()
        .map(|p| diorama_point_scaled(p, x_scale))
        .unwrap_or_else(|| {
            vec3(
                SOLAR_ARC_HALF_SPAN_X * x_scale,
                0.0,
                SOLAR_ARC_BASE_Z + DIORAMA_Z_SHIFT,
            )
        });

    for (pos, beacon_color) in [(east_base, east_beacon), (west_base, west_beacon)] {
        let pylon = build_lit_cylinder_mesh(
            vec3(pos.x, 0.19, pos.z),
            0.12,
            0.38,
            8,
            pylon_top,
            pylon_side,
            light_dir,
            view_dir,
        );
        draw_mesh(&pylon);

        let beacon = build_lit_icosphere_mesh(
            vec3(pos.x, 0.44, pos.z),
            0.10,
            1,
            beacon_color,
            light_dir,
            view_dir,
            0.48,
        );
        draw_mesh(&beacon);
    }

    let ew_rib = build_lit_prism_segment_mesh(
        vec3(east_base.x, 0.022, east_base.z),
        vec3(west_base.x, 0.022, west_base.z),
        0.028,
        blend_color(rib_color, scene.horizon_glow, 0.30),
        light_dir,
        view_dir,
    );
    let ns_rib = build_lit_prism_segment_mesh(
        vec3(0.0, 0.022, -DIORAMA_PEDESTAL_RZ * 0.92),
        vec3(0.0, 0.022, DIORAMA_PEDESTAL_RZ * 0.92),
        0.026,
        rib_color,
        light_dir,
        view_dir,
    );
    draw_mesh(&ew_rib);
    draw_mesh(&ns_rib);
}

fn draw_diorama_sun_and_armillary_opaque(
    scene: &SolarSceneState,
    sun_pos: Vec3,
    orbit_time_s: f32,
    light_dir: Vec3,
    view_dir: Vec3,
) {
    let sun_key_dir = (vec3(0.35, 0.85, 0.65) + view_dir * 0.45).normalize();
    let sun_radius = if sun_pos.y >= 0.0 { 0.36 } else { 0.24 };
    let sun_mesh = build_lit_icosphere_mesh(
        sun_pos,
        sun_radius,
        2,
        scene.sun_color,
        sun_key_dir,
        view_dir,
        0.50,
    );
    draw_mesh(&sun_mesh);

    let ring_segments = 16;
    let ring_radius = sun_radius * 1.56;
    let tilt_x = 0.50;
    let spin_y = orbit_time_s * 0.72;
    let ring_color = blend_color(Color::new(0.95, 0.78, 0.40, 1.0), scene.sun_color, 0.45);

    let mut ring_points = Vec::with_capacity(ring_segments);
    for i in 0..ring_segments {
        let angle = (i as f32 / ring_segments as f32) * TAU;
        let local = vec3(angle.cos() * ring_radius, 0.0, angle.sin() * ring_radius);
        let tilted = rotate_vec3_x(local, tilt_x);
        let spun = rotate_vec3_y(tilted, spin_y);
        ring_points.push(sun_pos + spun);
    }

    for i in 0..ring_segments {
        let p0 = ring_points[i];
        let p1 = ring_points[(i + 1) % ring_segments];
        let seg = build_lit_prism_segment_mesh(p0, p1, 0.028, ring_color, light_dir, view_dir);
        draw_mesh(&seg);
    }

    if sun_pos.y > 0.08 {
        let footprint = build_lit_cylinder_mesh(
            vec3(sun_pos.x, 0.014, sun_pos.z),
            0.24,
            0.020,
            12,
            scene.sun_color,
            scene.horizon_glow,
            light_dir,
            view_dir,
        );
        draw_mesh(&footprint);
    }
}

fn draw_diorama_clouds_opaque(clouds: &[Vec3], light_dir: Vec3, view_dir: Vec3) {
    if clouds.is_empty() {
        return;
    }

    let cloud_color = Color::new(0.76, 0.83, 0.92, 1.0);
    let puff_offsets: [(Vec3, f32); 4] = [
        (vec3(0.0, 0.05, 0.0), 0.30),
        (vec3(-0.28, -0.02, 0.06), 0.23),
        (vec3(0.28, -0.01, -0.04), 0.24),
        (vec3(0.06, 0.12, -0.14), 0.21),
    ];

    for &center in clouds {
        for &(offset, puff_r) in &puff_offsets {
            let puff = build_lit_icosphere_mesh(
                center + offset,
                puff_r,
                1,
                cloud_color,
                light_dir,
                view_dir,
                0.08,
            );
            draw_mesh(&puff);
        }
    }
}

fn draw_diorama_translucent_effects(
    scene: &SolarSceneState,
    sun_pos: Vec3,
    clouds: &[Vec3],
    precip_probability_pct: u8,
    orbit_time_s: f32,
    light_dir: Vec3,
    view_dir: Vec3,
) {
    let shadow_color = Color::new(0.03, 0.05, 0.10, 0.38);
    for &center in clouds {
        let shadow = build_lit_cylinder_mesh(
            vec3(center.x, 0.010, center.z),
            CLOUD_SHADOW_RADIUS,
            0.008,
            12,
            shadow_color,
            shadow_color,
            light_dir,
            view_dir,
        );
        draw_mesh(&shadow);
    }

    if sun_pos.y > 0.08 {
        let sun_radius = 0.36;
        let pillar_top_y = (sun_pos.y - sun_radius * 0.92).max(0.05);
        let pillar_color = Color::new(
            scene.horizon_glow.r,
            scene.horizon_glow.g,
            scene.horizon_glow.b,
            0.72,
        );
        let pillar = build_lit_prism_segment_mesh(
            vec3(sun_pos.x, 0.02, sun_pos.z),
            vec3(sun_pos.x, pillar_top_y, sun_pos.z),
            0.032,
            pillar_color,
            light_dir,
            view_dir,
        );
        draw_mesh(&pillar);
    }

    if clouds.is_empty() {
        return;
    }

    let rain_color = Color::new(0.45, 0.86, 1.0, 0.90);
    let rain_count = diorama_rain_streak_count(precip_probability_pct);
    for idx in 0..rain_count {
        let cloud = clouds[idx % clouds.len()];
        let slot = (idx / clouds.len()) as f32;
        let lane_x = ((slot * 0.37 + 0.19).fract() - 0.5) * 0.72;
        let lane_z = ((slot * 0.61 + 0.31).fract() - 0.5) * 0.42;
        let fall_t = (orbit_time_s * 1.45 + (idx as f32) * 0.23).fract();
        let top_y = (cloud.y - 0.22) * (1.0 - fall_t) + 0.28 * fall_t;
        let streak_len = 0.24;
        let start = vec3(cloud.x + lane_x, top_y, cloud.z + lane_z);
        let end = vec3(start.x + 0.05, (start.y - streak_len).max(0.03), start.z);
        let streak =
            build_lit_prism_segment_mesh(start, end, 0.017, rain_color, light_dir, view_dir);
        draw_mesh(&streak);
    }
}

pub fn render_solar_diorama_3d(
    scene: &SolarSceneState,
    precip_probability_pct: u8,
    orbit_time_s: f32,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
) {
    let screen_w = screen_width();
    let screen_h = screen_height();
    let Some((rect_x, rect_y, rect_w, rect_h)) =
        clamp_logical_rect(x, y, width, height, screen_w, screen_h)
    else {
        return;
    };
    let Some(viewport) =
        viewport_rect_to_gl(rect_x, rect_y, rect_w, rect_h, screen_h, screen_dpi_scale())
    else {
        return;
    };

    draw_rectangle(rect_x, rect_y, rect_w, rect_h, scene.sky_tint);
    let horizon_band_h = rect_h * 0.38;
    let horizon_band_color = Color::new(
        scene.horizon_glow.r,
        scene.horizon_glow.g,
        scene.horizon_glow.b,
        0.18,
    );
    draw_rectangle(
        rect_x,
        rect_y + rect_h - horizon_band_h,
        rect_w,
        horizon_band_h,
        horizon_band_color,
    );

    let safe_time = if orbit_time_s.is_finite() {
        orbit_time_s
    } else {
        0.0
    };
    let x_scale = solar_horizontal_scale(rect_w, rect_h);
    let mut camera = diorama_camera(rect_w, rect_h, safe_time);
    camera.viewport = Some(viewport);

    let light_dir = compute_diorama_light_dir(scene);
    let view_dir = (camera.position - camera.target).normalize_or_zero();

    set_camera(&camera);

    let water_top = blend_color(
        Color::new(0.14, 0.24, 0.38, 1.0),
        scene.horizon_glow,
        match scene.phase {
            SolarPhase::GoldenHour => 0.24,
            SolarPhase::CivilDawn | SolarPhase::CivilDusk => 0.18,
            SolarPhase::Day => 0.14,
            SolarPhase::Night => 0.04,
        },
    );
    let rim_side = Color::new(0.11, 0.15, 0.23, 1.0);
    let pedestal = build_lit_elliptical_cylinder_mesh(
        vec3(0.0, -0.21, 0.0),
        DIORAMA_PEDESTAL_BASE_RX * x_scale,
        DIORAMA_PEDESTAL_RZ,
        0.42,
        36,
        water_top,
        rim_side,
        light_dir,
        view_dir,
    );
    draw_mesh(&pedestal);

    let base_plinth = build_lit_elliptical_cylinder_mesh(
        vec3(0.0, -0.40, 0.0),
        DIORAMA_PLINTH_BASE_RX * x_scale,
        DIORAMA_PLINTH_RZ,
        0.12,
        36,
        Color::new(0.18, 0.24, 0.34, 1.0),
        Color::new(0.07, 0.10, 0.16, 1.0),
        light_dir,
        view_dir,
    );
    draw_mesh(&base_plinth);

    draw_diorama_archipelago_hills(scene, x_scale, light_dir, view_dir);
    draw_diorama_pylons_and_ribs(scene, x_scale, light_dir, view_dir);

    let total_segments = scene.arc_points.len().saturating_sub(1).max(1) as f32;
    let lit_arc_color = blend_color(scene.horizon_glow, Color::new(1.0, 0.80, 0.34, 1.0), 0.45);
    let remaining_arc_color = Color::new(0.28, 0.38, 0.54, 1.0);

    for (idx, pair) in scene.arc_points.windows(2).enumerate() {
        let seg_fraction = (idx as f32 + 0.5) / total_segments;
        let seg_color = if is_solar_arc_segment_lit(scene, seg_fraction) {
            lit_arc_color
        } else {
            remaining_arc_color
        };
        let seg = build_lit_prism_segment_mesh(
            diorama_point_scaled(pair[0], x_scale),
            diorama_point_scaled(pair[1], x_scale),
            0.055,
            seg_color,
            light_dir,
            view_dir,
        );
        draw_mesh(&seg);
    }

    let sun_pos = diorama_point_scaled(scene.sun_position, x_scale);
    let clouds = diorama_cloud_centers(precip_probability_pct, x_scale, safe_time);
    draw_diorama_sun_and_armillary_opaque(scene, sun_pos, safe_time, light_dir, view_dir);
    draw_diorama_clouds_opaque(&clouds, light_dir, view_dir);
    draw_diorama_translucent_effects(
        scene,
        sun_pos,
        &clouds,
        precip_probability_pct,
        safe_time,
        light_dir,
        view_dir,
    );

    set_default_camera();
}
