use nalgebra::{ComplexField, Matrix2, Vector2};
use num_traits::real::Real;

/// 粒子データ構造体
#[derive(Debug, Clone)]
pub struct Particle<R>
where
    R: Real,
{
    pub pos: Vector2<R>,
    pub vel: Vector2<R>,
    pub c: Matrix2<R>, // アフィン変形行列
    pub mass: R,
    pub volume: R,
}

impl Default for Particle<f32> {
    fn default() -> Self {
        Particle {
            pos: Vector2::new(0.0, 0.0),
            vel: Vector2::new(0.0, 0.0),
            c: Matrix2::zeros(),
            mass: 1.0,
            volume: 0.0,
        }
    }
}

/// グリッドセルデータ構造体
#[derive(Debug, Clone)]
pub struct Cell<R>
where
    R: Real,
{
    // セルの中心座標。グリッドを決めた時点基本的には固定される
    pos: Vector2<R>,
    v: Vector2<R>,
    mass: R,
}

impl<R> Default for Cell<R>
where
    R: Real,
{
    fn default() -> Self {
        Cell {
            pos: Vector2::new(R::zero(), R::zero()),
            v: Vector2::new(R::zero(), R::zero()),
            mass: R::zero(),
        }
    }
}

impl<R> Cell<R>
where
    R: Real,
{
    pub fn reset(&mut self) {
        self.v = Vector2::new(R::zero(), R::zero());
        self.mass = R::zero();
    }
}

/// Lamé parameters for stress-strain relationship
#[derive(Debug, Clone)]
pub struct ElasticConfig<R>
where
    R: Real,
{
    mu: R,
    lambda: R,
}

impl Default for ElasticConfig<f32> {
    fn default() -> Self {
        ElasticConfig {
            mu: 20.0,
            lambda: 10.0,
        }
    }
}

/// シミュレーション初期化時に使う設定
pub struct SimConfig<R>
where
    R: Real,
{
    // パーティクルの数
    num_of_particle: usize,
    // グリッド各次元の分割数
    grid_resolution: usize,
    // シミュレーション空間の幅
    space_width: R,
    // 重力
    gravity: Vector2<R>,
    // 弾性体の設定
    elastic_config: ElasticConfig<R>,
}

impl<R> SimConfig<R>
where
    R: Real,
{
    pub fn new(
        num_of_particle: usize,
        grid_resolution: usize,
        space_width: R,
        gravity: Vector2<R>,
        elastic_config: ElasticConfig<R>,
    ) -> Self {
        SimConfig {
            num_of_particle,
            grid_resolution,
            space_width,
            gravity,
            elastic_config,
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
    // 粒子データ
    particles: Vec<Particle<R>>,
    // グリッドデータ
    cells: Vec<Cell<R>>,
    // 変形勾配テンソル
    fs: Vec<Matrix2<R>>,
    // gridは正立方体として考える
    grid_resolution: usize,
    // セルの間隔。パーティクルはこの空間内に存在する必要がある
    cell_space: f32,
    outer: f32,
    gravity: Vector2<R>,
    elastic_config: ElasticConfig<R>,
}

impl Sim<f32> {
    pub fn new(config: SimConfig<f32>) -> Self {
        let (cell_space, cells) = Self::init_grid(&config);

        
        Sim {
            particles: vec![Particle::default(); config.num_of_particle],
            cells,
            fs: vec![Matrix2::identity(); config.num_of_particle],
            grid_resolution: config.grid_resolution,
            cell_space,
            outer: config.space_width * 0.495, // 0.5だとindexを超える場合があるのでマージンを撮っている,
            gravity: config.gravity,
            elastic_config: config.elastic_config,
        }
    }

    // グリッドの初期化
    fn init_grid(config: &SimConfig<f32>) -> (f32, Vec<Cell<f32>>) {
        let mut cells = vec![Cell::default(); config.num_of_grid()];
        let resolution = config.grid_resolution;
        // 最外周はGrid計算のためのマージンとして使うので空間の分割数は-2
        let active_res = resolution - 2;
        // セルの幅
        let cell_space = config.space_width / active_res as f32;
        let offset = (config.space_width + cell_space) * 0.5;
        let offset = Vector2::new(-offset, -offset);

        for x in 0..resolution {
            for y in 0..resolution {
                let pos = Vector2::new(x as f32, y as f32) * cell_space + offset;
                let idx = x * resolution + y;
                cells[idx].pos = pos;
            }
        }
        (cell_space, cells)
    }

    pub fn simulate(&mut self, dt_sec: f32) {
        self.reset_grid();
        self.p2g(dt_sec);
        self.update_volume();
        self.calc_grid_vel(dt_sec);
        self.g2p(dt_sec);
    }

    pub fn get_particles_mut(&mut self) -> &mut Vec<Particle<f32>> {
        &mut self.particles
    }

    // Particleの位置からグリッドセルのインデックスを計算する
    fn calc_cell_idx(&self, pos: Vector2<f32>) -> usize {
        // グリッド空間を正規化して解像度で割ることでセルのインデックスを計算
        let width = self.cell_space * self.grid_resolution as f32;
        let half_width = width / 2.0;
        let x = (pos.x + half_width) / width;
        let y = (pos.y + half_width) / width;
        let x = (x * self.grid_resolution as f32).floor() as usize;
        let y = (y * self.grid_resolution as f32).floor() as usize;
        x * self.grid_resolution + y
    }

    fn reset_grid(&mut self) {
        for cell in &mut self.cells {
            cell.reset();
        }
    }

    // Particle-Grid間の分配の重みを計算
    fn calc_weight(&self, cell_diff: Vector2<f32>) -> [Vector2<f32>; 3] {
        let cs = self.cell_space;
        let cell_diff_normalized = cell_diff / cs;
        fn calc_weight(diff: f32) -> (f32, f32, f32) {
            let x0 = 0.5 * (0.5 - diff).powi(2);
            let x1 = 0.75 - diff.powi(2);
            let x2 = 0.5 * (0.5 + diff).powi(2);
            (x0, x1, x2)
        }
        let (x0, x1, x2) = calc_weight(cell_diff_normalized.x);
        let (y0, y1, y2) = calc_weight(cell_diff_normalized.y);
        [
            Vector2::new(x0, y0),
            Vector2::new(x1, y1),
            Vector2::new(x2, y2),
        ]
    }

    // Particle to Grid
    fn p2g(&mut self, dt: f32) {
        // P2Gするときのセルへの分配重みを計算
        // 最も近いセルに75%を、その隣のセルに25%を分配し合計1.0にする
        // この計算方法がよく使われるらしい
        // 次元数が増えた場合も同様に計算できる
        // この求め方をしている限り、Particleは一番外側のセルに存在をすることが許されない
        for i in 0..self.particles.len() {
            let p = &self.particles[i];

            // MPM Course, page 13 page
            let f = self.fs[i];
            let j = f.determinant(); // 回転や並進では1で、膨らむ変形は1、縮む変形は1より小さい値になる
            let volume = p.volume * j; // 体積変化
            println!("momentum: {f} {j} {}", p.volume);

            // useful matrices for Neo-Hookean model
            let f_t = f.transpose();
            let f_inv_t = f.try_inverse().unwrap().transpose();
            let f_minus_f_inv_t = f - f_inv_t;

            // MPM course equation 48
            // 軸ごとの変形応力計算を合成して応力テンソルを計算
            let p_term_0 = self.elastic_config.mu * f_minus_f_inv_t;
            let p_term_1 = self.elastic_config.lambda * j.exp() * f_inv_t;
            let pw = p_term_0 + p_term_1;

            // コーシ応力テンソル(cauchy stress) = (1 / det(F)) * P * F_T
            // 物体を分割する任意の面上で、一方が他方に及ぼす作用は面の力と結合力のシステムと等価であると規定する
            // equation 38, MPM course
            let stress = (1.0 / j) * pw * &f_t;

            let cell_idx = self.calc_cell_idx(p.pos);
            let cell = &self.cells[cell_idx];

            // セルとの距離に応じてグリッドに寄与する値を加算
            let cell_diff = p.pos - cell.pos;
            let q = p.c * cell_diff;
            let weights = self.calc_weight(cell_diff);

            // (M_p)^-1 = 4, see APIC paper and MPM course page 42
            // this term is used in MLS-MPM paper eq. 16. with quadratic weights, Mp = (1/4) * (delta_x)^2.
            let mp_r = 1.0 / (cell_diff.x.norm1().powi(2) * 0.25);
            let eq_16_term_0 = -volume * mp_r * stress * dt;

            for gx in 0..3 {
                let x_offset = gx as isize - 1;
                for gy in 0..3 {
                    // calc cell index
                    let y_offset = (gy as isize - 1) * self.grid_resolution as isize;
                    let idx = (cell_idx as isize + x_offset + y_offset) as usize;
                    let w = weights[gx].x * weights[gy].y;

                    // MPM course, equation 172
                    let mass_contrib = p.mass * w;

                    // 質量の加算
                    self.cells[idx].mass += mass_contrib;

                    // 力として加算
                    self.cells[idx].v += (p.vel + q) * mass_contrib;

                    // momentumの更新
                    // let momentum = eq_16_term_0 * w * cell_diff;
                    // self.cells[idx].v += momentum;
                }
            }
        }
    }

    // per-particle volume estimate has now been computed
    // 密度を計算して体積の・ようなものを計算している?
    // P2GでGridを更新した後に行う必要がある
    fn update_volume(&mut self) {
        for i in 0..self.particles.len() {
            let p = &self.particles[i];
            let cell_idx = self.calc_cell_idx(p.pos);
            let cell = &self.cells[cell_idx];

            let cell_diff = p.pos - cell.pos;
            let weights = self.calc_weight(cell_diff);

            let mut density = 0.0;

            for gx in 0..3 {
                let x_offset = gx as isize - 1;
                for gy in 0..3 {
                    // calc cell index
                    let y_offset = (gy as isize - 1) * self.grid_resolution as isize;
                    let idx = (cell_idx as isize + x_offset + y_offset) as usize;
                    let w = weights[gx].x * weights[gy].y;

                    density += self.cells[idx].mass * w;
                }
            }

            self.particles[i].volume = p.mass / density;
        }
    }

    // 各グリッドのベクトルを更新
    fn calc_grid_vel(&mut self, dt: f32) {
        for (index, cell) in self.cells.iter_mut().enumerate() {
            if cell.mass > 0.0 {
                // 速度に変換
                cell.v /= cell.mass;

                // 重力更新
                cell.v += self.gravity * dt;

                // 境界条件(BC: Boundary Conditions)を考慮
                // 画面端で速度を0にする
                let x = index / self.grid_resolution;
                let y = index % self.grid_resolution;
                if x < 2 || x > self.grid_resolution - 2 {
                    cell.v.x = 0.0;
                }
                if y < 2 || y > self.grid_resolution - 2 {
                    cell.v.y = 0.0;
                }
            }
        }
    }

    // グリッドの情報を元にParticleの速度を更新し、時間ステップを更新
    fn g2p(&mut self, dt: f32) {
        for i in 0..self.particles.len() {
            let p = &self.particles[i];

            // セルの中心座標を計算
            let cell_idx = self.calc_cell_idx(p.pos);
            let cell = &self.cells[cell_idx];
            let cell_diff = p.pos - cell.pos;
            let weights = self.calc_weight(cell_diff);

            // APICの計算
            // constructing affine per-particle momentum matrix from APIC / MLS-MPM.
            // see APIC paper (https://web.archive.org/web/20190427165435/https://www.math.ucla.edu/~jteran/papers/JSSTS15.pdf), page 6
            // below equation 11 for clarification. this is calculating C = B * (D^-1) for APIC equation 8,
            // where B is calculated in the inner loop at (D^-1) = 4 is a constant when using quadratic interpolation functions
            let mut b = Matrix2::zeros();

            // パーティクルの速度を初期化
            // Gridから反映するため、付近のセルの速度に引きづられて速度が変わる(Gridの大きさにおおじた粘性体のように振る舞う)
            let p = &mut self.particles[i];
            p.vel = Vector2::new(0.0, 0.0);
            for gx in 0..3 {
                let y_offset = (gx as isize - 1) * self.grid_resolution as isize;
                for gy in 0..3 {
                    let x_offset = gy as isize - 1;
                    let idx = (cell_idx as isize + x_offset + y_offset) as usize;
                    let w = weights[gx].x * weights[gy].y;

                    // セルの速度を取得
                    let dist =
                        self.cells[idx].pos - p.pos + (Vector2::new(0.5, 0.5) * self.cell_space);
                    let w_vel = self.cells[idx].v * w;

                    // APIC paper equation 10, constructing inner term for B
                    b += Matrix2::new(
                        dist.x * w_vel.x,
                        dist.x * w_vel.y,
                        dist.y * w_vel.x,
                        dist.y * w_vel.y,
                    );
                    println!("b {gx} {gy} {b:?} {:?} {:?}", self.cells[idx].pos, p.pos);

                    p.vel += w_vel;
                }
            }

            // 計算しているがまだこの値は使っていない
            println!("b: {}", b);
            p.c = b * 4.0;

            // 位置反映
            p.pos += p.vel * dt;

            // 位置をGridの空間内に制限、反射は考えずに境界を超えたら速度を0にする
            let outer = self.outer;
            if p.pos.x <= -outer || p.pos.x >= outer {
                p.pos.x = p.pos.x.clamp(-outer, outer);
                p.vel.x *= -1.0;
            }
            if p.pos.y <= -outer || p.pos.y >= outer {
                p.pos.y = p.pos.y.clamp(-outer, outer);
                p.vel.y *= -1.0;
            }

            // deformation gradient update - MPM course, equation 181
            self.fs[i] = (Matrix2::identity() + p.c * dt) * self.fs[i];
        }
    }
}

#[cfg(test)]
mod tests {
    use num_traits::Zero;

    use super::*;

    impl<R> Sim<R>
    where
        R: Real + Zero + std::fmt::Debug + std::ops::AddAssign + 'static,
    {
        // インデックスからXY座標を計算
        #[allow(dead_code)]
        fn index_to_xy(&self, idx: usize) -> (usize, usize) {
            let x = idx % self.grid_resolution;
            let y = idx / self.grid_resolution;
            (x, y)
        }

        // セルの質量の合計を計算
        fn sum_mass(&self) -> R {
            self.cells
                .iter()
                .fold(R::zero(), |acc, cell| acc + cell.mass)
        }
    }

    // 重みが常に1.0であることを確認
    // 重みが減るというのはシミュレーション空間からエネルギーが失われるということ
    #[test]
    fn test_weight() {
        // グリッドの解像度と幅を変えてテスト
        let grids = [4, 8, 16, 32, 64, 512];
        let widths = [1.0, 2.0, 5.0, 7.0, 10.0];
        // 点がどの位置でも重みの合計が1.0であることを確認
        let norm_offsets = [(-0.5, 0.0), (-0.1, 0.3), (0.1, 0.5)];
        for resolution in grids {
            for width in widths {
                let sim = Sim::new(SimConfig::new(
                    1,
                    resolution,
                    width,
                    Vector2::new(0.0, 0.0),
                    ElasticConfig::default(),
                ));
                for offset in norm_offsets.iter() {
                    let pos = Vector2::new(offset.0 * sim.cell_space, offset.1 * sim.cell_space);
                    let diff = sim.calc_weight(pos);
                    for d in diff.iter() {
                        assert!(d.x >= 0.0, "expect x > 0.0 but got {:?}", d.x);
                        assert!(d.y >= 0.0, "expect y > 0.0 but got {:?}", d.y);
                    }
                    let sum = diff.iter().fold(Vector2::zero(), |acc, v| acc + v);
                    assert_eq!(
                        sum,
                        Vector2::new(1.0, 1.0),
                        "expect 1.0 but got {:?} in resolution={resolution},width={width}, diff={diff:?}",
                        sum
                    );
                }
            }
        }
    }

    /// シミュレーション空間内に粒子がとどまることを確認する
    #[test]
    fn test_index_boundry() {
        let gs = [
            Vector2::new(10.0, 0.0),
            Vector2::new(-10.0, 0.0),
            Vector2::new(0.0, 10.0),
            Vector2::new(0.0, -10.0),
        ];
        let dt = 0.1;
        for g in gs.into_iter() {
            let mut sim = Sim::new(SimConfig::new(1, 128, 2.0, g, ElasticConfig::default()));
            for _ in 0..100 {
                sim.simulate(dt);
                let sum_mass = sim.sum_mass();
                approx::assert_relative_eq!(sum_mass, 1.0, epsilon = 0.01);
            }
        }
    }
}
