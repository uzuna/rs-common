use vulkan_demo::run;

fn main() {
    pollster::block_on(run(None));
}
