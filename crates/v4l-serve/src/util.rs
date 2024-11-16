pub fn open_device(index: usize) -> anyhow::Result<v4l::Device> {
    Ok(v4l::Device::new(index).inspect_err(|e| {
        tracing::error!("Failed to open device: {:?}", e);
    })?)
}
