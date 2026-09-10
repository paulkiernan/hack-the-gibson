//! Optional GPU timestamp profiling for the frame chain.
//!
//! Enabled only when `GIBSON_PROFILE=1` is set in the environment *and* the adapter offers
//! `Features::TIMESTAMP_QUERY`, so the default device keeps requesting no features at all and
//! the WebGL2 envelope is untouched. Every render pass in the chain records a begin/end
//! timestamp pair (two query slots per pass); [`Profile::read`] resolves them and converts the
//! ticks to milliseconds so the per-pass cost is visible instead of guessed at.

/// Maximum number of timestamp slots (2 per pass; the chain uses well under half of these).
pub(crate) const MAX_TIMESTAMPS: u32 = 64;

/// One measured pass: a name plus the query indices holding its begin/end ticks.
#[derive(Clone, Copy)]
pub(crate) struct Mark {
    pub name: &'static str,
    pub begin: u32,
    pub end: u32,
}

/// Timestamp query set plus its resolve/readback buffers.
pub(crate) struct Profile {
    query_set: wgpu::QuerySet,
    /// 1x1 attachment for a trailing marker pass: on Metal an end-of-pass timestamp is only
    /// recorded at the *next* pass boundary, so without a final trivial pass the last real
    /// pass (composite) would never get an end timestamp.
    #[allow(dead_code)] // kept alive for `tail_view`
    tail_tex: wgpu::Texture,
    tail_view: wgpu::TextureView,
    resolve: wgpu::Buffer,
    readback: wgpu::Buffer,
    /// Nanoseconds per GPU timestamp tick.
    period_ns: f32,
    marks: Vec<Mark>,
    next: u32,
}

impl Profile {
    /// Create the query set and readback buffers. `queue` supplies the timestamp period.
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Profile {
        let bytes = (MAX_TIMESTAMPS as u64) * 8;
        let tail_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("gibson-profile-tail"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        Profile {
            query_set: device.create_query_set(&wgpu::QuerySetDescriptor {
                label: Some("gibson-profile-queries"),
                ty: wgpu::QueryType::Timestamp,
                count: MAX_TIMESTAMPS,
            }),
            resolve: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("gibson-profile-resolve"),
                size: bytes,
                usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            }),
            readback: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("gibson-profile-readback"),
                size: bytes,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            }),
            tail_view: tail_tex.create_view(&wgpu::TextureViewDescriptor::default()),
            tail_tex,
            period_ns: queue.get_timestamp_period(),
            marks: Vec::new(),
            next: 0,
        }
    }

    /// Start a new frame: forget the previous marks and rewind the slot cursor.
    pub fn reset(&mut self) {
        self.marks.clear();
        self.next = 0;
    }

    /// Reserve the query slots for one pass and remember its name. Returns the
    /// `RenderPassTimestampWrites` to attach to that pass, or `None` when the slots are
    /// exhausted (the chain is bounded well below the limit).
    ///
    /// Note on Metal (the primary target): wgpu-hal exposes `TIMESTAMP_QUERY` (pass
    /// boundaries) but *not* `TIMESTAMP_QUERY_INSIDE_ENCODERS`, and Metal's per-pass *begin*
    /// timestamps all read back as the command-buffer start. Only the *end* timestamps are
    /// ordered, so [`Profile::read`] derives each pass duration from the end-to-end deltas.
    pub fn pass(&mut self, name: &'static str) -> Option<wgpu::RenderPassTimestampWrites<'_>> {
        if self.next + 2 > MAX_TIMESTAMPS {
            return None;
        }
        let begin = self.next;
        let end = self.next + 1;
        self.next += 2;
        self.marks.push(Mark { name, begin, end });
        Some(wgpu::RenderPassTimestampWrites {
            query_set: &self.query_set,
            beginning_of_pass_write_index: Some(begin),
            end_of_pass_write_index: Some(end),
        })
    }

    /// The 1x1 attachment a trailing marker pass renders into (see the field docs).
    pub fn tail_view(&self) -> &wgpu::TextureView {
        &self.tail_view
    }

    /// Record the query resolve + copy into the readback buffer.
    pub fn resolve(&self, encoder: &mut wgpu::CommandEncoder) {
        if self.next == 0 {
            return;
        }
        encoder.resolve_query_set(&self.query_set, 0..self.next, &self.resolve, 0);
        encoder.copy_buffer_to_buffer(&self.resolve, 0, &self.readback, 0, (self.next as u64) * 8);
    }

    /// Map the readback buffer and decode the marks into (name, milliseconds) pairs.
    pub fn read(&self, device: &wgpu::Device) -> Vec<(&'static str, f64)> {
        let slice = self.readback.slice(..(self.next as u64) * 8);
        slice.map_async(wgpu::MapMode::Read, |r| r.expect("map profile buffer"));
        let _ = device.poll(wgpu::PollType::wait_indefinitely());
        let mapped = slice.get_mapped_range().expect("map profile buffer");
        let ticks: Vec<u64> = mapped
            .chunks_exact(8)
            .map(|c| u64::from_le_bytes(c.try_into().unwrap()))
            .collect();
        drop(mapped);
        self.readback.unmap();
        if std::env::var_os("GIBSON_PROFILE_RAW").is_some() {
            eprintln!("raw ticks (period {:.4} ns):", self.period_ns);
            for m in &self.marks {
                eprintln!(
                    "  {:>2}/{:<2} {:<20} begin={} end={} delta={}",
                    m.begin,
                    m.end,
                    m.name,
                    ticks.get(m.begin as usize).copied().unwrap_or(0),
                    ticks.get(m.end as usize).copied().unwrap_or(0),
                    ticks
                        .get(m.end as usize)
                        .copied()
                        .unwrap_or(0)
                        .saturating_sub(ticks.get(m.begin as usize).copied().unwrap_or(0)),
                );
            }
        }
        let period = self.period_ns as f64;
        // Passes run back to back, and only the end timestamps are trustworthy, so each pass's
        // cost is the gap between its end and the previous pass's end (the first pass measures
        // from the begin timestamp, which is the command-buffer start).
        let mut out = Vec::with_capacity(self.marks.len());
        let mut prev_end: Option<u64> = None;
        for m in &self.marks {
            let (Some(&begin), Some(&end)) =
                (ticks.get(m.begin as usize), ticks.get(m.end as usize))
            else {
                continue;
            };
            if end == 0 {
                // Pass not executed this frame (skipped by settings) leaves zero ticks.
                continue;
            }
            let start = prev_end.unwrap_or(begin);
            if end > start {
                out.push((m.name, (end - start) as f64 * period / 1.0e6));
            }
            prev_end = Some(end);
        }
        out
    }
}
