fn main() {
    for backend in [wgpu::Backends::DX12, wgpu::Backends::VULKAN] {
        let mut options = wgpu::InstanceDescriptor::new_without_display_handle();
        options.backends = backend;
        options.backend_options.dx12.shader_compiler = wgpu::Dx12Compiler::Fxc;
        let instance = wgpu::Instance::new(options);
        for adapter in pollster::block_on(instance.enumerate_adapters(backend)) {
            let info = adapter.get_info();
            println!(
                "{:?}: {} ({:?}), driver {} {}",
                info.backend, info.name, info.device_type, info.driver, info.driver_info
            );
        }
    }
}
