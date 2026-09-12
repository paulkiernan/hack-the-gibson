//! Instanced lane-pulse ribbons.

use crate::util::GpuCensus;
use crate::{shaders, RenderError};
use bytemuck::{Pod, Zeroable};
use gibson_types::PulseInstance;

/// Static quad vertex: position along the ribbon (0 tail ..= 1 head) and across (-1 ..= 1).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct PulseVertex {
    along: f32,
    across: f32,
}

const QUAD_ATTRS: [wgpu::VertexAttribute; 2] = wgpu::vertex_attr_array![0 => Float32, 1 => Float32];
// Explicit offsets: PulseInstance is 48 bytes packed position(0) length(12) direction(16)
// intensity(28) color(32) _pad(44, unread).
const PULSE_ATTRS: [wgpu::VertexAttribute; 5] = [
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x3,
        offset: 0,
        shader_location: 2, // position (head)
    },
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32,
        offset: 12,
        shader_location: 3, // length
    },
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x3,
        offset: 16,
        shader_location: 4, // direction
    },
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32,
        offset: 28,
        shader_location: 5, // intensity
    },
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x3,
        offset: 32,
        shader_location: 6, // color (per-beam HDR glow)
    },
];

fn vertex_layout() -> wgpu::VertexBufferLayout<'static> {
    wgpu::VertexBufferLayout {
        array_stride: 8,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &QUAD_ATTRS,
    }
}

/// Instance layout for `PulseInstance` (buffer 1).
fn instance_layout() -> wgpu::VertexBufferLayout<'static> {
    wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<PulseInstance>() as u64,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &PULSE_ATTRS,
    }
}

pub struct Pulses {
    pub pipeline: wgpu::RenderPipeline,
    geometry: wgpu::Buffer,
    instance: wgpu::Buffer,
    capacity: u32,
}

impl Pulses {
    pub fn new(
        device: &wgpu::Device,
        layout: &wgpu::PipelineLayout,
        color_format: wgpu::TextureFormat,
        depth_format: wgpu::TextureFormat,
        census: &mut GpuCensus,
    ) -> Result<Pulses, RenderError> {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("gibson-pulses-shader"),
            source: wgpu::ShaderSource::Wgsl(shaders::PULSES.into()),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("gibson-pulses-pipeline"),
            layout: Some(layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(vertex_layout()), Some(instance_layout())],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: depth_format,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: color_format,
                    blend: Some(wgpu::BlendState {
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
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        // Two triangles: (0,1,2) (1,3,2) with corners (0,-1),(1,-1),(0,1),(1,1).
        let verts: [[f32; 2]; 6] = [
            [0.0, -1.0],
            [1.0, -1.0],
            [0.0, 1.0],
            [1.0, -1.0],
            [1.0, 1.0],
            [0.0, 1.0],
        ];
        let geometry = census.create_buffer_init(
            device,
            &wgpu::util::BufferInitDescriptor {
                label: Some("gibson-pulse-quad"),
                contents: bytemuck::cast_slice(&verts),
                usage: wgpu::BufferUsages::VERTEX,
            },
        );
        let instance = census.create_buffer(
            device,
            &wgpu::BufferDescriptor {
                label: Some("gibson-pulse-instances"),
                size: 4,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            },
        );
        Ok(Pulses {
            pipeline,
            geometry,
            instance,
            capacity: 0,
        })
    }

    pub fn upload(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pulses: &[PulseInstance],
        census: &mut GpuCensus,
    ) {
        let count = pulses.len() as u32;
        if count == 0 {
            return;
        }
        if count > self.capacity {
            let capacity = count.next_power_of_two().max(256);
            self.instance = census.create_buffer(
                device,
                &wgpu::BufferDescriptor {
                    label: Some("gibson-pulse-instances"),
                    size: capacity as u64 * std::mem::size_of::<PulseInstance>() as u64,
                    usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                },
            );
            self.capacity = capacity;
        }
        queue.write_buffer(&self.instance, 0, bytemuck::cast_slice(pulses));
    }

    pub fn draw<'a>(&'a self, rp: &mut wgpu::RenderPass<'a>, count: u32) {
        if count == 0 {
            return;
        }
        rp.set_vertex_buffer(0, self.geometry.slice(..));
        rp.set_vertex_buffer(1, self.instance.slice(..));
        rp.draw(0..6, 0..count);
    }
}
