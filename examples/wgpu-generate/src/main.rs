use wgpu_generate::run;

fn main() {
    pollster::block_on(run(None));
}
