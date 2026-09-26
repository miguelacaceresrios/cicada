//! Render sin ventana con wgpu: un shader a pantalla completa sobre una
//! textura, que después se copia a memoria para mandarla a FFmpeg o a un PNG.

use bytemuck::{Pod, Zeroable};
use cicada_core::SectionKind;

use crate::VisualError;
use crate::timeline::FrameState;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Uniforms {
    frame: [f32; 4],
    pulses: [f32; 4],
    scene: [f32; 4],
    extra: [f32; 4],
}

/// Debe coincidir con las constantes de escena de `shader.wgsl`.
fn scene_id(kind: SectionKind) -> f32 {
    match kind {
        SectionKind::Intro | SectionKind::Outro | SectionKind::Main | SectionKind::Custom => 0.0,
        SectionKind::Build => 1.0,
        SectionKind::Drop => 2.0,
        SectionKind::Break => 3.0,
    }
}

pub struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    uniforms: wgpu::Buffer,
    target: wgpu::Texture,
    readback: wgpu::Buffer,
    width: u32,
    height: u32,
    padded_row: u32,
    pub adapter_name: String,
}

impl Gpu {
    pub fn new(width: u32, height: u32) -> Result<Gpu, VisualError> {
        pollster::block_on(Self::init(width, height))
    }

    async fn init(width: u32, height: u32) -> Result<Gpu, VisualError> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                ..Default::default()
            })
            .await
            .map_err(|e| VisualError(format!("no se encontró una GPU compatible: {e}")))?;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor::default())
            .await
            .map_err(|e| VisualError(format!("no se pudo abrir la GPU: {e}")))?;

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("cicada"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
        });

        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("uniforms"),
            size: size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniforms.as_entire_binding(),
            }],
        });

        let format = wgpu::TextureFormat::Rgba8Unorm;
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("cicada"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("frame"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });

        // Las filas copiadas a un buffer deben ir alineadas a 256 bytes.
        let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let padded_row = (width * 4).div_ceil(align) * align;
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: (padded_row * height) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        Ok(Gpu {
            adapter_name: adapter.get_info().name,
            device,
            queue,
            pipeline,
            bind_group,
            uniforms,
            target,
            readback,
            width,
            height,
            padded_row,
        })
    }

    /// Renderiza un frame y lo escribe en `out` como RGBA sin relleno.
    pub fn render(&self, s: &FrameState, out: &mut Vec<u8>) -> Result<(), VisualError> {
        let uniforms = Uniforms {
            frame: [
                self.width as f32,
                self.height as f32,
                s.time as f32,
                s.cycle as f32,
            ],
            pulses: [s.kick, s.snare, s.hat, s.melodic],
            scene: [
                scene_id(s.scene),
                scene_id(s.prev_scene),
                s.transition,
                s.energy,
            ],
            extra: [s.flash, s.level, s.note, s.progress],
        };
        self.queue
            .write_buffer(&self.uniforms, 0, bytemuck::bytes_of(&uniforms));

        let view = self
            .target
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
        encoder.copy_texture_to_buffer(
            self.target.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &self.readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(self.padded_row),
                    rows_per_image: Some(self.height),
                },
            },
            wgpu::Extent3d {
                width: self.width,
                height: self.height,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit([encoder.finish()]);

        let slice = self.readback.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|e| VisualError(format!("la GPU no respondió: {e}")))?;

        let row = (self.width * 4) as usize;
        out.clear();
        {
            let data = slice
                .get_mapped_range()
                .map_err(|e| VisualError(format!("no se pudo leer el frame de la GPU: {e}")))?;
            for line in data.chunks(self.padded_row as usize) {
                out.extend_from_slice(&line[..row]);
            }
        }
        self.readback.unmap();
        Ok(())
    }
}
