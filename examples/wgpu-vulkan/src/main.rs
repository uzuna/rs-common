use wgpu_vulkan::run;

fn main() {
    pollster::block_on(run(None));
}
