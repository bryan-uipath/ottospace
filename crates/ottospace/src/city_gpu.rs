//! The code city's picture: the core's scene (`ottospace_core::scene`) rendered offscreen on
//! the GPU (wgpu, whichever backend the platform has), with a depth buffer, 4× multisampling, a
//! light from above and fog into the background, then read back as BGRA for GPUI to paint. Run
//! on a thread of its own; a still picture, made again only when the scene or its size changes.

use ottospace_core::scene::{Block, Kind, Scene};
use ottospace_core::theme;

/// The picture, `width` × `height` pixels, BGRA, rows packed.
pub struct Picture {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
}

const SAMPLES: u32 = 4;
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Bgra8Unorm;
const DEPTH: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
/// Per box: its corners (min, max), its top's colour, and 1 if it glows (lit).
const STRIDE: u64 = 10 * 4;

const SHADER: &str = r#"
struct Frame { matrix: mat4x4<f32>, eye: vec4<f32>, fog: vec4<f32>, background: vec4<f32>, light: vec4<f32> };
@group(0) @binding(0) var<uniform> frame: Frame;

struct Out {
    @builtin(position) at: vec4<f32>,
    @location(0) colour: vec3<f32>,
    @location(1) world: vec3<f32>,
    @location(2) lit: f32,
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
fn vs(@builtin(vertex_index) i: u32, @location(0) lo: vec3<f32>, @location(1) hi: vec3<f32>, @location(2) top: vec4<f32>) -> Out {
    let world = lo + (hi - lo) * CORNERS[FACES[i]];
    let normal = NORMALS[i / 6u];
    // Sides darker than the top: ambient, and a light from above to one side.
    let light = 0.3 + 0.7 * max(dot(normal, normalize(frame.light.xyz)), 0.);
    var out: Out;
    out.at = frame.matrix * vec4(world, 1.);
    // A lit building glows: its own colour on every face, a step darker on the sides.
    out.colour = mix(top.rgb * light, top.rgb * (0.75 + 0.25 * normal.y), top.a);
    out.lit = top.a;
    out.world = world;
    return out;
}

@fragment
fn fs(in: Out) -> @location(0) vec4<f32> {
    // Fog: from nothing at `fog.x` from the eye to `fog.z` at `fog.y`.
    let far = clamp((distance(in.world, frame.eye.xyz) - frame.fog.x) / (frame.fog.y - frame.fog.x), 0., 1.);
    return vec4(mix(in.colour, frame.background.rgb, far * frame.fog.z * (1. - in.lit)), 1.);
}
"#;

fn rgb(hex: u32) -> [f32; 3] {
    [(hex >> 16 & 0xff) as f32 / 255., (hex >> 8 & 0xff) as f32 / 255., (hex & 0xff) as f32 / 255.]
}

/// A box's top colour. Quiet behind the hints, and still a skyline: buildings well above the
/// ground's shade, districts a step lighter per depth.
fn colour(kind: Kind) -> [f32; 3] {
    match kind {
        Kind::Ground => rgb(0x101014),
        Kind::Plate(depth) => rgb(0x17171c + 0x050506 * (depth.min(4) as u32 - 1)),
        Kind::Building => rgb(0x5a5a68),
        Kind::Lit(hex) => rgb(hex),
    }
}

/// The device kept between pictures.
pub struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
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
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
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
        let attributes = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x4];
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
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
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState { count: SAMPLES, ..Default::default() },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(FORMAT.into())],
            }),
            multiview_mask: None,
            cache: None,
        });
        Ok(Self { device, queue, pipeline, layout })
    }

    /// `scene` as a picture `width` × `height` pixels. `None` with no camera.
    pub fn render(&self, scene: &Scene, width: u32, height: u32) -> Result<Picture, String> {
        let camera = scene.camera.ok_or("no camera")?;
        let limit = self.device.limits().max_texture_dimension_2d;
        if width == 0 || height == 0 || width > limit || height > limit {
            return Err(format!("no picture of {width}×{height}"));
        }
        let device = &self.device;
        let size = wgpu::Extent3d { width, height, depth_or_array_layers: 1 };
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
        let samples = texture(FORMAT, SAMPLES, target);
        let resolved = texture(FORMAT, 1, target | wgpu::TextureUsages::COPY_SRC);
        let depth = texture(DEPTH, SAMPLES, target);

        // The frame: the camera, the eye, the fog (from a bit before the target to far past it,
        // at most this thick) and the light.
        let d: f32 = (0..3).map(|i| (camera.eye[i] - camera.target[i]).powi(2)).sum::<f32>().sqrt();
        let reach = [d * 0.75, d * 1.6, 0.85, 0.];
        let bg = rgb(theme::BACKGROUND);
        let mut frame: Vec<f32> = camera.matrix().iter().flatten().copied().collect();
        frame.extend([camera.eye[0], camera.eye[1], camera.eye[2], 0.]);
        frame.extend(reach);
        frame.extend([bg[0], bg[1], bg[2], 1.]);
        frame.extend([-0.45, 1., -0.3, 0.]);
        let uniform = buffer(device, &frame, wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST);
        self.queue.write_buffer(&uniform, 0, &bytes(&frame));
        let boxes: Vec<f32> = scene.blocks.iter().flat_map(instance).collect();
        let instances = buffer(device, &boxes, wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST);
        self.queue.write_buffer(&instances, 0, &bytes(&boxes));
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() }],
        });

        // Rows of the read back are padded to 256 bytes.
        let row = (width * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let read = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: u64::from(row) * u64::from(height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        {
            let view = samples.create_view(&Default::default());
            let into = resolved.create_view(&Default::default());
            let depth_view = depth.create_view(&Default::default());
            let [r, g, b] = bg.map(f64::from);
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: Some(&into),
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color { r, g, b, a: 1. }),
                        store: wgpu::StoreOp::Discard,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth_view,
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.), store: wgpu::StoreOp::Discard }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.set_vertex_buffer(0, instances.slice(..));
            pass.draw(0..36, 0..scene.blocks.len() as u32);
        }
        encoder.copy_texture_to_buffer(
            resolved.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &read,
                layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: None },
            },
            size,
        );
        self.queue.submit([encoder.finish()]);
        let (done, mapped) = std::sync::mpsc::channel();
        read.slice(..).map_async(wgpu::MapMode::Read, move |result| drop(done.send(result)));
        device.poll(wgpu::PollType::wait_indefinitely()).map_err(|e| format!("poll: {e}"))?;
        mapped.recv().map_err(|e| e.to_string())?.map_err(|e| format!("map: {e}"))?;
        let padded = read.slice(..).get_mapped_range().map_err(|e| format!("map: {e}"))?;
        let bgra = padded.chunks(row as usize).flat_map(|line| &line[..width as usize * 4]).copied().collect();
        Ok(Picture { width, height, bgra })
    }
}

fn instance(block: &Block) -> [f32; 10] {
    let [r, g, b] = colour(block.kind);
    let (lo, hi) = (block.min, block.max);
    let glows = if matches!(block.kind, Kind::Lit(_)) { 1. } else { 0. };
    [lo[0], lo[1], lo[2], hi[0], hi[1], hi[2], r, g, b, glows]
}

fn bytes(values: &[f32]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_ne_bytes()).collect()
}

fn buffer(device: &wgpu::Device, values: &[f32], usage: wgpu::BufferUsages) -> wgpu::Buffer {
    let size = (values.len() as u64 * 4).max(16);
    device.create_buffer(&wgpu::BufferDescriptor { label: None, size, usage, mapped_at_creation: false })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ottospace_core::atlas;

    /// Renders a made-up repository and writes it where `CITY_PICTURE` says, as a BMP, with the
    /// time each step took: `CITY_PICTURE=/tmp/city.bmp cargo test -p ottospace --release --
    /// --ignored a_city_is_rendered --nocapture`. Needs a GPU; no window.
    #[test]
    #[ignore = "needs a GPU; writes a picture and prints timings"]
    fn a_city_is_rendered() {
        let gpu = Gpu::new().expect("a GPU");
        for count in [28, 5_000, 50_000] {
            let files: Vec<(String, u64)> = (0..count)
                .map(|i| (format!("d{}/d{}/f{i}.rs", i % 7, i % 5), 300 + (i as u64 * 7919) % 40_000))
                .collect();
            let started = std::time::Instant::now();
            let plan = atlas::atlas(&atlas::repo(&files));
            let laid = started.elapsed();
            let lit = [(plan.places.iter().rev().find(|p| !p.group).unwrap().id.as_str(), 0xff6467)];
            let started = std::time::Instant::now();
            let scene = ottospace_core::scene::scene(&plan, &lit, 1.6);
            let built = started.elapsed();
            let (w, h) = (1600, 1000);
            let started = std::time::Instant::now();
            let picture = gpu.render(&scene, w, h).unwrap();
            let rendered = started.elapsed();
            println!(
                "{count} files: layout {laid:.1?}, scene {built:.1?} ({} boxes), render and read back {w}×{h} {rendered:.1?}",
                scene.blocks.len()
            );
            if let Some(path) = std::env::var_os("CITY_PICTURE").filter(|_| count == 5_000) {
                std::fs::write(path, bmp(&picture)).unwrap();
            }
        }
    }

    /// A 32-bit BMP, top row first: enough to look at a picture.
    fn bmp(p: &Picture) -> Vec<u8> {
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
