//! Motion-blur and composite passes.
//!
//! Motion blur reconstructs world positions from the depth buffer and smears the HDR color along
//! the reprojected velocity into `color_b`. Composite tonemaps and outputs to the final target
//! (presentable surface or offscreen sRGB texture), so it is cached per output format.

use crate::shaders;
use crate::targets::HDR_FORMAT;
use crate::util::{FS_VERTEX_LAYOUT, fs_triangle};
use std::collections::HashMap;

pub struct Post {
    pub motion_bgl: wgpu::BindGroupLayout,
    pub composite_bgl: wgpu::BindGroupLayout,
    pub crt_bgl: wgpu::BindGroupLayout,
    pub sampler: wgpu::Sampler,
    motion: wgpu::RenderPipeline,
    composites: HashMap<wgpu::TextureFormat, wgpu::RenderPipeline>,
    crts: HashMap<wgpu::TextureFormat, wgpu::RenderPipeline>,
    tri: wgpu::Buffer,
}

fn uniform_binding(visibility: wgpu::ShaderStages) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding: 0,
        visibility,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn texture_binding(
    binding: u32,
    visibility: wgpu::ShaderStages,
    filterable: bool,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn sampler_binding(binding: u32, visibility: wgpu::ShaderStages) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        count: None,
    }
}

impl Post {
    pub fn new(device: &wgpu::Device) -> Post {
        let motion_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("gibson-motion-bgl"),
            entries: &[
                uniform_binding(wgpu::ShaderStages::FRAGMENT),
                texture_binding(1, wgpu::ShaderStages::FRAGMENT, true),
                sampler_binding(2, wgpu::ShaderStages::FRAGMENT),
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        let composite_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("gibson-composite-bgl"),
            entries: &[
                uniform_binding(wgpu::ShaderStages::FRAGMENT),
                texture_binding(1, wgpu::ShaderStages::FRAGMENT, true),
                sampler_binding(2, wgpu::ShaderStages::FRAGMENT),
                texture_binding(3, wgpu::ShaderStages::FRAGMENT, true),
            ],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("gibson-hdr-sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });
        let tri = fs_triangle(device, "gibson-post-tri");

        // The CRT pass reads the signal buffer with explicit `textureLoad`s (nearest by
        // construction), so its bind group is just the uniform plus the texture.
        let crt_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("gibson-crt-bgl"),
            entries: &[
                uniform_binding(wgpu::ShaderStages::FRAGMENT),
                texture_binding(1, wgpu::ShaderStages::FRAGMENT, false),
            ],
        });

        let motion_layout = crate::util::pipeline_layout(device, "gibson-motion-layout", &motion_bgl);
        let composite_layout =
            crate::util::pipeline_layout(device, "gibson-composite-layout", &composite_bgl);

        let motion = make_pipeline(
            device,
            &motion_layout,
            "gibson-motion",
            shaders::MOTION_BLUR,
            HDR_FORMAT,
        );
        // Composite pipelines are created lazily per final target format; seed the cache with
        // the motion format so the common path is prewarmed.
        let mut composites = HashMap::new();
        composites.insert(
            HDR_FORMAT,
            make_pipeline(
                device,
                &composite_layout,
                "gibson-composite",
                shaders::COMPOSITE,
                HDR_FORMAT,
            ),
        );
        // The CRT pass writes the final target only, so its pipelines are cached per output
        // format exactly like the composite's.
        let crts = HashMap::new();
        Post {
            motion_bgl,
            composite_bgl,
            crt_bgl,
            sampler,
            motion,
            composites,
            crts,
            tri,
        }
    }

    /// The render pipeline for a given final output format (cached).
    pub fn composite_pipeline(&mut self, device: &wgpu::Device, format: wgpu::TextureFormat) -> &wgpu::RenderPipeline {
        if !self.composites.contains_key(&format) {
            let layout = crate::util::pipeline_layout(
                device,
                "gibson-composite-layout",
                &self.composite_bgl,
            );
            let pipeline = make_pipeline(
                device,
                &layout,
                "gibson-composite",
                shaders::COMPOSITE,
                format,
            );
            self.composites.insert(format, pipeline);
        }
        &self.composites[&format]
    }

    pub fn motion(&self) -> &wgpu::RenderPipeline {
        &self.motion
    }

    /// The CRT pass's pipeline for a given final output format (cached).
    pub fn crt_pipeline(
        &mut self,
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
    ) -> &wgpu::RenderPipeline {
        if !self.crts.contains_key(&format) {
            let layout =
                crate::util::pipeline_layout(device, "gibson-crt-layout", &self.crt_bgl);
            let pipeline =
                make_pipeline(device, &layout, "gibson-crt", shaders::CRT, format);
            self.crts.insert(format, pipeline);
        }
        &self.crts[&format]
    }

    pub fn triangle(&self) -> &wgpu::Buffer {
        &self.tri
    }
}

fn make_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    label: &str,
    src: &str,
    format: wgpu::TextureFormat,
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
            buffers: &[Some(FS_VERTEX_LAYOUT)],
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some("fs_main"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}
