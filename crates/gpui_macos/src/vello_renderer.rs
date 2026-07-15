use anyhow::{Context as _, Result};
use cocoa::{
    base::{NO, YES, id},
    foundation::NSSize,
    quartzcore::AutoresizingMask,
};
use foreign_types::ForeignType;
use gpui::{DevicePixels, GpuSpecs, Scene, Size};
use gpui_wgpu::{GpuContext, WgpuRenderer, WgpuSurfaceConfig};
use metal::{CAMetalLayer, MTLPixelFormat, MetalLayerRef};
use objc::{msg_send, sel, sel_impl};
use raw_window_handle::{
    AppKitWindowHandle, DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle,
    RawWindowHandle, WindowHandle,
};
use std::{ffi::c_void, fmt, ptr::NonNull};

pub(crate) type Context = GpuContext;

#[derive(Clone, Copy)]
struct RawWindow {
    view: NonNull<c_void>,
}

impl fmt::Debug for RawWindow {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RawWindow")
            .field("view", &self.view)
            .finish()
    }
}

unsafe impl Send for RawWindow {}
unsafe impl Sync for RawWindow {}

impl HasWindowHandle for RawWindow {
    fn window_handle(&self) -> std::result::Result<WindowHandle<'_>, HandleError> {
        // SAFETY: The AppKit view remains alive until MacWindow drops the renderer.
        unsafe {
            Ok(WindowHandle::borrow_raw(RawWindowHandle::AppKit(
                AppKitWindowHandle::new(self.view),
            )))
        }
    }
}

impl HasDisplayHandle for RawWindow {
    fn display_handle(&self) -> std::result::Result<DisplayHandle<'_>, HandleError> {
        Ok(DisplayHandle::appkit())
    }
}

pub(crate) struct Renderer {
    layer: metal::MetalLayer,
    raw_window: RawWindow,
    renderer: WgpuRenderer,
    needs_redraw: bool,
}

pub(crate) unsafe fn new_renderer(
    context: Context,
    native_view: *mut c_void,
    size: Size<DevicePixels>,
    transparent: bool,
) -> Result<Renderer> {
    let view = NonNull::new(native_view).context("macOS renderer received a null NSView")?;
    let layer = metal::MetalLayer::new();
    layer.set_pixel_format(MTLPixelFormat::BGRA8Unorm);
    layer.set_opaque(!transparent);
    layer.set_maximum_drawable_count(3);

    unsafe {
        let _: () = msg_send![&*layer, setAllowsNextDrawableTimeout: NO];
        let _: () = msg_send![&*layer, setNeedsDisplayOnBoundsChange: YES];
        let _: () = msg_send![
            &*layer,
            setAutoresizingMask: AutoresizingMask::WIDTH_SIZABLE | AutoresizingMask::HEIGHT_SIZABLE
        ];
        let _: () = msg_send![
            &*layer,
            setDrawableSize: NSSize {
                width: size.width.0.max(1) as f64,
                height: size.height.0.max(1) as f64,
            }
        ];

        let native_view = native_view as id;
        let _: () = msg_send![native_view, setLayer: &*layer];
        let _: () = msg_send![native_view, setWantsLayer: YES];
    }

    let raw_window = RawWindow { view };
    let renderer = WgpuRenderer::new(
        context,
        &raw_window,
        WgpuSurfaceConfig {
            size,
            transparent,
            preferred_present_mode: Some(gpui_wgpu::wgpu::PresentMode::Fifo),
        },
        None,
    )?;

    Ok(Renderer {
        layer,
        raw_window,
        renderer,
        needs_redraw: false,
    })
}

impl Renderer {
    pub fn layer(&self) -> Option<&MetalLayerRef> {
        Some(self.layer.as_ref())
    }

    pub fn layer_ptr(&self) -> *mut CAMetalLayer {
        self.layer.as_ptr()
    }

    pub fn gpu_specs(&self) -> GpuSpecs {
        self.renderer.gpu_specs()
    }

    pub fn set_presents_with_transaction(&mut self, presents_with_transaction: bool) {
        self.layer
            .set_presents_with_transaction(presents_with_transaction);
    }

    pub fn update_drawable_size(&mut self, size: Size<DevicePixels>) {
        unsafe {
            let _: () = msg_send![
                self.layer.as_ref(),
                setDrawableSize: NSSize {
                    width: size.width.0.max(1) as f64,
                    height: size.height.0.max(1) as f64,
                }
            ];
        }
        self.renderer.update_drawable_size(size);
        self.needs_redraw = true;
    }

    pub fn update_transparency(&mut self, transparent: bool) {
        self.layer.set_opaque(!transparent);
        self.renderer.update_transparency(transparent);
        self.needs_redraw = true;
    }

    pub fn destroy(&mut self) {
        self.renderer.destroy();
    }

    pub fn draw(&mut self, scene: &Scene) {
        if self.renderer.device_lost()
            && let Err(error) = self.renderer.recover(&self.raw_window)
        {
            log::error!("failed to recover macOS Vello renderer: {error:#}");
            self.needs_redraw = true;
            return;
        }

        if !self.renderer.draw(scene) {
            self.needs_redraw = true;
        }
        self.needs_redraw |= self.renderer.needs_redraw();
    }

    pub fn needs_redraw(&mut self) -> bool {
        std::mem::take(&mut self.needs_redraw)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn render_to_image(&mut self, scene: &Scene) -> Result<image::RgbaImage> {
        use gpui::PlatformHeadlessRenderer as _;

        let mut renderer = gpui_wgpu::VelloHeadlessRenderer::new()?;
        renderer.render_scene_to_image(scene, self.renderer.viewport_size())
    }
}
