//! Instanced translucent tower boxes.

use crate::{shaders, RenderError};
use bytemuck::{Pod, Zeroable};
use gibson_types::TowerInstance;
use wgpu::util::DeviceExt;

/// Per-vertex geometry data (static box; instancing adds the per-tower data).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct TowerVertex {
    pos: [f32; 3],
    face: u32,
    uv: [f32; 2],
}

const GEOM_ATTRS: [wgpu::VertexAttribute; 3] =
    wgpu::vertex_attr_array![0 => Float32x3, 1 => Uint32, 2 => Float32x2];
// Explicit offsets: `TowerInstance` is 52 bytes packed position(0) anim_phase(12)
// face_layers(16) top_layer(32) highlight_block(36) highlight_t(40) height(44) siege_t(48).
const INST_ATTRS: [wgpu::VertexAttribute; 8] = [
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x3,
        offset: 0,
        shader_location: 3, // position (base center)
    },
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32,
        offset: 12,
        shader_location: 4, // anim_phase
    },
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Uint32x4,
        offset: 16,
        shader_location: 5, // face_layers
    },
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Uint32,
        offset: 32,
        shader_location: 6, // top_layer
    },
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Uint32,
        offset: 36,
        shader_location: 7, // highlight_block
    },
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32,
        offset: 40,
        shader_location: 8, // highlight_t
    },
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32,
        offset: 44,
        shader_location: 9, // height (actual tower height in world units)
    },
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32,
        offset: 48,
        shader_location: 10, // siege_t (0 = normal palette, 1 = fully siege)
    },
];

// `INST_ATTRS` hard-codes the contract's byte offsets and the stride is
// `size_of::<TowerInstance>()`; a contract change that moved either must fail the build here
// rather than silently misdraw the skyline.
const _: () = {
    assert!(std::mem::size_of::<TowerInstance>() == 52);
    assert!(std::mem::offset_of!(TowerInstance, position) == 0);
    assert!(std::mem::offset_of!(TowerInstance, anim_phase) == 12);
    assert!(std::mem::offset_of!(TowerInstance, face_layers) == 16);
    assert!(std::mem::offset_of!(TowerInstance, top_layer) == 32);
    assert!(std::mem::offset_of!(TowerInstance, highlight_block) == 36);
    assert!(std::mem::offset_of!(TowerInstance, highlight_t) == 40);
    assert!(std::mem::offset_of!(TowerInstance, height) == 44);
    assert!(std::mem::offset_of!(TowerInstance, siege_t) == 48);
};

/// Geometry vertex attribute layout (buffer 0, per-vertex).
fn vertex_layout() -> wgpu::VertexBufferLayout<'static> {
    wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<TowerVertex>() as u64,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &GEOM_ATTRS,
    }
}

/// Instance attribute layout for `TowerInstance` (buffer 1, per-instance).
fn instance_layout() -> wgpu::VertexBufferLayout<'static> {
    wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<TowerInstance>() as u64,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &INST_ATTRS,
    }
}

/// Build the 20 unique corners of the five faces as 30 non-indexed vertices.
fn build_box() -> Vec<TowerVertex> {
    let mut v: Vec<TowerVertex> = Vec::with_capacity(30);
    let quad = |verts: &mut Vec<TowerVertex>, face: u32, c: [[f32; 5]; 4]| {
        // c[i] = [x, y, z, u, v]
        let p = |c: &[f32; 5]| TowerVertex {
            pos: [c[0], c[1], c[2]],
            face,
            uv: [c[3], c[4]],
        };
        verts.push(p(&c[0]));
        verts.push(p(&c[1]));
        verts.push(p(&c[2]));
        verts.push(p(&c[1]));
        verts.push(p(&c[3]));
        verts.push(p(&c[2]));
    };
    // Side faces: v = 0 at the tower top so the atlas' top row (panel top) is the tower top.
    // face 0: +x; u increases along +z.
    quad(&mut v, 0, [
        [1.0, -1.0, -1.0, 0.0, 1.0],
        [1.0, 1.0, -1.0, 0.0, 0.0],
        [1.0, -1.0, 1.0, 1.0, 1.0],
        [1.0, 1.0, 1.0, 1.0, 0.0],
    ]);
    // face 1: -x; u increases along +z.
    quad(&mut v, 1, [
        [-1.0, -1.0, -1.0, 0.0, 1.0],
        [-1.0, 1.0, -1.0, 0.0, 0.0],
        [-1.0, -1.0, 1.0, 1.0, 1.0],
        [-1.0, 1.0, 1.0, 1.0, 0.0],
    ]);
    // face 2: +z; u increases along +x.
    quad(&mut v, 2, [
        [-1.0, -1.0, 1.0, 0.0, 1.0],
        [-1.0, 1.0, 1.0, 0.0, 0.0],
        [1.0, -1.0, 1.0, 1.0, 1.0],
        [1.0, 1.0, 1.0, 1.0, 0.0],
    ]);
    // face 3: -z; u increases along -x (u = 0 at +x).
    quad(&mut v, 3, [
        [1.0, -1.0, -1.0, 0.0, 1.0],
        [1.0, 1.0, -1.0, 0.0, 0.0],
        [-1.0, -1.0, -1.0, 1.0, 1.0],
        [-1.0, 1.0, -1.0, 1.0, 0.0],
    ]);
    // face 4: top; u along +x, v along +z.
    quad(&mut v, 4, [
        [-1.0, 1.0, -1.0, 0.0, 0.0],
        [1.0, 1.0, -1.0, 1.0, 0.0],
        [-1.0, 1.0, 1.0, 0.0, 1.0],
        [1.0, 1.0, 1.0, 1.0, 1.0],
    ]);
    v
}

// Box geometry is a 2x2x2 unit cube. The vertex shader maps local x/z to +/- TOWER_W and local
// y in [-1, 1] to [0, instance height] (the instance position is the base center at y = 0), so
// one static box serves towers of every height.

/// The tower pass: static box geometry + a growable per-frame instance buffer.
pub struct Towers {
    pub pipeline: wgpu::RenderPipeline,
    geometry: wgpu::Buffer,
    instance: wgpu::Buffer,
    capacity: u32,
}

impl Towers {
    /// `layout` is the shared scene pipeline layout (bind group 0).
    pub fn new(
        device: &wgpu::Device,
        layout: &wgpu::PipelineLayout,
        color_format: wgpu::TextureFormat,
        depth_format: wgpu::TextureFormat,
    ) -> Result<Towers, RenderError> {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("gibson-towers-shader"),
            source: wgpu::ShaderSource::Wgsl(shaders::TOWERS.into()),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("gibson-towers-pipeline"),
            layout: Some(layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(vertex_layout()), Some(instance_layout())],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None, // back faces draw: text bleeds through the glass
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: depth_format,
                depth_write_enabled: Some(false), // sorted back-to-front instead
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
                            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                            operation: wgpu::BlendOperation::Add,
                        },
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let box_verts = build_box();
        let geometry_bytes = bytemuck::cast_slice(&box_verts);
        let geometry = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("gibson-tower-box"),
            contents: geometry_bytes,
            usage: wgpu::BufferUsages::VERTEX,
        });
        let instance = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gibson-tower-instances"),
            size: 4, // replaced on first upload
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Ok(Towers {
            pipeline,
            geometry,
            instance,
            capacity: 0,
        })
    }

    /// Grow (recreate) or refresh the instance buffer for this frame's tower list.
    pub fn upload(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, towers: &[TowerInstance]) {
        let count = towers.len() as u32;
        if count == 0 {
            return;
        }
        if count > self.capacity {
            let capacity = count.next_power_of_two().max(256);
            self.instance = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("gibson-tower-instances"),
                size: capacity as u64 * std::mem::size_of::<TowerInstance>() as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            self.capacity = capacity;
        }
        queue.write_buffer(
            &self.instance,
            0,
            bytemuck::cast_slice(towers),
        );
    }

    /// Record the instanced draw (caller sets the pipeline and bind group).
    pub fn draw<'a>(&'a self, rp: &mut wgpu::RenderPass<'a>, count: u32) {
        if count == 0 {
            return;
        }
        rp.set_vertex_buffer(0, self.geometry.slice(..));
        rp.set_vertex_buffer(1, self.instance.slice(..));
        rp.draw(0..30, 0..count);
    }
}
