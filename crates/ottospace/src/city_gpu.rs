//! The code city's picture: the core's scene (`ottospace_core::scene`) rendered offscreen on
//! the GPU (wgpu, whichever backend the platform has), with a depth buffer, 4× multisampling, a
//! light from above, edge lines and fog into the background, then read back as BGRA for GPUI to
//! paint. Run
//! on a thread of its own; a still picture, made again only when the scene or its size changes.

use std::collections::HashMap;

use ottospace_core::scene::{Block, Kind, Layer, Scene};
use ottospace_core::theme;

use crate::city_text::{self, Label, Text};

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
/// its sides' edges are, a share of its top's, how opaque its faces are, and the floor it rises
/// from (a stratum's is its building's), then its roof cap's colour and height (0: none).
const STRIDE: u64 = 21 * 4;
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
    @location(9) @interpolate(flat) cap: vec4<f32>,
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
fn vs(@builtin(vertex_index) i: u32, @location(0) lo: vec3<f32>, @location(1) hi: vec3<f32>, @location(2) top: vec4<f32>, @location(3) edge: vec3<f32>, @location(4) delay: f32, @location(5) sides: f32, @location(6) alpha: f32, @location(7) base: f32, @location(8) cap: vec4<f32>) -> Out {
    // Rising (`frame.light.w`: the share of the rise, 1 at rest): as tall as `scene::risen` says,
    // from its floor (a stratum: its building's, so a stack grows as one).
    let at = clamp((frame.light.w - delay) / (1. - STAGGER), 0., 1.);
    let grown = select(1. - (1. - at) * (1. - at) * (1. - at), 1., delay < 0.);
    let low_y = base + (lo.y - base) * grown;
    let top_y = base + (hi.y - base) * grown;
    let corner = CORNERS[FACES[i]];
    let world = vec3(lo.x + (hi.x - lo.x) * corner.x, low_y + (top_y - low_y) * corner.y, lo.z + (hi.z - lo.z) * corner.z);
    let normal = NORMALS[i / 6u];
    // Sides darker than the top: ambient, and a light from above to one side.
    let light = 0.3 + 0.7 * max(dot(normal, normalize(frame.light.xyz)), 0.);
    var out: Out;
    out.at = frame.matrix * vec4(world, 1.);
    // A lit building glows: its own colour on every face, a step darker on the sides.
    out.colour = mix(top.rgb * light, top.rgb * (0.75 + 0.25 * normal.y), top.a);
    out.lit = top.a;
    out.world = world;
    out.lo = vec3(lo.x, low_y, lo.z);
    out.hi = vec3(hi.x, top_y, hi.z);
    out.edge = edge;
    out.axis = i / 12u;
    out.sides = sides;
    out.alpha = alpha;
    // Its cap grows with it.
    out.cap = vec4(cap.rgb * light, cap.w * grown);
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
    // A roof cap (`city_kinds`): the box's top `cap.w` in its kind's colour.
    let capped = in.cap.w > 0. && in.world.y >= in.hi.y - in.cap.w;
    let colour = mix(select(in.colour, in.cap.rgb, capped), edge, line);
    // Fog: from nothing at `fog.x` from the eye to `fog.z` at `fog.y`.
    let far = clamp((distance(in.world, frame.eye.xyz) - frame.fog.x) / (frame.fog.y - frame.fog.x), 0., 1.);
    // Glass: the faces see-through, the edges whole.
    return vec4(mix(colour, frame.background.rgb, far * frame.fog.z * (1. - in.lit)), max(in.alpha, line));
}
"#;

/// Text in the world (`city_text`): a glyph's quad from its corner and sides, its field sampled,
/// its edge smoothed over a screen pixel, fogged as the boxes are. Drawn over the solid boxes,
/// depth tested, writing none; under the glass.
const TEXT_SHADER: &str = r#"
struct Frame { matrix: mat4x4<f32>, eye: vec4<f32>, fog: vec4<f32>, background: vec4<f32>, light: vec4<f32> };
@group(0) @binding(0) var<uniform> frame: Frame;
@group(1) @binding(0) var atlas: texture_2d<f32>;
@group(1) @binding(1) var sampled: sampler;

struct Out {
    @builtin(position) at: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) world: vec3<f32>,
    @location(2) colour: vec4<f32>,
};

const QUAD = array<vec2<f32>, 6>(vec2(0., 0.), vec2(1., 0.), vec2(1., 1.), vec2(0., 0.), vec2(1., 1.), vec2(0., 1.));

@vertex
fn vs(@builtin(vertex_index) i: u32, @location(0) corner: vec3<f32>, @location(1) across: vec3<f32>, @location(2) up: vec3<f32>, @location(3) uv: vec4<f32>, @location(4) colour: vec4<f32>) -> Out {
    let c = QUAD[i];
    let world = corner + across * c.x + up * c.y;
    var out: Out;
    out.at = frame.matrix * vec4(world, 1.);
    // The atlas's rows run down: a glyph's bottom is its rect's.
    out.uv = vec2(mix(uv.x, uv.z, c.x), mix(uv.w, uv.y, c.y));
    out.world = world;
    out.colour = colour;
    return out;
}

@fragment
fn fs(in: Out) -> @location(0) vec4<f32> {
    let d = textureSample(atlas, sampled, in.uv).r;
    let w = max(fwidth(d) * 0.7, 1e-4);
    let far = clamp((distance(in.world, frame.eye.xyz) - frame.fog.x) / (frame.fog.y - frame.fog.x), 0., 1.);
    return vec4(mix(in.colour.rgb, frame.background.rgb, far * frame.fog.z), smoothstep(0.5 - w, 0.5 + w, d) * in.colour.a);
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

/// A live layer gave no drawable this time (`render_into`): not a failure of the picture.
pub(crate) const NO_DRAWABLE: &str = "no drawable";

/// The device kept between pictures.
pub struct Gpu {
    /// Kept for surfaces of live layers (`live_surface`).
    instance: wgpu::Instance,
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    /// For glass buildings: blended over what is drawn, writing no depth, drawn last, far first.
    glass: wgpu::RenderPipeline,
    /// The frame's uniform (camera, eye, fog, background, light: 32 floats), bound once.
    uniform: wgpu::Buffer,
    group: wgpu::BindGroup,
    layout: wgpu::BindGroupLayout,
    /// Kept from picture to picture: a camera move makes no texture or buffer.
    kept: std::cell::RefCell<Kept>,
    /// Text's pipeline and atlas, made at the first line measured (`measure`); none: no font.
    text: std::cell::RefCell<Option<Option<TextGpu>>>,
}

/// What draws text: its glyphs (`city_text`), pipeline, atlas and its bind group, and the glyphs'
/// instance buffer, as large as the most yet.
struct TextGpu {
    glyphs: Text,
    pipeline: wgpu::RenderPipeline,
    atlas: wgpu::Texture,
    group: wgpu::BindGroup,
    instances: Option<wgpu::Buffer>,
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
    base: Vec<[f32; 21]>,
    /// Each instance's block, and a stratum's slice of its height (`Scene::layers`).
    owner: Vec<u32>,
    layer: Vec<Option<[f32; 2]>>,
    /// Where the stratum above starts (none: infinity): a cap goes on the highest one shown.
    above: Vec<f32>,
    /// The scene's blocks, for a check that these are its boxes.
    blocks: usize,
    /// Indices into the instances: the solid ones, then the glass.
    solid: Vec<u32>,
    glass: Vec<u32>,
}

impl Boxes {
    pub fn new(scene: &Scene, glass: bool) -> Self {
        let delays = scene.rise_delays();
        let tint = |i: usize| scene.tints.get(i).copied().flatten();
        let see_through = |b: &Block| glass && b.kind == Kind::Building;
        // Each building's strata, by block: they come in the blocks' order.
        let mut strata: HashMap<usize, &[Layer]> = HashMap::new();
        for run in scene.layers.chunk_by(|a, b| a.block == b.block) {
            strata.insert(run[0].block, run);
        }
        let mut boxes = Self {
            base: Vec::new(),
            owner: Vec::new(),
            layer: Vec::new(),
            above: Vec::new(),
            blocks: scene.blocks.len(),
            solid: Vec::new(),
            glass: Vec::new(),
        };
        for (i, b) in scene.blocks.iter().enumerate() {
            let alpha = if see_through(b) { GLASS } else { 1. };
            let layers = strata.get(&i).copied().filter(|_| b.kind == Kind::Building).unwrap_or_default();
            // A building's strata in place of its box, each in its commit's colour edged a third of
            // the way to white (its stub in the block's own), rising from the building's floor.
            let slices = layers
                .iter()
                .map(|l| (Some(l.y), l.colour.map(|c| (c, ottospace_core::palette::dim(c, 0xffffff))).or(tint(i))));
            let whole = std::iter::once((None, tint(i)));
            let parts: Vec<_> = if layers.is_empty() { whole.collect() } else { slices.collect() };
            // Its cap's colour on each part; its height where it stands (`placed`).
            let cap = scene.caps.get(i).copied().flatten().map(|kind| {
                let [r, g, b] = rgb(kind.colour());
                [r, g, b, 1.]
            });
            let starts = layers.iter().skip(1).map(|l| l.y[0]).chain([f32::INFINITY]);
            let starts: Vec<f32> = if layers.is_empty() { vec![f32::INFINITY] } else { starts.collect() };
            for ((y, colours), above) in parts.into_iter().zip(starts) {
                boxes.above.push(above);
                let at = boxes.base.len() as u32;
                if see_through(b) {
                    boxes.glass.push(at)
                } else {
                    boxes.solid.push(at)
                }
                boxes.base.push(instance(b, delays[i], colours, alpha, b.min[1], cap));
                boxes.owner.push(i as u32);
                boxes.layer.push(y);
            }
        }
        boxes
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
            0 => Float32x3, 1 => Float32x3, 2 => Float32x4, 3 => Float32x3, 4 => Float32, 5 => Float32, 6 => Float32,
            7 => Float32, 8 => Float32x4
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
        let (kept, text) = Default::default();
        Ok(Self { instance, device, queue, pipeline, glass, uniform, group, layout, kept, text })
    }

    /// `text`'s width in ems as drawn (`city_text::Text::width`); none if it cannot be drawn.
    pub(crate) fn measure(&self, text: &str) -> Option<f32> {
        let mut made = self.text.borrow_mut();
        let made = made.get_or_insert_with(|| Text::new().map(|glyphs| self.text_gpu(glyphs)));
        made.as_mut()?.glyphs.width(text)
    }

    fn text_gpu(&self, glyphs: Text) -> TextGpu {
        let device = &self.device;
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("city text"),
            source: wgpu::ShaderSource::Wgsl(TEXT_SHADER.into()),
        });
        let entry = |binding, ty| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty,
            count: None,
        };
        let atlas_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[
                entry(
                    0,
                    wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                ),
                entry(1, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering)),
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&self.layout), Some(&atlas_layout)],
            immediate_size: 0,
        });
        let attributes =
            wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x3, 3 => Float32x4, 4 => Float32x4];
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("city text"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: (city_text::QUAD * 4) as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &attributes,
                })],
            },
            primitive: wgpu::PrimitiveState { cull_mode: None, ..Default::default() },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH,
                depth_write_enabled: Some(false),
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
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let atlas = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("city glyphs"),
            size: wgpu::Extent3d { width: city_text::ATLAS, height: city_text::ATLAS, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let view = atlas.create_view(&Default::default());
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &atlas_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&sampler) },
            ],
        });
        TextGpu { glyphs, pipeline, atlas, group, instances: None }
    }

    /// `labels`' glyphs as instances in the text's buffer, its atlas uploaded if it grew: how many.
    fn text_instances(&self, labels: &[Label]) -> u32 {
        let mut text = self.text.borrow_mut();
        let Some(Some(text)) = text.as_mut().filter(|_| !labels.is_empty()) else { return 0 };
        let mut data = Vec::new();
        for label in labels {
            text.glyphs.quads(label, &mut data);
        }
        if text.glyphs.dirty {
            let size = wgpu::Extent3d { width: city_text::ATLAS, height: city_text::ATLAS, depth_or_array_layers: 1 };
            let layout =
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(city_text::ATLAS), rows_per_image: None };
            self.queue.write_texture(text.atlas.as_image_copy(), &text.glyphs.pixels, layout, size);
            text.glyphs.dirty = false;
        }
        let bytes_of = bytes(&data);
        if text.instances.as_ref().is_none_or(|b| b.size() < bytes_of.len() as u64) {
            text.instances =
                Some(buffer(&self.device, &data, wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST));
        }
        if let Some(instances) = &text.instances {
            self.queue.write_buffer(instances, 0, &bytes_of);
        }
        text.glyphs.forget();
        (data.len() / city_text::QUAD) as u32
    }

    /// One draw of a one-box city at 16×16, its picture dropped: the first picture shown pays no
    /// first draw's setup (60 ms in the bench). Its failure is no failure of the GPU.
    pub fn warm(&self) {
        let one = ottospace_core::atlas::atlas(&ottospace_core::atlas::repo(&[("a/b.rs".into(), 900)]));
        let scene = ottospace_core::scene::scene(&one, &[], 1., &ottospace_core::scene::View::default());
        let warmed =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.render(&scene, 16, 16, 1., true, &[])));
        drop(warmed);
    }

    /// `scene` as a picture `width` × `height` pixels, `rise` (0 to 1) of the way through its
    /// buildings' rise (1: standing), its buildings of glass or not, `labels` written in it, or why
    /// there is none.
    pub(crate) fn render(
        &self,
        scene: &Scene,
        width: u32,
        height: u32,
        rise: f32,
        glass: bool,
        labels: &[Label],
    ) -> Result<Picture, String> {
        self.render_boxes(scene, &Boxes::new(scene, glass), width, height, rise, labels)
    }

    /// A surface over a live layer's `CAMetalLayer` (`ottospace_mac::live_layer`), to draw to
    /// with `render_into`.
    ///
    /// # Safety
    /// `layer` is a `CAMetalLayer` that outlives the surface.
    pub unsafe fn live_surface(&self, layer: *mut std::ffi::c_void) -> Result<wgpu::Surface<'static>, String> {
        // SAFETY: the caller's layer, kept alive by its view while the surface lives.
        unsafe { self.instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::CoreAnimationLayer(layer)) }
            .map_err(|e| format!("surface: {e}"))
    }

    /// `surface` set up for pictures `width` × `height`: presented a display refresh each (FIFO).
    pub fn configure(&self, surface: &wgpu::Surface<'static>, width: u32, height: u32) {
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: FORMAT,
            width: width.max(1),
            height: height.max(1),
            present_mode: wgpu::PresentMode::Fifo,
            desired_maximum_frame_latency: 2,
            alpha_mode: wgpu::CompositeAlphaMode::Opaque,
            view_formats: vec![],
            color_space: wgpu::SurfaceColorSpace::Auto,
        };
        surface.configure(&self.device, &config);
    }

    /// `render_boxes`, drawn straight into `surface`'s next drawable and presented: no read back.
    pub(crate) fn render_into(
        &self,
        surface: &wgpu::Surface<'static>,
        scene: &Scene,
        boxes: &Boxes,
        rise: f32,
        labels: &[Label],
    ) -> Result<Picture, String> {
        let frame = match surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame) | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            other => return Err(format!("{NO_DRAWABLE}: {other:?}")),
        };
        let (width, height) = (frame.texture.width(), frame.texture.height());
        let view = frame.texture.create_view(&Default::default());
        let picture = self.checked(|| self.draw(scene, boxes, width, height, rise, labels, Some(&view)))?;
        self.queue.present(frame);
        Ok(picture)
    }

    /// `render`, of `scene`'s boxes as made already (`Boxes::new`): a camera move's picture.
    pub(crate) fn render_boxes(
        &self,
        scene: &Scene,
        boxes: &Boxes,
        width: u32,
        height: u32,
        rise: f32,
        labels: &[Label],
    ) -> Result<Picture, String> {
        self.checked(|| self.draw(scene, boxes, width, height, rise, labels, None))
    }

    /// `draw` with the GPU's errors caught: a picture fails instead of wgpu panicking.
    fn checked(&self, draw: impl FnOnce() -> Result<Picture, String>) -> Result<Picture, String> {
        // An error of the GPU's (out of memory, a lost device) fails the picture; uncaught, wgpu
        // panics.
        let memory = self.device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let valid = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let picture = draw();
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

    /// The picture, resolved into `into` (a live layer's drawable) or read back.
    #[allow(clippy::too_many_arguments)] // A picture's scene, size, rise, text and target.
    fn draw(
        &self,
        scene: &Scene,
        boxes: &Boxes,
        width: u32,
        height: u32,
        rise: f32,
        labels: &[Label],
        into: Option<&wgpu::TextureView>,
    ) -> Result<Picture, String> {
        if boxes.blocks != scene.blocks.len() {
            return Err("boxes of another scene".into());
        }
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
            let top = scene.blocks[boxes.owner[*i as usize] as usize].top();
            // Farthest first: a distance's bits order as it does (never negative).
            std::cmp::Reverse((0..3).map(|k| (top[k] - camera.eye[k]).powi(2)).sum::<f32>().to_bits())
        };
        // Only the boxes drawn at this level of detail (`Scene::shown`), and a stratum only below
        // its building's top (a lit building's line of sight may cut it down).
        let shown = |i: &&u32| {
            let block = boxes.owner[**i as usize] as usize;
            let below = boxes.layer[**i as usize].is_none_or(|y| y[0] < scene.blocks[block].max[1]);
            below && scene.is_shown(block)
        };
        let mut clear: Vec<u32> = boxes.glass.iter().filter(shown).copied().collect();
        clear.sort_by_cached_key(far);
        let solid: Vec<u32> = boxes.solid.iter().filter(shown).copied().collect();
        let solid_count = solid.len() as u32;
        let placed = |i: &u32| {
            let block = boxes.owner[*i as usize] as usize;
            let (b, mut one) = (&scene.blocks[block], boxes.base[*i as usize]);
            // A closed box standing for a lit file glows in its colour.
            if let Some(colour) = scene.glows(block) {
                one = instance(&Block { kind: Kind::Lit(colour), ..*b }, one[13], None, one[15], one[16], None);
            }
            one[..3].copy_from_slice(&b.min);
            one[3..6].copy_from_slice(&b.max);
            // A stratum: its slice of the building's height.
            if let Some(y) = boxes.layer[*i as usize] {
                (one[1], one[4]) = (y[0], y[1].min(b.max[1]));
            }
            // A cap, on the highest part shown, as thick as the building now stands (a line of sight
            // may cut it).
            if one[20] > 0. {
                one[20] = if boxes.above[*i as usize] >= b.max[1] { cap(b) } else { 0. };
            }
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
        let glyphs = self.text_instances(labels);
        let text = self.text.borrow();
        let text = text.as_ref().and_then(Option::as_ref).filter(|_| glyphs > 0);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        {
            let [r, g, b] = bg.map(f64::from);
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &t.samples,
                    depth_slice: None,
                    resolve_target: Some(into.unwrap_or(&t.into)),
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
            let instances_all = instances.slice(..bytes_of.len() as u64);
            pass.set_vertex_buffer(0, instances_all);
            pass.draw(0..36, 0..solid_count);
            // Text over the solid boxes, under the glass.
            if let Some(TextGpu { pipeline, group, instances: Some(instances), .. }) = text {
                pass.set_pipeline(pipeline);
                pass.set_bind_group(1, group, &[]);
                pass.set_vertex_buffer(0, instances.slice(..u64::from(glyphs) * (city_text::QUAD * 4) as u64));
                pass.draw(0..6, 0..glyphs);
                pass.set_vertex_buffer(0, instances_all);
            }
            pass.set_pipeline(&self.glass);
            pass.draw(0..36, solid_count..solid_count + clear.len() as u32);
        }
        if into.is_some() {
            // Straight to the layer: no read back; presented by the caller.
            drop(kept);
            let prep_us = prepared.elapsed().as_micros() as u64;
            let started = std::time::Instant::now();
            self.queue.submit([encoder.finish()]);
            let gpu_us = started.elapsed().as_micros() as u64;
            let picture = Picture { width, height, bgra: Vec::new(), gpu_us, copy_us: 0, prep_us, boxes_us, alloc_us };
            return Ok(picture);
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

fn instance(
    block: &Block,
    delay: f32,
    tint: Option<(u32, u32)>,
    alpha: f32,
    base: f32,
    cap: Option<[f32; 4]>,
) -> [f32; 21] {
    let ([r, g, b], edge) = tint.map_or_else(|| colour(block.kind), |(top, edge)| (rgb(top), rgb(edge)));
    let (lo, hi) = (block.min, block.max);
    let glows = if matches!(block.kind, Kind::Lit(_)) { 1. } else { 0. };
    let sides = if matches!(block.kind, Kind::Plate(_)) { 0.25 } else { 1. };
    let [cr, cg, cb, ch] = cap.unwrap_or_default();
    [
        lo[0], lo[1], lo[2], hi[0], hi[1], hi[2], r, g, b, glows, edge[0], edge[1], edge[2], delay, sides, alpha, base,
        cr, cg, cb, ch,
    ]
}

/// A building's roof cap's height: a thin slab, a quarter of its shorter side thick (it scales
/// with the city's lots), at most a third of its height.
fn cap(block: &Block) -> f32 {
    let side = (block.max[0] - block.min[0]).min(block.max[2] - block.min[2]);
    (side * 0.25).min((block.max[1] - block.min[1]) / 3.)
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
    use ottospace_core::strata::{Change, History, Level};

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
            // A year of 25 commits, of each kind in turn: most files in one, a few in all.
            let kinds = [Some(Change::Feat), Some(Change::Fix), None, Some(Change::Refactor), Some(Change::Docs)];
            let levels = |i: u64| {
                let r = (i * 7919) % 1000;
                (1..=25u64)
                    .rev()
                    .filter(move |c| *c == 1 || r * c < 1000)
                    .map(move |c| Level { at: c, change: kinds[c as usize % 5] })
            };
            let paths = files.iter().enumerate().map(|(i, (path, _))| (path.clone(), levels(i as u64).collect()));
            let history = History { files: paths.collect(), span: (1, 25) };
            let started = std::time::Instant::now();
            let plan = atlas::atlas(&atlas::repo_with(&files, &history));
            let laid = started.elapsed();
            let lit = [(plan.places.iter().rev().find(|p| !p.group).unwrap().id.as_str(), 0xff6467)];
            let started = std::time::Instant::now();
            let scene = ottospace_core::scene::scene(&plan, &lit, 1.6, &View::default());
            let built = started.elapsed();
            let (w, h) = (1600, 1000);
            let started = std::time::Instant::now();
            let picture = gpu.render(&scene, w, h, 1., true, &[]).unwrap();
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
                let picture = gpu.render(&scene, w, h, step as f32 / 12., true, &[]).unwrap();
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
                    drop(gpu.render(&scene, w, h, 1., true, &[]).unwrap());
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
            let p = gpu.render(&scene, w, h, 1., false, &[]).unwrap();
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

    /// A cap (`city_kinds`) colours a building's roof in its kind's colour and leaves its walls'
    /// foot as they were. No GPU: nothing to check.
    #[test]
    fn a_cap_colours_the_roof_only() {
        let Ok(gpu) = Gpu::new() else { return eprintln!("no GPU: skipped") };
        let plan = atlas::atlas(&atlas::repo(&[("a/b.md".into(), 40_000), ("a/c.rs".into(), 40_000)]));
        let mut scene = ottospace_core::scene::scene(&plan, &[], 1.6, &View { pitch: 52., ..View::default() });
        let camera = scene.camera.unwrap();
        let i = scene.blocks.iter().position(|b| b.kind == Kind::Building).unwrap();
        let b = scene.blocks[i];
        let mid = |k: usize| (b.min[k] + b.max[k]) / 2.;
        let (w, h) = (320, 200);
        let pixel = |p: &Picture, at: [f32; 3]| {
            let (x, y) = camera.project(at).unwrap();
            let at = (((y * h as f32) as u32 * w + (x * w as f32) as u32) * 4) as usize;
            [p.bgra[at + 2], p.bgra[at + 1], p.bgra[at]]
        };
        let (roof, foot) = ([mid(0), b.max[1], mid(2)], [mid(0), b.min[1] + 0.002, b.min[2]]);
        let plain = gpu.render(&scene, w, h, 1., false, &[]).unwrap();
        scene.caps = ottospace_core::city_kinds::caps(&plan, &scene.blocks);
        let kind = scene.caps[i].unwrap();
        let capped = gpu.render(&scene, w, h, 1., false, &[]).unwrap();
        let [r, g, bl] = pixel(&capped, roof).map(i32::from);
        let [cr, cg, cb] = [16, 8, 0].map(|s| (kind.colour() >> s & 0xff) as i32);
        // Shaded and fogged a little: the kind's hue, its strongest channel strongest.
        let strongest = |c: [i32; 3]| (0..3).max_by_key(|k| c[*k]).unwrap();
        assert_eq!(strongest([r, g, bl]), strongest([cr, cg, cb]), "{kind:?}: {:?}", [r, g, bl]);
        assert_ne!(pixel(&plain, roof), pixel(&capped, roof));
        assert_eq!(pixel(&plain, foot), pixel(&capped, foot), "the walls' foot keeps its folder's colour");
        // Boxes made while a line of sight cut it to its floor still cap it standing whole.
        let mut cut = scene.clone();
        cut.blocks[i].max[1] = b.min[1];
        let kept = gpu.render_boxes(&scene, &Boxes::new(&cut, false), w, h, 1., &[]).unwrap();
        assert_eq!(pixel(&kept, roof), pixel(&capped, roof), "its cap as it stands now");
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
        let fresh = gpu.render(&scene, 320, 200, 1., true, &[]).unwrap();
        let boxes = Boxes::new(&scene, true);
        let other = ottospace_core::scene::scene(&plan, &[], 1.6, &View::default());
        drop(gpu.render_boxes(&other, &boxes, 160, 100, 1., &[]).unwrap());
        let kept = gpu.render_boxes(&scene, &boxes, 320, 200, 1., &[]).unwrap();
        assert!(kept.bgra == fresh.bgra, "the kept picture differs");
    }

    /// A plinth's name is drawn on its wall, and a tower between it and the camera hides it: the
    /// depth test. Light pixels (the name is white, fogged a little) over its's rect counted, with and without the tower. No GPU
    /// or no font: nothing to check. `CITY_PICTURE=<dir>` writes both.
    #[test]
    fn a_name_on_a_wall_is_drawn_and_a_tower_in_front_hides_it() {
        let Ok(gpu) = Gpu::new() else { return eprintln!("no GPU: skipped") };
        let Some(wide) = gpu.measure("SRC") else { return eprintln!("no font: skipped") };
        let plinth = Block {
            min: [0.4, 0., 0.3],
            max: [1.2, 0.06, 0.7],
            kind: Kind::Plate(1),
            place: Some(0),
            district: Some(0),
        };
        let fit = ottospace_core::scene::Fit { distance: 1.5, target: [0.8, 0., 0.5], lower: 0. };
        let seen = |blocks: Vec<Block>| Scene {
            blocks,
            camera: Some(fit.camera(&View::default(), 1.6)),
            fit: Some(fit),
            ..Scene::default()
        };
        let scene = seen(vec![plinth]);
        let (w, h) = (640, 400);
        let sign = ottospace_core::city_walls::signs(&scene, |_| Some(wide), [w as f32, h as f32], 12.)[0];
        let label = || Label {
            text: "SRC".into(),
            origin: sign.origin,
            across: sign.across,
            up: [0., 1., 0.],
            em: sign.em,
            colour: [1.; 4],
        };
        let white = |scene: &Scene, name: &str| {
            let p = gpu.render(scene, w, h, 1., false, &[label()]).unwrap();
            if let Some(dir) = std::env::var_os("CITY_PICTURE") {
                std::fs::write(std::path::Path::new(&dir).join(format!("wall-{name}.bmp")), bmp(&p)).unwrap();
            }
            let [x, y, rw, rh] = sign.rect.map(|v| v);
            let (x0, y0) = ((x * w as f32) as u32, (y * h as f32) as u32);
            let (x1, y1) = (((x + rw) * w as f32) as u32, ((y + rh) * h as f32) as u32);
            (y0..y1.min(h))
                .flat_map(|py| (x0..x1.min(w)).map(move |px| (px, py)))
                .filter(|(px, py)| p.bgra[((py * w + px) * 4) as usize..][..3].iter().all(|c| *c > 150))
                .count()
        };
        let drawn = white(&scene, "named");
        assert!(drawn > 40, "the name drawn: {drawn} white pixels");
        let camera = scene.camera.unwrap();
        let middle: [f32; 3] =
            std::array::from_fn(|i| sign.origin[i] + sign.across[i] * sign.width / 2. + [0., sign.em / 2., 0.][i]);
        let to_eye: [f32; 3] = std::array::from_fn(|i| camera.eye[i] - middle[i]);
        let on: [f32; 3] = std::array::from_fn(|i| middle[i] + to_eye[i] * 0.3);
        let tower = Block {
            min: [on[0] - 0.08, 0., on[2] - 0.02],
            max: [on[0] + 0.08, 1., on[2] + 0.02],
            kind: Kind::Building,
            place: Some(1),
            district: None,
        };
        let behind = seen(vec![plinth, tower]);
        assert!(ottospace_core::city_walls::hidden(&behind, &sign));
        assert_eq!(white(&behind, "hidden"), 0, "the tower hides it");
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
