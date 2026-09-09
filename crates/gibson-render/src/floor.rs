//! PCB floor quad pass.

use crate::{shaders, RenderError};
use wgpu::util::DeviceExt;

/// A floor corner is a world-space (x, z) pair at y = 0.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct FloorVertex {
    xz: [f32; 2],
}

const CORNER_ATTRS: [wgpu::VertexAttribute; 1] =
    wgpu::vertex_attr_array![0 => Float32x2];

fn vertex_layout() -> wgpu::VertexBufferLayout<'static> {
    wgpu::VertexBufferLayout {
        array_stride: 8,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &CORNER_ATTRS,
    }
}

pub struct Floor {
    pub pipeline: wgpu::RenderPipeline,
    geometry: wgpu::Buffer,
    vertex_count: u32,
    /// Half-extent the geometry was built for (world units).
    grid: f32,
}

impl Floor {
    pub fn new(
        device: &wgpu::Device,
        layout: &wgpu::PipelineLayout,
        color_format: wgpu::TextureFormat,
        depth_format: wgpu::TextureFormat,
    ) -> Result<Floor, RenderError> {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("gibson-floor-shader"),
            source: wgpu::ShaderSource::Wgsl(shaders::FLOOR.into()),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("gibson-floor-pipeline"),
            layout: Some(layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(vertex_layout())],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: depth_format,
                depth_write_enabled: Some(true),
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
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let geometry = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gibson-floor-quad"),
            size: 48, // two triangles, six vec2 corners
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Ok(Floor {
            pipeline,
            geometry,
            vertex_count: 6,
            grid: 0.0,
        })
    }

    /// Rebuild the quad corners if the grid changed (extent = grid * 15 + 600).
    pub fn ensure_grid(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, grid: f32) {
        if (self.grid - grid).abs() < 0.5 && self.grid > 0.0 {
            return;
        }
        self.grid = grid;
        let half = grid * 15.0 + 600.0;
        let corners: [[f32; 2]; 6] = [
            [-half, -half],
            [half, -half],
            [-half, half],
            [half, -half],
            [half, half],
            [-half, half],
        ];
        let bytes: &[u8] = bytemuck::cast_slice(&corners);
        // Recreate (strides/sizes fixed) or simply write.
        if self.geometry.size() < bytes.len() as u64 {
            let geo = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("gibson-floor-quad"),
                contents: bytes,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            });
            self.geometry = geo;
        } else {
            queue.write_buffer(&self.geometry, 0, bytes);
        }
    }

    pub fn draw<'a>(&'a self, rp: &mut wgpu::RenderPass<'a>) {
        rp.set_vertex_buffer(0, self.geometry.slice(..));
        rp.draw(0..self.vertex_count, 0..1);
    }
}
