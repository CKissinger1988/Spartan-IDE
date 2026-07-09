use std::sync::Arc;
use winit::window::Window;

/// Owns the wgpu instance/adapter/device/queue/surface for one window.
/// Promoted from `spikes/render-spike/src/gpu.rs` (§39.1, §47.9) verbatim --
/// this bootstrap sequence is already proven on this project's real Intel
/// UHD 620 / Vulkan / Windows-GNU setup and needed no changes to become
/// real product code. Deliberately still minimal -- no multi-window
/// support, no render-graph abstraction -- since those aren't required by
/// this increment's scope (see the crate README).
///
/// A real optimization attempt was tried and reverted here (§75.9): trying
/// a `Backends::VULKAN`-only `Instance::new()` first (skipping DX12/DX11/GL
/// probing) before falling back to `Backends::all()`, on the hypothesis
/// that probing every backend's loader was the reason `Instance::new()`
/// measured as the single largest cold-open cost (~221-306ms). Real,
/// repeated instrumented runs showed no measurable improvement (the
/// Vulkan-only path's timings, 261-334ms, fully overlap the original
/// `Backends::all()` range) -- so the ~220-300ms is apparently intrinsic to
/// Vulkan loader/ICD initialization itself on this hardware, not to
/// probing other backends. Reverted to keep the code simple rather than
/// keep unproven complexity around.
pub struct GpuState {
    pub surface: wgpu::Surface<'static>,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub config: wgpu::SurfaceConfiguration,
    pub size: winit::dpi::PhysicalSize<u32>,
    pub adapter_info: wgpu::AdapterInfo,
}

/// Real, pure detection of a software or virtualized GPU adapter (§75.50,
/// user-requested "virtual GPU support") -- `wgpu::DeviceType::Cpu`
/// (software rasterizers like the `llvmpipe` adapter this whole project's
/// own Linux-container sessions have run on throughout its history) and
/// `wgpu::DeviceType::VirtualGpu` (a real, distinct wgpu-defined category
/// for "Virtual / Hosted" adapters -- exactly what a VM's `virtio-gpu`/
/// SR-IOV/vGPU passthrough exposes to the guest) both count. `IntegratedGpu`
/// and `DiscreteGpu` are real hardware, even if the hardware itself is
/// modest.
pub fn is_software_or_virtual(info: &wgpu::AdapterInfo) -> bool {
    matches!(
        info.device_type,
        wgpu::DeviceType::Cpu | wgpu::DeviceType::VirtualGpu
    )
}

/// Real, pure parsing of a `--gpu-backend:<name>` override (§75.50,
/// user-requested QA/testing support for forcing a specific backend --
/// e.g. to test the renderer against a different backend than whatever
/// `wgpu::Backends::all()` would pick by default, or to work around a
/// broken virtualized backend by forcing a working one). Case-insensitive;
/// `None` for an unrecognized name, so the caller can decide how to warn
/// rather than this function silently guessing.
pub fn parse_backend_override(name: &str) -> Option<wgpu::Backends> {
    match name.to_ascii_lowercase().as_str() {
        "vulkan" => Some(wgpu::Backends::VULKAN),
        "gl" | "opengl" => Some(wgpu::Backends::GL),
        "dx12" | "directx12" => Some(wgpu::Backends::DX12),
        "metal" => Some(wgpu::Backends::METAL),
        "browser-webgpu" | "webgpu" => Some(wgpu::Backends::BROWSER_WEBGPU),
        _ => None,
    }
}

impl GpuState {
    /// `backend_override` restricts the real `wgpu::Instance` to a single
    /// backend family (§75.50) instead of the default `Backends::all()`
    /// probe-everything behavior -- `None` preserves the exact prior
    /// behavior for every existing call site.
    pub async fn new(window: Arc<Window>, backend_override: Option<wgpu::Backends>) -> Self {
        let size = window.inner_size();

        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: backend_override.unwrap_or(wgpu::Backends::all()),
            ..Default::default()
        });

        let surface = instance
            .create_surface(window.clone())
            .expect("create_surface should succeed on a real OS window");

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::default(),
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .expect(
                "request_adapter returned None -- no usable GPU backend reachable from this \
                 process (confirmed reachable via a standalone probe and spikes/render-spike \
                 before this crate was written; if this fails now, something about the \
                 environment changed)",
            );

        let adapter_info = adapter.get_info();

        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("spartan-editor-core device"),
                    required_features: wgpu::Features::empty(),
                    required_limits: wgpu::Limits::default(),
                },
                None,
            )
            .await
            .expect("request_device failed on an adapter that was just successfully obtained");

        let surface_caps = surface.get_capabilities(&adapter);
        let surface_format = surface_caps
            .formats
            .iter()
            .find(|f| f.is_srgb())
            .copied()
            .unwrap_or(surface_caps.formats[0]);

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: surface_caps.present_modes[0],
            alpha_mode: surface_caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);

        Self {
            surface,
            device,
            queue,
            config,
            size,
            adapter_info,
        }
    }

    pub fn resize(&mut self, new_size: winit::dpi::PhysicalSize<u32>) {
        if new_size.width > 0 && new_size.height > 0 {
            self.size = new_size;
            self.config.width = new_size.width;
            self.config.height = new_size.height;
            self.surface.configure(&self.device, &self.config);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn adapter_info(device_type: wgpu::DeviceType) -> wgpu::AdapterInfo {
        wgpu::AdapterInfo {
            name: "test adapter".to_string(),
            vendor: 0,
            device: 0,
            device_type,
            driver: String::new(),
            driver_info: String::new(),
            backend: wgpu::Backend::Vulkan,
        }
    }

    #[test]
    fn cpu_adapters_are_real_software_rendering() {
        assert!(is_software_or_virtual(&adapter_info(wgpu::DeviceType::Cpu)));
    }

    #[test]
    fn virtual_gpu_adapters_are_flagged_too() {
        assert!(is_software_or_virtual(&adapter_info(
            wgpu::DeviceType::VirtualGpu
        )));
    }

    #[test]
    fn real_hardware_adapters_are_not_flagged() {
        assert!(!is_software_or_virtual(&adapter_info(
            wgpu::DeviceType::IntegratedGpu
        )));
        assert!(!is_software_or_virtual(&adapter_info(
            wgpu::DeviceType::DiscreteGpu
        )));
    }

    #[test]
    fn backend_override_parses_every_real_supported_name_case_insensitively() {
        assert_eq!(
            parse_backend_override("vulkan"),
            Some(wgpu::Backends::VULKAN)
        );
        assert_eq!(
            parse_backend_override("VULKAN"),
            Some(wgpu::Backends::VULKAN)
        );
        assert_eq!(parse_backend_override("gl"), Some(wgpu::Backends::GL));
        assert_eq!(parse_backend_override("opengl"), Some(wgpu::Backends::GL));
        assert_eq!(parse_backend_override("dx12"), Some(wgpu::Backends::DX12));
        assert_eq!(parse_backend_override("metal"), Some(wgpu::Backends::METAL));
    }

    #[test]
    fn backend_override_rejects_an_unrecognized_name() {
        assert_eq!(parse_backend_override("not-a-real-backend"), None);
    }
}
