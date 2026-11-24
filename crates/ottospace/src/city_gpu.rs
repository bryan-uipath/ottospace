//! The code city's picture: the core's scene (`ottospace_core::scene`) rendered offscreen on
//! the GPU (wgpu, whichever backend the platform has), with a depth buffer, 4× multisampling, a
//! light from above, edge lines and fog into the background, then read back as BGRA for GPUI to
//! paint. Run
//! on a thread of its own; a still picture, made again only when the scene or its size changes.

use ottospace_core::scene::{Block, Kind, Scene};
use ottospace_core::theme;

/// The picture, `width` × `height` pixels, BGRA, rows packed.
pub struct Picture {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
    /// Microseconds the GPU took (drawn and copied to the read buffer, waited on), then the copy
    /// out of it into `bgra`.
    pub gpu_us: u64,
    pub copy_us: u64,
    /// Before the GPU: its boxes, buffers and textures made.
    pub prep_us: u64,
    /// Of it: the boxes' instances ordered and placed, and textures and buffers made.
    pub boxes_us: u64,
    pub alloc_us: u64,
}

const SAMPLES: u32 = 4;
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Bgra8Unorm;
const DEPTH: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
/// Per box: its corners (min, max), its top's colour, 1 if it glows (lit), its edges' colour,
/// when it starts to rise (`Scene::rise_delays`; below 0: it stands from the start), how bright
/// its sides' edges are, a share of its top's, and how opaque its faces are.
const STRIDE: u64 = 16 * 4;
/// How opaque a glass building's faces are (the original city's are 0.92; the owner asked for a
/// little more see-through): its edges are whole.
const GLASS: f32 = 0.8;

const SHADER: &str = r#"
struct Frame { matrix: mat4x4<f32>, eye: vec4<f32>, fog: vec4<f32>, background: vec4<f32>, light: vec4<f32> };
@group(0) @binding(0) var<uniform> frame: Frame;
// `scene::STAGGER`, written in at start.
const STAGGER: f32 = {STAGGER};

struct Out {
    @builtin(position) at: vec4<f32>,
    @location(0) colour: vec3<f32>,
    @location(1) world: vec3<f32>,
    @location(2) lit: f32,
    @location(3) @interpolate(flat) lo: vec3<f32>,
    @location(4) @interpolate(flat) hi: vec3<f32>,
    @location(5) @interpolate(flat) edge: vec3<f32>,
    // The axis the face is across: 0 x, 1 y, 2 z.
    @location(6) @interpolate(flat) axis: u32,
    @location(7) @interpolate(flat) sides: f32,
    @location(8) @interpolate(flat) alpha: f32,
};

// A cube's 36 corners (two triangles a face) and each face's normal: -x, +x, -y, +y, -z, +z.
const CORNERS = array<vec3<f32>, 8>(
    vec3(0., 0., 0.), vec3(1., 0., 0.), vec3(0., 1., 0.), vec3(1., 1., 0.),
    vec3(0., 0., 1.), vec3(1., 0., 1.), vec3(0., 1., 1.), vec3(1., 1., 1.),
);
const FACES = array<u32, 36>(
    0u, 4u, 6u, 0u, 6u, 2u,   1u, 3u, 7u, 1u, 7u, 5u,
    0u, 1u, 5u, 0u, 5u, 4u,   2u, 6u, 7u, 2u, 7u, 3u,
    0u, 2u, 3u, 0u, 3u, 1u,   4u, 5u, 7u, 4u, 7u, 6u,
);
const NORMALS = array<vec3<f32>, 6>(
    vec3(-1., 0., 0.), vec3(1., 0., 0.), vec3(0., -1., 0.), vec3(0., 1., 0.), vec3(0., 0., -1.), vec3(0., 0., 1.),
);

@vertex
fn vs(@builtin(vertex_index) i: u32, @location(0) lo: vec3<f32>, @location(1) hi: vec3<f32>, @location(2) top: vec4<f32>, @location(3) edge: vec3<f32>, @location(4) delay: f32, @location(5) sides: f32, @location(6) alpha: f32) -> Out {
    // Rising (`frame.light.w`: the share of the rise, 1 at rest): as tall as `scene::risen` says.
    let at = clamp((frame.light.w - delay) / (1. - STAGGER), 0., 1.);
    let grown = select(1. - (1. - at) * (1. - at) * (1. - at), 1., delay < 0.);
    let top_y = lo.y + (hi.y - lo.y) * grown;
    let corner = CORNERS[FACES[i]];
    let world = vec3(lo.x + (hi.x - lo.x) * corner.x, lo.y + (top_y - lo.y) * corner.y, lo.z + (hi.z - lo.z) * corner.z);
    let normal = NORMALS[i / 6u];
    // Sides darker than the top: ambient, and a light from above to one side.
    let light = 0.3 + 0.7 * max(dot(normal, normalize(frame.light.xyz)), 0.);
    var out: Out;
    out.at = frame.matrix * vec4(world, 1.);
    // A lit building glows: its own colour on every face, a step darker on the sides.
    out.colour = mix(top.rgb * light, top.rgb * (0.75 + 0.25 * normal.y), top.a);
    out.lit = top.a;
    out.world = world;
    out.lo = lo;
    out.hi = vec3(hi.x, top_y, hi.z);
    out.edge = edge;
    out.axis = i / 12u;
    out.sides = sides;
    out.alpha = alpha;
    return out;
}

@fragment
fn fs(in: Out) -> @location(0) vec4<f32> {
    // Edges: on the face's two axes, pixels to its nearest edge and its size in pixels. A face
    // draws two thirds of a device pixel of line (two meeting: a little over one), at any
    // distance and scale; none on a face too small to hold one.
    let step = fwidth(in.world);
    var to_edge = 1e6;
    var size = 1e6;
    for (var k = 0u; k < 3u; k++) {
        if (k != in.axis) {
            let px = max(step[k], 1e-7);
            to_edge = min(to_edge, min(in.world[k] - in.lo[k], in.hi[k] - in.world[k]) / px);
            size = min(size, (in.hi[k] - in.lo[k]) / px);
        }
    }
    let line = (1. - smoothstep(0.25, 1.1, to_edge)) * smoothstep(4., 10., size);
    // A side's edges (any face but the top and bottom) dimmed: a plinth's lit rim.
    let edge = select(in.edge * in.sides, in.edge, in.axis == 1u);
    let colour = mix(in.colour, edge, line);
    // Fog: from nothing at `fog.x` from the eye to `fog.z` at `fog.y`.
    let far = clamp((distance(in.world, frame.eye.xyz) - frame.fog.x) / (frame.fog.y - frame.fog.x), 0., 1.);
    // Glass: the faces see-through, the edges whole.
    return vec4(mix(colour, frame.background.rgb, far * frame.fog.z * (1. - in.lit)), max(in.alpha, line));
}
"#;

fn rgb(hex: u32) -> [f32; 3] {
    let c = gpui::rgb(hex);
    [c.r, c.g, c.b]
}

/// A box's top colour and its edges'. Four tones, quiet behind the hints: the background, the
/// ground (streets), districts (plinths a step lighter per tier, their top edge lit: dimmed on
/// their sides, `instance`) and buildings (edged lighter).
fn colour(kind: Kind) -> ([f32; 3], [f32; 3]) {
    match kind {
        Kind::Ground => (rgb(0x111115), rgb(0x111115)),
        Kind::Plate(tier) => {
            let tier = tier.clamp(1, 5) as u32 - 1;
            (rgb(0x1f2632 + 0x06080a * tier), rgb(0x56cde8))
        }
        Kind::Building => (rgb(0x50505e), rgb(0x9a9aac)),
        Kind::Lit(hex) => (rgb(hex), rgb(0xffffff)),
    }
}

/// The device kept between pictures.
pub struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    /// For glass buildings: blended over what is drawn, writing no depth, drawn last, far first.
    glass: wgpu::RenderPipeline,
    /// The frame's uniform (camera, eye, fog, background, light: 32 floats), bound once.
    uniform: wgpu::Buffer,
    group: wgpu::BindGroup,
    /// Kept from picture to picture: a camera move makes no texture or buffer.
    kept: std::cell::RefCell<Kept>,
}

/// The targets and read buffer of the last picture's size, and the instance buffer, as large as
/// the largest city yet.
#[derive(Default)]
struct Kept {
    targets: Option<Targets>,
    instances: Option<wgpu::Buffer>,
}

struct Targets {
    size: (u32, u32),
    samples: wgpu::TextureView,
    resolved: wgpu::Texture,
    into: wgpu::TextureView,
    depth: wgpu::TextureView,
    read: wgpu::Buffer,
    row: u32,
}

/// A city's boxes as instances, but for where they stand: their colours, rise delays and how
/// opaque, the solid first, then the glass. Made once a layout (`Boxes::new`); a camera move only
/// orders the glass and copies each box's corners.
pub struct Boxes {
    base: Vec<[f32; 16]>,
    /// Indices into the scene's blocks: the solid ones, then the glass.
    solid: Vec<u32>,
    glass: Vec<u32>,
}

impl Boxes {
    pub fn new(scene: &Scene, glass: bool) -> Self {
        let delays = scene.rise_delays();
        let tint = |i: usize| scene.tints.get(i).copied().flatten();
        let see_through = |b: &Block| glass && b.kind == Kind::Building;
        let (mut solid, mut clear) = (Vec::new(), Vec::new());
        let base = (scene.blocks.iter().enumerate())
            .map(|(i, b)| {
                let alpha = if see_through(b) { GLASS } else { 1. };
                if see_through(b) {
                    clear.push(i as u32)
                } else {
                    solid.push(i as u32)
                }
                instance(b, delays[i], tint(i), alpha)
            })
            .collect();
        Self { base, solid, glass: clear }
    }
}

impl Gpu {
    /// The platform's GPU, or why there is none.
    pub fn new() -> Result<Self, String> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
            .map_err(|e| format!("no adapter: {e}"))?;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
            .map_err(|e| format!("no device: {e}"))?;
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("city"),
            source: wgpu::ShaderSource::Wgsl(
                SHADER.replace("{STAGGER}", &format!("{:?}", ottospace_core::scene::STAGGER)).into(),
            ),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let attributes = wgpu::vertex_attr_array![
            0 => Float32x3, 1 => Float32x3, 2 => Float32x4, 3 => Float32x3, 4 => Float32, 5 => Float32, 6 => Float32
        ];
        let pipeline = |glass: bool| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("city"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vs"),
                    compilation_options: Default::default(),
                    buffers: &[Some(wgpu::VertexBufferLayout {
                        array_stride: STRIDE,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &attributes,
                    })],
                },
                primitive: wgpu::PrimitiveState { cull_mode: None, ..Default::default() },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH,
                    depth_write_enabled: Some(!glass),
                    depth_compare: Some(wgpu::CompareFunction::Less),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: wgpu::MultisampleState { count: SAMPLES, ..Default::default() },
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some("fs"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: FORMAT,
                        blend: glass.then_some(wgpu::BlendState::ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let (glass, pipeline) = (pipeline(true), pipeline(false));
        let uniform = buffer(&device, &[0.; 32], wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST);
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() }],
        });
        Ok(Self { device, queue, pipeline, glass, uniform, group, kept: Default::default() })
    }

    /// One draw of a one-box city at 16×16, its picture dropped: the first picture shown pays no
    /// first draw's setup (60 ms in the bench). Its failure is no failure of the GPU.
    pub fn warm(&self) {
        let one = ottospace_core::atlas::atlas(&ottospace_core::atlas::repo(&[("a/b.rs".into(), 900)]));
        let scene = ottospace_core::scene::scene(&one, &[], 1., &ottospace_core::scene::View::default());
        let warmed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.render(&scene, 16, 16, 1., true)));
        drop(warmed);
    }

    /// `scene` as a picture `width` × `height` pixels, `rise` (0 to 1) of the way through its
    /// buildings' rise (1: standing), its buildings of glass or not, or why there is none.
    pub fn render(&self, scene: &Scene, width: u32, height: u32, rise: f32, glass: bool) -> Result<Picture, String> {
        self.render_boxes(scene, &Boxes::new(scene, glass), width, height, rise)
    }

    /// `render`, of `scene`'s boxes as made already (`Boxes::new`): a camera move's picture.
    pub fn render_boxes(
        &self,
        scene: &Scene,
        boxes: &Boxes,
        width: u32,
        height: u32,
        rise: f32,
    ) -> Result<Picture, String> {
        if boxes.base.len() != scene.blocks.len() {
            return Err("boxes of another scene".into());
        }
        // An error of the GPU's (out of memory, a lost device) fails the picture; uncaught, wgpu
        // panics.
        let memory = self.device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let valid = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let picture = self.draw(scene, boxes, width, height, rise);
        let errors = [pollster::block_on(valid.pop()), pollster::block_on(memory.pop())];
        let picture = match errors.into_iter().flatten().next() {
            Some(error) => Err(format!("render: {error}")),
            None => picture,
        };
        // A failed one may leave its read buffer mapped: the next makes its targets again.
        if picture.is_err() {
            self.kept.borrow_mut().targets = None;
        }
        picture
    }

    fn draw(&self, scene: &Scene, boxes: &Boxes, width: u32, height: u32, rise: f32) -> Result<Picture, String> {
        let prepared = std::time::Instant::now();
        let camera = scene.camera.ok_or("no camera")?;
        let limit = self.device.limits().max_texture_dimension_2d;
        if width == 0 || height == 0 || width > limit || height > limit {
            return Err(format!("no picture of {width}×{height}"));
        }
        let device = &self.device;
        let size = wgpu::Extent3d { width, height, depth_or_array_layers: 1 };
        let mut kept = self.kept.borrow_mut();
        let allocating = std::time::Instant::now();
        // Made again only when the size changes (a drag's half size, a resize).
        if kept.targets.as_ref().is_none_or(|t| t.size != (width, height)) {
            kept.targets = Some(targets(device, size));
        }
        let mut alloc_us = allocating.elapsed().as_micros() as u64;

        // The frame: the camera, the eye, the fog (from a bit before the target to far past it,
        // at most this thick) and the light.
        let d: f32 = (0..3).map(|i| (camera.eye[i] - camera.target[i]).powi(2)).sum::<f32>().sqrt();
        let reach = [d * 0.75, d * 1.6, 0.85, 0.];
        let bg = rgb(theme::BACKGROUND);
        let mut frame: Vec<f32> = camera.matrix().iter().flatten().copied().collect();
        frame.extend([camera.eye[0], camera.eye[1], camera.eye[2], 0.]);
        frame.extend(reach);
        frame.extend([bg[0], bg[1], bg[2], 1.]);
        frame.extend([-0.45, 1., -0.3, rise.clamp(0., 1.)]);
        self.queue.write_buffer(&self.uniform, 0, &bytes(&frame));
        // The solid boxes first; then the glass ones, the farthest first, each over what is behind
        // it. Each as made once (`Boxes`), standing where the scene has it (a lit one's line of
        // sight may cut others down).
        let boxing = std::time::Instant::now();
        let far = |i: &u32| {
            let top = scene.blocks[*i as usize].top();
            // Farthest first: a distance's bits order as it does (never negative).
            std::cmp::Reverse((0..3).map(|k| (top[k] - camera.eye[k]).powi(2)).sum::<f32>().to_bits())
        };
        // Only the boxes drawn at this level of detail (`Scene::shown`).
        let shown = |i: &&u32| scene.is_shown(**i as usize);
        let mut clear: Vec<u32> = boxes.glass.iter().filter(shown).copied().collect();
        clear.sort_by_cached_key(far);
        let solid: Vec<u32> = boxes.solid.iter().filter(shown).copied().collect();
        let solid_count = solid.len() as u32;
        let placed = |i: &u32| {
            let (b, mut one) = (&scene.blocks[*i as usize], boxes.base[*i as usize]);
            // A closed box standing for a lit file glows in its colour.
            if let Some(colour) = scene.glows(*i as usize) {
                one = instance(&Block { kind: Kind::Lit(colour), ..*b }, one[13], None, one[15]);
            }
            one[..3].copy_from_slice(&b.min);
            one[3..6].copy_from_slice(&b.max);
            one
        };
        let data: Vec<f32> = solid.iter().chain(&clear).map(placed).flatten().collect();
        let bytes_of = bytes(&data);
        let boxes_us = boxing.elapsed().as_micros() as u64;
        let allocating = std::time::Instant::now();
        if kept.instances.as_ref().is_none_or(|b| b.size() < bytes_of.len() as u64) {
            kept.instances = Some(buffer(device, &data, wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST));
        }
        alloc_us += allocating.elapsed().as_micros() as u64;
        let (Some(t), Some(instances)) = (&kept.targets, &kept.instances) else { return Err("no targets".into()) };
        let (row, read) = (t.row, t.read.clone());
        self.queue.write_buffer(instances, 0, &bytes_of);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        {
            let [r, g, b] = bg.map(f64::from);
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &t.samples,
                    depth_slice: None,
                    resolve_target: Some(&t.into),
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color { r, g, b, a: 1. }),
                        store: wgpu::StoreOp::Discard,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &t.depth,
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.), store: wgpu::StoreOp::Discard }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.group, &[]);
            pass.set_vertex_buffer(0, instances.slice(..bytes_of.len() as u64));
            pass.draw(0..36, 0..solid_count);
            pass.set_pipeline(&self.glass);
            pass.draw(0..36, solid_count..solid_count + clear.len() as u32);
        }
        encoder.copy_texture_to_buffer(
            t.resolved.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &read,
                layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: None },
            },
            size,
        );
        drop(kept);
        let prep_us = prepared.elapsed().as_micros() as u64;
        let started = std::time::Instant::now();
        self.queue.submit([encoder.finish()]);
        let (done, mapped) = std::sync::mpsc::channel();
        read.slice(..).map_async(wgpu::MapMode::Read, move |result| drop(done.send(result)));
        device.poll(wgpu::PollType::wait_indefinitely()).map_err(|e| format!("poll: {e}"))?;
        mapped.recv().map_err(|e| e.to_string())?.map_err(|e| format!("map: {e}"))?;
        let gpu_us = started.elapsed().as_micros() as u64;
        let started = std::time::Instant::now();
        let mut bgra = Vec::with_capacity(width as usize * 4 * height as usize);
        {
            let padded = read.slice(..).get_mapped_range().map_err(|e| format!("map: {e}"))?;
            // Row by row, as slices: a byte at a time took 15 ms for a 3200×2000 picture.
            for line in padded.chunks(row as usize) {
                bgra.extend_from_slice(&line[..width as usize * 4]);
            }
        }
        // Unmapped: the next picture copies into it again.
        read.unmap();
        let copy_us = started.elapsed().as_micros() as u64;
        Ok(Picture { width, height, bgra, gpu_us, copy_us, prep_us, boxes_us, alloc_us })
    }
}

/// A picture's targets: multisampled colour and depth, the resolved colour, and its read buffer
/// (rows padded to 256 bytes).
fn targets(device: &wgpu::Device, size: wgpu::Extent3d) -> Targets {
    let texture = |format, samples, usage| {
        device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size,
            mip_level_count: 1,
            sample_count: samples,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage,
            view_formats: &[],
        })
    };
    let target = wgpu::TextureUsages::RENDER_ATTACHMENT;
    let resolved = texture(FORMAT, 1, target | wgpu::TextureUsages::COPY_SRC);
    let row = (size.width * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let read = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: u64::from(row) * u64::from(size.height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    Targets {
        size: (size.width, size.height),
        samples: texture(FORMAT, SAMPLES, target).create_view(&Default::default()),
        into: resolved.create_view(&Default::default()),
        depth: texture(DEPTH, SAMPLES, target).create_view(&Default::default()),
        resolved,
        read,
        row,
    }
}

fn instance(block: &Block, delay: f32, tint: Option<(u32, u32)>, alpha: f32) -> [f32; 16] {
    let ([r, g, b], edge) = tint.map_or_else(|| colour(block.kind), |(top, edge)| (rgb(top), rgb(edge)));
    let (lo, hi) = (block.min, block.max);
    let glows = if matches!(block.kind, Kind::Lit(_)) { 1. } else { 0. };
    let sides = if matches!(block.kind, Kind::Plate(_)) { 0.25 } else { 1. };
    [lo[0], lo[1], lo[2], hi[0], hi[1], hi[2], r, g, b, glows, edge[0], edge[1], edge[2], delay, sides, alpha]
}

fn bytes(values: &[f32]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_ne_bytes()).collect()
}

fn buffer(device: &wgpu::Device, values: &[f32], usage: wgpu::BufferUsages) -> wgpu::Buffer {
    let size = (values.len() as u64 * 4).max(16);
    device.create_buffer(&wgpu::BufferDescriptor { label: None, size, usage, mapped_at_creation: false })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use ottospace_core::atlas;
    use ottospace_core::scene::View;

    /// Renders made-up repositories and writes each into the folder `CITY_PICTURE` names, as a
    /// BMP, with the time each step took: `CITY_PICTURE=/tmp cargo test -p ottospace --release
    /// -- --ignored a_city_is_rendered --nocapture`. Needs a GPU; no window.
    #[test]
    #[ignore = "needs a GPU; writes a picture and prints timings"]
    fn a_city_is_rendered() {
        let gpu = Gpu::new().expect("a GPU");
        for count in [28, 2_000, 5_000, 20_000] {
            // Nested folders, unevenly: d0 to d6, then up to three deeper by the file's number.
            let files: Vec<(String, u64)> = (0..count)
                .map(|i| {
                    let deep = ["", "e1/", "e1/f2/", "e2/"][(i / 7) % 4];
                    // Sizes as a repository's: most files small, a few large (200 B to about 50 KB).
                    let r = (i as u64 * 7919) % 1000;
                    (format!("d{}/{deep}g{}/f{i}.rs", i % 7, i % 5), 200 + r * r * r / 20_000)
                })
                .collect();
            let started = std::time::Instant::now();
            let plan = atlas::atlas(&atlas::repo(&files));
            let laid = started.elapsed();
            let lit = [(plan.places.iter().rev().find(|p| !p.group).unwrap().id.as_str(), 0xff6467)];
            let started = std::time::Instant::now();
            let scene = ottospace_core::scene::scene(&plan, &lit, 1.6, &View::default());
            let built = started.elapsed();
            let (w, h) = (1600, 1000);
            let started = std::time::Instant::now();
            let picture = gpu.render(&scene, w, h, 1., true).unwrap();
            let rendered = started.elapsed();
            println!(
                "{count} files: layout {laid:.1?}, scene {built:.1?} ({} boxes), render and read back {w}×{h} {rendered:.1?}",
                scene.blocks.len()
            );
            if let Some(dir) = std::env::var_os("CITY_PICTURE") {
                std::fs::write(std::path::Path::new(&dir).join(format!("city-{count}.bmp")), bmp(&picture)).unwrap();
            }
            // Frames of the rise: each a render of its own, the slowest of them.
            let started = std::time::Instant::now();
            let mut slowest = std::time::Duration::ZERO;
            for step in 0..=12 {
                let frame = std::time::Instant::now();
                let picture = gpu.render(&scene, w, h, step as f32 / 12., true).unwrap();
                slowest = slowest.max(frame.elapsed());
                if let (6, Some(dir)) = (step, std::env::var_os("CITY_PICTURE")) {
                    std::fs::write(std::path::Path::new(&dir).join(format!("city-{count}-rising.bmp")), bmp(&picture))
                        .unwrap();
                }
            }
            println!("  its rise: 13 frames in {:.1?}, the slowest {slowest:.1?}", started.elapsed());
        }
    }

    /// What a moving camera costs: each frame of a turn, its scene, render and read back, at a
    /// Retina tab's size (2400 × 1500 pixels), for 2,000 and 20,000 files. Prints the mean and
    /// the slowest tenth. `cargo test -p ottospace --release -- --ignored a_moving_camera_is_timed
    /// --nocapture`. Needs a GPU.
    #[test]
    #[ignore = "needs a GPU; prints timings"]
    fn a_moving_camera_is_timed() {
        let gpu = Gpu::new().expect("a GPU");
        for count in [2_000, 20_000] {
            let files: Vec<(String, u64)> = (0..count)
                .map(|i| {
                    let deep = ["", "e1/", "e1/f2/", "e2/"][(i / 7) % 4];
                    let r = (i as u64 * 7919) % 1000;
                    (format!("d{}/{deep}g{}/f{i}.rs", i % 7, i % 5), 200 + r * r * r / 20_000)
                })
                .collect();
            let plan = atlas::atlas(&atlas::repo(&files));
            let (w, h) = (2400, 1500);
            let mut times: Vec<f64> = (0..40)
                .map(|step| {
                    let view = View::default().orbited(step as f32 * 3., (step % 7) as f32).zoomed(0.9, [0., 0.]);
                    let started = std::time::Instant::now();
                    let scene = ottospace_core::scene::scene(&plan, &[], w as f32 / h as f32, &view);
                    drop(gpu.render(&scene, w, h, 1., true).unwrap());
                    started.elapsed().as_secs_f64() * 1000.
                })
                .collect();
            times.sort_by(f64::total_cmp);
            let mean = times.iter().sum::<f64>() / times.len() as f64;
            println!(
                "{count} files, {w}×{h}: mean {mean:.1} ms, slowest tenth from {:.1} ms",
                times[times.len() * 9 / 10]
            );
        }
    }

    /// An edge line is about one device pixel wide at 1× and at 2×: one box, the same view at
    /// both sizes, the rows brighter than its top face counted down the face's middle column at
    /// its back and front edges. No GPU (a CI runner without one): nothing to check.
    #[test]
    fn an_edge_is_one_device_pixel_at_any_scale() {
        let Ok(gpu) = Gpu::new() else { return eprintln!("no GPU: skipped") };
        let plan = atlas::atlas(&atlas::repo(&[("a.rs".into(), 40_000), ("b.rs".into(), 40_000)]));
        // Looking well down (52°): the faces' edges apart, each counted on its own.
        let steep = View { pitch: 52., ..View::default() };
        let scene = ottospace_core::scene::scene(&plan, &[], 1.6, &steep);
        let camera = scene.camera.unwrap();
        let building = scene.blocks.iter().find(|b| b.kind == Kind::Building).unwrap();
        let mid = (building.min[0] + building.max[0]) / 2.;
        let at = |z: f32| camera.project([mid, building.max[1], z]).unwrap();
        let (back, front) = (at(building.max[2]), at(building.min[2]));
        let face = at((building.min[2] + building.max[2]) / 2.);
        for scale in [1, 2] {
            let (w, h) = (320 * scale, 200 * scale);
            let p = gpu.render(&scene, w, h, 1., false).unwrap();
            let px = |v: f32, n: u32| (v * n as f32) as u32;
            let light = |(x, y): (f32, f32)| {
                let at = ((px(y, h) * w + px(x, w)) * 4) as usize;
                p.bgra[at..at + 3].iter().map(|c| *c as f32 / 255.).sum::<f32>()
            };
            let plain = light(face);
            for (edge, (x, y)) in [("back", back), ("front", front)] {
                let rows = (-4..=4).filter(|d| light((x, y + *d as f32 / h as f32)) > plain + 0.15).count();
                assert!((1..=2).contains(&rows), "{scale}×: the {edge} edge is {rows} rows");
            }
        }
    }

    /// Kept targets and boxes draw what fresh ones do: a view, another size and view, then the
    /// first again from the kept boxes, byte for byte. No GPU: nothing to check.
    #[test]
    fn kept_targets_and_boxes_draw_the_same_picture() {
        let Ok(gpu) = Gpu::new() else { return eprintln!("no GPU: skipped") };
        let files: Vec<(String, u64)> =
            (0..300).map(|i| (format!("d{}/e{}/f{i}.rs", i % 5, i % 3), 300 + i * 97)).collect();
        let plan = atlas::atlas(&atlas::repo(&files));
        let turned = View::default().orbited(40., -10.);
        let scene = ottospace_core::scene::scene(&plan, &[], 1.6, &turned);
        let fresh = gpu.render(&scene, 320, 200, 1., true).unwrap();
        let boxes = Boxes::new(&scene, true);
        let other = ottospace_core::scene::scene(&plan, &[], 1.6, &View::default());
        drop(gpu.render_boxes(&other, &boxes, 160, 100, 1.).unwrap());
        let kept = gpu.render_boxes(&scene, &boxes, 320, 200, 1.).unwrap();
        assert!(kept.bgra == fresh.bgra, "the kept picture differs");
    }

    /// A 32-bit BMP, top row first: enough to look at a picture.
    pub(crate) fn bmp(p: &Picture) -> Vec<u8> {
        let size = 54 + p.bgra.len() as u32;
        let mut out = b"BM".to_vec();
        for v in [size, 0, 54, 40, p.width, (-(p.height as i32)) as u32] {
            out.extend(v.to_le_bytes());
        }
        out.extend(1u16.to_le_bytes());
        out.extend(32u16.to_le_bytes());
        for v in [0u32, p.bgra.len() as u32, 2835, 2835, 0, 0] {
            out.extend(v.to_le_bytes());
        }
        out.extend(&p.bgra);
        out
    }
}
