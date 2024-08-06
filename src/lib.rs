use ahash::AHashMap as HashMap;
use imgui::internal::RawWrapper;
use std::{
    borrow::Cow,
    cell::{Cell, Ref, RefCell, RefMut},
    mem::{replace, size_of, size_of_val},
    num::NonZeroU64,
    slice,
};

pub struct TextureDescriptor {
    pub width: u32,
    pub height: u32,
    pub mip_level_count: u32,
    pub format: wgpu::TextureFormat,
    pub usage: wgpu::TextureUsages,
}

impl TextureDescriptor {
    fn to_raw<'a>(&'a self, label: Option<&'a str>) -> wgpu::TextureDescriptor<'a> {
        wgpu::TextureDescriptor {
            label,
            size: wgpu::Extent3d {
                width: self.width,
                height: self.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: self.mip_level_count,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.format,
            usage: self.usage,
            view_formats: &[],
        }
    }

    fn bytes_per_row(&self) -> Option<u32> {
        let block_dimensions = self.format.block_dimensions();
        let block_size = self.format.block_copy_size(None)?;
        Some(self.width / block_dimensions.0 * block_size)
    }
}

impl Default for TextureDescriptor {
    fn default() -> Self {
        TextureDescriptor {
            width: 1,
            height: 1,
            mip_level_count: 1,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING,
        }
    }
}

pub struct SamplerDescriptor {
    pub address_mode_u: wgpu::AddressMode,
    pub address_mode_v: wgpu::AddressMode,
    pub mag_filter: wgpu::FilterMode,
    pub min_filter: wgpu::FilterMode,
    pub mipmap_filter: wgpu::FilterMode,
    pub lod_min_clamp: f32,
    pub lod_max_clamp: f32,
    pub anisotropy_clamp: u16,
    pub border_color: Option<wgpu::SamplerBorderColor>,
}

impl SamplerDescriptor {
    fn to_raw<'a>(&'a self, label: Option<&'a str>) -> wgpu::SamplerDescriptor<'a> {
        wgpu::SamplerDescriptor {
            label,
            address_mode_u: self.address_mode_u,
            address_mode_v: self.address_mode_v,
            mag_filter: self.mag_filter,
            min_filter: self.min_filter,
            mipmap_filter: self.mipmap_filter,
            lod_min_clamp: self.lod_min_clamp,
            lod_max_clamp: self.lod_max_clamp,
            anisotropy_clamp: self.anisotropy_clamp,
            border_color: self.border_color,
            ..Default::default()
        }
    }
}

impl Default for SamplerDescriptor {
    fn default() -> Self {
        SamplerDescriptor {
            address_mode_u: Default::default(),
            address_mode_v: Default::default(),
            mag_filter: Default::default(),
            min_filter: Default::default(),
            mipmap_filter: Default::default(),
            lod_min_clamp: 0.0,
            lod_max_clamp: std::f32::MAX,
            anisotropy_clamp: 1,
            border_color: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct TextureSetRange {
    pub mip_level: u32,
    pub x: u32,
    pub y: u32,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub offset: u64,
}

pub struct OwnedTexture {
    label: Option<Cow<'static, str>>,
    texture_desc: TextureDescriptor,
    texture_bytes_per_row: Option<u32>,
    texture_data: RefCell<Option<(wgpu::Texture, wgpu::TextureView)>>,
    sampler_desc: SamplerDescriptor,
    sampler: RefCell<Option<wgpu::Sampler>>,
    bind_group: RefCell<Option<wgpu::BindGroup>>,
}

macro_rules! owned_texture_texture_data {
    ($texture_data: expr, $self: expr, $device: expr) => {
        $texture_data.get_or_insert_with(|| {
            let raw_desc = $self.texture_desc.to_raw($self.label.as_deref());
            let texture = $device.create_texture(&raw_desc);
            let view = texture.create_view(&Default::default());
            (texture, view)
        })
    };
}

impl OwnedTexture {
    #[must_use]
    fn new(
        label: Option<Cow<'static, str>>,
        texture_desc: TextureDescriptor,
        sampler_desc: SamplerDescriptor,
    ) -> Self {
        OwnedTexture {
            label,
            texture_bytes_per_row: texture_desc.bytes_per_row(),
            texture_desc,
            texture_data: RefCell::new(None),
            sampler_desc,
            sampler: RefCell::new(None),
            bind_group: RefCell::new(None),
        }
    }

    pub fn label(&self) -> Option<&str> {
        self.label.as_deref()
    }

    pub fn set_label(&mut self, value: Option<Cow<'static, str>>) {
        self.label = value;
        *self.texture_data.get_mut() = None;
        *self.sampler.get_mut() = None;
        *self.bind_group.get_mut() = None;
    }

    pub fn texture_desc(&self) -> &TextureDescriptor {
        &self.texture_desc
    }

    pub fn set_texture_desc(&mut self, value: TextureDescriptor) {
        self.texture_desc = value;
        *self.texture_data.get_mut() = None;
        *self.bind_group.get_mut() = None;
    }

    pub fn texture_bytes_per_row(&mut self) -> Option<u32> {
        self.texture_bytes_per_row
    }

    pub fn sampler_desc(&self) -> &SamplerDescriptor {
        &self.sampler_desc
    }

    pub fn set_sampler_desc(&mut self, value: SamplerDescriptor) {
        self.sampler_desc = value;
        *self.sampler.get_mut() = None;
        *self.bind_group.get_mut() = None;
    }

    fn update_bind_group(&self, device: &wgpu::Device, bind_group_layout: &wgpu::BindGroupLayout) {
        let mut texture_data = self.texture_data.borrow_mut();
        let texture_view = &owned_texture_texture_data!(texture_data, self, device).1;
        let mut sampler = self.sampler.borrow_mut();
        let sampler = sampler.get_or_insert_with(|| {
            device.create_sampler(&self.sampler_desc.to_raw(self.label.as_deref()))
        });
        let mut bind_group = self.bind_group.borrow_mut();
        if bind_group.is_none() {
            *bind_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: self.label.as_deref(),
                layout: bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(texture_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(sampler),
                    },
                ],
            }));
        }
    }

    pub fn set_data(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        data: &[u8],
        range: TextureSetRange,
    ) {
        let mut texture_data = self.texture_data.borrow_mut();
        let texture = &owned_texture_texture_data!(texture_data, self, device).0;
        queue.write_texture(
            wgpu::ImageCopyTexture {
                texture,
                mip_level: range.mip_level,
                origin: wgpu::Origin3d {
                    x: range.x,
                    y: range.y,
                    z: 0,
                },
                aspect: wgpu::TextureAspect::All,
            },
            data,
            wgpu::ImageDataLayout {
                offset: range.offset,
                bytes_per_row: self.texture_bytes_per_row,
                rows_per_image: None,
            },
            wgpu::Extent3d {
                width: range.width.unwrap_or(self.texture_desc.width),
                height: range.height.unwrap_or(self.texture_desc.height),
                depth_or_array_layers: 1,
            },
        );
    }
}

pub struct TextureView {
    label: Option<Cow<'static, str>>,
    texture_view: wgpu::TextureView,
    sampler_desc: SamplerDescriptor,
    sampler: RefCell<Option<wgpu::Sampler>>,
    bind_group: RefCell<Option<wgpu::BindGroup>>,
}

impl TextureView {
    #[must_use]
    fn new(
        label: Option<Cow<'static, str>>,
        texture_view: wgpu::TextureView,
        sampler_desc: SamplerDescriptor,
    ) -> Self {
        TextureView {
            label,
            texture_view,
            sampler_desc,
            sampler: RefCell::new(None),
            bind_group: RefCell::new(None),
        }
    }

    pub fn label(&self) -> Option<&str> {
        self.label.as_deref()
    }

    pub fn set_label(&mut self, value: Option<Cow<'static, str>>) {
        self.label = value;
        *self.sampler.get_mut() = None;
        *self.bind_group.get_mut() = None;
    }

    pub fn texture_view(&self) -> &wgpu::TextureView {
        &self.texture_view
    }

    pub fn set_texture_view(&mut self, value: wgpu::TextureView) -> wgpu::TextureView {
        *self.bind_group.get_mut() = None;
        replace(&mut self.texture_view, value)
    }

    pub fn sampler_desc(&self) -> &SamplerDescriptor {
        &self.sampler_desc
    }

    pub fn set_sampler_desc(&mut self, value: SamplerDescriptor) {
        self.sampler_desc = value;
        *self.sampler.get_mut() = None;
        *self.bind_group.get_mut() = None;
    }

    fn update_bind_group(&self, device: &wgpu::Device, bind_group_layout: &wgpu::BindGroupLayout) {
        let mut sampler = self.sampler.borrow_mut();
        let sampler = sampler.get_or_insert_with(|| {
            device.create_sampler(&self.sampler_desc.to_raw(self.label.as_deref()))
        });
        let mut bind_group = self.bind_group.borrow_mut();
        if bind_group.is_none() {
            *bind_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: self.label.as_deref(),
                layout: bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&self.texture_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(sampler),
                    },
                ],
            }));
        }
    }
}

#[allow(clippy::large_enum_variant)]
pub enum Texture {
    Owned(OwnedTexture),
    View(TextureView),
}

macro_rules! unwrap {
    ($($fn: ident, $self: ty, $variant: ident, $ty: ty);*) => {
        $(
            pub fn $fn(self: $self) -> $ty {
                match self {
                    Texture::$variant(v) => v,
                    _ => panic!(),
                }
            }
        )*
    };
}

impl Texture {
    unwrap!(
        unwrap_owned, Self, Owned, OwnedTexture;
        unwrap_owned_ref, &Self, Owned, &OwnedTexture;
        unwrap_owned_mut, &mut Self, Owned, &mut OwnedTexture;
        unwrap_view, Self, View, TextureView;
        unwrap_view_ref, &Self, View, &TextureView;
        unwrap_view_mut, &mut Self, View, &mut TextureView
    );

    pub fn bind_group(
        &self,
        device: &wgpu::Device,
        bind_group_layout: &wgpu::BindGroupLayout,
    ) -> &wgpu::BindGroup {
        unsafe {
            match self {
                Texture::Owned(texture) => {
                    texture.update_bind_group(device, bind_group_layout);
                    texture
                        .bind_group
                        .try_borrow_unguarded()
                        .unwrap_unchecked()
                        .as_ref()
                        .unwrap_unchecked()
                }
                Texture::View(texture) => {
                    texture.update_bind_group(device, bind_group_layout);
                    texture
                        .bind_group
                        .try_borrow_unguarded()
                        .unwrap_unchecked()
                        .as_ref()
                        .unwrap_unchecked()
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SrgbMode {
    None,
    Linear,
    // TODO: Alpha blending is actually still very broken like this
    Srgb,
}

pub struct Renderer {
    view_buffer: wgpu::Buffer,
    view_bind_group: wgpu::BindGroup,
    texture_bind_group_layout: wgpu::BindGroupLayout,
    vtx_buffer: Option<wgpu::Buffer>,
    vtx_buffer_capacity: u64,
    idx_buffer: Option<wgpu::Buffer>,
    idx_buffer_capacity: u64,
    pipeline_layout: wgpu::PipelineLayout,
    shader_module: wgpu::ShaderModule,
    pipeline: wgpu::RenderPipeline,
    textures: RefCell<HashMap<imgui::TextureId, Texture>>,
    next_texture_id: Cell<usize>,
    srgb_mode: SrgbMode,
}

impl Renderer {
    fn rebuild_pipeline(
        device: &wgpu::Device,
        layout: &wgpu::PipelineLayout,
        shader_module: &wgpu::ShaderModule,
        output_format: wgpu::TextureFormat,
        srgb_mode: SrgbMode,
    ) -> wgpu::RenderPipeline {
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("ImGui"),
            layout: Some(layout),
            vertex: wgpu::VertexState {
                module: shader_module,
                entry_point: "vs_main",
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: 20,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x2,
                            offset: 0,
                            shader_location: 0,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x2,
                            offset: 8,
                            shader_location: 1,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Unorm8x4,
                            offset: 16,
                            shader_location: 2,
                        },
                    ],
                }],
                compilation_options: Default::default(),
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: shader_module,
                entry_point: "fs_main",
                targets: &[Some(wgpu::ColorTargetState {
                    format: output_format,
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: if srgb_mode == SrgbMode::Srgb {
                                wgpu::BlendFactor::One
                            } else {
                                wgpu::BlendFactor::SrcAlpha
                            },
                            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: if srgb_mode == SrgbMode::Srgb {
                            wgpu::BlendComponent::REPLACE
                        } else {
                            wgpu::BlendComponent {
                                src_factor: wgpu::BlendFactor::One,
                                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                                operation: wgpu::BlendOperation::Add,
                            }
                        },
                    }),
                    write_mask: wgpu::ColorWrites::all(),
                })],
                compilation_options: Default::default(),
            }),
            multiview: None,
            cache: None,
        })
    }

    #[must_use]
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        imgui: &mut imgui::Context,
        output_format: wgpu::TextureFormat,
        srgb_mode: SrgbMode,
    ) -> Self {
        imgui
            .io_mut()
            .backend_flags
            .insert(imgui::BackendFlags::RENDERER_HAS_VTX_OFFSET);

        let view_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("imgui view"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });
        let view_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("imgui view"),
            size: 16,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::UNIFORM,
            mapped_at_creation: false,
        });
        let view_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("imgui view"),
            layout: &view_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &view_buffer,
                    offset: 0,
                    size: Some(NonZeroU64::new(16).unwrap()),
                }),
            }],
        });
        let texture_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("imgui texture"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ImGui"),
            bind_group_layouts: &[&view_bind_group_layout, &texture_bind_group_layout],
            push_constant_ranges: &[],
        });
        let shader_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ImGui"),
            source: wgpu::ShaderSource::Wgsl(
                match srgb_mode {
                    SrgbMode::None => include_str!("imgui.wgsl"),
                    SrgbMode::Linear => include_str!("imgui-linear.wgsl"),
                    SrgbMode::Srgb => include_str!("imgui-srgb.wgsl"),
                }
                .into(),
            ),
        });
        let pipeline = Self::rebuild_pipeline(
            device,
            &pipeline_layout,
            &shader_module,
            output_format,
            srgb_mode,
        );

        let mut renderer = Renderer {
            view_buffer,
            view_bind_group,
            texture_bind_group_layout,
            pipeline_layout,
            shader_module,
            pipeline,
            textures: RefCell::new(HashMap::with_capacity(1)),
            next_texture_id: Cell::new(1),
            vtx_buffer: None,
            vtx_buffer_capacity: 0,
            idx_buffer: None,
            idx_buffer_capacity: 0,
            srgb_mode,
        };

        renderer.reload_fonts(device, queue, imgui);

        renderer
    }

    #[inline]
    pub fn change_swapchain_format(&mut self, device: &wgpu::Device, format: wgpu::TextureFormat) {
        self.pipeline = Self::rebuild_pipeline(
            device,
            &self.pipeline_layout,
            &self.shader_module,
            format,
            self.srgb_mode,
        );
    }

    #[inline]
    pub fn add_texture(&self, texture: Texture) -> imgui::TextureId {
        let id = self.next_texture_id.get();
        self.next_texture_id.set(id + 1);
        self.textures.borrow_mut().insert(id.into(), texture);
        id.into()
    }

    #[inline]
    pub fn create_owned_texture(
        &self,
        label: Option<Cow<'static, str>>,
        texture_desc: TextureDescriptor,
        sampler_desc: SamplerDescriptor,
    ) -> OwnedTexture {
        OwnedTexture::new(label, texture_desc, sampler_desc)
    }

    #[inline]
    pub fn create_and_add_owned_texture(
        &self,
        label: Option<Cow<'static, str>>,
        texture_desc: TextureDescriptor,
        sampler_desc: SamplerDescriptor,
    ) -> imgui::TextureId {
        let texture = self.create_owned_texture(label, texture_desc, sampler_desc);
        self.add_texture(Texture::Owned(texture))
    }

    #[inline]
    pub fn create_texture_view(
        &self,
        label: Option<Cow<'static, str>>,
        texture_view: wgpu::TextureView,
        sampler_desc: SamplerDescriptor,
    ) -> TextureView {
        TextureView::new(label, texture_view, sampler_desc)
    }

    #[inline]
    pub fn create_and_add_texture_view(
        &self,
        label: Option<Cow<'static, str>>,
        texture_view: wgpu::TextureView,
        sampler_desc: SamplerDescriptor,
    ) -> imgui::TextureId {
        let texture = self.create_texture_view(label, texture_view, sampler_desc);
        self.add_texture(Texture::View(texture))
    }

    #[inline]
    pub fn remove_texture(&self, id: imgui::TextureId) -> Option<Texture> {
        self.textures.borrow_mut().remove(&id)
    }

    #[inline]
    pub fn texture(&self, id: imgui::TextureId) -> Ref<Texture> {
        Ref::map(self.textures.borrow(), |textures| &textures[&id])
    }

    #[inline]
    pub fn texture_mut(&self, id: imgui::TextureId) -> RefMut<Texture> {
        RefMut::map(self.textures.borrow_mut(), |textures| {
            textures.get_mut(&id).unwrap()
        })
    }

    pub fn reload_fonts(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        imgui: &mut imgui::Context,
    ) {
        let font_tex_id = imgui.fonts().tex_id;
        if font_tex_id.id() != 0 {
            self.textures.get_mut().remove(&font_tex_id);
        }
        let fonts = imgui.fonts();
        let font_atlas = fonts.build_rgba32_texture();
        let font_texture = self.create_owned_texture(
            Some("ImGui font atlas".into()),
            TextureDescriptor {
                width: font_atlas.width,
                height: font_atlas.height,
                mip_level_count: 1,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING,
            },
            SamplerDescriptor {
                min_filter: wgpu::FilterMode::Linear,
                mag_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            },
        );
        font_texture.set_data(device, queue, font_atlas.data, TextureSetRange::default());
        fonts.clear_tex_data();
        self.textures
            .get_mut()
            .insert(font_tex_id, Texture::Owned(font_texture));
    }

    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        frame: &wgpu::TextureView,
        draw_data: &imgui::DrawData,
    ) {
        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: None,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: frame,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });

        if draw_data.total_vtx_count == 0 || draw_data.total_idx_count == 0 {
            return;
        }

        let fb_width = draw_data.display_size[0] * draw_data.framebuffer_scale[0];
        let fb_height = draw_data.display_size[1] * draw_data.framebuffer_scale[1];
        if fb_width <= 0.0 || fb_height <= 0.0 {
            return;
        }

        let mut vtx_size = draw_data.total_vtx_count as u64 * size_of::<imgui::DrawVert>() as u64;
        vtx_size += wgpu::COPY_BUFFER_ALIGNMENT - 1;
        vtx_size -= vtx_size % wgpu::COPY_BUFFER_ALIGNMENT;
        let mut idx_size = draw_data.total_idx_count as u64 * size_of::<imgui::DrawIdx>() as u64;
        idx_size += wgpu::COPY_BUFFER_ALIGNMENT - 1;
        idx_size -= idx_size % wgpu::COPY_BUFFER_ALIGNMENT;

        if self.vtx_buffer.is_none() || vtx_size > self.vtx_buffer_capacity {
            self.vtx_buffer.take();
            self.vtx_buffer_capacity = vtx_size.next_power_of_two();
            self.vtx_buffer = Some(device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: self.vtx_buffer_capacity,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
        }
        let vtx_buffer = self.vtx_buffer.as_ref().unwrap();

        if self.idx_buffer.is_none() || idx_size > self.idx_buffer_capacity {
            self.idx_buffer.take();
            self.idx_buffer_capacity = idx_size.next_power_of_two();
            self.idx_buffer = Some(device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: self.idx_buffer_capacity,
                usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
        }
        let idx_buffer = self.idx_buffer.as_ref().unwrap();

        let mut vtx = Vec::with_capacity(vtx_size as usize);
        let mut idx = Vec::with_capacity(idx_size as usize);
        for draw_list in draw_data.draw_lists() {
            let vtx_buffer = draw_list.vtx_buffer();
            let idx_buffer = draw_list.idx_buffer();
            unsafe {
                vtx.extend_from_slice(slice::from_raw_parts(
                    vtx_buffer.as_ptr() as *const u8,
                    size_of_val(vtx_buffer),
                ));
                idx.extend_from_slice(slice::from_raw_parts(
                    idx_buffer.as_ptr() as *const u8,
                    size_of_val(idx_buffer),
                ));
            }
        }
        vtx.resize(vtx_size as usize, 0);
        idx.resize(idx_size as usize, 0);
        queue.write_buffer(vtx_buffer, 0, &vtx);
        queue.write_buffer(idx_buffer, 0, &idx);

        render_pass.set_pipeline(&self.pipeline);
        render_pass.set_index_buffer(
            idx_buffer.slice(..),
            if size_of::<imgui::DrawIdx>() == 2 {
                wgpu::IndexFormat::Uint16
            } else {
                wgpu::IndexFormat::Uint32
            },
        );
        render_pass.set_viewport(0.0, 0.0, fb_width, fb_height, 0.0, 1.0);

        let scale = [
            2.0 / draw_data.display_size[0],
            2.0 / draw_data.display_size[1],
        ];
        let scale_translate = [
            scale[0],
            scale[1],
            -1.0 - draw_data.display_pos[0] * scale[0],
            -1.0 - draw_data.display_pos[1] * scale[1],
        ];
        unsafe {
            queue.write_buffer(
                &self.view_buffer,
                0,
                slice::from_raw_parts(scale_translate.as_ptr() as *const u8, 16),
            );
        }
        render_pass.set_bind_group(0, &self.view_bind_group, &[]);

        let textures = self.textures.get_mut();
        let mut vtx_base = 0;
        let mut idx_base = 0;
        for draw_list in draw_data.draw_lists() {
            for cmd in draw_list.commands() {
                match cmd {
                    imgui::DrawCmd::Elements { count, cmd_params } => {
                        let texture = match textures.get(&cmd_params.texture_id) {
                            Some(texture) => texture,
                            None => continue,
                        };

                        render_pass.set_vertex_buffer(0, vtx_buffer.slice(..));

                        let clip_rect = [
                            (cmd_params.clip_rect[0] - draw_data.display_pos[0])
                                * draw_data.framebuffer_scale[0],
                            (cmd_params.clip_rect[1] - draw_data.display_pos[1])
                                * draw_data.framebuffer_scale[1],
                            (cmd_params.clip_rect[2] - draw_data.display_pos[0])
                                * draw_data.framebuffer_scale[0],
                            (cmd_params.clip_rect[3] - draw_data.display_pos[1])
                                * draw_data.framebuffer_scale[1],
                        ];
                        if clip_rect[0] >= fb_width
                            || clip_rect[1] >= fb_height
                            || clip_rect[2] <= 0.0
                            || clip_rect[3] <= 0.0
                        {
                            continue;
                        }

                        let scissor_size = [
                            (clip_rect[2] - clip_rect[0]).abs().min(fb_width).ceil() as u32,
                            (clip_rect[3] - clip_rect[1]).abs().min(fb_height).ceil() as u32,
                        ];

                        if scissor_size[0] == 0 || scissor_size[1] == 0 {
                            continue;
                        }

                        render_pass.set_scissor_rect(
                            clip_rect[0].max(0.0).floor() as u32,
                            clip_rect[1].max(0.0).floor() as u32,
                            scissor_size[0],
                            scissor_size[1],
                        );

                        render_pass.set_bind_group(
                            1,
                            texture.bind_group(device, &self.texture_bind_group_layout),
                            &[],
                        );

                        let idx_start = idx_base + cmd_params.idx_offset;
                        render_pass.draw_indexed(
                            idx_start as u32..(idx_start + count) as u32,
                            (vtx_base + cmd_params.vtx_offset) as i32,
                            0..1,
                        );
                    }

                    imgui::DrawCmd::ResetRenderState => {
                        render_pass.set_pipeline(&self.pipeline);
                        render_pass.set_index_buffer(
                            idx_buffer.slice(..),
                            if size_of::<imgui::DrawIdx>() == 2 {
                                wgpu::IndexFormat::Uint16
                            } else {
                                wgpu::IndexFormat::Uint32
                            },
                        );
                        render_pass.set_viewport(0.0, 0.0, fb_width, fb_height, 0.0, 1.0);
                        render_pass.set_bind_group(0, &self.view_bind_group, &[]);
                    }

                    imgui::DrawCmd::RawCallback { callback, raw_cmd } => unsafe {
                        callback(draw_list.raw(), raw_cmd);
                    },
                }
            }
            vtx_base += draw_list.vtx_buffer().len();
            idx_base += draw_list.idx_buffer().len();
        }
    }
}
