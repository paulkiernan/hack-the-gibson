//! Bloom chain: prefilter at 1/2 res, 13-tap downsamples to 1/32, then additive 3x3-tent
//! upsamples back up so the final half-res level carries the whole soft bloom.

use crate::shaders;
use crate::targets::HDR_FORMAT;

/// Chain length: levels at 1/2, 1/4, 1/8, 1/16, 1/32 of the render resolution.
pub const LEVELS: usize = 5;

#[allow(dead_code)]
struct Level {
    texture: wgpu::Texture, // keeps the GPU allocation alive for `view`
    view: wgpu::TextureView,
}

pub struct Bloom {
    /// Bind group layout shared by every bloom pass (uniform + input texture + sampler).
    pub bgl: wgpu::BindGroupLayout,
    prefilter: wgpu::RenderPipeline,
    down: wgpu::RenderPipeline,
    up: wgpu::RenderPipeline,
    tri: wgpu::Buffer,
    levels: Vec<Level>,
    /// Bind group per input level (index k = levels[k] as the sampled input).
    input_bgs: Vec<wgpu::BindGroup>,
    /// Prefilter input: the full-res HDR scene texture.
    prefilter_bg: Option<wgpu::BindGroup>,
    width: u32,
    height: u32,
}

fn additive_blend() -> wgpu::BlendState {
    wgpu::BlendState {
        color: wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::One,
            operation: wgpu::BlendOperation::Add,
        },
        alpha: wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::One,
            operation: wgpu::BlendOperation::Add,
        },
    }
}

impl Bloom {
    pub fn new(device: &wgpu::Device) -> Bloom {
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("gibson-bloom-bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("gibson-bloom-layout"),
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });
        let prefilter = make_pipeline(device, &layout, "gibson-bloom-prefilter", shaders::BLOOM_PREFILTER, None);
        let down = make_pipeline(device, &layout, "gibson-bloom-down", shaders::BLOOM_DOWN, None);
        let up = make_pipeline(device, &layout, "gibson-bloom-up", shaders::BLOOM_UP, Some(additive_blend()));
        Bloom {
            bgl,
            prefilter,
            down,
            up,
            tri: crate::util::fs_triangle(device, "gibson-bloom-tri"),
            levels: Vec::new(),
            input_bgs: Vec::new(),
            prefilter_bg: None,
            width: 0,
            height: 0,
        }
    }

    /// Recreate the level textures and their bind groups for a new render size.
    pub fn rebuild(
        &mut self,
        device: &wgpu::Device,
        width: u32,
        height: u32,
        uniform: &wgpu::Buffer,
        sampler: &wgpu::Sampler,
        src_view: &wgpu::TextureView,
    ) {
        self.width = width;
        self.height = height;
        let mut levels = Vec::with_capacity(LEVELS);
        for i in 0..LEVELS {
            let w = (width >> (i + 1)).max(1);
            let h = (height >> (i + 1)).max(1);
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("gibson-bloom-level"),
                size: wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: HDR_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            levels.push(Level { texture, view });
        }
        let mut input_bgs = Vec::with_capacity(LEVELS);
        for lvl in &levels {
            input_bgs.push(make_input_bg(device, &self.bgl, uniform, &lvl.view, sampler));
        }
        let prefilter_bg = make_input_bg(device, &self.bgl, uniform, src_view, sampler);
        self.levels = levels;
        self.input_bgs = input_bgs;
        self.prefilter_bg = Some(prefilter_bg);
    }

    /// True when the level-0 (half res) target is at least 2x2.
    pub fn usable(&self) -> bool {
        self.width >= 4 && self.height >= 4
    }

    /// View of the accumulated half-res bloom (what the composite samples).
    pub fn level0_view(&self) -> Option<&wgpu::TextureView> {
        self.levels.first().map(|l| &l.view)
    }

    /// Record the whole bloom chain. `encoder` must not have an open pass.
    pub fn run(&self, encoder: &mut wgpu::CommandEncoder) {
        if self.levels.len() != LEVELS || !self.usable() {
            return;
        }
        let Some(prefilter_bg) = &self.prefilter_bg else {
            return;
        };
        // Prefilter: full-res HDR -> level 0 (1/2 res).
        self.blit(
            encoder,
            prefilter_bg,
            &self.prefilter,
            &self.levels[0],
            wgpu::LoadOp::Clear(wgpu::Color::BLACK),
        );
        // Downsample 1/2 -> 1/4 -> 1/8 -> 1/16 -> 1/32.
        for i in 0..LEVELS - 1 {
            self.blit(
                encoder,
                &self.input_bgs[i],
                &self.down,
                &self.levels[i + 1],
                wgpu::LoadOp::Clear(wgpu::Color::BLACK),
            );
        }
        // Upsample back up, accumulating additively onto each level.
        for i in (0..LEVELS - 1).rev() {
            self.blit(
                encoder,
                &self.input_bgs[i + 1],
                &self.up,
                &self.levels[i],
                wgpu::LoadOp::Load,
            );
        }
    }

    fn blit(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        bg: &wgpu::BindGroup,
        pipeline: &wgpu::RenderPipeline,
        target: &Level,
        load: wgpu::LoadOp<wgpu::Color>,
    ) {
        let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("gibson-bloom-pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &target.view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        rp.set_pipeline(pipeline);
        rp.set_bind_group(0, bg, &[]);
        rp.set_vertex_buffer(0, self.tri.slice(..));
        rp.draw(0..3, 0..1);
    }
}

fn make_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    label: &str,
    src: &str,
    blend: Option<wgpu::BlendState>,
) -> wgpu::RenderPipeline {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(label),
        source: wgpu::ShaderSource::Wgsl(src.into()),
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some("vs_main"),
            compilation_options: Default::default(),
            buffers: &[Some(crate::util::FS_VERTEX_LAYOUT)],
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some("fs_main"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: HDR_FORMAT,
                blend,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

fn make_input_bg(
    device: &wgpu::Device,
    bgl: &wgpu::BindGroupLayout,
    uniform: &wgpu::Buffer,
    view: &wgpu::TextureView,
    sampler: &wgpu::Sampler,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("gibson-bloom-input"),
        layout: bgl,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(view),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ],
    })
}
