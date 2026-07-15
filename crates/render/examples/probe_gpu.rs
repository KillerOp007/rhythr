//! Confirms a wgpu device can be acquired headlessly in this environment.
fn main() {
    let instance = wgpu::Instance::default();
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        force_fallback_adapter: false,
        compatible_surface: None,
        ..Default::default()
    }));
    match adapter {
        Ok(a) => {
            let info = a.get_info();
            println!(
                "ADAPTER OK: {} | backend={:?} | type={:?} | driver={}",
                info.name, info.backend, info.device_type, info.driver
            );
            let dev = pollster::block_on(a.request_device(&wgpu::DeviceDescriptor::default()));
            match dev {
                Ok(_) => println!("DEVICE OK"),
                Err(e) => println!("DEVICE FAIL: {e}"),
            }
        }
        Err(e) => println!("NO ADAPTER: {e}"),
    }
}
