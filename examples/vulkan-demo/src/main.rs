use vulkan_demo::{env::AppEnv, run};

fn main() {
    let env = AppEnv::from_env();
    pollster::block_on(run(env, None));
}
