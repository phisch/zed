use crate::{
    CompositorGpuHint, WgpuContext,
    vello_scene::{VelloResourceCache, rebuild_vello_scene},
};
use gpui::{DevicePixels, GpuSpecs, Scene, Size};
use parking_lot::Mutex;
#[cfg(not(target_family = "wasm"))]
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use std::{cell::RefCell, rc::Rc, sync::Arc, time::Instant};

pub struct WgpuSurfaceConfig {
    pub size: Size<DevicePixels>,
    pub transparent: bool,
    pub preferred_present_mode: Option<wgpu::PresentMode>,
}

pub type GpuContext = Rc<RefCell<Option<WgpuContext>>>;

struct WgpuResources {
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    surface: wgpu::Surface<'static>,
    vello_renderer: vello::Renderer,
    vello_scene: vello::Scene,
    vello_resource_cache: VelloResourceCache,
    vello_target: Option<(wgpu::Texture, wgpu::TextureView)>,
    vello_blitter: wgpu::util::TextureBlitter,
}

impl WgpuResources {
    fn invalidate_target(&mut self) {
        self.vello_target = None;
    }
}

pub struct WgpuRenderer {
    #[cfg(not(target_family = "wasm"))]
    context: Option<GpuContext>,
    #[cfg(not(target_family = "wasm"))]
    compositor_gpu: Option<CompositorGpuHint>,
    resources: Option<WgpuResources>,
    surface_config: wgpu::SurfaceConfiguration,
    adapter_info: wgpu::AdapterInfo,
    transparent_alpha_mode: wgpu::CompositeAlphaMode,
    opaque_alpha_mode: wgpu::CompositeAlphaMode,
    max_texture_size: u32,
    last_error: Arc<Mutex<Option<String>>>,
    device_lost: Arc<std::sync::atomic::AtomicBool>,
    surface_configured: bool,
    needs_redraw: bool,
}

impl WgpuRenderer {
    #[cfg(not(target_family = "wasm"))]
    pub fn new<W>(
        gpu_context: GpuContext,
        window: &W,
        config: WgpuSurfaceConfig,
        compositor_gpu: Option<CompositorGpuHint>,
    ) -> anyhow::Result<Self>
    where
        W: HasWindowHandle + HasDisplayHandle + std::fmt::Debug + Send + Sync + Clone + 'static,
    {
        let window_handle = window
            .window_handle()
            .map_err(|error| anyhow::anyhow!("failed to get window handle: {error}"))?;
        let instance = gpu_context
            .borrow()
            .as_ref()
            .map(|context| context.instance.clone())
            .unwrap_or_else(|| WgpuContext::instance(Box::new(window.clone())));
        let surface = create_surface(&instance, window_handle.as_raw())?;

        let mut context_slot = gpu_context.borrow_mut();
        let context = match context_slot.as_mut() {
            Some(context) => {
                context.check_compatible_with_surface(&surface)?;
                context
            }
            None => context_slot.insert(WgpuContext::new(instance, &surface, compositor_gpu)?),
        };
        Self::new_internal(
            Some(Rc::clone(&gpu_context)),
            context,
            surface,
            config,
            compositor_gpu,
        )
    }

    #[cfg(target_family = "wasm")]
    pub fn new_from_canvas(
        context: &WgpuContext,
        canvas: &web_sys::HtmlCanvasElement,
        config: WgpuSurfaceConfig,
    ) -> anyhow::Result<Self> {
        let surface = context
            .instance
            .create_surface(wgpu::SurfaceTarget::Canvas(canvas.clone()))
            .map_err(|error| anyhow::anyhow!("failed to create web surface: {error}"))?;
        Self::new_internal(None, context, surface, config, None)
    }

    fn new_internal(
        gpu_context: Option<GpuContext>,
        context: &WgpuContext,
        surface: wgpu::Surface<'static>,
        config: WgpuSurfaceConfig,
        compositor_gpu: Option<CompositorGpuHint>,
    ) -> anyhow::Result<Self> {
        #[cfg(target_family = "wasm")]
        let _ = (&gpu_context, &compositor_gpu);
        let capabilities = surface.get_capabilities(&context.adapter);
        let surface_format = [
            wgpu::TextureFormat::Bgra8Unorm,
            wgpu::TextureFormat::Rgba8Unorm,
        ]
        .into_iter()
        .find(|format| capabilities.formats.contains(format))
        .or_else(|| {
            capabilities
                .formats
                .iter()
                .find(|format| !format.is_srgb())
                .copied()
        })
        .or_else(|| capabilities.formats.first().copied())
        .ok_or_else(|| anyhow::anyhow!("surface reports no supported texture formats"))?;

        let select_alpha_mode = |preferences: &[wgpu::CompositeAlphaMode]| {
            preferences
                .iter()
                .find(|preference| capabilities.alpha_modes.contains(preference))
                .copied()
        };
        let opaque_alpha_mode = select_alpha_mode(&[
            wgpu::CompositeAlphaMode::Opaque,
            wgpu::CompositeAlphaMode::Inherit,
        ])
        .ok_or_else(|| {
            anyhow::anyhow!(
                "surface does not support opaque composition; supported modes: {:?}",
                capabilities.alpha_modes
            )
        })?;
        let transparent_alpha_mode = select_alpha_mode(&[
            wgpu::CompositeAlphaMode::PreMultiplied,
            wgpu::CompositeAlphaMode::PostMultiplied,
            wgpu::CompositeAlphaMode::Inherit,
        ])
        .or_else(|| (!config.transparent).then_some(opaque_alpha_mode))
        .ok_or_else(|| {
            anyhow::anyhow!(
                "surface does not support transparent composition; supported modes: {:?}",
                capabilities.alpha_modes
            )
        })?;
        let alpha_mode = if config.transparent {
            transparent_alpha_mode
        } else {
            opaque_alpha_mode
        };

        let max_texture_size = context.device.limits().max_texture_dimension_2d;
        let width = surface_dimension(config.size.width, max_texture_size);
        let height = surface_dimension(config.size.height, max_texture_size);
        let present_mode = config
            .preferred_present_mode
            .filter(|mode| capabilities.present_modes.contains(mode))
            .unwrap_or(wgpu::PresentMode::Fifo);
        let surface_config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width,
            height,
            present_mode,
            desired_maximum_frame_latency: 2,
            alpha_mode,
            view_formats: vec![],
        };
        surface.configure(&context.device, &surface_config);

        let device = Arc::clone(&context.device);
        let queue = Arc::clone(&context.queue);
        let vello_renderer = vello::Renderer::new(
            &device,
            vello::RendererOptions {
                antialiasing_support: vello::AaSupport::area_only(),
                ..Default::default()
            },
        )
        .map_err(|error| anyhow::anyhow!("failed to create Vello renderer: {error}"))?;
        let vello_blitter = wgpu::util::TextureBlitter::new(&device, surface_format);
        let last_error = Arc::new(Mutex::new(None));
        device.on_uncaptured_error({
            let last_error = Arc::clone(&last_error);
            Arc::new(move |error| {
                log::warn!("uncaptured WGPU error during Vello rendering: {error}");
                *last_error.lock() = Some(error.to_string());
            })
        });

        let resources = WgpuResources {
            device,
            queue,
            surface,
            vello_renderer,
            vello_scene: vello::Scene::new(),
            vello_resource_cache: VelloResourceCache::default(),
            vello_target: None,
            vello_blitter,
        };

        Ok(Self {
            #[cfg(not(target_family = "wasm"))]
            context: gpu_context,
            #[cfg(not(target_family = "wasm"))]
            compositor_gpu,
            resources: Some(resources),
            surface_config,
            adapter_info: context.adapter.get_info(),
            transparent_alpha_mode,
            opaque_alpha_mode,
            max_texture_size,
            last_error,
            device_lost: context.device_lost_flag(),
            surface_configured: true,
            needs_redraw: false,
        })
    }

    pub fn update_drawable_size(&mut self, size: Size<DevicePixels>) {
        let width = surface_dimension(size.width, self.max_texture_size);
        let height = surface_dimension(size.height, self.max_texture_size);
        if width == self.surface_config.width && height == self.surface_config.height {
            return;
        }

        self.surface_config.width = width;
        self.surface_config.height = height;
        let config = self.surface_config.clone();
        if let Some(resources) = self.resources.as_mut() {
            resources.surface.configure(&resources.device, &config);
            resources.invalidate_target();
        }
    }

    pub fn update_transparency(&mut self, transparent: bool) {
        let alpha_mode = if transparent {
            self.transparent_alpha_mode
        } else {
            self.opaque_alpha_mode
        };
        if alpha_mode == self.surface_config.alpha_mode {
            return;
        }

        self.surface_config.alpha_mode = alpha_mode;
        let config = self.surface_config.clone();
        if let Some(resources) = self.resources.as_mut() {
            resources.surface.configure(&resources.device, &config);
            resources.invalidate_target();
        }
    }

    pub fn viewport_size(&self) -> Size<DevicePixels> {
        Size {
            width: DevicePixels(self.surface_config.width as i32),
            height: DevicePixels(self.surface_config.height as i32),
        }
    }

    pub fn gpu_specs(&self) -> GpuSpecs {
        GpuSpecs {
            is_software_emulated: self.adapter_info.device_type == wgpu::DeviceType::Cpu,
            device_name: self.adapter_info.name.clone(),
            driver_name: self.adapter_info.driver.clone(),
            driver_info: self.adapter_info.driver_info.clone(),
        }
    }

    pub fn max_texture_size(&self) -> u32 {
        self.max_texture_size
    }

    pub fn draw(&mut self, scene: &Scene) -> bool {
        let frame_started = Instant::now();
        if let Some(error) = self.last_error.lock().take() {
            log::error!("GPU error during Vello frame: {error}");
            if let Some(resources) = self.resources.as_mut() {
                resources.invalidate_target();
            }
            self.needs_redraw = true;
            return false;
        }
        if !self.surface_configured {
            return false;
        }

        let Some(resources) = self.resources.as_mut() else {
            log::error!("attempted to draw after WGPU renderer destruction");
            return false;
        };
        let frame = match resources.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame) => frame,
            wgpu::CurrentSurfaceTexture::Suboptimal(frame) => {
                log::warn!("Vello acquired a suboptimal surface texture; reconfiguring");
                drop(frame);
                resources
                    .surface
                    .configure(&resources.device, &self.surface_config);
                resources.invalidate_target();
                self.needs_redraw = true;
                return false;
            }
            wgpu::CurrentSurfaceTexture::Lost | wgpu::CurrentSurfaceTexture::Outdated => {
                log::warn!("Vello surface was lost or outdated; reconfiguring");
                resources
                    .surface
                    .configure(&resources.device, &self.surface_config);
                resources.invalidate_target();
                self.needs_redraw = true;
                return false;
            }
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                log::warn!("Vello surface was unavailable due to timeout or occlusion");
                self.needs_redraw = true;
                return false;
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                *self.last_error.lock() = Some("Vello surface texture validation error".to_owned());
                self.needs_redraw = true;
                return false;
            }
        };
        let acquired_at = Instant::now();

        let width = self.surface_config.width;
        let height = self.surface_config.height;
        if resources.vello_target.is_none() {
            let texture = resources.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("vello_target"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            resources.vello_target = Some((texture, view));
        }

        let target_ready_at = Instant::now();
        let scene_build = rebuild_vello_scene(
            &mut resources.vello_scene,
            &mut resources.vello_resource_cache,
            scene,
        );
        let scene_built_at = Instant::now();
        if !scene_build.unsupported.is_empty() {
            log::debug!(
                "Vello frame omitted unsupported GPUI primitives: {:?}",
                scene_build.unsupported
            );
        }
        let Some((_, target_view)) = resources.vello_target.as_ref() else {
            log::error!("Vello target was not initialized");
            self.needs_redraw = true;
            return false;
        };
        if let Err(error) = resources.vello_renderer.render_to_texture(
            &resources.device,
            &resources.queue,
            &resources.vello_scene,
            target_view,
            &vello::RenderParams {
                base_color: vello::peniko::Color::TRANSPARENT,
                width,
                height,
                antialiasing_method: vello::AaConfig::Area,
            },
        ) {
            log::error!("Vello failed to render GPUI scene: {error}");
            resources.invalidate_target();
            self.needs_redraw = true;
            return false;
        }
        let vello_encoded_at = Instant::now();

        let frame_view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder =
            resources
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("vello_blit_encoder"),
                });
        resources
            .vello_blitter
            .copy(&resources.device, &mut encoder, target_view, &frame_view);
        resources.queue.submit([encoder.finish()]);
        let submitted_at = Instant::now();
        frame.present();
        let presented_at = Instant::now();
        log::debug!(
            "Vello frame {}x{}: quads={}, paths={}, underlines={}, glyph_runs={}, glyphs={}, glyph_orders={}, glyph_clips={}, glyph_draws={}, images={}, svgs={}, surfaces={}; acquire={:?}, target={:?}, rebuild={:?}, vello={:?}, blit_submit={:?}, present={:?}, total={:?}",
            width,
            height,
            scene_build.stats.quads,
            scene_build.stats.paths,
            scene_build.stats.underlines,
            scene_build.stats.glyph_runs,
            scene_build.stats.glyphs,
            scene_build.stats.glyph_orders,
            scene_build.stats.glyph_clip_batches,
            scene_build.stats.glyph_draws,
            scene_build.stats.images,
            scene_build.stats.svgs,
            scene_build.stats.surfaces,
            acquired_at.duration_since(frame_started),
            target_ready_at.duration_since(acquired_at),
            scene_built_at.duration_since(target_ready_at),
            vello_encoded_at.duration_since(scene_built_at),
            submitted_at.duration_since(vello_encoded_at),
            presented_at.duration_since(submitted_at),
            presented_at.duration_since(frame_started),
        );
        true
    }

    pub fn unconfigure_surface(&mut self) {
        self.surface_configured = false;
        if let Some(resources) = self.resources.as_mut() {
            resources.invalidate_target();
        }
    }

    #[cfg(not(target_family = "wasm"))]
    pub fn replace_surface<W: HasWindowHandle>(
        &mut self,
        window: &W,
        config: WgpuSurfaceConfig,
        instance: &wgpu::Instance,
    ) -> anyhow::Result<()> {
        let window_handle = window
            .window_handle()
            .map_err(|error| anyhow::anyhow!("failed to get window handle: {error}"))?;
        let surface = create_surface(instance, window_handle.as_raw())?;
        self.surface_config.width = surface_dimension(config.size.width, self.max_texture_size);
        self.surface_config.height = surface_dimension(config.size.height, self.max_texture_size);
        self.surface_config.alpha_mode = if config.transparent {
            self.transparent_alpha_mode
        } else {
            self.opaque_alpha_mode
        };
        if let Some(present_mode) = config.preferred_present_mode {
            self.surface_config.present_mode = present_mode;
        }

        let resources = self
            .resources
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("cannot replace surface after renderer destruction"))?;
        surface.configure(&resources.device, &self.surface_config);
        resources.surface = surface;
        resources.invalidate_target();
        self.surface_configured = true;
        self.needs_redraw = true;
        Ok(())
    }

    pub fn destroy(&mut self) {
        self.resources.take();
    }

    pub fn device_lost(&self) -> bool {
        self.device_lost.load(std::sync::atomic::Ordering::SeqCst)
    }

    pub fn needs_redraw(&mut self) -> bool {
        std::mem::take(&mut self.needs_redraw)
    }

    #[cfg(not(target_family = "wasm"))]
    pub fn recover<W>(&mut self, window: &W) -> anyhow::Result<()>
    where
        W: HasWindowHandle + HasDisplayHandle + std::fmt::Debug + Send + Sync + Clone + 'static,
    {
        let gpu_context = self
            .context
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("renderer recovery requires a shared GPU context"))?
            .clone();
        let needs_new_context = gpu_context
            .borrow()
            .as_ref()
            .is_none_or(WgpuContext::device_lost);
        let window_handle = window
            .window_handle()
            .map_err(|error| anyhow::anyhow!("failed to get window handle: {error}"))?;

        let surface = if needs_new_context {
            log::warn!("GPU device lost, recreating Vello context");
            self.resources = None;
            *gpu_context.borrow_mut() = None;
            let instance = WgpuContext::instance(Box::new(window.clone()));
            let surface = create_surface(&instance, window_handle.as_raw())?;
            let context =
                WgpuContext::new_rejecting_software(instance, &surface, self.compositor_gpu)?;
            *gpu_context.borrow_mut() = Some(context);
            surface
        } else {
            let context_slot = gpu_context.borrow();
            let context = context_slot
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("shared GPU context disappeared during recovery"))?;
            create_surface(&context.instance, window_handle.as_raw())?
        };

        let config = WgpuSurfaceConfig {
            size: self.viewport_size(),
            transparent: self.surface_config.alpha_mode != self.opaque_alpha_mode,
            preferred_present_mode: Some(self.surface_config.present_mode),
        };
        let context_slot = gpu_context.borrow();
        let context = context_slot
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("shared GPU context was not recreated"))?;
        *self = Self::new_internal(
            Some(gpu_context.clone()),
            context,
            surface,
            config,
            self.compositor_gpu,
        )?;
        log::info!("Vello GPU recovery complete");
        Ok(())
    }
}

fn surface_dimension(value: DevicePixels, maximum: u32) -> u32 {
    u32::try_from(value.0).unwrap_or(1).clamp(1, maximum)
}

#[cfg(not(target_family = "wasm"))]
fn create_surface(
    instance: &wgpu::Instance,
    raw_window_handle: raw_window_handle::RawWindowHandle,
) -> anyhow::Result<wgpu::Surface<'static>> {
    // SAFETY: Platform windows keep their raw handles alive until the renderer is destroyed.
    unsafe {
        instance
            .create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                raw_display_handle: None,
                raw_window_handle,
            })
            .map_err(|error| anyhow::anyhow!("failed to create WGPU surface: {error}"))
    }
}
