//! Small shared device helpers, including the GPU resource census.

use std::fmt;
use wgpu::util::DeviceExt;

/// Running totals of the GPU resources one renderer has created, with their byte sizes.
///
/// The frame path is meant to allocate nothing: every target, buffer and bind group is built
/// once -- at startup, or on a resize -- and reused forever after. That was an argument until
/// now; with a counter it is a measurement. A long run whose totals do not move has allocated
/// nothing per frame, which is exactly what a "does this leak GPU memory" report needs to know.
///
/// Why a census in the renderer rather than `MTLDevice.currentAllocatedSize` or
/// `ioreg -r -c IOAccelResource`: `currentAllocatedSize` is only reachable from the process
/// that owns the device (through Objective-C, which this portable crate has no bindings for)
/// and `ioreg` reports driver-side allocations across the whole system that are neither
/// attributable to one engine nor comparable run to run. Counting at the creation sites covers
/// every allocation this crate is responsible for, which is the only allocation set it can
/// govern.
///
/// It is per-renderer state rather than a process-wide counter: two engines in one process must
/// report their own totals, and a test that asserts "this operation allocates nothing" must not
/// be perturbed by a sibling test building a renderer on another thread.
///
/// These are creation totals, not live totals. Nothing frees a texture or buffer in the steady
/// state either, so a flat total means both "created nothing" and "accumulating nothing".
/// [`Renderer::gpu_census`](crate::Renderer::gpu_census) exposes it to hosts; the renderer also
/// logs it periodically.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GpuCensus {
    /// Textures created by this renderer.
    pub textures: u64,
    /// Bytes those textures cover (`mip_level_count` levels included).
    pub texture_bytes: u64,
    /// Buffers created by this renderer.
    pub buffers: u64,
    /// Bytes those buffers were created with.
    pub buffer_bytes: u64,
}

impl fmt::Display for GpuCensus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "textures={} ({:.1} MiB) buffers={} ({:.1} MiB)",
            self.textures,
            self.texture_bytes as f64 / (1024.0 * 1024.0),
            self.buffers,
            self.buffer_bytes as f64 / (1024.0 * 1024.0),
        )
    }
}

/// Bytes a texture of this shape covers, summed over its mip chain. Formats whose block size
/// wgpu cannot report (none are used here) count as zero rather than failing the census.
fn texture_bytes(desc: &wgpu::TextureDescriptor<'_>) -> u64 {
    let block = desc.format.block_copy_size(None).unwrap_or(0) as u64;
    let mut total = 0;
    for mip in 0..desc.mip_level_count {
        let w = (desc.size.width >> mip).max(1) as u64;
        let h = (desc.size.height >> mip).max(1) as u64;
        total += w * h * desc.size.depth_or_array_layers as u64;
    }
    total * block
}

impl GpuCensus {
    /// Create a texture and count it. Every texture this crate allocates goes through here or
    /// [`GpuCensus::create_buffer`], so the totals are complete for the renderer.
    pub fn create_texture(
        &mut self,
        device: &wgpu::Device,
        desc: &wgpu::TextureDescriptor<'_>,
    ) -> wgpu::Texture {
        let bytes = texture_bytes(desc);
        let texture = device.create_texture(desc);
        self.textures += 1;
        self.texture_bytes += bytes;
        texture
    }

    /// Create a buffer and count it.
    pub fn create_buffer(
        &mut self,
        device: &wgpu::Device,
        desc: &wgpu::BufferDescriptor<'_>,
    ) -> wgpu::Buffer {
        let buffer = device.create_buffer(desc);
        self.buffers += 1;
        self.buffer_bytes += desc.size;
        buffer
    }

    /// Create a buffer from CPU bytes (mapped-at-creation so no queue is needed) and count it.
    pub fn buffer_init(
        &mut self,
        device: &wgpu::Device,
        label: &str,
        contents: &[u8],
        usage: wgpu::BufferUsages,
    ) -> wgpu::Buffer {
        self.create_buffer_init(
            device,
            &wgpu::util::BufferInitDescriptor {
                label: Some(label),
                contents,
                usage,
            },
        )
    }

    /// Create a buffer from CPU bytes and count it.
    pub fn create_buffer_init(
        &mut self,
        device: &wgpu::Device,
        desc: &wgpu::util::BufferInitDescriptor<'_>,
    ) -> wgpu::Buffer {
        let buffer = device.create_buffer_init(desc);
        self.buffers += 1;
        self.buffer_bytes += desc.contents.len() as u64;
        buffer
    }
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
pub fn fs_triangle(device: &wgpu::Device, label: &str, census: &mut GpuCensus) -> wgpu::Buffer {
    const TRI: [[f32; 2]; 3] = [[-1.0, -1.0], [3.0, -1.0], [-1.0, 3.0]];
    census.buffer_init(
        device,
        label,
        bytemuck::cast_slice(&TRI),
        wgpu::BufferUsages::VERTEX,
    )
}
