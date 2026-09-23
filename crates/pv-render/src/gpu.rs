use crate::RenderError;

/// A GPU device and queue, with the adapter they came from.
pub struct Gpu {
    pub instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
}

impl Gpu {
    /// A device with no window, for export, the CLI, and tests. Falls back
    /// to a software adapter (lavapipe, WARP) when no GPU is present, so
    /// rendering works over SSH and in CI.
    pub fn headless() -> Result<Self, RenderError> {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        pollster::block_on(Self::with_instance(instance, None))
    }

    /// A device able to present to `surface`.
    pub async fn with_instance(
        instance: wgpu::Instance,
        surface: Option<&wgpu::Surface<'_>>,
    ) -> Result<Self, RenderError> {
        let mut options = wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: surface,
            ..Default::default()
        };
        let adapter = match instance.request_adapter(&options).await {
            Ok(a) => a,
            Err(_) => {
                options.force_fallback_adapter = true;
                instance
                    .request_adapter(&options)
                    .await
                    .map_err(|e| RenderError::NoAdapter(e.to_string()))?
            }
        };
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("piano-viz"),
                required_features: wgpu::Features::empty(),
                // Ask for what the hardware has, not the WebGPU minimum: big
                // exports need textures larger than 8192.
                required_limits: wgpu::Limits::defaults().using_resolution(adapter.limits()),
                ..Default::default()
            })
            .await
            .map_err(|e| RenderError::NoAdapter(e.to_string()))?;
        Ok(Self { instance, adapter, device, queue })
    }

    /// "NVIDIA GeForce RTX 3060 (Vulkan)", for logs and the HUD.
    pub fn describe(&self) -> String {
        let i = self.adapter.get_info();
        format!("{} ({:?})", i.name, i.backend)
    }

    pub fn max_texture_size(&self) -> u32 {
        self.device.limits().max_texture_dimension_2d
    }
}
