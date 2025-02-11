use nalgebra::Vector2;
use num_traits::real::Real;

/// 粒子データ構造体
#[derive(Debug, Clone)]
pub struct Particle<R>
where
    R: Real,
{
    pos: Vector2<R>,
    vel: Vector2<R>,
    mass: R,
}

impl Default for Particle<f32> {
    fn default() -> Self {
        Particle {
            pos: Vector2::new(0.0, 0.0),
            vel: Vector2::new(0.0, 0.0),
            mass: 1.0,
        }
    }
}

/// グリッドセルデータ構造体
#[derive(Debug, Clone)]
pub struct Cell<R>
where
    R: Real,
{
    vel: Vector2<R>,
    mass: R,
}

impl Default for Cell<f32> {
    fn default() -> Self {
        Cell {
            vel: Vector2::new(0.0, 0.0),
            mass: 0.0,
        }
    }
}

/// シミュレーション初期化時に使う設定
pub struct SimConfig {
    num_of_particle: usize,
    grid_resolution: usize,
}

impl SimConfig {
    pub fn new(num_of_particle: usize, grid_resolution: usize) -> Self {
        SimConfig {
            num_of_particle,
            grid_resolution,
        }
    }

    fn num_of_grid(&self) -> usize {
        self.grid_resolution * self.grid_resolution
    }
}

/// シミュレーションの状態保持とステップ更新を行う
pub struct Sim<R>
where
    R: Real,
{
    particles: Vec<Particle<R>>,
    cells: Vec<Cell<R>>,
}

impl Sim<f32> {
    pub fn init(config: SimConfig) -> Self {
        Sim {
            particles: vec![Particle::default(); config.num_of_particle],
            cells: vec![Cell::default(); config.num_of_grid()],
        }
    }
}

impl<R> Sim<R>
where
    R: Real,
{
    pub fn simulate(&mut self, dt_sec: R) {
        self.reset_grid();
        self.p2g();
        self.calc_grid_vel();
        self.g2p();
    }

    fn reset_grid(&mut self) {}
    fn p2g(&mut self) {}
    fn calc_grid_vel(&mut self) {}
    fn g2p(&mut self) {}
}
