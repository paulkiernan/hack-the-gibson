//! Small shared device helpers.

use wgpu::util::DeviceExt;

/// Create a GPU buffer from CPU bytes (mapped-at-creation so no queue is needed).
pub fn buffer_init(
    device: &wgpu::Device,
    label: &str,
    contents: &[u8],
    usage: wgpu::BufferUsages,
) -> wgpu::Buffer {
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some(label),
        contents,
        usage,
    })
}

/// Build the pipeline layout for a single bind-group pipeline.
pub fn pipeline_layout(
    device: &wgpu::Device,
    label: &str,
    group: &wgpu::BindGroupLayout,
) -> wgpu::PipelineLayout {
    device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some(label),
        bind_group_layouts: &[Some(group)],
        immediate_size: 0,
    })
}

/// Vertex layout for full-screen passes (one `vec2` position per vertex).
pub const FS_VERTEX_LAYOUT: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
    array_stride: 8,
    step_mode: wgpu::VertexStepMode::Vertex,
    attributes: &wgpu::vertex_attr_array![0 => Float32x2],
};

/// Full-screen triangle covering clip space: (-1,-1), (3,-1), (-1,3). Three vertices, no index.
pub fn fs_triangle(device: &wgpu::Device, label: &str) -> wgpu::Buffer {
    const TRI: [[f32; 2]; 3] = [[-1.0, -1.0], [3.0, -1.0], [-1.0, 3.0]];
    buffer_init(
        device,
        label,
        bytemuck::cast_slice(&TRI),
        wgpu::BufferUsages::VERTEX,
    )
}
